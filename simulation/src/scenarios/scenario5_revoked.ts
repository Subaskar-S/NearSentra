// Scenario 5: Revoked Agent. After revocation the agent is permanently locked out.
import { NEAR, ONE_YOCTO, assertOutcome, assertRejected, callVault, registerAgent, requestTransfer, show } from "../helpers.js";
import type { Context } from "../helpers.js";

export async function run(ctx: Context): Promise<void> {
  const name = "Scenario 5: Revoked Agent";
  const agent = ctx.agents[4]!;
  await registerAgent(ctx, agent.id, NEAR("1"), NEAR("3"), [ctx.recipientId]);

  const before = show(`${name} / transfer before revocation`, await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("0.1")));
  assertOutcome(`${name} / works before revocation`, "SUCCESS", before.status);

  const revoke = show(`${name} / revoke_agent signed by admin`, await callVault(ctx, ctx.admin, "revoke_agent", { agent_id: agent.id }, ONE_YOCTO));
  assertOutcome(`${name} / revoke_agent`, "SUCCESS", revoke.status);

  const after = show(`${name} / transfer after revocation`, await requestTransfer(ctx, agent.account, ctx.recipientId, NEAR("0.1")));
  assertRejected(`${name} / revoked agent is blocked`, after, "ERR_AGENT_REVOKED");
}
