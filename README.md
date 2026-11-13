# NearSentra

Programmable security infrastructure for autonomous AI agents on NEAR Protocol.

An agent never holds funds. It asks a **vault contract** to pay someone, and the contract checks the request against
an on-chain policy (allowlist, per-transaction limit, per-epoch budget, expiry, pause/revoke state) before any money moves.
The contract is the final authority; the other components add visibility and early warning:

| Component | Path | Role |
|---|---|---|
| Vault contract (Rust, near-sdk 5) | `contracts/vault` | Enforces every policy and holds the funds |
| Gateway (TypeScript, Fastify) | `services/gateway` | HTTP front door for agents; signs with a key that can only call `request_transfer` |
| Sentinel (Python, FastAPI) | `services/sentinel` | Watches events, pauses an agent that floods requests; its key can only call `pause_agent` |
| Dashboard (React, Vite) | `frontend` | Agent status, transfer feed, alerts, wallet-signed pause/revoke |
| Attack simulations (TypeScript) | `simulation` | Five scenarios that run real transactions on a local network |

Read next: [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) (16 attack scenarios) and [`docs/SECURITY_TESTS.md`](docs/SECURITY_TESTS.md) (contract audit).

## Quick Start

Everything below runs on a **local sandbox**. Nothing touches testnet or mainnet.

### Prerequisites

- Rust (version pinned by `contracts/vault/rust-toolchain.toml`) with the `wasm32-unknown-unknown` target, and [`cargo-near`](https://github.com/near/cargo-near)
- Node.js 22.22+ or 24+ and npm
- Python 3.11+ (macOS ships 3.9; pass `PYTHON=/opt/homebrew/bin/python3` to `make install` if needed)
- No funded accounts or deployed contract address are needed: the sandbox creates funded accounts and deploys the vault itself. The vault account is always `vault.sandbox`.
- For wallet-signed dashboard actions you would need a testnet deployment and an admin wallet; the local dashboard is read-only.

### 1. Install dependencies

```bash
make install
```

Expected final line:

```
Dependencies installed.
```

### 2. Run the contract tests

```bash
make test
```

Runs 79 unit tests and 52 sandbox integration tests. Expected final line:

```
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

(That is the empty doc-test run, which comes last. `make` exits 0 only if every test passed.)

### 3. Start the local network (sandbox)

```bash
make sandbox-deploy
```

Deploys the vault, creates the admin, Sentinel, recipient, attacker and five agent accounts, writes their sandbox-only keys to
`.localnet/localnet.env` (git-ignored), and keeps running. Leave it running in its own terminal. Expected final line:

```
LOCALNET_READY /path/to/NearSentra/.localnet/localnet.env
```

Add `ONCE=1` (`make sandbox-deploy ONCE=1`) to deploy, print the vault account ID, and exit.

### 4. Run the attack simulations

```bash
make simulate
```

This starts its own throwaway network and Sentinel, so it does not need step 3. It runs five scenarios (normal agent, rogue agent,
runaway agent stopped by the Sentinel, compromised Sentinel, revoked agent) as real transactions, and prints the 64-character hex
transaction hash, the status (`SUCCESS`/`FAILURE`/`UNKNOWN`) and the full error text for every attempt. The exit code is 0 only if every
scenario produced the expected on-chain outcome. Expected final line:

```
All 5 scenarios produced the expected on-chain outcome.
```

### 5. Start all services

With the network from step 3 running, in another terminal:

```bash
make services
```

Starts the Gateway (`http://127.0.0.1:3000`), the Sentinel (`http://127.0.0.1:8080`) and the dashboard (`http://localhost:5173`).
Each service receives only its own scoped key. Expected final line:

```
All services started. Press Ctrl-C to stop.
```

Try the gateway (after registering an agent as admin, for example with the simulation helpers):

```bash
curl -s -X POST localhost:3000/transfer -H 'content-type: application/json' \
  -d '{"agent_id":"agent.sandbox","recipient":"recipient.sandbox","amount":"500000000000000000000000","request_id":"demo-1"}'
```

## Other commands

```bash
make lint    # cargo clippy -D warnings, tsc --noEmit (gateway, simulation, dashboard), ruff
(cd services/gateway && npx vitest run)                       # gateway tests
(cd services/sentinel && .venv/bin/python -m pytest -q)       # sentinel tests
```

## Gateway authentication and key custody

The Gateway signs for agents with their function-call keys (limited to `request_transfer` on the vault), so it is a
custodial component. It refuses to start unless every agent has its own `AGENT_<n>_API_TOKEN` (32+ characters, unique),
and `POST /transfer` needs `Authorization: Bearer <that agent's token>`. `GATEWAY_ALLOW_UNAUTHENTICATED=true` skips this
for a local sandbox and is refused when `HOST` is not loopback (`make services` sets it). Whoever steals a Gateway key
still gets only the agent's policy: per-transaction limit, epoch budget, allowlist, one pending transfer at a time. Agent-side
signing (the agent keeps its key and calls the vault directly) avoids the custody risk and works without the Gateway.

## Sentinel event source

`SENTINEL_EVENT_SOURCE=rpc` (default) walks blocks over JSON-RPC and is meant for the local sandbox or a node you run.
For testnet use `neardata`, which needs a FastNEAR API key to keep up. Set `CHECKPOINT_PATH` to resume after a restart.
`/status` shows lag and `/health` turns 503 when the Sentinel is behind. See [`docs/SENTINEL_INGESTION.md`](docs/SENTINEL_INGESTION.md).

## Dashboard configuration

Copy `frontend/.env.example` to `frontend/.env.local`. `VITE_NEAR_RPC_URL` and `VITE_VAULT_ACCOUNT_ID` choose the chain and vault,
`VITE_SENTINEL_URL` the Sentinel, and `VITE_AGENT_IDS` the agents to watch (the vault has no "list agents" method; agents seen in
events are added automatically). The dashboard holds no keys: pause/revoke/unpause are signed in the connected wallet with the
admin account, and each call attaches 1 yoctoNEAR so the wallet asks for a full-access confirmation.

## Security notes

- Admin methods need the admin account **and** exactly 1 yoctoNEAR. Only a full-access key can attach it.
- Agent keys must be function-call keys limited to `request_transfer` on the vault; the Sentinel key must be limited to `pause_agent`.
  `make sandbox-deploy` creates them this way, and the Sentinel refuses to act with a broader key.
- Never commit keys. `.env`, `*.env`, `*.key`, `*.pem`, `.near-credentials/` and keystores are git-ignored.
- Sandbox and testnet only. Do not point this project at mainnet funds before a real audit.

## Tracks

- **security** — AI Security & Defense (NEAR Hackathon 2026)
