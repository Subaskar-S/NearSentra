"""Persists the last fully processed block so a restart resumes instead of skipping ahead.

The block hash is stored with the height. On load the Sentinel re-reads that block and only trusts the
checkpoint if the hash matches, so a checkpoint left over from another chain (a restarted local sandbox,
a different network) is ignored instead of being replayed against the wrong history.
"""

from __future__ import annotations

import json
import logging
import os
import tempfile
from pathlib import Path

log = logging.getLogger("sentinel.checkpoint")


class Checkpoint:
    def __init__(self, path: str, vault_id: str) -> None:
        self._path = Path(path)
        self._vault_id = vault_id

    def load(self) -> tuple[int, str] | None:
        """Return ``(height, block_hash)`` or ``None`` if there is no usable checkpoint."""
        try:
            doc = json.loads(self._path.read_text())
        except FileNotFoundError:
            return None
        except (OSError, ValueError) as exc:
            log.warning("ignoring unreadable checkpoint %s: %s", self._path, exc)
            return None
        if not isinstance(doc, dict) or doc.get("vault") != self._vault_id:
            log.warning("ignoring checkpoint %s: it belongs to another vault", self._path)
            return None
        height, block_hash = doc.get("height"), doc.get("block_hash")
        if not isinstance(height, int) or height < 0 or not isinstance(block_hash, str) or not block_hash:
            log.warning("ignoring malformed checkpoint %s", self._path)
            return None
        return height, block_hash

    def save(self, height: int, block_hash: str) -> None:
        """Write atomically (temp file + rename): a crash never leaves a half-written checkpoint."""
        payload = json.dumps({"vault": self._vault_id, "height": height, "block_hash": block_hash})
        try:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            fd, tmp = tempfile.mkstemp(dir=self._path.parent, prefix=self._path.name, suffix=".tmp")
            try:
                with os.fdopen(fd, "w") as fh:
                    fh.write(payload)
                os.replace(tmp, self._path)
            except BaseException:
                Path(tmp).unlink(missing_ok=True)
                raise
        except OSError as exc:
            # Losing a checkpoint costs a longer catch-up after a restart; it must never stop monitoring.
            log.error("could not save checkpoint %s: %s", self._path, exc)
