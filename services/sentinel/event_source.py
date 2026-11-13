"""Where the Sentinel reads vault events from.

NEAR's JSON-RPC has no event subscription and no "logs of this contract" query, so events have to be
read from blocks. Two sources are provided:

* ``RpcEventSource``: walks ``block`` -> ``chunk`` -> ``tx`` over plain JSON-RPC. One shard (a local
  sandbox) costs about 2 calls per block, which is fine. A public testnet/mainnet RPC has around ten
  shards (11 calls per block) and cannot be kept up with, and this source only sees transactions
  addressed to the vault (not receipts sent by other contracts). Use it for the sandbox or a node you run.
* ``NeardataEventSource``: reads one JSON document per block from a FastNEAR neardata server, which
  already contains every receipt execution outcome with its logs on every shard.

Both return the events of a *whole* block or raise. Half a block is never delivered, so a retry after a
failure cannot deliver the same events twice.
"""

from __future__ import annotations

import asyncio
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol
from urllib.parse import quote

import httpx

from models import VaultEvent
from rpc import RpcClient, RpcError, extract_events


@dataclass(frozen=True)
class BlockEvents:
    """All vault events of one block. ``block_hash`` identifies the chain a checkpoint belongs to."""

    height: int
    block_hash: str
    timestamp_ns: int
    events: list[VaultEvent]


class EventSource(Protocol):
    name: str

    async def head_height(self) -> int:
        """Height of the latest final block."""

    async def block_events(self, height: int) -> BlockEvents | None:
        """Events of block ``height``; ``None`` for a height the chain skipped. Raises ``RpcError``."""

    async def aclose(self) -> None: ...


class RpcEventSource:
    name = "rpc"

    def __init__(self, rpc: RpcClient, vault_id: str) -> None:
        self._rpc = rpc
        self._vault_id = vault_id

    async def head_height(self) -> int:
        head = await self._rpc.final_block()
        return int(head["header"]["height"])

    async def block_events(self, height: int) -> BlockEvents | None:
        block = await self._rpc.block_at(height)
        if block is None:
            return None
        timestamp_ns = int(block["header"]["timestamp"])
        # A chunk listed with an older height_included was missed in this block; its transactions
        # were already handled when that earlier block was read.
        refs = [c for c in block.get("chunks", []) if int(c.get("height_included", height)) == height]
        chunks = await asyncio.gather(*(self._rpc.chunk(c["chunk_hash"]) for c in refs))
        txs = [
            t
            for chunk in chunks
            for t in chunk.get("transactions", [])
            if t.get("receiver_id") == self._vault_id
        ]
        statuses = await asyncio.gather(*(self._rpc.tx_status(t["hash"], t["signer_id"]) for t in txs))
        events: list[VaultEvent] = []
        for status in statuses:
            for receipt in status.get("receipts_outcome", []):
                logs = receipt.get("outcome", {}).get("logs", [])
                events.extend(extract_events(logs, timestamp_ns, height, receipt_id=receipt.get("id")))
        return BlockEvents(height, str(block["header"].get("hash", "")), timestamp_ns, events)

    async def aclose(self) -> None:
        await self._rpc.aclose()


# Measured: after a 429 the free tier keeps refusing for about 40 seconds.
MIN_429_BACKOFF_SECS = 40.0


class RateLimiter:
    """Spaces request starts evenly (``per_minute``) and lets a 429 push every later request back.

    The free neardata tier is documented at 180 requests per minute per IP, but measured on testnet
    (November 2026) it answers 429 after about 30 requests and recovers only after about 40 seconds, so
    the sustainable free rate is about 45 requests per minute. Testnet produces about 105 blocks per
    minute, so the free tier CANNOT keep a Sentinel at the head; an API key (NEARDATA_API_KEY) or a
    self-hosted source is required for real use. The limiter keeps the client inside whatever budget is
    configured instead of hammering the server and getting further behind.
    """

    def __init__(
        self,
        per_minute: int,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
    ) -> None:
        self._interval = 60.0 / per_minute
        self._clock = clock
        self._sleep = sleep
        self._next = 0.0
        self._lock = asyncio.Lock()

    async def acquire(self) -> None:
        async with self._lock:
            now = self._clock()
            wait = self._next - now
            self._next = max(now, self._next) + self._interval
        if wait > 0:
            await self._sleep(wait)

    def backoff(self, seconds: float) -> None:
        """A 429 arrived: nothing may start before ``seconds`` from now."""
        self._next = max(self._next, self._clock() + seconds)


class NeardataEventSource:
    name = "neardata"

    def __init__(
        self,
        base_url: str,
        vault_id: str,
        api_key: str | None = None,
        client: httpx.AsyncClient | None = None,
        timeout: float = 30.0,
        requests_per_minute: int = 40,
        limiter: RateLimiter | None = None,
    ) -> None:
        self._base = base_url.rstrip("/")
        self._vault_id = vault_id
        self._api_key = api_key
        self._client = client or httpx.AsyncClient(timeout=timeout)
        self._limiter = limiter or RateLimiter(requests_per_minute)

    def _url(self, path: str) -> str:
        url = f"{self._base}{path}"
        return f"{url}?apiKey={quote(self._api_key, safe='')}" if self._api_key else url

    async def _get(self, path: str, follow_redirects: bool) -> httpx.Response:
        # Never put the URL in an error message: it can carry the API key.
        await self._limiter.acquire()
        try:
            resp = await self._client.get(self._url(path), follow_redirects=follow_redirects)
        except httpx.HTTPError as exc:
            raise RpcError(f"neardata {path}: transport failure: {type(exc).__name__}") from exc
        if resp.status_code == 429:
            # The server sends `Retry-After: 0` even though it keeps refusing for ~40 s (measured), so
            # never back off for less than MIN_429_BACKOFF_SECS, whatever the header says.
            retry_after = resp.headers.get("retry-after", "")
            hinted = float(retry_after) if retry_after.replace(".", "", 1).isdigit() else 0.0
            delay = min(max(hinted, MIN_429_BACKOFF_SECS), 120.0)
            self._limiter.backoff(delay)
            raise RpcError(
                f"neardata {path}: rate limited (HTTP 429), backing off {delay:.0f}s; "
                "lower NEARDATA_REQUESTS_PER_MINUTE or set NEARDATA_API_KEY"
            )
        return resp

    async def head_height(self) -> int:
        # /v0/last_block/final answers 302 with `Location: /v0/block/<height>`; the height is all we
        # need, so do not follow it (that would download a whole block just to read one number).
        resp = await self._get("/v0/last_block/final", follow_redirects=False)
        location = resp.headers.get("location", "")
        tail = location.rstrip("/").rsplit("/", 1)[-1].split("?", 1)[0]
        if resp.status_code not in (301, 302, 303, 307, 308) or not tail.isdigit():
            raise RpcError(f"neardata last_block/final: unexpected answer (HTTP {resp.status_code})")
        return int(tail)

    async def block_events(self, height: int) -> BlockEvents | None:
        resp = await self._get(f"/v0/block/{height}", follow_redirects=True)
        if resp.status_code >= 400:
            raise RpcError(f"neardata block {height}: HTTP {resp.status_code}")
        try:
            doc = resp.json()
        except ValueError as exc:
            raise RpcError(f"neardata block {height}: response is not JSON") from exc
        if doc is None:  # a height the chain skipped
            return None
        try:
            header = doc["block"]["header"]
            timestamp_ns = int(header["timestamp"])
            events: list[VaultEvent] = []
            for shard in doc.get("shards") or []:
                for item in shard.get("receipt_execution_outcomes") or []:
                    outcome = item.get("execution_outcome") or {}
                    body = outcome.get("outcome") or {}
                    if body.get("executor_id") != self._vault_id:
                        continue
                    receipt_id = (item.get("receipt") or {}).get("receipt_id") or outcome.get("id")
                    events.extend(extract_events(body.get("logs") or [], timestamp_ns, height, receipt_id=receipt_id))
            return BlockEvents(height, str(header.get("hash", "")), timestamp_ns, events)
        except (KeyError, TypeError, ValueError) as exc:
            raise RpcError(f"neardata block {height}: unexpected document shape ({type(exc).__name__})") from exc

    async def aclose(self) -> None:
        await self._client.aclose()
