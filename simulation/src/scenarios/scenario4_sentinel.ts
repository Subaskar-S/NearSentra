// Scenario 4: Compromised Sentinel. Even with the Sentinel's full-access key, the attacker
// can do nothing but pause agents.
import { assertRejected, callVault, NEAR, ONE_YOCTO, printScenarioResult, show } from "../helpers.js";
import type { Context } from "../helpers.js";

export async function run(ctx: Context): Promise<void> {
  const name = "Scenario 4: Compromised Sentinel";

  // The Sentinel account is not a registered agent, so it cannot move funds.
  const transfer = show(
    `${name} / request_transfer`,
    await callVault(ctx, ctx.sentinelOwner, "request_transfer", { recipient: ctx.attackerId, amount: NEAR("1").toString() }),
  );
  assertRejected(`${name} / request_transfer`, transfer, "ERR_AGENT_NOT_REGISTERED");

  const update = show(
    `${name} / update_agent_policy`,
    await callVault(ctx, ctx.sentinelOwner, "update_agent_policy", {
      agent_id: ctx.agents[0]!.id,
      updates: { per_tx_limit: NEAR("1000").toString(), epoch_limit: NEAR("1000").toString(), allowlist_add: [ctx.attackerId] },
    }, ONE_YOCTO),
  );
  assertRejected(`${name} / update_agent_policy`, update, "ERR_ACCESS_DENIED");

  const unpause = show(`${name} / unpause_vault`, await callVault(ctx, ctx.sentinelOwner, "unpause_vault", {}, ONE_YOCTO));
  assertRejected(`${name} / unpause_vault`, unpause, "ERR_ACCESS_DENIED");

  // The Sentinel's real runtime key is a function-call key limited to pause_agent, so the
  // network refuses these calls before the contract is even reached.
  for (const method of ["request_transfer", "update_agent_policy", "unpause_vault"]) {
    let refusal: string;
    try {
      const res = await callVault(ctx, ctx.sentinelScoped, method, {});
      if (res.status === "SUCCESS") refusal = "";
      else {
        printScenarioResult(`${name} / scoped key, ${method}`, res.txHashHex, res.status, res.error);
        refusal = res.error ?? res.status;
      }
    } catch (err) {
      refusal = err instanceof Error ? err.message : String(err);
      console.log(`  [${name} / scoped key, ${method}]`);
      console.log(`    not included in a block; refused by the network: ${refusal}`);
    }
    if (refusal === "") {
      console.error(`ASSERTION FAILED in ${name}: scoped sentinel key executed ${method}`);
      process.exit(1);
    }
  }
}
