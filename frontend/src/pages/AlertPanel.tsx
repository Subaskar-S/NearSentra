import { useAlerts } from '@/hooks/useAlerts';

export function AlertPanel() {
  const { alerts, error, loading } = useAlerts();
  return (
    <section className="card mb-4">
      <div className="card-header fw-semibold">Sentinel alerts</div>
      {error && <div className="alert alert-warning m-2 py-2 mb-0">{error}</div>}
      <div className="table-responsive">
        <table className="table table-sm align-middle mb-0">
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
                <td className="font-monospace small">{a.agent_id}</td>
                <td>{a.pattern}</td>
                <td className="text-end">
                  {a.observed} / {a.threshold}
                </td>
                <td>{a.action}</td>
              </tr>
            ))}
            {alerts.length === 0 && !error && (
              <tr>
                <td colSpan={5} className="text-muted text-center py-3">
                  {loading ? 'Loading…' : 'No alerts.'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
