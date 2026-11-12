const env = import.meta.env;

function clamp(n: number, min: number, max: number, fallback: number): number {
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
}

const rpcUrl: string = env.VITE_NEAR_RPC_URL ?? 'https://test.rpc.fastnear.com';

export const config = {
  /** Wallet network. The wallet connector only supports testnet/mainnet; this app targets testnet. */
  walletNetwork: 'testnet' as const,
  vaultAccountId: env.VITE_VAULT_ACCOUNT_ID ?? 'vault.sandbox',
  rpcUrl,
  sentinelUrl: (env.VITE_SENTINEL_URL ?? 'http://127.0.0.1:8080').replace(/\/$/, ''),
  agentIds: (env.VITE_AGENT_IDS ?? '')
    .split(',')
    .map((s: string) => s.trim())
    .filter(Boolean),
  /** Requirement 12.1: data refreshed at intervals of no more than 15 seconds. */
  pollIntervalMs: clamp(Number(env.VITE_POLL_INTERVAL_MS ?? 10_000), 2_000, 15_000, 10_000),
  eventLookbackBlocks: clamp(Number(env.VITE_EVENT_LOOKBACK_BLOCKS ?? 300), 10, 5_000, 300),
  /** A local sandbox is not reachable by a testnet wallet, so admin actions are disabled there. */
  isLocalRpc: /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])/.test(rpcUrl),
};
