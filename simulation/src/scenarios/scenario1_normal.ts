// Scenario 1: Normal Agent. A well-behaved agent makes an in-policy transfer.
import { NEAR, assertOutcome, getPolicy, registerAgent, requestTransfer, show } from "../helpers.js";
import type { Context } from "../helpers.js";

export async function run(ctx: Context): Promise<void> {
  const name = "Scenario 1: Normal Agent";
  const agent = ctx.agents[0]!;
  await registerAgent(ctx, agent.id, NEAR("1"), NEAR("3"), [ctx.recipientId]);

  const res = show(`${name} / 0.5 NEAR to allowlisted recipient`, await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("0.5")));
  assertOutcome(name, "SUCCESS", res.status);

  const policy = await getPolicy(ctx, agent.id);
  assertOutcome(`${name} / spent_in_epoch`, NEAR("0.5").toString(), policy?.spent_in_epoch ?? "no policy");
}
