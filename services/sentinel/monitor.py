"""Anomaly detection and the chain-polling loop.

Detection is deterministic and rule based (Requirement 11.5). Time is taken from
block timestamps, never the wall clock, so behaviour is reproducible and replays
of old data cannot be mistaken for fresh activity.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections import OrderedDict, deque
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import UTC, datetime

from checkpoint import Checkpoint
from event_source import EventSource, RpcEventSource
from models import Alert, SentinelConfig, VaultEvent
from pauser import Pauser
from rpc import RpcClient, RpcError

log = logging.getLogger("sentinel.monitor")

MAX_ALERTS = 1000
NS = 1_000_000_000
# Remembered event identities, enough for many minutes of busy traffic; the oldest are forgotten first.
MAX_SEEN_EVENTS = 50_000


@dataclass(frozen=True)
class Detection:
    agent_id: str
    pattern: str  # "velocity_exceeded" | "consecutive_failures"
    threshold: int
    observed: int
    timestamp_ns: int


class AnomalyDetector:
    """Per-agent request-rate and consecutive-failure counters (deque based)."""

    def __init__(self, config: SentinelConfig) -> None:
        self._cfg = config
        self._requests: dict[str, deque[int]] = {}
        self._failures: dict[str, deque[int]] = {}
        self._velocity_suppressed_until: dict[str, int] = {}
        self._failure_alerted: set[str] = set()

    def request_count(self, agent_id: str) -> int:
        return len(self._requests.get(agent_id, ()))

    def failure_count(self, agent_id: str) -> int:
        return len(self._failures.get(agent_id, ()))

    def on_agent_unpaused(self, agent_id: str) -> None:
        """An admin re-activated the agent: start counting from scratch."""
        self._requests.pop(agent_id, None)
        self._failures.pop(agent_id, None)
        self._velocity_suppressed_until.pop(agent_id, None)
        self._failure_alerted.discard(agent_id)

    def on_transfer_requested(self, agent_id: str, approved: bool, ts_ns: int) -> list[Detection]:
        found: list[Detection] = []

        # ── velocity: every request (approved or rejected) counts ────────────
        window = self._cfg.velocity_window_secs * NS
        reqs = self._requests.setdefault(agent_id, deque())
        reqs.append(ts_ns)
        while reqs and reqs[0] <= ts_ns - window:
            reqs.popleft()
        if len(reqs) > self._cfg.velocity_threshold and ts_ns >= self._velocity_suppressed_until.get(agent_id, 0):
            found.append(
                Detection(agent_id, "velocity_exceeded", self._cfg.velocity_threshold, len(reqs), ts_ns)
            )
            # Do not re-fire for every following request; re-arm after one window.
            self._velocity_suppressed_until[agent_id] = ts_ns + window

        # ── consecutive failures within the detection window ─────────────────
        fails = self._failures.setdefault(agent_id, deque())
        if approved:
            fails.clear()
            self._failure_alerted.discard(agent_id)
        else:
            fwindow = self._cfg.failure_window_secs * NS
            fails.append(ts_ns)
            while fails and fails[0] <= ts_ns - fwindow:
                fails.popleft()
            if len(fails) >= self._cfg.failure_threshold and agent_id not in self._failure_alerted:
                self._failure_alerted.add(agent_id)
                found.append(
                    Detection(agent_id, "consecutive_failures", self._cfg.failure_threshold, len(fails), ts_ns)
                )
        return found


class Monitor:
    def __init__(
        self,
        config: SentinelConfig,
        rpc: RpcClient,
        pauser: Pauser,
        detector: AnomalyDetector | None = None,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
        source: EventSource | None = None,
        checkpoint: Checkpoint | None = None,
    ) -> None:
        self.config = config
        self.rpc = rpc
        self.pauser = pauser
        self.source: EventSource = source or RpcEventSource(rpc, config.vault_account_id)
        self.checkpoint = checkpoint
        self.detector = detector or AnomalyDetector(config)
        self._clock = clock
        self._sleep = sleep
        self.alerts: deque[Alert] = deque(maxlen=MAX_ALERTS)
        self.last_height: int | None = None
        self.head_height: int | None = None
        self.last_progress = clock()  # last time the final head advanced
        self.interrupted = False
        self._replay_until_height = -1  # events at or below this are catch-up, never trigger pauses
        self._seen: OrderedDict[str, None] = OrderedDict()
        self.duplicates_dropped = 0
        self.running = False

    @property
    def lag_blocks(self) -> int:
        """How many final blocks the Sentinel is behind the chain head (0 until it has seen the head)."""
        if self.head_height is None or self.last_height is None:
            return 0
        return max(0, self.head_height - self.last_height)

    @property
    def lagging(self) -> bool:
        return self.lag_blocks > self.config.max_lag_blocks

    # ── alerts ───────────────────────────────────────────────────────────────

    def recent_alerts(self) -> list[Alert]:
        return list(reversed(self.alerts))

    async def _raise(self, det: Detection, replay: bool) -> None:
        action: str
        if det.pattern == "velocity_exceeded":
            if replay:
                action = "logged_only"
                log.warning("velocity anomaly for %s seen while catching up; not pausing", det.agent_id)
            elif not self.pauser.available:
                action = "advisory_only"
                log.error("velocity anomaly for %s but pause key unavailable (advisory only)", det.agent_id)
            else:
                action = "pause_attempted"
                await self._try_pause(det.agent_id)
        else:
            action = "logged_only"
            log.warning(
                "suspicious activity: agent=%s consecutive_failures=%d window=%ds",
                det.agent_id,
                det.observed,
                self.config.failure_window_secs,
            )
        self.alerts.append(
            Alert(
                agent_id=det.agent_id,
                pattern=det.pattern,  # type: ignore[arg-type]
                threshold=det.threshold,
                observed=det.observed,
                action=action,  # type: ignore[arg-type]
                timestamp_utc=datetime.now(UTC),
            )
        )

    async def _try_pause(self, agent_id: str) -> None:
        try:
            policy = await self.rpc.view_agent_policy(self.config.vault_account_id, agent_id)
        except RpcError as exc:
            log.error("could not read policy for %s before pausing: %s", agent_id, exc)
            policy = {}
        if policy is None:
            # Unregistered account: nothing to pause (and don't let spam burn gas allowance).
            log.warning("agent %s is not registered; skipping pause", agent_id)
            return
        result = await self.pauser.pause_agent(agent_id)
        if not result.ok:
            log.error("FAILED to pause %s: %s", agent_id, result.error)

    # ── event handling ───────────────────────────────────────────────────────

    def _is_duplicate(self, ev: VaultEvent) -> bool:
        """True if this exact log line was already processed. Events without an identity are never dropped."""
        if ev.uid is None:
            return False
        if ev.uid in self._seen:
            self._seen.move_to_end(ev.uid)
            self.duplicates_dropped += 1
            return True
        self._seen[ev.uid] = None
        while len(self._seen) > MAX_SEEN_EVENTS:
            self._seen.popitem(last=False)
        return False

    async def handle_event(self, ev: VaultEvent) -> None:
        if self._is_duplicate(ev):
            return
        if ev.agent_id is None:
            return
        if ev.event == "agent_unpaused":
            self.detector.on_agent_unpaused(ev.agent_id)
        elif ev.event == "transfer_requested":
            replay = ev.block_height <= self._replay_until_height
            for det in self.detector.on_transfer_requested(ev.agent_id, ev.decision == "approved", ev.timestamp_ns):
                await self._raise(det, replay)

    # ── polling ──────────────────────────────────────────────────────────────

    async def start_from_head(self) -> None:
        """Start at the current head. Anything produced before now is not looked at."""
        head = await self.source.head_height()
        self.last_height = head
        self.head_height = head
        self.last_progress = self._clock()

    async def startup(self) -> None:
        """Resume from the checkpoint when it is valid for this chain, otherwise start from the head.

        Blocks produced while the Sentinel was down are read as catch-up: they are recorded and can
        raise alerts, but never trigger a pause (the agent may have stopped long ago).
        """
        head = await self.source.head_height()
        self.head_height = head
        self.last_progress = self._clock()
        resumed = self.checkpoint.load() if self.checkpoint else None
        if resumed is not None:
            height, block_hash = resumed
            if height > head:
                log.warning("checkpoint is ahead of the chain (%d > %d): ignoring it", height, head)
            else:
                block = await self.source.block_events(height)
                if block is None or block.block_hash != block_hash:
                    log.warning("checkpoint block %d does not match this chain: ignoring it", height)
                else:
                    gap = head - height
                    if gap > self.config.max_catchup_blocks:
                        skipped_to = head - self.config.max_catchup_blocks
                        self._record_gap(gap, f"checkpoint is {gap} blocks behind; skipped to block {skipped_to}")
                        height = skipped_to
                    self.last_height = height
                    self._replay_until_height = head
                    log.warning(
                        "resumed from checkpoint at block %d (%d blocks behind, catch-up is alert-only)",
                        height,
                        head - height,
                    )
                    return
        self.last_height = head
        log.warning(
            "starting from block %d: events produced while the Sentinel was not running are NOT recovered "
            "(set CHECKPOINT_PATH to resume after a restart)",
            head,
        )

    def _record_gap(self, observed: int, message: str) -> None:
        log.error("ingestion gap: %s", message)
        self.alerts.append(
            Alert(
                agent_id="*",
                pattern="ingestion_gap",
                threshold=self.config.max_catchup_blocks,
                observed=observed,
                action="logged_only",
                timestamp_utc=datetime.now(UTC),
            )
        )

    async def poll_once(self) -> int:
        """Process every new final block, strictly in order. Raises RpcError if the source is unreachable.

        Blocks are fetched a few at a time, but each block is applied only after the whole block was
        read, so a failure never leaves half a block applied and a retry never delivers an event twice.
        """
        head = await self.source.head_height()
        self.head_height = head
        if self.last_height is None:
            self.last_height = head
        if head > self.last_height:
            self.last_progress = self._clock()
        if self.interrupted:
            # The first successful read after an outage: everything up to the head was produced while
            # we could not see the chain. Mark it as catch-up BEFORE it is processed, so it is recorded
            # but can never trigger a pause (the agent may have stopped long ago).
            self._replay_until_height = max(self._replay_until_height, head)
        events = 0
        while self.last_height < head:
            batch = list(range(self.last_height + 1, min(head, self.last_height + self.config.fetch_concurrency) + 1))
            results = await asyncio.gather(*(self.source.block_events(h) for h in batch), return_exceptions=True)
            for height, result in zip(batch, results, strict=True):
                if isinstance(result, BaseException):
                    raise result  # earlier blocks of this batch are already applied and saved
                if result is not None:
                    for ev in result.events:
                        events += 1
                        await self.handle_event(ev)
                    if self.checkpoint is not None:
                        self.checkpoint.save(result.height, result.block_hash)
                self.last_height = height
        return events

    async def run(self, stop: asyncio.Event) -> None:
        """Poll until ``stop`` is set. Never pauses anything on stale or absent data."""
        self.running = True
        try:
            if self.last_height is None:
                while not stop.is_set():
                    try:
                        await self.startup()
                        break
                    except RpcError as exc:
                        log.error("cannot reach the chain at startup: %s", exc)
                        await self._sleep(min(self.config.reconnect_interval_secs, 10))
            last_ok = self._clock()
            while not stop.is_set():
                try:
                    await self.poll_once()
                    last_ok = self._clock()
                    stalled = (self._clock() - self.last_progress) > self.config.stream_timeout_secs
                    if stalled and not self.interrupted:
                        self.interrupted = True
                        log.error("stream interruption: no new blocks for >%ss", self.config.stream_timeout_secs)
                    elif not stalled and self.interrupted:
                        self._recover()
                    delay = self.config.poll_interval_secs
                except RpcError as exc:
                    if (self._clock() - last_ok) > self.config.stream_timeout_secs:
                        if not self.interrupted:
                            self.interrupted = True
                        log.error("stream interruption: event stream unavailable: %s", exc)
                    else:
                        log.warning("poll failed, will retry: %s", exc)
                    delay = min(self.config.reconnect_interval_secs, 10)
                try:
                    await asyncio.wait_for(stop.wait(), timeout=delay)
                except TimeoutError:
                    pass
        finally:
            self.running = False

    def _recover(self) -> None:
        self.interrupted = False
        # Everything up to the current head was produced during/after the outage:
        # still record anomalies, but never pause based on it.
        self._replay_until_height = self.last_height if self.last_height is not None else -1
        log.warning("event stream recovered; catching-up events are alert-only")
