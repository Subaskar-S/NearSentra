#!/usr/bin/env bash
# Run the six NearSentra simulations (five attacks and an agent-to-agent payment) against a throwaway LOCAL sandbox.
#
# Starts the local network (contracts/vault/examples/localnet.rs), then the Sentinel
# with only its own scoped key, then the simulation runner. Everything started here is
# stopped on exit and the script exits with the runner's exit code.
# Never touches testnet or mainnet.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VAULT_DIR="$ROOT/contracts/vault"
SENTINEL_DIR="$ROOT/services/sentinel"
LOCALNET_DIR="$ROOT/.localnet"
ENV_FILE="$LOCALNET_DIR/localnet.env"
SENTINEL_PORT="${SENTINEL_PORT:-8080}"
VELOCITY_THRESHOLD="${VELOCITY_THRESHOLD:-4}"
LOCALNET_PID=""
SENTINEL_PID=""

mkdir -p "$LOCALNET_DIR"

cleanup() {
  local code=$?
  trap - EXIT INT TERM
  for pid in "$SENTINEL_PID" "$LOCALNET_PID"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  exit "$code"
}
trap cleanup EXIT INT TERM

fail() {
  echo "run-simulation: $*" >&2
  exit 1
}

SENTINEL_PY="$SENTINEL_DIR/.venv/bin/python"
[ -x "$SENTINEL_PY" ] || fail "Sentinel venv missing. Create it: cd services/sentinel && uv venv --python 3.12 .venv && uv pip install -r requirements.txt"
[ -d "$ROOT/simulation/node_modules" ] || fail "simulation dependencies missing. Run: cd simulation && npm install"
if curl -fsS "http://127.0.0.1:$SENTINEL_PORT/health" >/dev/null 2>&1; then
  fail "port $SENTINEL_PORT is already in use; set SENTINEL_PORT to a free port"
fi

echo "==> building and starting the local network"
(cd "$VAULT_DIR" && cargo build --quiet --example localnet)
rm -f "$ENV_FILE"
# Run from the crate dir: the launcher builds the contract wasm relative to it.
(cd "$VAULT_DIR" && exec ./target/debug/examples/localnet) >"$LOCALNET_DIR/localnet.log" 2>&1 &
LOCALNET_PID=$!

for _ in $(seq 1 240); do
  [ -f "$ENV_FILE" ] && break
  kill -0 "$LOCALNET_PID" 2>/dev/null || { cat "$LOCALNET_DIR/localnet.log" >&2; fail "local network exited early"; }
  sleep 0.5
done
[ -f "$ENV_FILE" ] || fail "local network did not become ready in time (see $LOCALNET_DIR/localnet.log)"

set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
echo "    vault: $VAULT_ACCOUNT_ID   rpc: $NEAR_RPC_URL"

echo "==> starting the Sentinel (velocity threshold $VELOCITY_THRESHOLD, scoped pause key only)"
# Explicit environment: the Sentinel never sees admin, agent or sentinel-owner keys.
env -i PATH="$PATH" HOME="$HOME" \
  NEAR_RPC_URL="$NEAR_RPC_URL" \
  VAULT_ACCOUNT_ID="$VAULT_ACCOUNT_ID" \
  SENTINEL_ACCOUNT_ID="$SENTINEL_ACCOUNT_ID" \
  SENTINEL_PRIVATE_KEY="$SENTINEL_PRIVATE_KEY" \
  VELOCITY_THRESHOLD="$VELOCITY_THRESHOLD" \
  VELOCITY_WINDOW_SECS=60 \
  POLL_INTERVAL_SECS=1 \
  LOG_LEVEL=INFO \
  "$SENTINEL_PY" -m uvicorn main:app --app-dir "$SENTINEL_DIR" --host 127.0.0.1 --port "$SENTINEL_PORT" \
  >"$LOCALNET_DIR/sentinel.log" 2>&1 &
SENTINEL_PID=$!

for _ in $(seq 1 60); do
  curl -fsS "http://127.0.0.1:$SENTINEL_PORT/health" >/dev/null 2>&1 && break
  kill -0 "$SENTINEL_PID" 2>/dev/null || { cat "$LOCALNET_DIR/sentinel.log" >&2; fail "Sentinel exited early"; }
  sleep 0.5
done
curl -fsS "http://127.0.0.1:$SENTINEL_PORT/health" >/dev/null 2>&1 || fail "Sentinel did not become healthy"
sleep 2 # let the monitor record its starting block

echo "==> running the attack simulations"
set +e
(cd "$ROOT/simulation" && SENTINEL_URL="http://127.0.0.1:$SENTINEL_PORT" VELOCITY_THRESHOLD="$VELOCITY_THRESHOLD" npx tsx src/runner.ts)
RUNNER_CODE=$?
set -e

if [ "$RUNNER_CODE" -ne 0 ]; then
  echo "==> simulation FAILED (exit $RUNNER_CODE). Last Sentinel log lines:" >&2
  tail -n 25 "$LOCALNET_DIR/sentinel.log" >&2 || true
fi
exit "$RUNNER_CODE"
