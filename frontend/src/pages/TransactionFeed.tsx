import { useState } from 'react';
import { AgentName } from '@/components/AgentName';
import { useNow } from '@/hooks/useNow';
import type { TransferEntry } from '@/lib/events';
import { formatNear, formatUtc, shortHash, timeAgo } from '@/lib/format';
import { describeReason } from '@/lib/reasons';

function CopyHash({ hash }: { hash: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="ns-copy"
      title={`Copy transaction hash ${hash}`}
      onClick={() => {
        void navigator.clipboard
          ?.writeText(hash)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          })
          .catch(() => undefined);
      }}
    >
      {copied ? 'copied' : shortHash(hash)}
    </button>
  );
}

export function TransactionFeed({ transfers }: { transfers: TransferEntry[] }) {
  const now = useNow();
  return (
    <section className="mb-4">
      <h2 className="ns-section-title">
        Payment decisions <small>last 50, read from the vault's event logs</small>
      </h2>
      <div className="card">
        <div className="table-responsive">
          <table className="table align-middle mb-0 ns-feed">
            <thead>
              <tr>
                <th>When</th>
                <th>Decision</th>
                <th className="text-end">Amount</th>
                <th>From → To</th>
                <th>Tx</th>
              </tr>
            </thead>
            <tbody>
              {transfers.map((t) => {
                const reason = t.decision === 'rejected' ? describeReason(t.rejectionReason) : null;
                return (
                  <tr key={`${t.txHash}-${t.blockHeight}`} className={t.decision}>
                    <td className="text-nowrap" title={formatUtc(t.timestampMs)}>
                      {timeAgo(t.timestampMs, now)}
                    </td>
                    <td>
                      <span className={`ns-badge ${t.decision}`}>{t.decision === 'approved' ? 'approved' : 'blocked'}</span>
                      {reason && (
                        <div className="small mt-1">
                          {reason.title}
                          {reason.code && <div className="ns-reason-code">{reason.code}</div>}
                        </div>
                      )}
                    </td>
                    <td className="text-end ns-amount" title={`${t.amount} yoctoNEAR`}>
                      {formatNear(t.amount)} NEAR
                    </td>
                    <td>
                      <span className="d-inline-flex align-items-center flex-wrap">
                        <AgentName id={t.agentId} />
                        <span className="ns-arrow">→</span>
                        <AgentName id={t.recipient} />
                      </span>
                    </td>
                    <td>
                      <CopyHash hash={t.txHash} />
                    </td>
                  </tr>
                );
              })}
              {transfers.length === 0 && (
                <tr>
                  <td colSpan={5} className="text-center text-dim py-4">
                    No payment requests in the scanned block range. New requests appear here within seconds.
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
