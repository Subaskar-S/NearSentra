// Scenario 6: Agent-to-Agent Payment. ResearchAgent buys a 2 NEAR service from AnalysisAgent.
import { payDirect, runAgentToAgent } from "./agent_to_agent.js";
import type { Context } from "../helpers.js";

export async function run(ctx: Context): Promise<void> {
  await runAgentToAgent(ctx, "Scenario 6: Agent-to-Agent Payment", payDirect(ctx));
}
