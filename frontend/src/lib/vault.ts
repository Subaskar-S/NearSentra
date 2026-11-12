import { JsonRpcProvider } from 'near-api-js';

/** `get_agent_policy` result (U128 / U64 are decimal strings). */
export interface AgentPolicy {
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

export interface VaultStatus {
  paused: boolean;
  admin_account_id: string;
  sentinel_account_id: string | null;
}

export type AgentState = 'active' | 'paused' | 'revoked';

export function agentState(p: AgentPolicy): AgentState {
  return p.revoked ? 'revoked' : p.active ? 'active' : 'paused';
}

export function createProvider(rpcUrl: string): JsonRpcProvider {
  return new JsonRpcProvider({ url: rpcUrl });
}

export function viewVaultStatus(provider: JsonRpcProvider, vaultId: string): Promise<VaultStatus> {
  return provider.callFunction<VaultStatus>({ contractId: vaultId, method: 'get_vault_status', args: {} }) as Promise<VaultStatus>;
}

export async function viewAgentPolicy(
  provider: JsonRpcProvider,
  vaultId: string,
  agentId: string,
): Promise<AgentPolicy | null> {
  const r = await provider.callFunction<AgentPolicy>({
    contractId: vaultId,
    method: 'get_agent_policy',
    args: { agent_id: agentId },
  });
  return r ?? null;
}
