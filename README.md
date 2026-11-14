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
| Simulations (TypeScript) | `simulation` | Five attack scenarios and an agent-to-agent payment that run real transactions on a local network |

Read next: [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) (17 attack scenarios) and [`docs/SECURITY_TESTS.md`](docs/SECURITY_TESTS.md) (contract audit).

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

Runs 79 unit tests and 53 sandbox integration tests. Expected final line:

```
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

(That is the empty doc-test run, which comes last. `make` exits 0 only if every test passed.)

### 3. Start the local network (sandbox)

```bash
make sandbox-deploy
```

Deploys the vault, creates the admin, Sentinel, recipient, attacker, five agent accounts, plus `research.sandbox` (a sixth agent) and
`analysis.sandbox` (its payment recipient) for the agent-to-agent demo, writes their sandbox-only keys to
`.localnet/localnet.env` (git-ignored), and keeps running. Leave it running in its own terminal. Expected final line:

```
LOCALNET_READY /path/to/NearSentra/.localnet/localnet.env
```

Add `ONCE=1` (`make sandbox-deploy ONCE=1`) to deploy, print the vault account ID, and exit.

### 4. Run the attack simulations

```bash
make simulate
```

This starts its own throwaway network and Sentinel, so it does not need step 3. It runs six scenarios (normal agent, rogue agent,
runaway agent stopped by the Sentinel, compromised Sentinel, revoked agent, [agent-to-agent payment](#agent-to-agent-payment-demo)) as real transactions, and prints the 64-character hex
transaction hash, the status (`SUCCESS`/`FAILURE`/`UNKNOWN`) and the full error text for every attempt. The exit code is 0 only if every
scenario produced the expected on-chain outcome. Expected final line:

```
All 6 scenarios produced the expected on-chain outcome.
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

## Agent-to-agent payment demo

ResearchAgent (`research.sandbox`) buys a 2 NEAR market-analysis service from AnalysisAgent (`analysis.sandbox`). Nothing new is
trusted: ResearchAgent holds only a `request_transfer` key, the Gateway just forwards the request, and the vault contract decides.
The vault takes the agent's identity from the on-chain caller, never from a request field, and reserves the amount before it pays, so
a transfer settles exactly once. ResearchAgent's policy: 2 NEAR per payment, 3 NEAR per epoch, only `analysis.sandbox` allowlisted.

Run it through the Gateway so the dashboard shows it. Use three terminals, each from the repository root:

```bash
make sandbox-deploy        # terminal 1, leave running (wait for LOCALNET_READY)
make services              # terminal 2, leave running (wait for "All services started")
make demo-agent-payment    # terminal 3
```

Open `http://localhost:5173` (the dashboard) while or after step 3 runs. Expected result, one line per request with the real transaction hash:

| Step | Request | Expected |
|---|---|---|
| 1 | `research` pays 2 NEAR to `analysis` | `SUCCESS`. `analysis.sandbox` gains exactly 2 NEAR and the reservation is settled |
| 2 | Seller now asks 2.5 NEAR | `FAILURE` `ERR_PER_TX_LIMIT` |
| 3 | A second 2 NEAR purchase (2 + 2 > 3) | `FAILURE` `ERR_EPOCH_LIMIT` |
| 4 | Prompt-injected payment to `attacker.sandbox` | `FAILURE` `ERR_RECIPIENT_NOT_ALLOWED` |
| 5 | `attacker.sandbox` signs a payment with its own key | `FAILURE` `ERR_AGENT_NOT_REGISTERED` (identity comes from the chain) |
| 6 | Admin pauses `research`, then it pays 0.1 NEAR | pause `SUCCESS`, payment `FAILURE` `ERR_AGENT_PAUSED` |

The final line is `Agent-to-agent demo finished: every payment had the expected on-chain outcome.` The dashboard shows one approved 2 NEAR
transfer and five rejected ones with their reasons in the transfer feed, and `research.sandbox` as `paused` in the agent list. The run
changes the state of the local network, so for a second run stop both terminals and start again from `make sandbox-deploy`.

Without the Gateway or dashboard, `make simulate` runs the same steps as scenario 6 with the agent signing directly. The contract-level
version is `contracts/vault/tests/test_agent_to_agent.rs` (part of `make test`). The demo uses the admin key only to register the
agent and to pause it, like the other simulation scenarios; no service or the dashboard receives it.

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

## NEAR tools used

| NEAR feature | How NearSentra uses it |
|---|---|
| **Function-call access keys** | Each agent's runtime key can only call `request_transfer` on the vault, and the Sentinel's key can only call `pause_agent`. The Sentinel checks its own key scope at startup and refuses to act with a broader key. |
| **1 yoctoNEAR full-access gate** | Admin methods (register, update, revoke, unpause) need exactly 1 yoctoNEAR, which only a full-access key can attach. A leaked function-call key on the admin account cannot change policy. |
| **Smart contract in Rust (`near-sdk` 5)** | The vault holds funds and enforces the policy. It reserves each amount before the payout, then settles it in a cross-contract callback (promise chaining), and rolls back if the payout fails. |
| **NEP-297 events** | The vault logs every approved and rejected request as a structured event. The Sentinel and the dashboard read these events from the chain. |
| **Named accounts and sub-accounts** | Every actor is a named account (`vault.sandbox`, `research.sandbox`, and so on), so the policy allowlist reads as account IDs. |
| **NEAR RPC and `near-api-js` 7** | The Gateway, simulations and dashboard submit transactions and read state over JSON-RPC. |
| **`near-sandbox` and `near-api` (Rust)** | The contract's 53 integration tests and the local network run on a real sandbox node. |
| **`@hot-labs/near-connect`** | The dashboard uses it so the admin can sign pause, revoke and unpause in a wallet. The dashboard holds no keys. |

Not used: NEAR Intents, Chain Signatures, NEAR AI Cloud and TEEs. The vault moves native NEAR only. Fungible-token and cross-chain
payments are future work.

## Tracks

- **security** — AI Security & Defense (NEAR Hackathon 2026)
