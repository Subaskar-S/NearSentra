"""Ingestion tests that exercise the REAL block walk (``Monitor.poll_once`` + an event source) against
a scripted fake chain: duplicates, partial failures, disconnection and recovery, restarts, lag."""

import asyncio
import json

import httpx
import pytest
from fastapi.testclient import TestClient

from checkpoint import Checkpoint
from event_source import BlockEvents, NeardataEventSource
from main import create_app
from models import SentinelConfig, VaultEvent
from monitor import NS, Monitor
from pauser import PauseResult
from rpc import RpcError

VAULT = "vault.sandbox"
AGENT = "a.sandbox"


def log_line(agent=AGENT, decision="approved", event="transfer_requested"):
    return "EVENT_JSON:" + json.dumps(
        {
            "standard": "nearsentra",
            "version": "1.0.0",
            "event": event,
            "data": {"agent_id": agent, "recipient": "r.sandbox", "amount": "1", "decision": decision},
        }
    )


class FakeChainRpc:
    """Just enough of the NEAR JSON-RPC for the block walk. A block is a list of shards; a shard is a
    list of ``(tx_hash, receiver_id, [log lines])``."""

    def __init__(self, head=10):
        self.head = head
        self.blocks: dict[int, list[list[tuple[str, str, list[str]]]]] = {}
        self.fail_tx_once: set[str] = set()
        self.fail_block_once: set[int] = set()
        self.down = False
        self.tx_calls = 0

    def add_block(self, height, *shards):
        self.blocks[height] = [list(s) for s in shards]

    def _check(self):
        if self.down:
            raise RpcError("node down")

    async def final_block(self):
        self._check()
        return {"header": {"height": self.head, "timestamp": str(self.head * NS)}, "chunks": []}

    async def block_at(self, height):
        self._check()
        if height in self.fail_block_once:
            self.fail_block_once.discard(height)
            raise RpcError(f"block {height}: timeout")
        if height not in self.blocks:
            return None  # a skipped height
        shards = self.blocks[height]
        return {
            "header": {"height": height, "hash": f"hash-{height}", "timestamp": str(height * NS)},
            "chunks": [{"chunk_hash": f"chunk-{height}-{i}", "height_included": height} for i in range(len(shards))],
        }

    async def chunk(self, chunk_hash):
        self._check()
        _, height, shard = chunk_hash.split("-")
        txs = self.blocks[int(height)][int(shard)]
        return {"transactions": [{"hash": h, "signer_id": AGENT, "receiver_id": to} for h, to, _ in txs]}

    async def tx_status(self, tx_hash, signer):
        self._check()
        self.tx_calls += 1
        if tx_hash in self.fail_tx_once:
            self.fail_tx_once.discard(tx_hash)
            raise RpcError("tx: timeout")
        for shards in self.blocks.values():
            for shard in shards:
                for h, _, logs in shard:
                    if h == tx_hash:
                        return {"receipts_outcome": [{"id": f"receipt-{tx_hash}", "outcome": {"logs": logs}}]}
        raise RpcError("tx: unknown transaction")

    async def view_agent_policy(self, vault, agent):
        return {"agent_id": agent}

    async def aclose(self):
        pass


class FakePauser:
    available = True

    def __init__(self):
        self.paused: list[str] = []

    async def pause_agent(self, agent_id):
        self.paused.append(agent_id)
        return PauseResult(ok=True)


def make_monitor(rpc, pauser=None, checkpoint=None, clock=None, **cfg):
    cfg.setdefault("velocity_threshold", 4)
    config = SentinelConfig(
        velocity_window_secs=60,
        vault_account_id=VAULT,
        poll_interval_secs=0.005,
        reconnect_interval_secs=0.005,
        **cfg,
    )
    kwargs = {"clock": clock} if clock else {}
    return Monitor(config, rpc, pauser or FakePauser(), checkpoint=checkpoint, **kwargs)


def requests_seen(mon, agent=AGENT):
    return mon.detector.request_count(agent)


# ── the real block walk ──────────────────────────────────────────────────────


async def test_poll_once_reads_vault_events_and_ignores_everything_else():
    rpc = FakeChainRpc(head=10)
    rpc.add_block(
        11,
        [("tx-a", VAULT, [log_line(), "plain log, not an event"])],
        [("tx-foreign", "someone-else.sandbox", [log_line()])],  # another contract's transaction
    )
    # height 12 is skipped by the chain (no block)
    rpc.add_block(13, [("tx-b", VAULT, [log_line(decision="rejected")])], [])
    rpc.head = 13
    mon = make_monitor(rpc)
    mon.last_height = 10

    assert await mon.poll_once() == 2
    assert requests_seen(mon) == 2
    assert mon.last_height == 13 and mon.lag_blocks == 0


async def test_a_failure_in_the_middle_of_a_block_never_duplicates_events():
    # 2 real requests, threshold 2 (fires above 2). Before the fix the retry counted 3 and paused.
    rpc = FakeChainRpc(head=10)
    rpc.add_block(11, [("tx-a", VAULT, [log_line()])], [("tx-b", VAULT, [log_line()])])
    rpc.head = 11
    rpc.fail_tx_once = {"tx-b"}
    pauser = FakePauser()
    mon = make_monitor(rpc, pauser, velocity_threshold=2)
    mon.last_height = 10

    with pytest.raises(RpcError):
        await mon.poll_once()
    assert requests_seen(mon) == 0, "half a block must never be applied"
    assert mon.last_height == 10

    assert await mon.poll_once() == 2
    assert requests_seen(mon) == 2
    assert pauser.paused == []


async def test_blocks_are_applied_in_order_and_a_retry_resumes_at_the_failed_block():
    rpc = FakeChainRpc(head=10)
    for h in range(11, 17):
        rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line()])])
    rpc.head = 16
    rpc.fail_block_once = {13}
    mon = make_monitor(rpc, fetch_concurrency=4)
    mon.last_height = 10

    with pytest.raises(RpcError):
        await mon.poll_once()
    assert mon.last_height == 12, "blocks 11 and 12 were applied, 13 failed, later blocks were not applied"
    assert requests_seen(mon) == 2

    assert await mon.poll_once() == 4
    assert requests_seen(mon) == 6, "every event exactly once"
    assert mon.last_height == 16


async def test_duplicate_delivery_of_the_same_log_line_is_ignored():
    mon = make_monitor(FakeChainRpc())
    ev = VaultEvent(event="transfer_requested", agent_id=AGENT, decision="approved", timestamp_ns=NS, block_height=5, uid="r1:0")
    await mon.handle_event(ev)
    await mon.handle_event(ev)
    await mon.handle_event(ev.model_copy(update={"uid": "r1:1"}))  # a different log line of the same receipt
    assert requests_seen(mon) == 2
    assert mon.duplicates_dropped == 1
    # Events without an identity (hand-built) are never dropped.
    anon = ev.model_copy(update={"uid": None})
    await mon.handle_event(anon)
    await mon.handle_event(anon)
    assert requests_seen(mon) == 4


# ── disconnection and recovery ───────────────────────────────────────────────


class StepClock:
    """Every reading advances time by one second, so interruption detection depends on how many polls
    failed, never on real time."""

    def __init__(self):
        self.t = 0.0

    def __call__(self):
        self.t += 1.0
        return self.t


async def wait_for(predicate, timeout=5.0):
    deadline = asyncio.get_running_loop().time() + timeout
    while not predicate():
        if asyncio.get_running_loop().time() > deadline:
            raise AssertionError("condition not reached in time")
        await asyncio.sleep(0.005)


async def test_disconnection_is_flagged_and_events_missed_meanwhile_are_alert_only():
    rpc = FakeChainRpc(head=10)
    pauser = FakePauser()
    mon = make_monitor(rpc, pauser, clock=StepClock(), stream_timeout_secs=3)
    stop = asyncio.Event()
    runner = asyncio.create_task(mon.run(stop))
    try:
        await wait_for(lambda: mon.last_height == 10)

        # Healthy: a few requests are read normally.
        for h in (11, 12):
            rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line()])])
        rpc.head = 12
        await wait_for(lambda: mon.last_height == 12)
        assert requests_seen(mon) == 2 and not mon.interrupted

        # The node goes away. The chain keeps producing a flood (well above the threshold of 4).
        rpc.down = True
        await wait_for(lambda: mon.interrupted)
        for h in range(13, 21):
            rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line(), log_line()])])
        rpc.head = 20
        assert mon.last_height == 12, "nothing is read while the source is down"

        # It comes back: the flood is read, recorded, but never acted on (it may be long over).
        rpc.down = False
        await wait_for(lambda: mon.last_height == 20 and not mon.interrupted)
        assert requests_seen(mon) == 2 + 16
        assert pauser.paused == [], "catch-up events must not pause anything"
        assert any(a.action == "logged_only" and a.pattern == "velocity_exceeded" for a in mon.alerts)

        # Fresh activity after recovery is acted on again. (Block time is height seconds here, so jump
        # past the detector's 60 s re-arm window left by the catch-up alert; real outages are longer.)
        for h in range(200, 206):
            rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line(), log_line()])])
        rpc.head = 205
        await wait_for(lambda: mon.last_height == 205)
        assert pauser.paused == [AGENT]
    finally:
        stop.set()
        await runner


# ── restarts: checkpoint, gaps ───────────────────────────────────────────────


async def test_restart_with_a_checkpoint_resumes_and_reads_the_blocks_missed_while_down(tmp_path):
    path = str(tmp_path / "cp.json")
    rpc = FakeChainRpc(head=10)
    for h in range(11, 15):
        rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line()])])
    rpc.head = 14
    first = make_monitor(rpc, checkpoint=Checkpoint(path, VAULT))
    first.last_height = 10
    await first.poll_once()
    assert Checkpoint(path, VAULT).load() == (14, "hash-14")

    # The Sentinel is down while the chain produces blocks 15..20, a flood among them.
    for h in range(15, 21):
        rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line(), log_line()])])
    rpc.head = 20
    pauser = FakePauser()
    second = make_monitor(rpc, pauser, checkpoint=Checkpoint(path, VAULT))
    await second.startup()
    assert second.last_height == 14, "resumed where the first run stopped"
    assert await second.poll_once() == 12
    assert requests_seen(second) == 12, "every missed event was read"
    assert pauser.paused == [], "the gap is read as catch-up: recorded, not acted on"


async def test_restart_without_a_checkpoint_skips_to_the_head_and_says_so(caplog):
    rpc = FakeChainRpc(head=10)
    for h in range(11, 21):
        rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line()])])
    rpc.head = 20
    mon = make_monitor(rpc)
    with caplog.at_level("WARNING"):
        await mon.startup()
    assert mon.last_height == 20
    assert await mon.poll_once() == 0, "without a checkpoint the downtime events are not recovered"
    assert any("NOT recovered" in r.message for r in caplog.records)


async def test_a_checkpoint_from_another_chain_is_ignored(tmp_path):
    path = str(tmp_path / "cp.json")
    Checkpoint(path, VAULT).save(14, "hash-of-some-other-chain")
    rpc = FakeChainRpc(head=20)
    for h in range(11, 21):
        rpc.add_block(h, [(f"tx-{h}", VAULT, [log_line()])])
    mon = make_monitor(rpc, checkpoint=Checkpoint(path, VAULT))
    await mon.startup()
    assert mon.last_height == 20, "a hash mismatch means a different chain: start from the head"


@pytest.mark.parametrize("content", ["not json", "[]", '{"vault": "other.sandbox", "height": 3, "block_hash": "x"}',
                                      '{"vault": "vault.sandbox", "height": -1, "block_hash": "x"}',
                                      '{"vault": "vault.sandbox", "height": 3}'])
def test_unusable_checkpoints_are_ignored(tmp_path, content):
    path = tmp_path / "cp.json"
    path.write_text(content)
    assert Checkpoint(str(path), VAULT).load() is None


async def test_a_checkpoint_ahead_of_the_chain_is_ignored(tmp_path):
    path = str(tmp_path / "cp.json")
    Checkpoint(path, VAULT).save(500, "hash-500")
    mon = make_monitor(FakeChainRpc(head=20), checkpoint=Checkpoint(path, VAULT))
    await mon.startup()
    assert mon.last_height == 20


async def test_a_checkpoint_too_far_behind_skips_ahead_and_records_a_gap(tmp_path):
    path = str(tmp_path / "cp.json")
    Checkpoint(path, VAULT).save(11, "hash-11")
    rpc = FakeChainRpc(head=100)
    rpc.add_block(11, [("tx-11", VAULT, [log_line()])])
    mon = make_monitor(rpc, checkpoint=Checkpoint(path, VAULT), max_catchup_blocks=20)
    await mon.startup()
    assert mon.last_height == 80, "catches up at most max_catchup_blocks"
    gap = [a for a in mon.alerts if a.pattern == "ingestion_gap"]
    assert len(gap) == 1 and gap[0].observed == 89 and gap[0].action == "logged_only"


def test_checkpoint_roundtrip_is_atomic_and_leaves_no_temp_files(tmp_path):
    cp = Checkpoint(str(tmp_path / "sub" / "cp.json"), VAULT)
    assert cp.load() is None
    cp.save(7, "h7")
    cp.save(8, "h8")
    assert cp.load() == (8, "h8")
    assert [p.name for p in (tmp_path / "sub").iterdir()] == ["cp.json"]


# ── lag ──────────────────────────────────────────────────────────────────────


def test_health_turns_unhealthy_when_the_sentinel_is_too_far_behind():
    mon = make_monitor(FakeChainRpc(), max_lag_blocks=100)
    mon.last_height, mon.head_height = 1000, 1100
    with TestClient(create_app(mon, autostart=False)) as client:
        assert client.get("/health").status_code == 200
        status = client.get("/status").json()
        assert status["lag_blocks"] == 100 and status["source"] == "rpc"
        mon.head_height = 1101
        res = client.get("/health")
        assert res.status_code == 503 and res.json()["reason"] == "lagging"
        assert client.get("/status").json()["lag_blocks"] == 101


# ── the neardata source ──────────────────────────────────────────────────────


def neardata(handler, api_key=None):
    client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    return NeardataEventSource("https://testnet.neardata.example", VAULT, api_key=api_key, client=client)


def block_doc(height=100, outcomes=()):
    return {
        "block": {"header": {"height": height, "hash": f"hash-{height}", "timestamp": height * NS}},
        "shards": [{"receipt_execution_outcomes": list(outcomes)}, {"receipt_execution_outcomes": []}],
    }


def outcome(executor, logs, receipt_id):
    return {
        "receipt": {"receipt_id": receipt_id, "receiver_id": executor},
        "execution_outcome": {"id": receipt_id, "outcome": {"executor_id": executor, "logs": logs}},
        "tx_hash": "tx",
    }


async def test_neardata_head_is_read_from_the_redirect_without_downloading_the_block():
    seen = []

    def handler(request):
        seen.append(request)
        return httpx.Response(302, headers={"location": "/v0/block/277515892"})

    assert await neardata(handler).head_height() == 277515892
    assert len(seen) == 1 and seen[0].url.path == "/v0/last_block/final"


@pytest.mark.parametrize("response", [httpx.Response(200, json={}), httpx.Response(302, headers={"location": "/x"}),
                                      httpx.Response(500)])
async def test_neardata_head_with_an_unexpected_answer_is_an_rpc_error(response):
    with pytest.raises(RpcError):
        await neardata(lambda request: response).head_height()


async def test_neardata_block_keeps_only_events_the_vault_emitted_and_gives_them_stable_ids():
    doc = block_doc(
        100,
        [
            outcome(VAULT, [log_line(), "noise"], "receipt-1"),
            outcome(VAULT, [log_line(event="transfer_rolled_back")], "receipt-2"),  # the callback receipt
            outcome("other.sandbox", [log_line()], "receipt-3"),  # another contract's receipt
        ],
    )
    src = neardata(lambda request: httpx.Response(200, json=doc))
    block = await src.block_events(100)
    assert isinstance(block, BlockEvents) and block.block_hash == "hash-100" and block.timestamp_ns == 100 * NS
    assert [(e.event, e.uid) for e in block.events] == [
        ("transfer_requested", "receipt-1:0"),
        ("transfer_rolled_back", "receipt-2:0"),
    ]
    again = await src.block_events(100)
    assert [e.uid for e in again.events] == [e.uid for e in block.events], "ids are stable across reads"


async def test_neardata_skipped_height_is_none():
    assert await neardata(lambda request: httpx.Response(200, content=b"null")).block_events(5) is None


async def test_neardata_follows_redirects_for_blocks_and_sends_the_api_key_only_in_the_query():
    seen = []

    def handler(request):
        seen.append(request)
        if request.url.host == "testnet.neardata.example":
            # The canonical server hands out the archive host that owns this block.
            return httpx.Response(302, headers={"location": "https://archive.neardata.example/v0/block/100?apiKey=k%2F1"})
        return httpx.Response(200, json=block_doc(100))

    block = await neardata(handler, api_key="k/1").block_events(100)
    assert block is not None
    assert [r.url.host for r in seen] == ["testnet.neardata.example", "archive.neardata.example"]
    assert seen[0].url.params["apiKey"] == "k/1"


@pytest.mark.parametrize(
    ("response", "needle"),
    [
        (httpx.Response(429), "429"),
        (httpx.Response(500), "HTTP 500"),
        (httpx.Response(200, content=b"<html>"), "not JSON"),
        (httpx.Response(200, json={"block": {}}), "unexpected document"),
    ],
)
async def test_neardata_errors_are_rpc_errors_and_never_leak_the_api_key(response, needle):
    with pytest.raises(RpcError) as err:
        await neardata(lambda request: response, api_key="SECRET-KEY").block_events(7)
    assert needle in str(err.value)
    assert "SECRET-KEY" not in str(err.value)


async def test_neardata_transport_failure_is_an_rpc_error_without_the_url():
    def boom(request):
        raise httpx.ConnectError("connection refused", request=request)

    with pytest.raises(RpcError) as err:
        await neardata(boom, api_key="SECRET-KEY").head_height()
    assert "SECRET-KEY" not in str(err.value) and "ConnectError" in str(err.value)


def test_config_for_neardata_polls_less_often_unless_told_otherwise():
    cfg = SentinelConfig.from_env({"SENTINEL_EVENT_SOURCE": "neardata", "NEARDATA_URL": "https://x.example"})
    assert cfg.event_source == "neardata" and cfg.poll_interval_secs == 2.0
    explicit = SentinelConfig.from_env({"SENTINEL_EVENT_SOURCE": "neardata", "POLL_INTERVAL_SECS": "1.5"})
    assert explicit.poll_interval_secs == 1.5
    assert SentinelConfig.from_env({}).event_source == "rpc"
    with pytest.raises(ValueError):
        SentinelConfig.from_env({"SENTINEL_EVENT_SOURCE": "carrier-pigeon"})


# ── neardata rate limiting ───────────────────────────────────────────────────


class FakeTime:
    def __init__(self):
        self.now = 1000.0
        self.slept: list[float] = []

    def clock(self):
        return self.now

    async def sleep(self, seconds):
        self.slept.append(seconds)
        self.now += seconds


async def test_rate_limiter_spaces_request_starts_evenly():
    from event_source import RateLimiter

    t = FakeTime()
    limiter = RateLimiter(per_minute=120, clock=t.clock, sleep=t.sleep)  # one request per 0.5 s
    start = t.now
    for _ in range(5):
        await limiter.acquire()
    assert t.now - start == pytest.approx(2.0), "5 requests need 4 gaps of 0.5 s; the first goes at once"
    assert all(s == pytest.approx(0.5) for s in t.slept)


async def test_rate_limiter_does_not_delay_when_traffic_is_light():
    from event_source import RateLimiter

    t = FakeTime()
    limiter = RateLimiter(per_minute=60, clock=t.clock, sleep=t.sleep)
    for _ in range(3):
        await limiter.acquire()
        t.now += 5  # slower than the budget
    assert t.slept == []


@pytest.mark.parametrize(
    ("retry_after", "expected_wait"),
    [
        ("0", 40.0),  # what the real server sends, although it keeps refusing for ~40 s
        (None, 40.0),  # no header at all
        ("90", 90.0),  # a longer hint is honoured
        ("9999", 120.0),  # but capped
    ],
)
async def test_a_429_pushes_every_later_request_back(retry_after, expected_wait):
    from event_source import RateLimiter

    t = FakeTime()
    limiter = RateLimiter(per_minute=600, clock=t.clock, sleep=t.sleep)
    calls = {"n": 0}

    def handler(request):
        calls["n"] += 1
        if calls["n"] == 1:
            headers = {"retry-after": retry_after} if retry_after is not None else {}
            return httpx.Response(429, headers=headers)
        return httpx.Response(302, headers={"location": "/v0/block/42"})

    client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    src = NeardataEventSource("https://n.example", VAULT, client=client, limiter=limiter)
    with pytest.raises(RpcError) as err:
        await src.head_height()
    assert "429" in str(err.value) and f"{expected_wait:.0f}s" in str(err.value)

    before = t.now
    assert await src.head_height() == 42
    assert t.now - before == pytest.approx(expected_wait), "the next request waited out the back-off"


async def test_every_neardata_request_goes_through_the_limiter():
    from event_source import RateLimiter

    acquired = []

    class Counting(RateLimiter):
        async def acquire(self):
            acquired.append(1)

    def handler(request):
        if request.url.path == "/v0/last_block/final":
            return httpx.Response(302, headers={"location": "/v0/block/9"})
        return httpx.Response(200, json=block_doc(9))

    client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    src = NeardataEventSource("https://n.example", VAULT, client=client, limiter=Counting(60))
    await src.head_height()
    await src.block_events(9)
    assert len(acquired) == 2
