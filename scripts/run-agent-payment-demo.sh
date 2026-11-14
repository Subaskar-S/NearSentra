#!/usr/bin/env bash
# Run the agent-to-agent payment demo against the LOCAL network that `make sandbox-deploy` started,
# sending ResearchAgent's requests through the running Gateway (`make services`). Local sandbox only.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.localnet/localnet.env"

[ -f "$ENV_FILE" ] || { echo "run-agent-payment-demo: $ENV_FILE not found. Run 'make sandbox-deploy' in another terminal first." >&2; exit 1; }
[ -d "$ROOT/simulation/node_modules" ] || { echo "run-agent-payment-demo: run 'make install' first." >&2; exit 1; }

set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a

cd "$ROOT/simulation"
exec npx tsx src/agent_payment_demo.ts
