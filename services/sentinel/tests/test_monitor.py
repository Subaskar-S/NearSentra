import asyncio
import json

import pytest
from fastapi.testclient import TestClient

from main import create_app
from models import SentinelConfig, VaultEvent
from monitor import NS, Monitor
from pauser import PauseResult
from rpc import RpcError, extract_events


def ev(kind="transfer_requested", agent="a.sandbox", decision="approved", ts=0, height=10):
    return VaultEvent(event=kind, agent_id=agent, decision=decision, timestamp_ns=ts * NS, block_height=height)


class FakePauser:
    def __init__(self, available=True, ok=True):
        self.available = available
        self.ok = ok
        self.paused: list[str] = []

    async def pause_agent(self, agent_id):
        self.paused.append(agent_id)
        return PauseResult(ok=self.ok, error=None if self.ok else "boom")


class FakeRpc:
    def __init__(self, registered=True):
        self.registered = registered
        self.heads = []
        self.fail = False
        self.closed = False

    async def view_agent_policy(self, vault, agent):
        return {"agent_id": agent} if self.registered else None

    async def aclose(self):
        self.closed = True


def make(rpc=None, pauser=None, **cfg):
    config = SentinelConfig(velocity_threshold=4, velocity_window_secs=60, **cfg)
    return Monitor(config, rpc or FakeRpc(), pauser or FakePauser())


async def feed(mon, n, **kw):
    for i in range(n):
        await mon.handle_event(ev(ts=i, **kw))


async def test_velocity_pauses_agent_and_records_alert():
    p = FakePauser()
    mon = make(pauser=p)
    await feed(mon, 5)
    assert p.paused == ["a.sandbox"]
    [alert] = mon.recent_alerts()
    assert (alert.pattern, alert.action, alert.threshold, alert.observed) == ("velocity_exceeded", "pause_attempted", 4, 5)
    assert alert.agent_id == "a.sandbox" and alert.timestamp_utc.tzinfo is not None


async def test_under_threshold_never_pauses():
    p = FakePauser()
    mon = make(pauser=p)
    await feed(mon, 4)
    assert p.paused == [] and mon.recent_alerts() == []


async def test_failed_pause_is_still_recorded_as_attempt(caplog):
    p = FakePauser(ok=False)
    mon = make(pauser=p)
    with caplog.at_level("ERROR"):
        await feed(mon, 5)
    assert p.paused == ["a.sandbox"]
    assert mon.recent_alerts()[0].action == "pause_attempted"
    assert any("FAILED to pause" in r.message for r in caplog.records)


async def test_advisory_only_when_key_unavailable():
    p = FakePauser(available=False)
    mon = make(pauser=p)
    await feed(mon, 5)
    assert p.paused == []
    assert mon.recent_alerts()[0].action == "advisory_only"


async def test_unregistered_account_is_not_paused():
    p = FakePauser()
    mon = make(rpc=FakeRpc(registered=False), pauser=p)
    await feed(mon, 5)
    assert p.paused == []


async def test_consecutive_failures_are_logged_only():
    p = FakePauser()
    mon = make(pauser=p, failure_threshold=3)
    await feed(mon, 3, decision="rejected")
    [alert] = mon.recent_alerts()
    assert (alert.pattern, alert.action) == ("consecutive_failures", "logged_only")
    assert p.paused == []


async def test_replayed_events_after_outage_never_pause():
    p = FakePauser()
    mon = make(pauser=p)
    mon._replay_until_height = 100
    await feed(mon, 5, height=50)
    assert p.paused == []
    assert mon.recent_alerts()[0].action == "logged_only"
    await mon.handle_event(ev(ts=1000, height=101))  # fresh again


async def test_alerts_capped_at_1000_most_recent_first():
    mon = make(pauser=FakePauser(available=False), failure_threshold=1)
    for i in range(1200):
        mon.detector.on_agent_unpaused("a.sandbox")  # re-arm
        await mon.handle_event(ev(agent=f"agent{i}.sandbox", decision="rejected", ts=i))
    alerts = mon.recent_alerts()
    assert len(alerts) == 1000
    assert alerts[0].agent_id == "agent1199.sandbox"


def test_extract_events_ignores_foreign_and_malformed_logs():
    good = 'EVENT_JSON:' + json.dumps({"standard": "nearsentra", "version": "1.0.0", "event": "transfer_requested",
                                       "data": {"agent_id": "a.sandbox", "decision": "rejected"}})
    other = 'EVENT_JSON:' + json.dumps({"standard": "nep141", "event": "ft_transfer", "data": {}})
    out = extract_events([good, other, "EVENT_JSON:{not json", "plain log"], 5, 9)
    assert len(out) == 1 and out[0].agent_id == "a.sandbox" and out[0].decision == "rejected"


# ── polling / stream-interruption behaviour ─────────────────────────────────


class ScriptedMonitor(Monitor):
    """Monitor whose poll_once is scripted; time is a controllable fake."""

    def __init__(self, script, **kw):
        self.now = 0.0
        super().__init__(SentinelConfig(stream_timeout_secs=30, reconnect_interval_secs=5, poll_interval_secs=1),
                         FakeRpc(), FakePauser(), clock=lambda: self.now)
        self.script = list(script)
        self.last_height = 1
        self.sleeps: list[float] = []

    async def poll_once(self):
        step = self.script.pop(0)
        self.now += step["dt"]
        if step.get("error"):
            raise RpcError("node down")
        self.last_progress = self.now
        return 0


async def test_outage_over_30s_logs_error_retries_fast_and_recovers_without_pausing(caplog):
    script = [{"dt": 1}] + [{"dt": 10, "error": True} for _ in range(4)] + [{"dt": 5}]
    mon = ScriptedMonitor(script)
    stop = asyncio.Event()
    waits = []

    real_wait_for = asyncio.wait_for

    async def fake_wait_for(coro, timeout):
        waits.append(timeout)
        coro.close()
        if not mon.script:
            stop.set()
        raise TimeoutError

    asyncio.wait_for = fake_wait_for  # type: ignore[assignment]
    try:
        with caplog.at_level("ERROR"):
            await mon.run(stop)
    finally:
        asyncio.wait_for = real_wait_for  # type: ignore[assignment]

    assert any("stream interruption" in r.message for r in caplog.records)
    assert all(w <= 10 for w in waits), waits  # reconnect interval never exceeds 10s
    assert not mon.interrupted  # recovered
    assert mon._replay_until_height >= 1  # catch-up events are alert-only
    assert mon.pauser.paused == []  # nothing paused from stale data


# ── HTTP API ─────────────────────────────────────────────────────────────────


def test_health_and_alerts_endpoints():
    mon = make(pauser=FakePauser(available=False), failure_threshold=1)
    asyncio.run(mon.handle_event(ev(decision="rejected", ts=1)))
    with TestClient(create_app(mon, autostart=False)) as client:
        r = client.get("/health")
        assert r.status_code == 200 and r.json() == {"status": "healthy"}
        alerts = client.get("/alerts").json()
        assert len(alerts) == 1
        assert {"agent_id", "pattern", "timestamp_utc", "threshold", "observed", "action"} <= alerts[0].keys()
        mon.interrupted = True
        assert client.get("/health").status_code == 503


@pytest.mark.parametrize("n", [0, 3])
def test_alerts_endpoint_orders_newest_first(n):
    mon = make(pauser=FakePauser(available=False), failure_threshold=1)
    for i in range(n):
        asyncio.run(mon.handle_event(ev(agent=f"a{i}.sandbox", decision="rejected", ts=i)))
    with TestClient(create_app(mon, autostart=False)) as client:
        ids = [a["agent_id"] for a in client.get("/alerts").json()]
        assert ids == [f"a{i}.sandbox" for i in reversed(range(n))]
