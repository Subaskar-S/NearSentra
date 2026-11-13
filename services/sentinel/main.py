"""NearSentra Sentinel: FastAPI app exposing /health and /alerts, plus the monitor loop."""

from __future__ import annotations

import asyncio
import logging
import os
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from checkpoint import Checkpoint
from event_source import EventSource, NeardataEventSource, RpcEventSource
from models import Alert, SentinelConfig
from monitor import Monitor
from pauser import Pauser
from rpc import RpcClient

logging.basicConfig(
    level=os.environ.get("LOG_LEVEL", "INFO").upper(),
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
# httpx logs every RPC call at INFO (several per block); keep the log readable.
logging.getLogger("httpx").setLevel(logging.WARNING)
log = logging.getLogger("sentinel")


def build_source(config: SentinelConfig, rpc: RpcClient) -> EventSource:
    if config.event_source == "neardata":
        # The API key (optional, raises the rate limit) comes from the environment and is never logged.
        return NeardataEventSource(
            config.neardata_url,
            config.vault_account_id,
            api_key=os.environ.get("NEARDATA_API_KEY"),
            requests_per_minute=config.neardata_requests_per_minute,
        )
    return RpcEventSource(rpc, config.vault_account_id)


def create_app(monitor: Monitor | None = None, autostart: bool = True) -> FastAPI:
    """Build the app. Tests pass a prepared ``monitor`` and ``autostart=False``."""
    state: dict[str, object] = {"monitor": monitor, "task": None, "stop": asyncio.Event}

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        mon: Monitor | None = state["monitor"]  # type: ignore[assignment]
        stop = asyncio.Event()
        task: asyncio.Task[None] | None = None
        if mon is None:
            config = SentinelConfig.from_env()
            rpc = RpcClient(config.rpc_url)
            # Only the Sentinel's own scoped key is read here; never an admin or agent key.
            pauser = Pauser(
                rpc,
                config.vault_account_id,
                os.environ.get("SENTINEL_ACCOUNT_ID"),
                os.environ.get("SENTINEL_PRIVATE_KEY"),
            )
            if pauser.available:
                await pauser.verify_scope()
            checkpoint = Checkpoint(config.checkpoint_path, config.vault_account_id) if config.checkpoint_path else None
            mon = Monitor(config, rpc, pauser, source=build_source(config, rpc), checkpoint=checkpoint)
            state["monitor"] = mon
            log.info("event source: %s, checkpoint: %s", mon.source.name, config.checkpoint_path or "disabled")
        if autostart:
            task = asyncio.create_task(mon.run(stop))
            state["task"] = task
        try:
            yield
        finally:
            stop.set()
            if task is not None:
                await task
            await mon.source.aclose()
            await mon.rpc.aclose()

    app = FastAPI(title="NearSentra Sentinel", lifespan=lifespan)
    origins = (monitor.config.cors_origins if monitor else SentinelConfig.from_env().cors_origins)
    app.add_middleware(CORSMiddleware, allow_origins=origins, allow_methods=["GET"], allow_headers=[])

    @app.get("/health")
    async def health() -> JSONResponse:
        mon: Monitor | None = state["monitor"]  # type: ignore[assignment]
        task = state["task"]
        crashed = isinstance(task, asyncio.Task) and task.done()
        if mon is None or crashed or mon.interrupted:
            return JSONResponse({"status": "unhealthy"}, status_code=503)
        if mon.lagging:
            # Behind the chain head by more than MAX_LAG_BLOCKS: protection is late, say so.
            body = {"status": "unhealthy", "reason": "lagging", "lag_blocks": mon.lag_blocks}
            return JSONResponse(body, status_code=503)
        return JSONResponse({"status": "healthy"})

    @app.get("/status")
    async def status() -> JSONResponse:
        mon: Monitor | None = state["monitor"]  # type: ignore[assignment]
        if mon is None:
            return JSONResponse({"running": False})
        return JSONResponse(
            {
                "running": mon.running,
                "source": mon.source.name,
                "last_height": mon.last_height,
                "head_height": mon.head_height,
                "lag_blocks": mon.lag_blocks,
                "max_lag_blocks": mon.config.max_lag_blocks,
                "interrupted": mon.interrupted,
                "checkpoint": mon.checkpoint is not None,
                "duplicates_dropped": mon.duplicates_dropped,
            }
        )

    @app.get("/alerts", response_model=list[Alert])
    async def alerts() -> list[Alert]:
        mon: Monitor | None = state["monitor"]  # type: ignore[assignment]
        return mon.recent_alerts() if mon else []

    return app


app = create_app()
