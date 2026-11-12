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
            mon = Monitor(config, rpc, pauser)
            state["monitor"] = mon
        if autostart:
            task = asyncio.create_task(mon.run(stop))
            state["task"] = task
        try:
            yield
        finally:
            stop.set()
            if task is not None:
                await task
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
        return JSONResponse({"status": "healthy"})

    @app.get("/alerts", response_model=list[Alert])
    async def alerts() -> list[Alert]:
        mon: Monitor | None = state["monitor"]  # type: ignore[assignment]
        return mon.recent_alerts() if mon else []

    return app


app = create_app()
