import type { TransferEntry } from '@/lib/events';
import { formatNear, formatUtc, shortHash } from '@/lib/format';

export function TransactionFeed({ transfers }: { transfers: TransferEntry[] }) {
  return (
    <section className="card mb-4">
      <div className="card-header fw-semibold">Recent transfer requests (last 50, from vault event logs)</div>
      <div className="table-responsive">
        <table className="table table-sm align-middle mb-0">
          <thead>
            <tr>
              <th>Time</th>
              <th>Decision</th>
              <th className="text-end">Amount</th>
              <th>Agent</th>
              <th>Recipient</th>
              <th>Tx</th>
            </tr>
          </thead>
          <tbody>
            {transfers.map((t) => (
              <tr key={`${t.txHash}-${t.blockHeight}`}>
                <td className="text-nowrap">{formatUtc(t.timestampMs)}</td>
                <td>
                  <span className={`badge ${t.decision === 'approved' ? 'bg-success' : 'bg-danger'}`}>{t.decision}</span>
                  {t.rejectionReason && <span className="small text-muted ms-2">{t.rejectionReason}</span>}
                </td>
                <td className="text-end" title={`${t.amount} yoctoNEAR`}>
                  {formatNear(t.amount)} NEAR
                </td>
                <td className="font-monospace small">{t.agentId}</td>
                <td className="font-monospace small">{t.recipient}</td>
                <td className="font-monospace small" title={t.txHash}>
                  {shortHash(t.txHash)}
                </td>
              </tr>
            ))}
            {transfers.length === 0 && (
              <tr>
                <td colSpan={6} className="text-muted text-center py-3">
                  No transfer requests in the scanned block range.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
