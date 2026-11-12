import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { config } from '@/config';
import type { TransferEntry } from '@/lib/events';
import { EventScanner } from '@/lib/scanner';
import { createProvider, viewAgentPolicy, viewVaultStatus } from '@/lib/vault';
import type { AgentPolicy, VaultStatus } from '@/lib/vault';

export interface VaultData {
  vaultStatus: VaultStatus | null;
  agents: AgentPolicy[];
  recentTransfers: TransferEntry[];
  loading: boolean;
  /** Set when the last refresh failed; the previous data stays visible. */
  error: string | null;
  lastUpdated: number | null;
  refresh: () => Promise<void>;
}

/** Polls the vault contract (state) and the RPC (event logs) at most every 15 seconds. */
export function useVault(): VaultData {
  const provider = useMemo(() => createProvider(config.rpcUrl), []);
  const scanner = useRef(new EventScanner(config.rpcUrl, config.vaultAccountId, config.eventLookbackBlocks));
  const [vaultStatus, setVaultStatus] = useState<VaultStatus | null>(null);
  const [agents, setAgents] = useState<AgentPolicy[]>([]);
  const [recentTransfers, setRecentTransfers] = useState<TransferEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const busy = useRef(false);

  const refresh = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    try {
      const status = await viewVaultStatus(provider, config.vaultAccountId);
      // Events are best effort: a scan failure must not hide the authoritative contract state.
      let scanError: string | null = null;
      try {
        await scanner.current.poll();
      } catch (e) {
        scanError = `event feed: ${e instanceof Error ? e.message : String(e)}`;
      }
      const ids = new Set([...config.agentIds, ...scanner.current.agents]);
      const policies = await Promise.all([...ids].map((id) => viewAgentPolicy(provider, config.vaultAccountId, id)));
      setVaultStatus(status);
      setAgents(policies.filter((p): p is AgentPolicy => p !== null));
      setRecentTransfers(scanner.current.recentTransfers);
      setError(scanError);
      setLastUpdated(Date.now());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy.current = false;
      setLoading(false);
    }
  }, [provider]);

  useEffect(() => {
    void refresh();
    const id = setInterval(() => void refresh(), config.pollIntervalMs);
    return () => clearInterval(id);
  }, [refresh]);

  return { vaultStatus, agents, recentTransfers, loading, error, lastUpdated, refresh };
}
