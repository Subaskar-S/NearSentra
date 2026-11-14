import { AgentName } from '@/components/AgentName';
import { useAlerts } from '@/hooks/useAlerts';

const PATTERNS: Record<string, string> = {
  velocity_exceeded: 'Too many requests too fast',
  consecutive_failures: 'Repeated rejected requests',
  ingestion_gap: 'Sentinel fell behind the chain',
};

export function AlertPanel() {
  const { alerts, error, loading } = useAlerts();
  return (
    <section className="mb-4">
      <h2 className="ns-section-title">
        Sentinel alerts <small>early warning; the contract still decides</small>
      </h2>
      <div className="card">
        {error && <div className="alert alert-warning m-2 py-2 mb-0">{error}</div>}
        <div className="table-responsive">
          <table className="table align-middle mb-0">
            <thead>
              <tr>
                <th>Time (UTC)</th>
                <th>Agent</th>
                <th>Pattern</th>
                <th className="text-end">Observed / threshold</th>
                <th>Action</th>
              </tr>
            </thead>
            <tbody>
              {alerts.map((a, i) => (
                <tr key={`${a.timestamp_utc}-${a.agent_id}-${i}`}>
                  <td className="text-nowrap">{a.timestamp_utc}</td>
                  <td>
                    <AgentName id={a.agent_id} />
                  </td>
                  <td>
                    {PATTERNS[a.pattern] ?? a.pattern}
                    <div className="ns-reason-code">{a.pattern}</div>
                  </td>
                  <td className="text-end">
                    {a.observed} / {a.threshold}
                  </td>
                  <td>
                    <span className={`ns-badge ${a.action === 'pause_attempted' ? 'paused' : 'active'}`}>
                      {a.action.replace(/_/g, ' ')}
                    </span>
                  </td>
                </tr>
              ))}
              {alerts.length === 0 && !error && (
                <tr>
                  <td colSpan={5} className="text-center text-dim py-4">
                    {loading ? 'Loading…' : 'No alerts. The Sentinel is watching.'}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
