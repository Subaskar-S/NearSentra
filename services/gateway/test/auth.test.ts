import { afterEach, describe, expect, it } from "vitest";
import type { FastifyInstance } from "fastify";
import { buildApp } from "../src/app.js";
import { isAuthorized, parseBearer } from "../src/auth.js";
import { loadConfig } from "../src/config.js";
import type { PolicyView, TransferOutcome, VaultClient } from "../src/near/client.js";

const TOKEN_A = "a".repeat(40);
const TOKEN_B = "b".repeat(40);
const KEY = "ed25519:" + "1".repeat(64);

const policy: PolicyView = {
  agent_id: "a.sandbox", active: true, revoked: false, per_tx_limit: "1000", epoch_limit: "3000",
  epoch_duration_ns: "1", epoch_start_ns: "1", spent_in_epoch: "0", remaining_epoch_budget: "3000",
  allowlist: ["r.sandbox"], expiry_ns: null, version: 1,
};
const ok: TransferOutcome = {
  txHash: "9ZB4kYDdTqE5P3Z8jX3m6bY8mC4s7zQ2Jq8h5N1vW3xA", status: "success",
  failureMessage: null, rejectionReason: null, rolledBack: false, logs: [],
};

function vault() {
  const v = {
    submitted: [] as string[],
    hasAgentKey: () => true,
    viewAgentPolicy: async () => policy,
    submitTransfer: async (agentId: string): Promise<TransferOutcome> => {
      v.submitted.push(agentId);
      return ok;
    },
  };
  return v satisfies VaultClient & { submitted: string[] };
}

let app: FastifyInstance | undefined;
afterEach(async () => {
  await app?.close();
  app = undefined;
});

const tokens = new Map([["a.sandbox", TOKEN_A], ["b.sandbox", TOKEN_B]]);
const body = (agent = "a.sandbox", id = "r1") => ({ agent_id: agent, recipient: "r.sandbox", amount: "5", request_id: id });

async function send(v: ReturnType<typeof vault>, payload: unknown, authorization?: string, allowUnauthenticated = false) {
  app = await buildApp({ vault: v, auth: { tokens, allowUnauthenticated }, logLevel: "silent" });
  return app.inject({
    method: "POST", url: "/transfer", payload: payload as object,
    headers: authorization ? { authorization } : {},
  });
}

describe("POST /transfer authentication", () => {
  it("accepts the agent's own token", async () => {
    const v = vault();
    const res = await send(v, body(), `Bearer ${TOKEN_A}`);
    expect(res.statusCode).toBe(200);
    expect(v.submitted).toEqual(["a.sandbox"]);
  });

  it.each([
    ["no header", undefined],
    ["wrong token", `Bearer ${"c".repeat(40)}`],
    ["another agent's token", `Bearer ${TOKEN_B}`],
    ["wrong scheme", `Basic ${TOKEN_A}`],
    ["token without scheme", TOKEN_A],
    ["empty bearer", "Bearer "],
  ])("rejects %s with 401 and submits nothing", async (_name, header) => {
    const v = vault();
    const res = await send(v, body(), header);
    expect(res.statusCode).toBe(401);
    expect(res.headers["www-authenticate"]).toBe("Bearer");
    expect(res.json()).toMatchObject({ error: "unauthorized" });
    expect(v.submitted).toEqual([]);
  });

  it("is checked before validation: a bad body gets 401, not field errors", async () => {
    const res = await send(vault(), { agent_id: "a.sandbox", amount: 5 });
    expect(res.statusCode).toBe(401);
    expect(JSON.stringify(res.json())).not.toContain("amount");
  });

  it("answers an unknown agent exactly like a wrong token", async () => {
    const unknown = await send(vault(), body("ghost.sandbox"), `Bearer ${TOKEN_A}`);
    const wrong = await send(vault(), body("a.sandbox"), "Bearer nope");
    expect(unknown.statusCode).toBe(401);
    expect(unknown.json()).toEqual(wrong.json());
  });

  it("a token for agent A cannot spend agent B's budget", async () => {
    const v = vault();
    const res = await send(v, body("b.sandbox"), `Bearer ${TOKEN_A}`);
    expect(res.statusCode).toBe(401);
    expect(v.submitted).toEqual([]);
  });

  it("JSON bodies that are not objects are 401 when authentication is required", async () => {
    // (Raw strings or an empty body never reach the handler: Fastify answers 415/400 first.)
    for (const payload of [[], [1, 2], { agent_id: 7 }, { agent_id: null }]) {
      expect((await send(vault(), payload as never)).statusCode).toBe(401);
    }
  });

  it("allowUnauthenticated serves agents without a token but still enforces tokens that exist", async () => {
    const v = vault();
    const open = await send(v, body("free.sandbox"), undefined, true);
    expect(open.statusCode).toBe(200);
    const stillGuarded = await send(v, body("a.sandbox", "r2"), undefined, true);
    expect(stillGuarded.statusCode).toBe(401);
  });

  it("never logs the token (authorization header is redacted)", async () => {
    const lines: string[] = [];
    app = await buildApp({ vault: vault(), auth: { tokens, allowUnauthenticated: false }, logLevel: "info" });
    const origWrite = process.stdout.write.bind(process.stdout);
    process.stdout.write = ((chunk: string | Uint8Array) => { lines.push(String(chunk)); return true; }) as typeof process.stdout.write;
    try {
      await app.inject({ method: "POST", url: "/transfer", payload: body(), headers: { authorization: `Bearer ${TOKEN_A}` } });
    } finally {
      process.stdout.write = origWrite;
    }
    expect(lines.join("")).not.toContain(TOKEN_A);
  });
});

describe("auth helpers", () => {
  it("parseBearer", () => {
    expect(parseBearer("Bearer abc")).toBe("abc");
    expect(parseBearer("bearer abc")).toBe("abc");
    expect(parseBearer("Bearer a b")).toBeNull();
    expect(parseBearer(undefined)).toBeNull();
    expect(parseBearer(["Bearer abc"])).toBeNull();
  });
  it("isAuthorized treats tokens of different lengths safely", () => {
    const auth = { tokens: new Map([["a.sandbox", TOKEN_A]]), allowUnauthenticated: false };
    expect(isAuthorized(auth, "a.sandbox", `Bearer ${TOKEN_A}`)).toBe(true);
    expect(isAuthorized(auth, "a.sandbox", "Bearer x")).toBe(false);
    expect(isAuthorized(auth, "a.sandbox", `Bearer ${TOKEN_A}${TOKEN_A}`)).toBe(false);
  });
});

describe("loadConfig authentication rules", () => {
  const base = { NEAR_RPC_URL: "http://127.0.0.1:3030", VAULT_ACCOUNT_ID: "vault.sandbox" };
  const agent = (n: number, token?: string) => ({
    [`AGENT_${n}_ACCOUNT_ID`]: `a${n}.sandbox`,
    [`AGENT_${n}_PRIVATE_KEY`]: KEY,
    ...(token ? { [`AGENT_${n}_API_TOKEN`]: token } : {}),
  });

  it("fails closed: a held key without a token refuses to start", () => {
    expect(() => loadConfig({ ...base, ...agent(1) })).toThrow(/no AGENT_<n>_API_TOKEN configured for a1.sandbox/);
  });
  it("reads per-agent tokens", () => {
    const c = loadConfig({ ...base, ...agent(1, TOKEN_A), ...agent(2, TOKEN_B) });
    expect(c.agentTokens.get("a1.sandbox")).toBe(TOKEN_A);
    expect(c.agentTokens.get("a2.sandbox")).toBe(TOKEN_B);
    expect(c.allowUnauthenticated).toBe(false);
  });
  it("rejects short, whitespace and shared tokens without echoing them", () => {
    expect(() => loadConfig({ ...base, ...agent(1, "short") })).toThrow(/at least 32 characters/);
    expect(() => loadConfig({ ...base, ...agent(1, "x".repeat(20) + " " + "y".repeat(20)) })).toThrow(/without whitespace/);
    expect(() => loadConfig({ ...base, ...agent(1, TOKEN_A), ...agent(2, TOKEN_A) })).toThrow(/duplicates another agent/);
    try {
      loadConfig({ ...base, ...agent(1, "short-secret") });
    } catch (e) {
      expect(String(e)).not.toContain("short-secret");
    }
  });
  it("a token without an account id is an error", () => {
    expect(() => loadConfig({ ...base, AGENT_3_API_TOKEN: TOKEN_A })).toThrow(/AGENT_3_ACCOUNT_ID is missing/);
  });
  it("unauthenticated mode is loopback only", () => {
    expect(loadConfig({ ...base, ...agent(1), GATEWAY_ALLOW_UNAUTHENTICATED: "true" }).allowUnauthenticated).toBe(true);
    expect(() => loadConfig({ ...base, ...agent(1), GATEWAY_ALLOW_UNAUTHENTICATED: "true", HOST: "0.0.0.0" })).toThrow(/loopback/);
    expect(() => loadConfig({ ...base, ...agent(1), GATEWAY_ALLOW_UNAUTHENTICATED: "yes" })).toThrow(/Invalid gateway configuration/);
  });
});
