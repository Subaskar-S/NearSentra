import { parseEventLogs, toTransferEntry } from './events';
import type { TransferEntry } from './events';

const MAX_FEED = 50;
const BATCH = 6;

type Json = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

class RpcError extends Error {
  constructor(
    message: string,
    readonly cause_name?: string,
  ) {
    super(message);
  }
}

async function rpc(url: string, method: string, params: unknown): Promise<Json> {
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 'nearsentra', method, params }),
  });
  // Some nodes answer JSON-RPC errors (e.g. UNKNOWN_BLOCK) with a 4xx status and a JSON body.
  let body: Json;
  try {
    body = (await resp.json()) as Json;
  } catch {
    throw new RpcError(`${method}: HTTP ${resp.status}`);
  }
  if (body.error) throw new RpcError(`${method}: ${body.error.message}`, body.error.cause?.name);
  if (!resp.ok) throw new RpcError(`${method}: HTTP ${resp.status}`);
  return body.result as Json;
}

/**
 * Reads the vault's NEP-297 events straight from the NEAR RPC: it walks final blocks, looks at
 * every transaction addressed to the vault and parses the logs of its (final) execution outcome.
 * Rejections are included because the vault logs the rejection event before it panics.
 */
export class EventScanner {
  private lastHeight: number | null = null;
  private seen = new Set<string>();
  private feed: TransferEntry[] = [];
  readonly agents = new Set<string>();

  constructor(
    private readonly rpcUrl: string,
    private readonly vaultId: string,
    private readonly lookbackBlocks: number,
  ) {}

  /** Newest first, at most 50 entries. */
  get recentTransfers(): TransferEntry[] {
    return this.feed;
  }

  async poll(): Promise<void> {
    const head = await rpc(this.rpcUrl, 'block', { finality: 'final' });
    const headHeight = Number(head.header.height);
    const earliest = Math.max(1, headHeight - this.lookbackBlocks + 1);
    const from = this.lastHeight === null ? earliest : Math.max(this.lastHeight + 1, earliest);
    const heights: number[] = [];
    for (let h = from; h <= headHeight; h++) heights.push(h);

    for (let i = 0; i < heights.length; i += BATCH) {
      const batch = heights.slice(i, i + BATCH);
      const results = await Promise.all(batch.map((h) => this.scanBlock(h)));
      // Only advance after the whole batch succeeded, so a failure is retried next poll.
      for (const entries of results) for (const e of entries) this.add(e);
      this.lastHeight = batch[batch.length - 1]!;
    }
    this.lastHeight ??= headHeight;
  }

  private add(entry: TransferEntry): void {
    const key = `${entry.txHash}:${entry.blockHeight}:${entry.agentId}:${entry.amount}:${entry.decision}`;
    if (this.seen.has(key)) return;
    this.seen.add(key);
    this.agents.add(entry.agentId);
    this.feed = [...this.feed, entry]
      .sort((a, b) => b.blockHeight - a.blockHeight || b.timestampMs - a.timestampMs)
      .slice(0, MAX_FEED);
  }

  private async scanBlock(height: number): Promise<TransferEntry[]> {
    let block: Json;
    try {
      block = await rpc(this.rpcUrl, 'block', { block_id: height });
    } catch (err) {
      if (err instanceof RpcError && err.cause_name === 'UNKNOWN_BLOCK') return []; // skipped height
      throw err;
    }
    const timestampMs = Math.floor(Number(block.header.timestamp) / 1e6);
    const entries: TransferEntry[] = [];
    for (const ref of block.chunks as Json[]) {
      // A chunk listed with an older height_included was missed in this block: its txs are not new.
      if (ref.height_included !== undefined && Number(ref.height_included) !== height) continue;
      let chunk: Json;
      try {
        chunk = await rpc(this.rpcUrl, 'chunk', { chunk_id: ref.chunk_hash });
      } catch (err) {
        // Genesis-era or garbage-collected chunks cannot hold anything we still need.
        if (err instanceof RpcError && (err.cause_name === 'UNKNOWN_BLOCK' || err.cause_name === 'UNKNOWN_CHUNK')) continue;
        throw err;
      }
      const txs = (chunk.transactions as Json[]).filter((t) => t.receiver_id === this.vaultId);
      const statuses = await Promise.all(
        txs.map((t) => rpc(this.rpcUrl, 'tx', { tx_hash: t.hash, sender_account_id: t.signer_id, wait_until: 'FINAL' })),
      );
      statuses.forEach((status, idx) => {
        const logs: string[] = (status.receipts_outcome as Json[]).flatMap((r) => r.outcome?.logs ?? []);
        for (const ev of parseEventLogs(logs)) {
          const entry = toTransferEntry(ev, { txHash: txs[idx]!.hash as string, blockHeight: height, timestampMs });
          if (entry) entries.push(entry);
        }
      });
    }
    return entries;
  }
}
