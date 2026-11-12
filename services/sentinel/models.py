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
    pattern: Literal["velocity_exceeded", "consecutive_failures"]
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
        }
        values: dict[str, object] = {dst: env[src] for src, dst in mapping.items() if src in env}
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
