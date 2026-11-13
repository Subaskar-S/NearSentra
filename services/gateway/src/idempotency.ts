/**
 * In-memory idempotency store for POST /transfer.
 *
 * An agent that retries (timeout, crash, LLM loop) with the same `request_id` must never
 * cause a second payout. The first request for a (agent_id, request_id) pair is processed;
 * later requests with the same pair get the stored response instead of a new transaction.
 * Concurrent duplicates wait for the first request to finish.
 *
 * Limits: state is per process and is lost on restart, so this is a safety net against
 * accidental replays, not a substitute for the vault's on-chain limits (which stay the
 * final authority).
 */

export interface StoredResponse {
  status: number;
  body: unknown;
}

interface Entry {
  fingerprint: string;
  promise: Promise<StoredResponse>;
  settled: boolean;
  /** Epoch ms when the entry was created or settled; used for TTL. */
  touchedAt: number;
}

export interface IdempotencyOptions {
  ttlMs?: number;
  maxEntries?: number;
  now?: () => number;
}

export type Claim =
  /** First time this key is seen: the caller must call `complete` or `release`. */
  | { kind: "new"; complete: (res: StoredResponse, cache: boolean) => void; fail: (err: unknown) => void }
  /** Same key and same request: reuse this response. */
  | { kind: "replay"; promise: Promise<StoredResponse> }
  /** Same key but a different recipient/amount: a client bug or an attack. */
  | { kind: "conflict" }
  /** Store is full of in-flight requests; refuse rather than skip protection. */
  | { kind: "full" };

export const DEFAULT_TTL_MS = 24 * 60 * 60 * 1000;
export const DEFAULT_MAX_ENTRIES = 10_000;

export class IdempotencyStore {
  private readonly entries = new Map<string, Entry>();
  private readonly ttlMs: number;
  private readonly maxEntries: number;
  private readonly now: () => number;

  constructor({ ttlMs = DEFAULT_TTL_MS, maxEntries = DEFAULT_MAX_ENTRIES, now = Date.now }: IdempotencyOptions = {}) {
    this.ttlMs = ttlMs;
    this.maxEntries = maxEntries;
    this.now = now;
  }

  get size(): number {
    return this.entries.size;
  }

  /** Key is scoped per agent so one agent can never collide with another's request IDs. */
  static key(agentId: string, requestId: string): string {
    return `${agentId}\u0000${requestId}`;
  }

  static fingerprint(recipient: string, amount: string): string {
    return `${recipient}\u0000${amount}`;
  }

  claim(key: string, fingerprint: string): Claim {
    this.evict();

    const existing = this.entries.get(key);
    if (existing) {
      if (existing.fingerprint !== fingerprint) return { kind: "conflict" };
      return { kind: "replay", promise: existing.promise };
    }
    if (this.entries.size >= this.maxEntries) return { kind: "full" };

    let resolve!: (res: StoredResponse) => void;
    let reject!: (err: unknown) => void;
    const promise = new Promise<StoredResponse>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    // A duplicate may never attach; avoid an unhandled-rejection crash.
    promise.catch(() => undefined);

    const entry: Entry = { fingerprint, promise, settled: false, touchedAt: this.now() };
    this.entries.set(key, entry);

    return {
      kind: "new",
      complete: (res, cache) => {
        entry.settled = true;
        entry.touchedAt = this.now();
        // Results that did not reach the chain (nothing was submitted) are safe to retry.
        if (!cache && this.entries.get(key) === entry) this.entries.delete(key);
        resolve(res);
      },
      fail: (err) => {
        // Unexpected crash before we know whether anything was submitted: do not cache,
        // let waiting duplicates see the failure.
        if (this.entries.get(key) === entry) this.entries.delete(key);
        reject(err);
      },
    };
  }

  /** Drop expired settled entries, then the oldest settled ones if the store is full. */
  private evict(): void {
    const now = this.now();
    for (const [key, entry] of this.entries) {
      if (entry.settled && now - entry.touchedAt > this.ttlMs) this.entries.delete(key);
    }
    if (this.entries.size < this.maxEntries) return;
    // Map iterates in insertion order, so the first settled entries are the oldest.
    for (const [key, entry] of this.entries) {
      if (this.entries.size < this.maxEntries) break;
      if (entry.settled) this.entries.delete(key);
    }
  }
}
