// Agent-to-agent payment demo through the Gateway, for the dashboard.
// Run via `make demo-agent-payment` (scripts/run-agent-payment-demo.sh) with `make sandbox-deploy`
// and `make services` already running.
//
// ResearchAgent's payment requests go to the Gateway over HTTP; the Gateway signs with
// ResearchAgent's request_transfer-only key and the vault contract decides. The admin key is used
// only for setup and the final pause, exactly as in the simulation.
import { randomUUID } from "node:crypto";
import { base58ToHex, loadContext } from "./helpers.js";
import type { TxResult } from "./helpers.js";
import { runAgentToAgent } from "./scenarios/agent_to_agent.js";
import type { Pay } from "./scenarios/agent_to_agent.js";

interface GatewayBody {
  txHash?: string;
  decision?: "approved" | "rejected";
  rejectionReason?: string | null;
  error?: string;
  message?: string;
}

function payViaGateway(gatewayUrl: string, agentId: string): Pay {
  return async (recipient, amount): Promise<TxResult> => {
    const resp = await fetch(`${gatewayUrl}/transfer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ agent_id: agentId, recipient, amount: amount.toString(), request_id: randomUUID() }),
    });
    const body = (await resp.json()) as GatewayBody;
    if (resp.status !== 200 || !body.txHash || !body.decision) {
      throw new Error(`gateway answered HTTP ${resp.status}: ${body.error ?? "unknown"} ${body.message ?? ""}`.trim());
    }
    const approved = body.decision === "approved";
    return {
      txHashHex: base58ToHex(body.txHash),
      status: approved ? "SUCCESS" : "FAILURE",
      error: approved ? null : (body.rejectionReason ?? "rejected without a reason"),
      logs: [],
    };
  };
}

async function main(): Promise<void> {
  const ctx = loadContext();
  const gatewayUrl = process.env["GATEWAY_URL"] ?? "http://127.0.0.1:3000";
  const health = await fetch(`${gatewayUrl}/health`).catch(() => null);
  if (!health?.ok) {
    throw new Error(`the Gateway is not reachable at ${gatewayUrl}. Start it first with "make services".`);
  }
  const research = ctx.agents[5]!;
  console.log(`Agent-to-agent payment through the Gateway at ${gatewayUrl}`);
  await runAgentToAgent(ctx, "Agent-to-Agent Demo", payViaGateway(gatewayUrl, research.id));
  console.log("\nAgent-to-agent demo finished: every payment had the expected on-chain outcome.");
  console.log("Open the dashboard at http://localhost:5173 to see the transfer feed and ResearchAgent's paused status.");
}

main().catch((err: unknown) => {
  console.error("Demo aborted:", err instanceof Error ? err.message : err);
  process.exit(1);
});
