import { AgentName } from '@/components/AgentName';
import { useNow } from '@/hooks/useNow';
import { formatDuration, formatNear, secondsUntilEpochReset } from '@/lib/format';
import { agentState } from '@/lib/vault';
import type { AgentPolicy } from '@/lib/vault';

export function StatusBadge({ policy }: { policy: AgentPolicy }) {
  const state = agentState(policy);
  return <span className={`ns-badge ${state}`}>{state}</span>;
}

function spentPercent(policy: AgentPolicy): number {
  try {
    const limit = BigInt(policy.epoch_limit);
    if (limit === 0n) return 0;
    const spent = limit - BigInt(policy.remaining_epoch_budget);
    return Math.min(100, Math.max(0, Number((spent * 10_000n) / limit) / 100));
  } catch {
    return 0;
  }
}

function AgentCard({ policy, now }: { policy: AgentPolicy; now: number }) {
  const state = agentState(policy);
  const pct = spentPercent(policy);
  const tone = pct >= 90 ? 'high' : pct >= 60 ? 'mid' : '';
  const left = secondsUntilEpochReset(policy.epoch_start_ns, policy.epoch_duration_ns, now);
  const spent = BigInt(policy.epoch_limit) - BigInt(policy.remaining_epoch_budget);
  return (
    <article className={`ns-agent ${state}`}>
      <div className="d-flex justify-content-between align-items-start mb-3">
        <AgentName id={policy.agent_id} />
        <StatusBadge policy={policy} />
      </div>

      <div className="d-flex justify-content-between small mb-1">
        <span className="text-dim">Epoch budget used</span>
        <span>
          <strong>{formatNear(spent.toString())}</strong>
          <span className="text-dim"> / {formatNear(policy.epoch_limit)} NEAR</span>
        </span>
      </div>
      <div
        className="ns-meter mb-3"
        role="progressbar"
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label="Epoch budget used"
      >
        <div className={tone} style={{ width: `${pct}%` }} />
      </div>

      <div className="ns-kv">
        <span>Per-payment limit</span>
        <strong>{formatNear(policy.per_tx_limit)} NEAR</strong>
      </div>
      <div className="ns-kv">
        <span>Remaining this epoch</span>
        <strong title={`${policy.remaining_epoch_budget} yoctoNEAR`}>{formatNear(policy.remaining_epoch_budget)} NEAR</strong>
      </div>
      <div className="ns-kv">
        <span>Epoch resets in</span>
        <strong>{left > 0 ? formatDuration(left) : 'on next request'}</strong>
      </div>

      <div className="mt-2">
        <div className="text-dim small">Can only pay</div>
        {policy.allowlist.length > 0 ? (
          policy.allowlist.map((r) => (
            <span key={r} className="ns-chip">
              {r}
            </span>
          ))
        ) : (
          <span className="text-dim small">nobody (empty allowlist)</span>
        )}
      </div>
    </article>
  );
}

export function AgentList({ agents, loading }: { agents: AgentPolicy[]; loading: boolean }) {
  const now = useNow();
  return (
    <section className="mb-4">
      <h2 className="ns-section-title">
        Agents <small>spending policy enforced by the vault contract</small>
      </h2>
      {agents.length === 0 ? (
        <div className="card">
          <div className="card-body text-center text-dim py-4">
            {loading ? 'Loading…' : 'No registered agents found. Set VITE_AGENT_IDS or wait for transfer events.'}
          </div>
        </div>
      ) : (
        <div className="row row-cols-1 row-cols-md-2 row-cols-xl-3 g-3">
          {agents.map((a) => (
            <div className="col" key={a.agent_id}>
              <AgentCard policy={a} now={now} />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
