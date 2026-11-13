import { describe, expect, it } from "vitest";
import { IdempotencyStore } from "../src/idempotency.js";

const res = { status: 200, body: { ok: true } };

describe("IdempotencyStore", () => {
  it("expires settled entries after the TTL", () => {
    let t = 0;
    const s = new IdempotencyStore({ ttlMs: 1000, now: () => t });
    const c = s.claim("k", "f");
    if (c.kind !== "new") throw new Error("expected new");
    c.complete(res, true);
    expect(s.claim("k", "f").kind).toBe("replay");
    t = 1001;
    expect(s.claim("k", "f").kind).toBe("new");
  });

  it("never expires in-flight entries", () => {
    let t = 0;
    const s = new IdempotencyStore({ ttlMs: 1000, now: () => t });
    s.claim("k", "f");
    t = 10_000;
    expect(s.claim("k", "f").kind).toBe("replay");
  });

  it("evicts the oldest settled entries when full, and refuses when all are in flight", () => {
    const s = new IdempotencyStore({ maxEntries: 2 });
    const a = s.claim("a", "f");
    const b = s.claim("b", "f");
    expect(s.claim("c", "f").kind).toBe("full");
    if (a.kind !== "new") throw new Error("expected new");
    a.complete(res, true);
    expect(s.claim("c", "f").kind).toBe("new"); // "a" (settled) was evicted to make room
    expect(s.size).toBe(2);
    expect(s.claim("a", "f").kind).toBe("full"); // "b" and "c" are both still in flight
    void b;
  });

  it("uncached completions free the key", () => {
    const s = new IdempotencyStore();
    const c = s.claim("k", "f");
    if (c.kind !== "new") throw new Error("expected new");
    c.complete(res, false);
    expect(s.size).toBe(0);
    expect(s.claim("k", "f").kind).toBe("new");
  });

  it("a crash releases the key and fails waiting duplicates", async () => {
    const s = new IdempotencyStore();
    const c = s.claim("k", "f");
    const dup = s.claim("k", "f");
    if (c.kind !== "new" || dup.kind !== "replay") throw new Error("unexpected");
    c.fail(new Error("boom"));
    await expect(dup.promise).rejects.toThrow("boom");
    expect(s.claim("k", "f").kind).toBe("new");
  });

  it("keys are scoped per agent", () => {
    expect(IdempotencyStore.key("a.near", "1")).not.toBe(IdempotencyStore.key("b.near", "1"));
  });
});
