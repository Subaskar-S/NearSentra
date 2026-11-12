"""Anomaly detection and the chain-polling loop.

Detection is deterministic and rule based (Requirement 11.5). Time is taken from
block timestamps, never the wall clock, so behaviour is reproducible and replays
of old data cannot be mistaken for fresh activity.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections import deque
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import UTC, datetime

from models import Alert, SentinelConfig, VaultEvent
from pauser import Pauser
from rpc import RpcClient, RpcError, extract_events

log = logging.getLogger("sentinel.monitor")

MAX_ALERTS = 1000
NS = 1_000_000_000


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
    ) -> None:
        self.config = config
        self.rpc = rpc
        self.pauser = pauser
        self.detector = detector or AnomalyDetector(config)
        self._clock = clock
        self._sleep = sleep
        self.alerts: deque[Alert] = deque(maxlen=MAX_ALERTS)
        self.last_height: int | None = None
        self.last_progress = clock()  # last time the final head advanced
        self.interrupted = False
        self._replay_until_height = -1  # events at or below this are catch-up, never trigger pauses
        self.running = False

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

    async def handle_event(self, ev: VaultEvent) -> None:
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
        head = await self.rpc.final_block()
        self.last_height = int(head["header"]["height"])
        self.last_progress = self._clock()

    async def poll_once(self) -> int:
        """Process every new final block. Raises RpcError if the node is unreachable."""
        head = await self.rpc.final_block()
        head_height = int(head["header"]["height"])
        if self.last_height is None:
            self.last_height = head_height
        if head_height > self.last_height:
            self.last_progress = self._clock()
        events = 0
        for height in range(self.last_height + 1, head_height + 1):
            block = await self.rpc.block_at(height)
            if block is None:
                self.last_height = height
                continue
            ts_ns = int(block["header"]["timestamp"])
            for chunk_ref in block.get("chunks", []):
                # A chunk listed with an older height_included was missed in this block;
                # its transactions were already handled when that earlier block was scanned.
                if int(chunk_ref.get("height_included", height)) != height:
                    continue
                chunk = await self.rpc.chunk(chunk_ref["chunk_hash"])
                txs = [t for t in chunk.get("transactions", []) if t.get("receiver_id") == self.config.vault_account_id]
                statuses = await asyncio.gather(
                    *(self.rpc.tx_status(t["hash"], t["signer_id"]) for t in txs)
                )
                for status in statuses:
                    logs = [
                        line
                        for r in status.get("receipts_outcome", [])
                        for line in r.get("outcome", {}).get("logs", [])
                    ]
                    for ev in extract_events(logs, ts_ns, height):
                        events += 1
                        await self.handle_event(ev)
            self.last_height = height
        return events

    async def run(self, stop: asyncio.Event) -> None:
        """Poll until ``stop`` is set. Never pauses anything on stale or absent data."""
        self.running = True
        try:
            if self.last_height is None:
                while not stop.is_set():
                    try:
                        await self.start_from_head()
                        break
                    except RpcError as exc:
                        log.error("cannot reach NEAR RPC at startup: %s", exc)
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
