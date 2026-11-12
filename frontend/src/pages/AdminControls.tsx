import { useState } from 'react';
import { useNearWallet } from 'near-connect-hooks';
import { config } from '@/config';
import { shortHash } from '@/lib/format';
import { agentState } from '@/lib/vault';
import type { AgentPolicy, VaultStatus } from '@/lib/vault';
import { StatusBadge } from './AgentList';

interface Props {
  vaultStatus: VaultStatus | null;
  agents: AgentPolicy[];
  /** Re-read chain state after a call (never an optimistic local change). */
  onChanged: () => Promise<void>;
}

interface CallResult {
  label: string;
  ok: boolean;
  txHash: string | null;
  message: string;
}

const GAS = '30000000000000'; // 30 Tgas
/** Admin methods need exactly 1 yoctoNEAR, which forces a full-access key and wallet confirmation. */
const ONE_YOCTO = '1';

/** Pull the hash and on-chain outcome out of a final execution outcome without guessing. */
function describeOutcome(label: string, raw: unknown): CallResult {
  const o = raw as {
    transaction_outcome?: { id?: string };
    transaction?: { hash?: string };
    status?: unknown;
  };
  const txHash = o.transaction_outcome?.id ?? o.transaction?.hash ?? null;
  const s = o.status as Record<string, unknown> | undefined;
  if (s && typeof s === 'object' && 'SuccessValue' in s) {
    return { label, ok: true, txHash, message: 'SUCCESS' };
  }
  if (s && typeof s === 'object' && 'Failure' in s) {
    return { label, ok: false, txHash, message: `FAILURE: ${JSON.stringify(s.Failure)}` };
  }
  return { label, ok: false, txHash, message: 'UNKNOWN: the outcome is not final yet; check the transaction hash on an explorer' };
}

export function AdminControls({ vaultStatus, agents, onChanged }: Props) {
  const { signedAccountId, signIn, signOut, callFunctionRaw, loading } = useNearWallet();
  const [pending, setPending] = useState<string | null>(null);
  const [result, setResult] = useState<CallResult | null>(null);

  const signedIn = signedAccountId !== '';
  const isAdmin = signedIn && vaultStatus !== null && signedAccountId === vaultStatus.admin_account_id;
  const isSentinel = signedIn && vaultStatus !== null && signedAccountId === vaultStatus.sentinel_account_id;
  const walletUsable = signedIn && !config.isLocalRpc;

  async function run(label: string, method: string, args: Record<string, unknown>, deposit = '0') {
    setPending(label);
    setResult(null);
    try {
      const outcome = await callFunctionRaw({ contractId: config.vaultAccountId, method, args, gas: GAS, deposit });
      setResult(describeOutcome(label, outcome));
    } catch (e) {
      // Rejected in the wallet or failed on-chain: show why, change nothing locally.
      setResult({ label, ok: false, txHash: null, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setPending(null);
      await onChanged();
    }
  }

  const busy = pending !== null;
  const canPauseAgent = walletUsable && (isAdmin || isSentinel);

  return (
    <section className="card mb-4">
      <div className="card-header d-flex justify-content-between align-items-center">
        <span className="fw-semibold">Admin controls</span>
        {loading ? (
          <span className="text-muted small">Loading wallet…</span>
        ) : signedIn ? (
          <span>
            <span className="font-monospace small me-2">{signedAccountId}</span>
            <button className="btn btn-sm btn-outline-secondary" onClick={() => void signOut()}>
              Sign out
            </button>
          </span>
        ) : (
          <button className="btn btn-sm btn-primary" onClick={() => void signIn()}>
            Connect wallet
          </button>
        )}
      </div>
      <div className="card-body">
        {config.isLocalRpc && (
          <div className="alert alert-info py-2">
            Read-only: the RPC is a local sandbox that a testnet wallet cannot sign for. Point VITE_NEAR_RPC_URL at a
            testnet deployment to use admin controls.
          </div>
        )}
        {!signedIn && !config.isLocalRpc && <p className="text-muted">Sign in with the admin wallet to enable controls.</p>}
        {signedIn && vaultStatus && !isAdmin && (
          <div className="alert alert-warning py-2">
            {signedAccountId} is not the vault admin ({vaultStatus.admin_account_id}).
            {isSentinel ? ' It is the Sentinel and may only pause agents.' : ' Admin-only actions are disabled.'}
          </div>
        )}

        <div className="d-flex align-items-center gap-3 mb-3">
          <span>
            Vault:{' '}
            {vaultStatus ? (
              <span className={`badge ${vaultStatus.paused ? 'bg-danger' : 'bg-success'}`}>
                {vaultStatus.paused ? 'PAUSED' : 'running'}
              </span>
            ) : (
              '…'
            )}
          </span>
          <button
            className="btn btn-sm btn-warning"
            disabled={busy || !walletUsable || !isAdmin || vaultStatus?.paused !== false}
            onClick={() => void run('Pause Vault', 'pause_vault', {}, ONE_YOCTO)}
          >
            Pause Vault
          </button>
          <button
            className={`btn btn-sm btn-success ${isAdmin ? '' : 'disabled opacity-50'}`}
            aria-disabled={!isAdmin}
            title={isAdmin ? undefined : 'Only the registered admin account can unpause the vault'}
            disabled={busy || !walletUsable || !isAdmin || vaultStatus?.paused !== true}
            onClick={() => void run('Unpause Vault', 'unpause_vault', {}, ONE_YOCTO)}
          >
            Unpause Vault
          </button>
        </div>

        <table className="table table-sm align-middle mb-0">
          <thead>
            <tr>
              <th>Agent</th>
              <th>Status</th>
              <th className="text-end">Actions</th>
            </tr>
          </thead>
          <tbody>
            {agents.map((a) => {
              const state = agentState(a);
              return (
                <tr key={a.agent_id}>
                  <td className="font-monospace">{a.agent_id}</td>
                  <td>
                    <StatusBadge policy={a} />
                  </td>
                  <td className="text-end">
                    <button
                      className="btn btn-sm btn-outline-warning me-2"
                      disabled={busy || !canPauseAgent || state !== 'active'}
                      onClick={() => void run(`Pause ${a.agent_id}`, 'pause_agent', { agent_id: a.agent_id })}
                    >
                      Pause Agent
                    </button>
                    <button
                      className="btn btn-sm btn-outline-danger"
                      disabled={busy || !walletUsable || !isAdmin || state === 'revoked'}
                      onClick={() => {
                        if (window.confirm(`Revoke ${a.agent_id}? Revocation is permanent.`)) {
                          void run(`Revoke ${a.agent_id}`, 'revoke_agent', { agent_id: a.agent_id }, ONE_YOCTO);
                        }
                      }}
                    >
                      Revoke Agent
                    </button>
                  </td>
                </tr>
              );
            })}
            {agents.length === 0 && (
              <tr>
                <td colSpan={3} className="text-muted text-center py-3">
                  No agents to control.
                </td>
              </tr>
            )}
          </tbody>
        </table>

        {pending && <div className="alert alert-secondary mt-3 mb-0 py-2">{pending}: waiting for wallet and confirmation…</div>}
        {result && (
          <div className={`alert ${result.ok ? 'alert-success' : 'alert-danger'} mt-3 mb-0 py-2`}>
            <strong>{result.label}</strong>: {result.message}
            {result.txHash && (
              <div className="font-monospace small text-break" title={result.txHash}>
                tx {result.txHash.length > 20 ? result.txHash : shortHash(result.txHash)}
              </div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
