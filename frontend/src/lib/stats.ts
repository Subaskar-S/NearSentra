import type { TransferEntry } from '@/lib/events';
import type { AgentPolicy } from '@/lib/vault';
import { agentState } from '@/lib/vault';

export interface Summary {
  agents: number;
  active: number;
  paused: number;
  revoked: number;
  approved: number;
  rejected: number;
  /** yoctoNEAR the vault approved to pay out, from the events shown in the feed. */
  approvedYocto: bigint;
  /** yoctoNEAR the vault refused to pay out, from the events shown in the feed. */
  rejectedYocto: bigint;
}

function toBigInt(v: string): bigint {
  try {
    return BigInt(v);
  } catch {
    return 0n;
  }
}

/** Totals over the agents and the recent-transfer feed. Pure, so it is easy to check. */
export function summarize(agents: AgentPolicy[], transfers: TransferEntry[]): Summary {
  const s: Summary = {
    agents: agents.length,
    active: 0,
    paused: 0,
    revoked: 0,
    approved: 0,
    rejected: 0,
    approvedYocto: 0n,
    rejectedYocto: 0n,
  };
  for (const a of agents) s[agentState(a)] += 1;
  for (const t of transfers) {
    if (t.decision === 'approved') {
      s.approved += 1;
      s.approvedYocto += toBigInt(t.amount);
    } else {
      s.rejected += 1;
      s.rejectedYocto += toBigInt(t.amount);
    }
  }
  return s;
}
