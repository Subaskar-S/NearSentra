"""Thin async JSON-RPC client for the few NEAR RPC methods the Sentinel needs."""

from __future__ import annotations

import itertools
import json
from typing import Any

import httpx

from models import VaultEvent


class RpcError(Exception):
    """RPC transport failure or JSON-RPC error."""

    def __init__(self, message: str, name: str | None = None) -> None:
        super().__init__(message)
        self.name = name


class RpcClient:
    def __init__(self, url: str, client: httpx.AsyncClient | None = None, timeout: float = 15.0) -> None:
        self._url = url
        self._client = client or httpx.AsyncClient(timeout=timeout)
        self._ids = itertools.count(1)

    async def aclose(self) -> None:
        await self._client.aclose()

    async def call(self, method: str, params: Any) -> Any:
        payload = {"jsonrpc": "2.0", "id": next(self._ids), "method": method, "params": params}
        try:
            resp = await self._client.post(self._url, json=payload)
        except httpx.HTTPError as exc:
            raise RpcError(f"{method}: transport failure: {exc}") from exc
        # Some nodes answer JSON-RPC errors (e.g. UNKNOWN_BLOCK) with a 4xx status and a JSON body.
        try:
            body = resp.json()
        except ValueError as exc:
            raise RpcError(f"{method}: transport failure: HTTP {resp.status_code}: {exc}") from exc
        if body.get("error") is not None:
            err = body["error"]
            cause = (err.get("cause") or {}).get("name")
            raise RpcError(f"{method}: {err.get('message')}: {err.get('data')}", name=cause)
        if resp.status_code >= 400:
            raise RpcError(f"{method}: transport failure: HTTP {resp.status_code}")
        return body["result"]

    # ── chain reads ──────────────────────────────────────────────────────────

    async def final_block(self) -> dict[str, Any]:
        return await self.call("block", {"finality": "final"})

    async def block_at(self, height: int) -> dict[str, Any] | None:
        try:
            return await self.call("block", {"block_id": height})
        except RpcError as exc:
            if exc.name == "UNKNOWN_BLOCK":  # skipped height
                return None
            raise

    async def chunk(self, chunk_hash: str) -> dict[str, Any]:
        return await self.call("chunk", {"chunk_id": chunk_hash})

    async def tx_status(self, tx_hash: str, sender_id: str) -> dict[str, Any]:
        return await self.call(
            "tx",
            {"tx_hash": tx_hash, "sender_account_id": sender_id, "wait_until": "FINAL"},
        )

    async def view_access_key(self, account_id: str, public_key: str) -> dict[str, Any]:
        return await self.call(
            "query",
            {
                "request_type": "view_access_key",
                "finality": "optimistic",
                "account_id": account_id,
                "public_key": public_key,
            },
        )

    async def view_agent_policy(self, vault_id: str, agent_id: str) -> dict[str, Any] | None:
        import base64

        args = base64.b64encode(json.dumps({"agent_id": agent_id}).encode()).decode()
        result = await self.call(
            "query",
            {
                "request_type": "call_function",
                "finality": "optimistic",
                "account_id": vault_id,
                "method_name": "get_agent_policy",
                "args_base64": args,
            },
        )
        return json.loads(bytes(result["result"]).decode())

    async def send_tx(self, signed_base64: str) -> dict[str, Any]:
        return await self.call("send_tx", {"signed_tx_base64": signed_base64, "wait_until": "FINAL"})


def extract_events(
    logs: list[str], timestamp_ns: int, block_height: int, receipt_id: str | None = None
) -> list[VaultEvent]:
    """Parse ``EVENT_JSON:`` lines of the ``nearsentra`` standard; ignore anything else.

    With a ``receipt_id`` every event gets a stable ``uid`` (``<receipt id>:<log index>``) so a duplicate
    delivery of the same log line can be recognised later.
    """
    events: list[VaultEvent] = []
    for index, line in enumerate(logs):
        if not line.startswith("EVENT_JSON:"):
            continue
        try:
            doc = json.loads(line[len("EVENT_JSON:") :])
        except ValueError:
            continue
        if not isinstance(doc, dict) or doc.get("standard") != "nearsentra":
            continue
        data = doc.get("data") if isinstance(doc.get("data"), dict) else {}
        events.append(
            VaultEvent(
                event=str(doc.get("event")),
                agent_id=data.get("agent_id"),
                decision=data.get("decision"),
                timestamp_ns=timestamp_ns,
                block_height=block_height,
                uid=f"{receipt_id}:{index}" if receipt_id else None,
            )
        )
    return events
