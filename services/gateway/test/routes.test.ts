import { afterEach, describe, expect, it } from "vitest";
import type { FastifyInstance } from "fastify";
import { buildApp } from "../src/app.js";
import type { PolicyView, TransferOutcome, VaultClient } from "../src/near/client.js";

const body = { agent_id: "agent.sandbox", recipient: "recipient.sandbox", amount: "500", request_id: "req-1" };

const policy: PolicyView = {
  agent_id: "agent.sandbox",
  active: true,
  revoked: false,
  per_tx_limit: "1000",
  epoch_limit: "3000",
  epoch_duration_ns: "3600000000000",
  epoch_start_ns: "1",
  spent_in_epoch: "0",
  remaining_epoch_budget: "3000",
  allowlist: ["recipient.sandbox"],
  expiry_ns: null,
  version: 1,
};

const ok: TransferOutcome = {
  txHash: "9ZB4kYDdTqE5P3Z8jX3m6bY8mC4s7zQ2Jq8h5N1vW3xA",
  status: "success",
  failureMessage: null,
  rejectionReason: null,
  rolledBack: false,
  logs: [],
};

function fake(over: Partial<VaultClient> = {}): VaultClient & { submitted: number } {
  const v = {
    submitted: 0,
    hasAgentKey: () => true,
    viewAgentPolicy: async () => policy,
    submitTransfer: async () => {
      v.submitted++;
      return ok;
    },
    ...over,
  };
  return v;
}

let app: FastifyInstance | undefined;
afterEach(async () => {
  await app?.close();
  app = undefined;
});

async function post(vault: VaultClient, payload: unknown = body) {
  app = await buildApp({ vault, logLevel: "silent" });
  return app.inject({ method: "POST", url: "/transfer", payload: payload as object });
}

describe("POST /transfer", () => {
  it("returns approved with tx hash and preflight policy", async () => {
    const res = await post(fake());
    expect(res.statusCode).toBe(200);
    expect(res.json()).toMatchObject({
      decision: "approved",
      txHash: ok.txHash,
      rejectionReason: null,
      requestId: "req-1",
      preflightPolicy: { agent_id: "agent.sandbox" },
    });
  });

  it("contract rejection is HTTP 200 with decision rejected and the reason", async () => {
    const v = fake({
      submitTransfer: async () => ({
        ...ok,
        status: "failure",
        failureMessage: "Smart contract panicked: ERR_PER_TX_LIMIT: amount exceeds per-transaction limit",
        rejectionReason: "ERR_PER_TX_LIMIT",
      }),
    });
    const res = await post(v);
    expect(res.statusCode).toBe(200);
    expect(res.json()).toMatchObject({ decision: "rejected", rejectionReason: "ERR_PER_TX_LIMIT", txHash: ok.txHash });
  });

  it("still forwards requests the preflight thinks will fail (contract is the authority)", async () => {
    const v = fake({ viewAgentPolicy: async () => ({ ...policy, active: false }) });
    const res = await post(v, { ...body, amount: "999999" });
    expect(res.statusCode).toBe(200);
    expect(v.submitted).toBe(1);
  });

  it("reports an approved transfer that was rolled back", async () => {
    const res = await post(fake({ submitTransfer: async () => ({ ...ok, rolledBack: true }) }));
    expect(res.json()).toMatchObject({ decision: "approved", rolledBack: true });
  });

  it("preflight RPC failure => 503 and nothing submitted", async () => {
    const v = fake({
      viewAgentPolicy: async () => {
        throw new Error("rpc down");
      },
    });
    const res = await post(v);
    expect(res.statusCode).toBe(503);
    expect(res.json().error).toBe("preflight_failed");
    expect(v.submitted).toBe(0);
  });

  it("submission failure => 502 with no fabricated outcome", async () => {
    const v = fake({
      submitTransfer: async () => {
        throw new Error("nonce error");
      },
    });
    const res = await post(v);
    expect(res.statusCode).toBe(502);
    const json = res.json();
    expect(json.error).toBe("submission_failed");
    expect(json.decision).toBeUndefined();
    expect(json.txHash).toBeUndefined();
  });

  it("non-final outcome => 502, no decision", async () => {
    const res = await post(fake({ submitTransfer: async () => ({ ...ok, status: "unknown" }) }));
    expect(res.statusCode).toBe(502);
    expect(res.json().decision).toBeUndefined();
  });

  it("unknown agent key => 422 and nothing submitted", async () => {
    const v = fake({ hasAgentKey: () => false });
    const res = await post(v);
    expect(res.statusCode).toBe(422);
    expect(v.submitted).toBe(0);
  });

  it("400 on invalid body, no vault interaction", async () => {
    const v = fake();
    const res = await post(v, { ...body, amount: "0" });
    expect(res.statusCode).toBe(400);
    expect(v.submitted).toBe(0);
  });

  it("GET /health", async () => {
    app = await buildApp({ vault: fake(), logLevel: "silent" });
    const res = await app.inject({ method: "GET", url: "/health" });
    expect(res.json()).toEqual({ status: "ok" });
  });
});

describe("POST /transfer idempotency", () => {
  async function appWith(vault: VaultClient) {
    app = await buildApp({ vault, logLevel: "silent" });
    return app;
  }
  const send = (a: FastifyInstance, payload: object = body) =>
    a.inject({ method: "POST", url: "/transfer", payload });

  it("replayed request_id returns the original response and submits only once", async () => {
    const v = fake();
    const a = await appWith(v);
    const first = await send(a);
    const second = await send(a);
    expect(first.statusCode).toBe(200);
    expect(second.statusCode).toBe(200);
    expect(second.json()).toEqual(first.json());
    expect(second.headers["idempotent-replay"]).toBe("true");
    expect(first.headers["idempotent-replay"]).toBeUndefined();
    expect(v.submitted).toBe(1);
  });

  it("a replayed contract rejection is also not resubmitted", async () => {
    let calls = 0;
    const v = fake({
      submitTransfer: async () => {
        calls++;
        return { ...ok, status: "failure", failureMessage: "x", rejectionReason: "ERR_PER_TX_LIMIT" };
      },
    });
    const a = await appWith(v);
    await send(a);
    const again = await send(a);
    expect(again.json()).toMatchObject({ decision: "rejected", rejectionReason: "ERR_PER_TX_LIMIT" });
    expect(calls).toBe(1);
  });

  it("concurrent duplicates share one submission", async () => {
    let calls = 0;
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const v = fake({
      submitTransfer: async () => {
        calls++;
        await gate;
        return ok;
      },
    });
    const a = await appWith(v);
    const p1 = send(a);
    const p2 = send(a);
    await new Promise((r) => setTimeout(r, 20));
    release();
    const [r1, r2] = await Promise.all([p1, p2]);
    expect(calls).toBe(1);
    expect(r1.json()).toEqual(r2.json());
  });

  it("same request_id with a different amount or recipient => 409, nothing submitted", async () => {
    const v = fake();
    const a = await appWith(v);
    await send(a);
    const diffAmount = await send(a, { ...body, amount: "501" });
    const diffRecipient = await send(a, { ...body, recipient: "attacker.sandbox" });
    expect(diffAmount.statusCode).toBe(409);
    expect(diffAmount.json().error).toBe("request_id_conflict");
    expect(diffRecipient.statusCode).toBe(409);
    expect(v.submitted).toBe(1);
  });

  it("the same request_id from a different agent is independent", async () => {
    const v = fake();
    const a = await appWith(v);
    await send(a);
    const other = await send(a, { ...body, agent_id: "agent2.sandbox" });
    expect(other.statusCode).toBe(200);
    expect(v.submitted).toBe(2);
  });

  it("an ambiguous submission failure is remembered, so a retry cannot double pay", async () => {
    let calls = 0;
    const v = fake({
      submitTransfer: async () => {
        calls++;
        throw new Error("timeout");
      },
    });
    const a = await appWith(v);
    const first = await send(a);
    const retry = await send(a);
    expect(first.statusCode).toBe(502);
    expect(retry.statusCode).toBe(502);
    expect(retry.headers["idempotent-replay"]).toBe("true");
    expect(calls).toBe(1);
  });

  it("failures before anything is submitted (preflight 503) can be retried", async () => {
    let views = 0;
    const v = fake({
      viewAgentPolicy: async () => {
        views++;
        if (views === 1) throw new Error("rpc down");
        return policy;
      },
    });
    const a = await appWith(v);
    const first = await send(a);
    const retry = await send(a);
    expect(first.statusCode).toBe(503);
    expect(retry.statusCode).toBe(200);
    expect(v.submitted).toBe(1);
  });

  it("no-key rejections (422) are not remembered", async () => {
    let hasKey = false;
    const v = fake({ hasAgentKey: () => hasKey });
    const a = await appWith(v);
    expect((await send(a)).statusCode).toBe(422);
    hasKey = true;
    expect((await send(a)).statusCode).toBe(200);
  });
});
