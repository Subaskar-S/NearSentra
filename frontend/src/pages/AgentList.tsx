import { useNow } from '@/hooks/useNow';
import { formatDuration, formatNear, secondsUntilEpochReset } from '@/lib/format';
import { agentState } from '@/lib/vault';
import type { AgentPolicy, AgentState } from '@/lib/vault';

const BADGE: Record<AgentState, string> = {
  active: 'bg-success',
  paused: 'bg-warning text-dark',
  revoked: 'bg-danger',
};

export function StatusBadge({ policy }: { policy: AgentPolicy }) {
  const state = agentState(policy);
  return <span className={`badge ${BADGE[state]}`}>{state}</span>;
}

export function AgentList({ agents, loading }: { agents: AgentPolicy[]; loading: boolean }) {
  const now = useNow();
  return (
    <section className="card mb-4">
      <div className="card-header fw-semibold">Agents</div>
      <div className="table-responsive">
        <table className="table table-sm align-middle mb-0">
          <thead>
            <tr>
              <th>Agent</th>
              <th>Status</th>
              <th className="text-end">Remaining epoch budget</th>
              <th className="text-end">Epoch resets in</th>
            </tr>
          </thead>
          <tbody>
            {agents.map((a) => {
              const left = secondsUntilEpochReset(a.epoch_start_ns, a.epoch_duration_ns, now);
              return (
                <tr key={a.agent_id}>
                  <td className="font-monospace">{a.agent_id}</td>
                  <td>
                    <StatusBadge policy={a} />
                  </td>
                  <td className="text-end" title={`${a.remaining_epoch_budget} yoctoNEAR`}>
                    {formatNear(a.remaining_epoch_budget)} NEAR
                    <span className="text-muted small"> of {formatNear(a.epoch_limit)}</span>
                  </td>
                  <td className="text-end">{left > 0 ? formatDuration(left) : 'on next request'}</td>
                </tr>
              );
            })}
            {agents.length === 0 && (
              <tr>
                <td colSpan={4} className="text-muted text-center py-3">
                  {loading ? 'Loading…' : 'No registered agents found. Set VITE_AGENT_IDS or wait for transfer events.'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
