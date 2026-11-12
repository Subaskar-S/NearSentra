// Scenario 2: Rogue Agent. An agent tries to exceed its limit and to pay a stranger.
import { NEAR, assertOutcome, assertRejected, getPolicy, registerAgent, requestTransfer, show } from "../helpers.js";
import type { Context } from "../helpers.js";

export async function run(ctx: Context): Promise<void> {
  const name = "Scenario 2: Rogue Agent";
  const agent = ctx.agents[1]!;
  await registerAgent(ctx, agent.id, NEAR("1"), NEAR("3"), [ctx.recipientId]);

  const overLimit = show(
    `${name} / 2 NEAR exceeds the 1 NEAR per-tx limit`,
    await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("2")),
  );
  assertRejected(`${name} / per-tx limit`, overLimit, "ERR_PER_TX_LIMIT");

  const stranger = show(
    `${name} / 0.5 NEAR to a recipient that is not allowlisted`,
    await requestTransfer(ctx, agent.account, ctx.attackerId, NEAR("0.5")),
  );
  assertRejected(`${name} / allowlist`, stranger, "ERR_RECIPIENT_NOT_ALLOWED");

  const policy = await getPolicy(ctx, agent.id);
  assertOutcome(`${name} / nothing was spent`, "0", policy?.spent_in_epoch ?? "no policy");
}
