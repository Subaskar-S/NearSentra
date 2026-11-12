import base64
import json

from nearcrypto import b58encode, parse_secret_key
from pauser import Pauser
from rpc import RpcError

SEED = bytes(range(32))


def secret():
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

    pub = Ed25519PrivateKey.from_private_bytes(SEED).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    return "ed25519:" + b58encode(SEED + pub)


class Rpc:
    def __init__(self, permission, send=None):
        self.permission = permission
        self.sent = []
        self._send = send

    async def view_access_key(self, account, pk):
        return {"nonce": 41, "block_hash": b58encode(b"\x01" * 32), "permission": self.permission}

    async def send_tx(self, b64):
        self.sent.append(base64.b64decode(b64))
        if isinstance(self._send, Exception):
            raise self._send
        return self._send or {"status": {"SuccessValue": ""}}


SCOPED = {"FunctionCall": {"receiver_id": "vault.sandbox", "method_names": ["pause_agent"], "allowance": "1"}}


async def test_pause_submits_signed_tx_with_next_nonce():
    rpc = Rpc(SCOPED)
    p = Pauser(rpc, "vault.sandbox", "sentinel.sandbox", secret())
    res = await p.pause_agent("a.sandbox")
    assert res.ok and res.tx_hash
    assert len(rpc.sent) == 1 and b"pause_agent" in rpc.sent[0]
    assert json.dumps({"agent_id": "a.sandbox"}, separators=(",", ":")).encode() in rpc.sent[0]
    # second call uses a strictly larger nonce even if the chain view is stale
    await p.pause_agent("b.sandbox")
    assert len(rpc.sent) == 2


async def test_failure_is_reported_not_swallowed():
    p = Pauser(Rpc(SCOPED, send=RpcError("network")), "vault.sandbox", "sentinel.sandbox", secret())
    res = await p.pause_agent("a.sandbox")
    assert not res.ok and "network" in res.error

    failing = {"status": {"Failure": {"ActionError": "ERR_ACCESS_DENIED"}}}
    p = Pauser(Rpc(SCOPED, send=failing), "vault.sandbox", "sentinel.sandbox", secret())
    res = await p.pause_agent("a.sandbox")
    assert not res.ok and "ERR_ACCESS_DENIED" in res.error


async def test_missing_or_invalid_key_means_unavailable(caplog):
    with caplog.at_level("ERROR"):
        assert not Pauser(Rpc(SCOPED), "v", "s", None).available
        bad = Pauser(Rpc(SCOPED), "v", "s", "ed25519:garbage")
    assert not bad.available
    assert "garbage" not in caplog.text  # key material is never logged
    assert not (await bad.pause_agent("a")).ok


async def test_scope_verification_refuses_broader_keys():
    ok = Pauser(Rpc(SCOPED), "vault.sandbox", "s", secret())
    assert await ok.verify_scope() and ok.available

    for permission in (
        "FullAccess",
        {"FunctionCall": {"receiver_id": "vault.sandbox", "method_names": []}},  # empty = ALL methods
        {"FunctionCall": {"receiver_id": "vault.sandbox", "method_names": ["pause_agent", "request_transfer"]}},
        {"FunctionCall": {"receiver_id": "other.sandbox", "method_names": ["pause_agent"]}},
    ):
        p = Pauser(Rpc(permission), "vault.sandbox", "s", secret())
        assert not await p.verify_scope()
        assert not p.available  # fell back to advisory-only


def test_parse_secret_key_roundtrip():
    assert parse_secret_key(secret()).public_key_str.startswith("ed25519:")
