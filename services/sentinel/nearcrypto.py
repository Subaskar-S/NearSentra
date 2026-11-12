"""Minimal NEAR transaction signing: base58, Borsh encoding, ed25519.

Only what the Sentinel needs: build and sign a single ``FunctionCall`` transaction
with a function-call access key. No third-party NEAR SDK is required.
"""

from __future__ import annotations

import base64
import hashlib
import struct
from dataclasses import dataclass

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

_B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_B58_INDEX = {c: i for i, c in enumerate(_B58_ALPHABET)}

ED25519_KEY_TYPE = 0
ACTION_FUNCTION_CALL = 2


def b58encode(data: bytes) -> str:
    n = int.from_bytes(data, "big")
    out = ""
    while n > 0:
        n, rem = divmod(n, 58)
        out = _B58_ALPHABET[rem] + out
    pad = len(data) - len(data.lstrip(b"\x00"))
    return "1" * pad + out


def b58decode(text: str) -> bytes:
    n = 0
    for ch in text:
        if ch not in _B58_INDEX:
            raise ValueError("invalid base58 character")
        n = n * 58 + _B58_INDEX[ch]
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    pad = len(text) - len(text.lstrip("1"))
    return b"\x00" * pad + body


@dataclass(frozen=True)
class KeyPair:
    private: Ed25519PrivateKey
    public_bytes: bytes

    @property
    def public_key_str(self) -> str:
        return "ed25519:" + b58encode(self.public_bytes)


def parse_secret_key(secret: str) -> KeyPair:
    """Parse ``ed25519:<base58>`` (64-byte seed||public, or a bare 32-byte seed)."""
    if not secret.startswith("ed25519:"):
        raise ValueError("secret key must start with 'ed25519:'")
    raw = b58decode(secret[len("ed25519:") :])
    if len(raw) not in (32, 64):
        raise ValueError("ed25519 secret key must decode to 32 or 64 bytes")
    private = Ed25519PrivateKey.from_private_bytes(raw[:32])
    public = private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    if len(raw) == 64 and raw[32:] != public:
        raise ValueError("ed25519 secret key is inconsistent")
    return KeyPair(private=private, public_bytes=public)


# ── Borsh primitives ─────────────────────────────────────────────────────────


def _u32(v: int) -> bytes:
    return struct.pack("<I", v)


def _u64(v: int) -> bytes:
    return struct.pack("<Q", v)


def _u128(v: int) -> bytes:
    return v.to_bytes(16, "little")


def _string(s: str) -> bytes:
    raw = s.encode()
    return _u32(len(raw)) + raw


def _bytes(b: bytes) -> bytes:
    return _u32(len(b)) + b


def encode_function_call_transaction(
    *,
    signer_id: str,
    public_key: bytes,
    nonce: int,
    receiver_id: str,
    block_hash: bytes,
    method_name: str,
    args: bytes,
    gas: int,
    deposit: int = 0,
) -> bytes:
    """Borsh-encode a transaction containing exactly one FunctionCall action."""
    if len(block_hash) != 32 or len(public_key) != 32:
        raise ValueError("block_hash and public_key must be 32 bytes")
    action = (
        bytes([ACTION_FUNCTION_CALL])
        + _string(method_name)
        + _bytes(args)
        + _u64(gas)
        + _u128(deposit)
    )
    return (
        _string(signer_id)
        + bytes([ED25519_KEY_TYPE])
        + public_key
        + _u64(nonce)
        + _string(receiver_id)
        + block_hash
        + _u32(1)  # one action
        + action
    )


@dataclass(frozen=True)
class SignedTransaction:
    base64: str
    tx_hash: str  # base58 sha256 of the encoded transaction


def sign_transaction(key: KeyPair, encoded_tx: bytes) -> SignedTransaction:
    digest = hashlib.sha256(encoded_tx).digest()
    signature = key.private.sign(digest)
    signed = encoded_tx + bytes([ED25519_KEY_TYPE]) + signature
    return SignedTransaction(base64=base64.b64encode(signed).decode(), tx_hash=b58encode(digest))
