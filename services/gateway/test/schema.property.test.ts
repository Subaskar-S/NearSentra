// Feature: nearsentra-security-vault, Property 10: Gateway validation rejects all schema-violating requests
import fc from "fast-check";
import { describe, expect, it } from "vitest";
import { buildApp } from "../src/app.js";
import type { VaultClient } from "../src/near/client.js";
import { transferRequestSchema, U128_MAX } from "../src/schemas/transfer.js";

const goodAccount = fc.constantFrom("agent.sandbox", "recipient.sandbox", "a1.b2.near");
const base = { agent_id: "agent.sandbox", recipient: "recipient.sandbox", amount: "10", request_id: "r1" };

const badAccount = fc.oneof(
  fc.constant(""),
  fc.constant("a"),
  fc.string({ minLength: 65, maxLength: 80 }),
  fc.stringMatching(/^[A-Z]{2,10}$/),
  fc.stringMatching(/^[a-z]{2,5} [a-z]{2,5}$/),
  fc.integer(),
  fc.constant(null),
);

const badAmount = fc.oneof(
  fc.constant("0"),
  fc.bigInt({ min: -(2n ** 130n), max: -1n }).map((n) => n.toString()),
  fc.bigInt({ min: U128_MAX + 1n, max: 2n ** 140n }).map((n) => n.toString()),
  fc.double({ noNaN: true }).map((d) => d.toString()).filter((s) => !/^(0|[1-9]\d*)$/.test(s)),
  fc.stringMatching(/^[a-z]{1,10}$/),
  fc.integer({ min: 1 }), // JSON number, not a string
  fc.constant(null),
);

describe("Property 10: schema-violating bodies are always rejected", () => {
  it("invalid agent_id / recipient", () => {
    fc.assert(
      fc.property(badAccount, goodAccount, fc.boolean(), (bad, good, onAgent) => {
        const body = onAgent ? { ...base, agent_id: bad, recipient: good } : { ...base, recipient: bad, agent_id: good };
        expect(transferRequestSchema.safeParse(body).success).toBe(false);
      }),
    );
  });

  it("invalid amount", () => {
    fc.assert(
      fc.property(badAmount, (amount) => {
        expect(transferRequestSchema.safeParse({ ...base, amount }).success).toBe(false);
      }),
    );
  });

  it("missing fields / wrong types / arbitrary junk", () => {
    fc.assert(
      fc.property(fc.constantFrom("agent_id", "recipient", "amount", "request_id"), (key) => {
        const body: Record<string, unknown> = { ...base };
        delete body[key];
        expect(transferRequestSchema.safeParse(body).success).toBe(false);
      }),
    );
    fc.assert(
      fc.property(fc.anything(), (junk) => {
        expect(transferRequestSchema.safeParse(junk).success).toBe(false);
      }),
    );
  });

  it("every violating HTTP request returns 400 and never reaches the vault client", async () => {
    let calls = 0;
    const vault: VaultClient = {
      hasAgentKey: () => (calls++, true),
      viewAgentPolicy: async () => (calls++, null),
      submitTransfer: async () => {
        calls++;
        throw new Error("must not be called");
      },
    };
    const app = await buildApp({ vault, logLevel: "silent" });
    await fc.assert(
      fc.asyncProperty(badAmount, badAccount, async (amount, agent_id) => {
        const res = await app.inject({ method: "POST", url: "/transfer", payload: { ...base, amount, agent_id } });
        expect(res.statusCode).toBe(400);
        const body = res.json();
        expect(body.error).toBe("validation_failed");
        expect(body.issues.length).toBeGreaterThan(0);
        for (const i of body.issues) {
          expect(typeof i.field).toBe("string");
          expect(typeof i.constraint).toBe("string");
        }
      }),
      { numRuns: 50 },
    );
    expect(calls).toBe(0);
    await app.close();
  });
});
