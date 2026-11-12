// Scenario 3: Runaway Agent. A request flood trips the Sentinel, which pauses the agent on-chain.
import {
  NEAR,
  assertOutcome,
  assertRejected,
  getPolicy,
  registerAgent,
  requestTransfer,
  show,
  waitFor,
} from "../helpers.js";
import type { Context } from "../helpers.js";

const PAUSE_TIMEOUT_MS = 60_000;

export async function run(ctx: Context): Promise<void> {
  const name = "Scenario 3: Runaway Agent";
  const agent = ctx.agents[2]!;
  // Limits are large enough that only the Sentinel (not the vault limits) can stop the flood.
  await registerAgent(ctx, agent.id, NEAR("0.1"), NEAR("10"), [ctx.recipientId]);

  // The Sentinel alerts when requests in the window exceed its threshold, so threshold + 1 trips it.
  const flood = ctx.velocityThreshold + 1;
  for (let i = 1; i <= flood; i++) {
    const res = show(`${name} / request ${i} of ${flood}`, await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("0.01")));
    assertOutcome(`${name} / request ${i} is within policy`, "SUCCESS", res.status);
  }

  console.log("  waiting for the Sentinel to pause the agent on-chain ...");
  const paused = await waitFor(async () => (await getPolicy(ctx, agent.id))?.active === false, PAUSE_TIMEOUT_MS);
  assertOutcome(`${name} / Sentinel paused the agent within ${PAUSE_TIMEOUT_MS / 1000}s`, "active=false", paused ? "active=false" : "active=true");

  const blocked = show(`${name} / request after the pause`, await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("0.01")));
  assertRejected(`${name} / paused agent is blocked`, blocked, "ERR_AGENT_PAUSED");

  if (ctx.sentinelUrl) {
    const resp = await fetch(`${ctx.sentinelUrl}/alerts`);
    const alerts = (await resp.json()) as Array<{ agent_id: string; pattern: string; action: string }>;
    const hit = alerts.some((a) => a.agent_id === agent.id && a.pattern === "velocity_exceeded" && a.action === "pause_attempted");
    assertOutcome(`${name} / Sentinel /alerts lists the velocity alert`, "velocity_exceeded/pause_attempted", hit ? "velocity_exceeded/pause_attempted" : JSON.stringify(alerts));
  }
}
