"""Golden test: bytes below were produced by this module AND verified identical to
near-api-js v7 (encodeTransaction + ed25519 signature) for the same inputs. The secret
key is a throwaway test vector (bytes 0..31), not a real key."""

import hashlib
import json

import pytest

from nearcrypto import (
    b58decode,
    b58encode,
    encode_function_call_transaction,
    parse_secret_key,
    sign_transaction,
)

# Throwaway TEST VECTOR for the sandbox account sentinel.sandbox. It guards no funds on any network;
# it only pins our signer's output to the bytes near-api-js produces. Never use it for anything else.
SECRET = "ed25519:1GMkH3brNXiNNs1tiFZHu4yZSRrzJwxi5wB9bHFtMikjwpAW9DMZzU2Pqakc5it8X3N5vPmqdN7KF4CCUpmKhq"
GOLD_TX = "1000000073656e74696e656c2e73616e64626f780003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b807000000000000000d0000007661756c742e73616e64626f78496aca80e4d8f29fb8e8cd816c3afb48d3f103970b3a2ee1600c08ca67326dee01000000020b00000070617573655f6167656e741c0000007b226167656e745f6964223a226167656e742e73616e64626f78227d00e057eb481b000000000000000000000000000000000000"
GOLD_SIGNED = "EAAAAHNlbnRpbmVsLnNhbmRib3gAA6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbgHAAAAAAAAAA0AAAB2YXVsdC5zYW5kYm94SWrKgOTY8p+46M2BbDr7SNPxA5cLOi7hYAwIymcybe4BAAAAAgsAAABwYXVzZV9hZ2VudBwAAAB7ImFnZW50X2lkIjoiYWdlbnQuc2FuZGJveCJ9AOBX60gbAAAAAAAAAAAAAAAAAAAAAAAAALVNt0Tla9TmZLCgZYST1zCKILL7cBQY/xj1Ai8P5nrXiNNXKHXvpZANQ2rXEBroB53tz8mtcI2atUowgdJGVwE="
GOLD_HASH = "FggmBBXUpjF8F4EmRPZT9ogzDZ3pBq5Fc45RWuUFHzXE"


def _tx(key):
    return encode_function_call_transaction(
        signer_id="sentinel.sandbox",
        public_key=key.public_bytes,
        nonce=7,
        receiver_id="vault.sandbox",
        block_hash=hashlib.sha256(b"block").digest(),
        method_name="pause_agent",
        args=json.dumps({"agent_id": "agent.sandbox"}, separators=(",", ":")).encode(),
        gas=30_000_000_000_000,
    )


def test_encoding_matches_near_api_js():
    assert _tx(parse_secret_key(SECRET)).hex() == GOLD_TX


def test_signature_and_hash_match_near_api_js():
    key = parse_secret_key(SECRET)
    signed = sign_transaction(key, _tx(key))
    assert signed.base64 == GOLD_SIGNED
    assert signed.tx_hash == GOLD_HASH


@pytest.mark.parametrize("data", [b"", b"\x00", b"\x00\x00ab", bytes(range(40)), b"\xff" * 32])
def test_base58_roundtrip(data):
    assert b58decode(b58encode(data)) == data


def test_rejects_malformed_keys():
    with pytest.raises(ValueError):
        parse_secret_key("secp256k1:abc")
    with pytest.raises(ValueError):
        parse_secret_key("ed25519:" + b58encode(b"short"))
    with pytest.raises(ValueError):
        b58decode("0OIl")  # characters outside the alphabet
