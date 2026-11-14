import { config } from '@/config';

/** Friendly name configured for an account (VITE_AGENT_LABELS), or null. */
export function labelFor(accountId: string): string | null {
  return config.agentLabels[accountId] ?? null;
}
