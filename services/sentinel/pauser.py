"""Submits ``pause_agent`` to the vault using the Sentinel's scoped function-call key.

The key is read from the environment by the caller and held in memory only. At
startup :meth:`Pauser.verify_scope` checks on-chain that the key can do nothing
but ``pause_agent`` on the vault; a broader key (e.g. full access) is refused so
the Sentinel can never hold the power to move funds or change policy.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass

from nearcrypto import (
    KeyPair,
    b58decode,
    encode_function_call_transaction,
    parse_secret_key,
    sign_transaction,
)
from rpc import RpcClient, RpcError

log = logging.getLogger("sentinel.pauser")

PAUSE_GAS = 30_000_000_000_000  # 30 Tgas


@dataclass(frozen=True)
class PauseResult:
    ok: bool
    tx_hash: str | None = None
    error: str | None = None


class Pauser:
    def __init__(
        self,
        rpc: RpcClient,
        vault_id: str,
        account_id: str | None,
        secret_key: str | None,
    ) -> None:
        self._rpc = rpc
        self._vault_id = vault_id
        self._account_id = account_id
        self._key: KeyPair | None = None
        self._last_nonce = 0
        if account_id and secret_key:
            try:
                self._key = parse_secret_key(secret_key)
            except ValueError as exc:
                # Never include the key material in the message.
                log.error("sentinel pause key is invalid (%s); running in advisory-only mode", exc)
        else:
            log.error("sentinel pause key/account not configured; running in advisory-only mode")

    @property
    def available(self) -> bool:
        return self._key is not None

    def disable(self, reason: str) -> None:
        log.error("disabling pause actions: %s", reason)
        self._key = None

    async def verify_scope(self) -> bool:
        """Confirm the key is a function-call key limited to ``pause_agent`` on the vault."""
        if self._key is None or self._account_id is None:
            return False
        try:
            info = await self._rpc.view_access_key(self._account_id, self._key.public_key_str)
        except RpcError as exc:
            log.error("could not verify sentinel key scope: %s", exc)
            return False
        permission = info.get("permission")
        fc = permission.get("FunctionCall") if isinstance(permission, dict) else None
        scoped = (
            fc is not None
            and fc.get("receiver_id") == self._vault_id
            and list(fc.get("method_names") or []) == ["pause_agent"]
        )
        if not scoped:
            self.disable("sentinel key is not restricted to pause_agent on the vault")
        return scoped

    async def pause_agent(self, agent_id: str) -> PauseResult:
        if self._key is None or self._account_id is None:
            return PauseResult(ok=False, error="pause key unavailable")
        try:
            info = await self._rpc.view_access_key(self._account_id, self._key.public_key_str)
            nonce = max(int(info["nonce"]), self._last_nonce) + 1
            tx = encode_function_call_transaction(
                signer_id=self._account_id,
                public_key=self._key.public_bytes,
                nonce=nonce,
                receiver_id=self._vault_id,
                block_hash=b58decode(info["block_hash"]),
                method_name="pause_agent",
                args=json.dumps({"agent_id": agent_id}, separators=(",", ":")).encode(),
                gas=PAUSE_GAS,
            )
            signed = sign_transaction(self._key, tx)
            self._last_nonce = nonce
            result = await self._rpc.send_tx(signed.base64)
        except (RpcError, KeyError, ValueError) as exc:
            msg = f"pause_agent({agent_id}) submission failed: {exc}"
            log.error(msg)
            return PauseResult(ok=False, error=msg)

        status = result.get("status")
        if isinstance(status, dict) and "SuccessValue" in status:
            log.warning("paused agent %s on-chain (tx %s)", agent_id, signed.tx_hash)
            return PauseResult(ok=True, tx_hash=signed.tx_hash)
        msg = f"pause_agent({agent_id}) failed on-chain: {json.dumps(status)}"
        log.error(msg)
        return PauseResult(ok=False, tx_hash=signed.tx_hash, error=msg)
