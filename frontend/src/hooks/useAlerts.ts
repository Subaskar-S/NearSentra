import { useEffect, useState } from 'react';
import { config } from '@/config';

export interface SentinelAlert {
  agent_id: string;
  pattern: string;
  threshold: number;
  observed: number;
  action: string;
  timestamp_utc: string;
}

/** Polls the Sentinel's `/alerts` (most recent first) at most every 15 seconds. */
export function useAlerts(): { alerts: SentinelAlert[]; error: string | null; loading: boolean } {
  const [alerts, setAlerts] = useState<SentinelAlert[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const resp = await fetch(`${config.sentinelUrl}/alerts`);
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const data = (await resp.json()) as SentinelAlert[];
        if (!cancelled) {
          setAlerts(data);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) setError(`Sentinel unreachable at ${config.sentinelUrl}: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void load();
    const id = setInterval(() => void load(), config.pollIntervalMs);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  return { alerts, error, loading };
}
