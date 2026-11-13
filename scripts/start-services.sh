#!/usr/bin/env bash
# Start the Gateway, the Sentinel and the dashboard against the local network started by
# `make sandbox-deploy` (reads .localnet/localnet.env). Ctrl-C stops everything.
#
# Each process gets only the keys it needs: the Gateway the agents' request_transfer keys,
# the Sentinel its pause_agent key. No admin key is ever passed to a service, and the
# dashboard holds no keys at all (the wallet signs).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.localnet/localnet.env"
GATEWAY_PORT="${GATEWAY_PORT:-3000}"
SENTINEL_PORT="${SENTINEL_PORT:-8080}"
PIDS=()

[ -f "$ENV_FILE" ] || { echo "start-services: $ENV_FILE not found. Run 'make sandbox-deploy' in another terminal first." >&2; exit 1; }
SENTINEL_PY="$ROOT/services/sentinel/.venv/bin/python"
[ -x "$SENTINEL_PY" ] || { echo "start-services: run 'make install' first." >&2; exit 1; }

cleanup() {
  trap - EXIT INT TERM
  for pid in "${PIDS[@]:-}"; do
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; fi
  done
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a

# The local sandbox gateway listens on loopback only, so it may run without per-agent API tokens.
# For anything else set AGENT_<n>_API_TOKEN instead (the gateway refuses to start unauthenticated
# on a non-loopback HOST).
GATEWAY_ENV=(NEAR_RPC_URL="$NEAR_RPC_URL" VAULT_ACCOUNT_ID="$VAULT_ACCOUNT_ID" HOST=127.0.0.1 PORT="$GATEWAY_PORT" GATEWAY_ALLOW_UNAUTHENTICATED=true)
AGENT_IDS=""
for n in 1 2 3 4 5; do
  id_var="AGENT_${n}_ACCOUNT_ID"
  key_var="AGENT_${n}_PRIVATE_KEY"
  GATEWAY_ENV+=("$id_var=${!id_var}" "$key_var=${!key_var}")
  AGENT_IDS="${AGENT_IDS:+$AGENT_IDS,}${!id_var}"
done

echo "==> gateway on http://127.0.0.1:$GATEWAY_PORT"
(cd "$ROOT/services/gateway" && exec env -i PATH="$PATH" HOME="$HOME" "${GATEWAY_ENV[@]}" npx tsx src/index.ts) &
PIDS+=($!)

echo "==> sentinel on http://127.0.0.1:$SENTINEL_PORT"
(cd "$ROOT/services/sentinel" && exec env -i PATH="$PATH" HOME="$HOME" \
  NEAR_RPC_URL="$NEAR_RPC_URL" VAULT_ACCOUNT_ID="$VAULT_ACCOUNT_ID" \
  SENTINEL_ACCOUNT_ID="$SENTINEL_ACCOUNT_ID" SENTINEL_PRIVATE_KEY="$SENTINEL_PRIVATE_KEY" \
  "$SENTINEL_PY" -m uvicorn main:app --host 127.0.0.1 --port "$SENTINEL_PORT") &
PIDS+=($!)

echo "==> dashboard on http://localhost:5173 (read-only against the local network)"
(cd "$ROOT/frontend" && exec env -i PATH="$PATH" HOME="$HOME" \
  VITE_NEAR_RPC_URL="$NEAR_RPC_URL" VITE_VAULT_ACCOUNT_ID="$VAULT_ACCOUNT_ID" \
  VITE_SENTINEL_URL="http://127.0.0.1:$SENTINEL_PORT" VITE_AGENT_IDS="$AGENT_IDS" \
  npx vite --host 127.0.0.1 --port 5173) &
PIDS+=($!)

echo "All services started. Press Ctrl-C to stop."
wait
