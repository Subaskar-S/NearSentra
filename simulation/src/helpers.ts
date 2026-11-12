// Shared utilities for the NearSentra attack simulations.
//
// All key material is read from environment variables (see .localnet/localnet.env).
// Nothing here prints a secret, and no result line is ever hardcoded: every
// SUCCESS / FAILURE shown comes from a transaction outcome read from the chain.
import { Account, JsonRpcProvider, actions, baseDecode, nearToYocto, teraToGas } from "near-api-js";
import type { KeyPairString } from "near-api-js";

export type TxStatus = "SUCCESS" | "FAILURE" | "UNKNOWN";

export interface TxResult {
  /** 64-character lowercase hex of the 32-byte transaction hash (NEAR itself shows base58). */
  txHashHex: string;
  status: TxStatus;
  /** Full failure description when status is FAILURE, otherwise null. */
  error: string | null;
  logs: string[];
}

/** Policy as returned by the vault view method (U128 / U64 are decimal strings). */
export interface PolicyView {
  active: boolean;
  revoked: boolean;
  per_tx_limit: string;
  epoch_limit: string;
  spent_in_epoch: string;
  remaining_epoch_budget: string;
  allowlist: string[];
}

export interface Context {
  provider: JsonRpcProvider;
  vaultId: string;
  admin: Account;
  recipientId: string;
  attackerId: string;
  sentinelId: string;
  /** Agent n (1..5): account ID and a function-call account limited to request_transfer. */
  agents: Array<{ id: string; account: Account }>;
  /** Sentinel account signed with its leaked *full-access* key (worst case for scenario 4). */
  sentinelOwner: Account;
  /** Sentinel account signed with its real, scoped (pause_agent only) key. */
  sentinelScoped: Account;
  /** Sentinel threshold; scenario 3 sends threshold + 1 requests. */
  velocityThreshold: number;
  sentinelUrl: string | null;
}

/** Admin methods require exactly 1 yoctoNEAR, which forces a full-access key. */
export const ONE_YOCTO = 1n;
export const NEAR = (n: `${number}`): bigint => BigInt(nearToYocto(n));
export const HOUR_NS = 3_600_000_000_000n;

export function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing required environment variable ${name} (run via scripts/run-simulation.sh)`);
  return v;
}

function account(id: string, key: string, provider: JsonRpcProvider): Account {
  return new Account(id, provider, key as KeyPairString);
}

export function loadContext(): Context {
  const provider = new JsonRpcProvider({ url: requireEnv("NEAR_RPC_URL") });
  const sentinelId = requireEnv("SENTINEL_ACCOUNT_ID");
  const agents = [1, 2, 3, 4, 5].map((n) => {
    const id = requireEnv(`AGENT_${n}_ACCOUNT_ID`);
    return { id, account: account(id, requireEnv(`AGENT_${n}_PRIVATE_KEY`), provider) };
  });
  return {
    provider,
    vaultId: requireEnv("VAULT_ACCOUNT_ID"),
    admin: account(requireEnv("ADMIN_ACCOUNT_ID"), requireEnv("ADMIN_PRIVATE_KEY"), provider),
    recipientId: requireEnv("RECIPIENT_ACCOUNT_ID"),
    attackerId: requireEnv("ATTACKER_ACCOUNT_ID"),
    sentinelId,
    agents,
    sentinelOwner: account(sentinelId, requireEnv("SENTINEL_OWNER_PRIVATE_KEY"), provider),
    sentinelScoped: account(sentinelId, requireEnv("SENTINEL_PRIVATE_KEY"), provider),
    velocityThreshold: Number(process.env["VELOCITY_THRESHOLD"] ?? "4"),
    sentinelUrl: process.env["SENTINEL_URL"] ?? null,
  };
}

/** Convert a base58 NEAR transaction hash into 64 hex characters. */
export function base58ToHex(hash: string): string {
  const bytes = baseDecode(hash);
  if (bytes.length !== 32) throw new Error(`transaction hash has ${bytes.length} bytes, expected 32`);
  return Buffer.from(bytes).toString("hex");
}

interface RawOutcome {
  transaction_outcome: { id: string };
  status: unknown;
  receipts_outcome?: Array<{ outcome: { logs?: string[] } }>;
}

/** Turn a final execution outcome from the RPC into a {@link TxResult} without guessing. */
export function toTxResult(raw: unknown): TxResult {
  const o = raw as RawOutcome;
  const logs = (o.receipts_outcome ?? []).flatMap((r) => r.outcome.logs ?? []);
  const s = o.status as Record<string, unknown> | string | undefined;
  let status: TxStatus = "UNKNOWN";
  let error: string | null = null;
  if (s && typeof s === "object") {
    if ("SuccessValue" in s) status = "SUCCESS";
    else if ("Failure" in s) {
      status = "FAILURE";
      error = JSON.stringify(s["Failure"]);
    }
  }
  return { txHashHex: base58ToHex(o.transaction_outcome.id), status, error, logs };
}

/** Sign and submit a vault call, waiting for the FINAL outcome. Contract panics come back as FAILURE. */
export async function callVault(
  ctx: Context,
  signer: Account,
  method: string,
  args: Record<string, unknown>,
  deposit: bigint = 0n,
): Promise<TxResult> {
  const raw = await signer.signAndSendTransaction({
    receiverId: ctx.vaultId,
    actions: [actions.functionCall(method, args, teraToGas("60"), deposit)],
    waitUntil: "FINAL",
    throwOnFailure: false,
  });
  return toTxResult(raw);
}

export const requestTransfer = (ctx: Context, agent: Account, recipient: string, amount: bigint): Promise<TxResult> =>
  callVault(ctx, agent, "request_transfer", { recipient, amount: amount.toString() });

export async function getPolicy(ctx: Context, agentId: string): Promise<PolicyView | null> {
  const result = await ctx.provider.callFunction<PolicyView>({
    contractId: ctx.vaultId,
    method: "get_agent_policy",
    args: { agent_id: agentId },
  });
  return result ?? null;
}

/** Admin registers an agent with a 1-hour epoch and the given limits / allowlist. */
export async function registerAgent(
  ctx: Context,
  agentId: string,
  perTx: bigint,
  epoch: bigint,
  allowlist: string[],
): Promise<TxResult> {
  const res = await callVault(
    ctx,
    ctx.admin,
    "register_agent",
    {
      agent_id: agentId,
      params: {
        per_tx_limit: perTx.toString(),
        epoch_limit: epoch.toString(),
        epoch_duration_ns: HOUR_NS.toString(),
        allowlist,
        expiry_ns: null,
      },
    },
    ONE_YOCTO,
  );
  if (res.status !== "SUCCESS") {
    throw new Error(`setup failed: register_agent(${agentId}) -> ${res.status}: ${res.error ?? "no outcome"}`);
  }
  return res;
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Poll `predicate` until it returns true or `timeoutMs` elapses. Returns whether it became true. */
export async function waitFor(predicate: () => Promise<boolean>, timeoutMs: number, everyMs = 500): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await predicate()) return true;
    if (Date.now() >= deadline) return false;
    await sleep(everyMs);
  }
}

const HEX64 = /^[0-9a-f]{64}$/;

/**
 * Print exactly three fields for one on-chain attempt: the 64-char hex tx hash,
 * the status, and (only when FAILURE) the full error message.
 */
export function printScenarioResult(
  scenarioName: string,
  txHash: string,
  status: TxStatus,
  errorMsg: string | null,
): void {
  if (!HEX64.test(txHash)) {
    throw new Error(`refusing to print ${scenarioName}: tx hash is not 64-char hex`);
  }
  console.log(`  [${scenarioName}]`);
  console.log(`    tx_hash: ${txHash}`);
  console.log(`    status:  ${status}`);
  if (status === "FAILURE") console.log(`    error:   ${errorMsg ?? "(no error message returned)"}`);
}

/** Print a result and return it, for the common "do it, show it, check it" flow. */
export function show(scenarioName: string, res: TxResult): TxResult {
  printScenarioResult(scenarioName, res.txHashHex, res.status, res.error);
  return res;
}

/** On mismatch print scenario name, expected and actual, then exit 1. */
export function assertOutcome(scenarioName: string, expected: string, actual: string): void {
  if (expected !== actual) {
    console.error(`ASSERTION FAILED in ${scenarioName}`);
    console.error(`  expected: ${expected}`);
    console.error(`  actual:   ${actual}`);
    process.exit(1);
  }
}

/** Assert the attempt FAILED and its error text contains the expected ERR_* label. */
export function assertRejected(scenarioName: string, res: TxResult, errLabel: string): void {
  assertOutcome(scenarioName, `FAILURE containing ${errLabel}`, describe(res, errLabel));
}

function describe(res: TxResult, errLabel: string): string {
  if (res.status === "FAILURE" && res.error?.includes(errLabel)) return `FAILURE containing ${errLabel}`;
  return `${res.status}${res.error ? `: ${res.error}` : ""}`;
}
