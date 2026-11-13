import { describe, expect, it } from "vitest";
import { parseOutcome } from "../src/near/client.js";
import { loadConfig } from "../src/config.js";

const event = (name: string) => `EVENT_JSON:{"standard":"nearsentra","version":"1.0.0","event":"${name}","data":{}}`;

describe("parseOutcome", () => {
  it("success", () => {
    const o = parseOutcome({
      transaction_outcome: { id: "HASH" },
      status: { SuccessValue: "" },
      receipts_outcome: [{ outcome: { logs: [event("transfer_requested")], status: {} } }],
    });
    expect(o).toMatchObject({ txHash: "HASH", status: "success", rejectionReason: null, rolledBack: false });
  });

  it("contract panic => failure with ERR code and full message", () => {
    const failure = {
      ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: "Smart contract panicked: ERR_EPOCH_LIMIT: amount would exceed epoch spending limit" } } },
    };
    const o = parseOutcome({ transaction_outcome: { id: "H" }, status: { Failure: failure }, receipts_outcome: [] });
    expect(o.status).toBe("failure");
    expect(o.rejectionReason).toBe("ERR_EPOCH_LIMIT");
    expect(o.failureMessage).toContain("amount would exceed epoch spending limit");
  });

  it("detects rollback", () => {
    const o = parseOutcome({
      transaction_outcome: { id: "H" },
      status: { SuccessValue: "" },
      receipts_outcome: [{ outcome: { logs: [event("transfer_rolled_back")], status: {} } }],
    });
    expect(o.rolledBack).toBe(true);
  });

  it("non-final status => unknown (never guessed)", () => {
    expect(parseOutcome({ transaction_outcome: { id: "H" }, status: "Started" }).status).toBe("unknown");
  });
});

describe("loadConfig", () => {
  const base = { NEAR_RPC_URL: "http://127.0.0.1:3030", VAULT_ACCOUNT_ID: "vault.sandbox" };
  const key = "ed25519:" + "1".repeat(64);

  it("reads numbered agent key pairs", () => {
    const c = loadConfig({
      ...base,
      AGENT_1_ACCOUNT_ID: "a.sandbox",
      AGENT_1_PRIVATE_KEY: key,
      GATEWAY_ALLOW_UNAUTHENTICATED: "true",
    });
    expect(c.agentKeys.get("a.sandbox")).toBe(key);
    expect(c.host).toBe("127.0.0.1");
  });

  it("rejects a missing or malformed key without echoing it", () => {
    expect(() => loadConfig({ ...base, AGENT_1_ACCOUNT_ID: "a.sandbox" })).toThrow(/PRIVATE_KEY is missing/);
    try {
      loadConfig({ ...base, AGENT_1_ACCOUNT_ID: "a.sandbox", AGENT_1_PRIVATE_KEY: "supersecret" });
      throw new Error("should have thrown");
    } catch (e) {
      expect((e as Error).message).not.toContain("supersecret");
    }
  });

  it("rejects an invalid RPC URL", () => {
    expect(() => loadConfig({ ...base, NEAR_RPC_URL: "nope" })).toThrow(/Invalid gateway configuration/);
  });
});
