"""Pydantic v2 models for the NearSentra Sentinel."""

from __future__ import annotations

import os
from collections.abc import Mapping
from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field


class Alert(BaseModel):
    """One anomaly-detection record (Requirement 11.4)."""

    agent_id: str
    pattern: Literal["velocity_exceeded", "consecutive_failures", "ingestion_gap"]
    threshold: int | float
    observed: int | float
    action: Literal["pause_attempted", "logged_only", "advisory_only"]
    timestamp_utc: datetime


class SentinelConfig(BaseModel):
    """Detection thresholds (Requirements 11.1, 11.3) and runtime settings."""

    velocity_threshold: int = Field(default=20, ge=1, le=10000)
    velocity_window_secs: int = Field(default=60, ge=10, le=3600)
    failure_threshold: int = Field(default=5, ge=1, le=100)
    failure_window_secs: int = Field(default=300, ge=1)

    # Connection settings (secrets are read separately and never stored here).
    rpc_url: str = "http://127.0.0.1:3030"
    vault_account_id: str = "vault.sandbox"
    poll_interval_secs: float = Field(default=1.0, gt=0, le=10)
    reconnect_interval_secs: float = Field(default=5.0, gt=0, le=10)
    stream_timeout_secs: float = Field(default=30.0, gt=0)
    # Where chain events come from. "rpc" walks blocks, chunks and transactions over plain JSON-RPC
    # (fine for a local sandbox or a private node). "neardata" reads one JSON document per block from a
    # FastNEAR neardata server (what a public testnet/mainnet RPC cannot sustain, see docs/SENTINEL_INGESTION.md).
    event_source: Literal["rpc", "neardata"] = "rpc"
    neardata_url: str = "https://testnet.neardata.xyz"
    # Client-side request budget for neardata. Measured sustainable free-tier rate is ~45/minute (see
    # event_source.RateLimiter); raise this only together with NEARDATA_API_KEY.
    neardata_requests_per_minute: int = Field(default=40, ge=10, le=100_000)
    # Where the last processed block is saved so a restart resumes instead of skipping ahead.
    checkpoint_path: str | None = None
    # How many blocks are fetched at once (they are still applied strictly in order).
    fetch_concurrency: int = Field(default=4, ge=1, le=32)
    # /health turns unhealthy when the Sentinel is this many blocks behind the chain head.
    max_lag_blocks: int = Field(default=120, ge=1)
    # A restart further behind than this skips ahead and records an `ingestion_gap` alert.
    max_catchup_blocks: int = Field(default=10_000, ge=1)
    cors_origins: list[str] = Field(
        default_factory=lambda: ["http://localhost:5173", "http://127.0.0.1:5173"]
    )

    @classmethod
    def from_env(cls, env: Mapping[str, str] | None = None) -> SentinelConfig:
        env = os.environ if env is None else env
        mapping = {
            "VELOCITY_THRESHOLD": "velocity_threshold",
            "VELOCITY_WINDOW_SECS": "velocity_window_secs",
            "FAILURE_THRESHOLD": "failure_threshold",
            "FAILURE_WINDOW_SECS": "failure_window_secs",
            "NEAR_RPC_URL": "rpc_url",
            "VAULT_ACCOUNT_ID": "vault_account_id",
            "POLL_INTERVAL_SECS": "poll_interval_secs",
            "RECONNECT_INTERVAL_SECS": "reconnect_interval_secs",
            "SENTINEL_EVENT_SOURCE": "event_source",
            "NEARDATA_URL": "neardata_url",
            "NEARDATA_REQUESTS_PER_MINUTE": "neardata_requests_per_minute",
            "CHECKPOINT_PATH": "checkpoint_path",
            "FETCH_CONCURRENCY": "fetch_concurrency",
            "MAX_LAG_BLOCKS": "max_lag_blocks",
            "MAX_CATCHUP_BLOCKS": "max_catchup_blocks",
        }
        values: dict[str, object] = {dst: env[src] for src, dst in mapping.items() if src in env}
        # The free neardata tier allows 180 requests/minute: poll the head less often by default.
        if values.get("event_source") == "neardata" and "poll_interval_secs" not in values:
            values["poll_interval_secs"] = 2.0
        if "CORS_ORIGINS" in env:
            values["cors_origins"] = [o.strip() for o in env["CORS_ORIGINS"].split(",") if o.strip()]
        return cls.model_validate(values)


class VaultEvent(BaseModel):
    """A NEP-297 ``nearsentra`` event extracted from a transaction log."""

    event: str
    agent_id: str | None = None
    decision: str | None = None
    timestamp_ns: int
    block_height: int
    # Stable identity of the log line (receipt id + index). Lets the Sentinel drop a duplicate delivery
    # of the same event (a retried block, a replay after reconnecting). None for hand-built events.
    uid: str | None = None
