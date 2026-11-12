/** NEP-297 events emitted by the vault (standard "nearsentra"). */
export interface ParsedEvent {
  event: string;
  data: Record<string, unknown>;
}

export interface TransferEntry {
  txHash: string;
  blockHeight: number;
  timestampMs: number;
  agentId: string;
  recipient: string;
  /** yoctoNEAR, decimal string */
  amount: string;
  decision: 'approved' | 'rejected';
  rejectionReason: string | null;
}

const PREFIX = 'EVENT_JSON:';

export function parseEventLogs(logs: string[]): ParsedEvent[] {
  const out: ParsedEvent[] = [];
  for (const line of logs) {
    if (!line.startsWith(PREFIX)) continue;
    try {
      const doc: unknown = JSON.parse(line.slice(PREFIX.length));
      if (typeof doc !== 'object' || doc === null) continue;
      const d = doc as Record<string, unknown>;
      if (d.standard !== 'nearsentra' || typeof d.event !== 'string') continue;
      const data = typeof d.data === 'object' && d.data !== null ? (d.data as Record<string, unknown>) : {};
      out.push({ event: d.event, data });
    } catch {
      // Not JSON: ignore, other contracts' logs are none of our business.
    }
  }
  return out;
}

export function toTransferEntry(
  ev: ParsedEvent,
  ctx: { txHash: string; blockHeight: number; timestampMs: number },
): TransferEntry | null {
  if (ev.event !== 'transfer_requested') return null;
  const { agent_id, recipient, amount, decision, rejection_reason } = ev.data;
  if (typeof agent_id !== 'string' || typeof recipient !== 'string' || typeof amount !== 'string') return null;
  if (decision !== 'approved' && decision !== 'rejected') return null;
  return {
    ...ctx,
    agentId: agent_id,
    recipient,
    amount,
    decision,
    rejectionReason: typeof rejection_reason === 'string' ? rejection_reason : null,
  };
}
