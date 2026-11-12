import { Account, JsonRpcProvider, actions, teraToGas } from "near-api-js";
import type { KeyPairString } from "near-api-js";

/** `get_agent_policy` result as returned by the vault (U128/U64 are decimal strings). */
export interface PolicyView {
  agent_id: string;
  active: boolean;
  revoked: boolean;
  per_tx_limit: string;
  epoch_limit: string;
  epoch_duration_ns: string;
  epoch_start_ns: string;
  spent_in_epoch: string;
  remaining_epoch_budget: string;
  allowlist: string[];
  expiry_ns: string | null;
  version: number;
}

export interface TransferOutcome {
  /** Base58 NEAR transaction hash. */
  txHash: string;
  /** Execution status of the whole transaction (including callbacks). */
  status: "success" | "failure" | "unknown";
  /** Full contract failure message when status is "failure". */
  failureMessage: string | null;
  /** `ERR_*` identifier extracted from the contract panic message, if any. */
  rejectionReason: string | null;
  /** True when a `transfer_rolled_back` event was emitted (approved, but the payout failed). */
  rolledBack: boolean;
  logs: string[];
}

/** Everything the route layer needs from the chain; swapped for a fake in tests. */
export interface VaultClient {
  hasAgentKey(agentId: string): boolean;
  viewAgentPolicy(agentId: string): Promise<PolicyView | null>;
  submitTransfer(agentId: string, recipient: string, amount: string): Promise<TransferOutcome>;
}

export function createProvider(rpcUrl: string): JsonRpcProvider {
  return new JsonRpcProvider({ url: rpcUrl });
}

/** Account signed with the agent's function-call key (never a full-access key). */
export function createAgentAccount(agentId: string, privateKey: string, provider: JsonRpcProvider): Account {
  return new Account(agentId, provider, privateKey as KeyPairString);
}

export async function viewAgentPolicy(
  contractId: string,
  agentId: string,
  provider: JsonRpcProvider,
): Promise<PolicyView | null> {
  const result = await provider.callFunction<PolicyView>({
    contractId,
    method: "get_agent_policy",
    args: { agent_id: agentId },
  });
  return result ?? null;
}

const ERR_CODE = /ERR_[A-Z0-9_]+/;

interface OutcomeLike {
  transaction_outcome: { id: string };
  status: unknown;
  receipts_outcome?: Array<{ outcome: { logs: string[]; status: unknown } }>;
}

/** Convert a raw RPC execution outcome into a {@link TransferOutcome} without guessing. */
export function parseOutcome(raw: unknown): TransferOutcome {
  const outcome = raw as OutcomeLike;
  const logs = (outcome.receipts_outcome ?? []).flatMap((r) => r.outcome.logs ?? []);
  const rolledBack = logs.some((l) => l.startsWith("EVENT_JSON:") && l.includes('"event":"transfer_rolled_back"'));

  let status: TransferOutcome["status"] = "unknown";
  let failureMessage: string | null = null;
  const s = outcome.status as Record<string, unknown> | string | undefined;
  if (s && typeof s === "object") {
    if ("SuccessValue" in s) {
      status = "success";
    } else if ("Failure" in s) {
      status = "failure";
      failureMessage = JSON.stringify(s["Failure"]);
    }
  }

  const rejectionReason = failureMessage ? (ERR_CODE.exec(failureMessage)?.[0] ?? null) : null;
  return { txHash: outcome.transaction_outcome.id, status, failureMessage, rejectionReason, rolledBack, logs };
}

/** near-api-js backed implementation. Key material is only held in memory. */
export class NearVaultClient implements VaultClient {
  private readonly provider: JsonRpcProvider;
  private readonly accounts = new Map<string, Account>();

  constructor(
    private readonly contractId: string,
    rpcUrl: string,
    agentKeys: Map<string, string>,
  ) {
    this.provider = createProvider(rpcUrl);
    for (const [agentId, key] of agentKeys) {
      this.accounts.set(agentId, createAgentAccount(agentId, key, this.provider));
    }
  }

  hasAgentKey(agentId: string): boolean {
    return this.accounts.has(agentId);
  }

  viewAgentPolicy(agentId: string): Promise<PolicyView | null> {
    return viewAgentPolicy(this.contractId, agentId, this.provider);
  }

  async submitTransfer(agentId: string, recipient: string, amount: string): Promise<TransferOutcome> {
    const account = this.accounts.get(agentId);
    if (!account) throw new Error(`no signing key configured for ${agentId}`);
    // throwOnFailure:false => a contract rejection comes back as an outcome, while
    // network / nonce / RPC problems still throw (and become HTTP 502 upstream).
    const raw = await account.signAndSendTransaction({
      receiverId: this.contractId,
      actions: [actions.functionCall("request_transfer", { recipient, amount }, teraToGas("60"), 0n)],
      waitUntil: "FINAL",
      throwOnFailure: false,
    });
    return parseOutcome(raw);
  }
}
