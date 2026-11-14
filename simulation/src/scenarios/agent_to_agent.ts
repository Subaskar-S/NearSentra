// Agent-to-agent payment: ResearchAgent buys a 2 NEAR market-analysis service from AnalysisAgent.
//
// The same steps run in two places: as scenario 6 of `make simulate` (the agent signs on-chain
// directly) and as `make demo-agent-payment` (requests go through the Gateway so the dashboard shows
// them). Only how a request reaches the vault differs; the vault contract makes every decision.
import {
  NEAR,
  assertOutcome,
  assertRejected,
  callVault,
  getBalance,
  getPolicy,
  registerAgent,
  requestTransfer,
  show,
} from "../helpers.js";
import type { Context, TxResult } from "../helpers.js";

/** Send one payment request from ResearchAgent to `recipient` and report what the chain decided. */
export type Pay = (recipient: string, amount: bigint) => Promise<TxResult>;

/** The price AnalysisAgent asks for the service. */
export const SERVICE_PRICE = NEAR("2");

function near(yocto: bigint): string {
  const whole = yocto / NEAR("1");
  const cents = ((yocto % NEAR("1")) * 100n) / NEAR("1");
  return `${whole}.${cents.toString().padStart(2, "0")} NEAR`;
}

/** The direct on-chain `Pay`: ResearchAgent signs `request_transfer` with its own scoped key. */
export function payDirect(ctx: Context): Pay {
  const research = ctx.agents[5]!;
  return (recipient, amount) => requestTransfer(ctx, research.account, recipient, amount);
}

export async function runAgentToAgent(ctx: Context, name: string, pay: Pay): Promise<void> {
  const research = ctx.agents[5]!;
  const seller = ctx.analysisId;

  // Setup: the admin gives ResearchAgent a policy that fits the deal. Per-tx limit equals the price,
  // the epoch budget is 3 NEAR, and AnalysisAgent is the only allowlisted recipient.
  if (await getPolicy(ctx, research.id)) {
    console.error(`${name}: ${research.id} is already registered on this network. Restart "make sandbox-deploy" for a fresh run.`);
    process.exit(1);
  }
  await registerAgent(ctx, research.id, SERVICE_PRICE, NEAR("3"), [seller]);
  console.log(`  ${research.id} may pay ${seller} at most ${near(SERVICE_PRICE)} per payment and 3.00 NEAR per epoch.`);

  const sellerStart = await getBalance(ctx, seller);

  // 1. The agreed purchase. The contract approves, pays, and settles the reservation.
  console.log(`\n  ${seller} offers the market analysis for ${near(SERVICE_PRICE)}.`);
  const paid = show(`${name} / ${research.id} pays ${near(SERVICE_PRICE)} to ${seller}`, await pay(seller, SERVICE_PRICE));
  assertOutcome(`${name} / agreed payment`, "SUCCESS", paid.status);
  const sellerPaid = await getBalance(ctx, seller);
  assertOutcome(`${name} / seller received exactly the price`, SERVICE_PRICE.toString(), (sellerPaid - sellerStart).toString());
  const afterPay = await getPolicy(ctx, research.id);
  assertOutcome(`${name} / spent_in_epoch`, SERVICE_PRICE.toString(), afterPay?.spent_in_epoch ?? "no policy");
  assertOutcome(`${name} / reservation settled`, "null", String(afterPay?.pending_transfer_id ?? null));

  // 2. The seller raises its price above the per-transaction limit.
  console.log(`\n  ${seller} now asks for ${near(NEAR("2.5"))}.`);
  const overcharge = show(`${name} / ${near(NEAR("2.5"))} exceeds the per-transaction limit`, await pay(seller, NEAR("2.5")));
  assertRejected(`${name} / per-tx limit`, overcharge, "ERR_PER_TX_LIMIT");

  // 3. A second purchase at the agreed price would pass the epoch budget (2 + 2 > 3).
  console.log(`\n  ${research.id} tries to buy the analysis a second time.`);
  const second = show(`${name} / a second ${near(SERVICE_PRICE)} exceeds the epoch budget`, await pay(seller, SERVICE_PRICE));
  assertRejected(`${name} / epoch budget`, second, "ERR_EPOCH_LIMIT");

  // 4. A prompt-injected instruction redirects the payment to a stranger.
  console.log(`\n  A prompt injection tells ${research.id} to pay ${ctx.attackerId} instead.`);
  const redirect = show(`${name} / payment to a recipient that is not allowlisted`, await pay(ctx.attackerId, NEAR("0.5")));
  assertRejected(`${name} / allowlist`, redirect, "ERR_RECIPIENT_NOT_ALLOWED");

  // 5. Someone else tries to spend ResearchAgent's budget. The vault uses the on-chain caller as the
  // identity, so the attacker's own account is rejected no matter what any request claims.
  console.log(`\n  ${ctx.attackerId} signs a payment to ${seller} with its own key.`);
  const impostor = show(`${name} / unregistered caller`, await requestTransfer(ctx, ctx.attacker, seller, SERVICE_PRICE));
  assertRejected(`${name} / identity comes from the chain`, impostor, "ERR_AGENT_NOT_REGISTERED");

  // 6. The admin pauses ResearchAgent. Even an in-policy payment is now rejected.
  console.log(`\n  The admin pauses ${research.id}.`);
  const pause = show(`${name} / pause_agent signed by admin`, await callVault(ctx, ctx.admin, "pause_agent", { agent_id: research.id }));
  assertOutcome(`${name} / pause_agent`, "SUCCESS", pause.status);
  const paused = show(`${name} / in-policy payment while paused`, await pay(seller, NEAR("0.1")));
  assertRejected(`${name} / paused agent`, paused, "ERR_AGENT_PAUSED");

  // None of the rejections moved money or changed the counter.
  const sellerEnd = await getBalance(ctx, seller);
  assertOutcome(`${name} / seller balance unchanged by rejections`, sellerPaid.toString(), sellerEnd.toString());
  const end = await getPolicy(ctx, research.id);
  assertOutcome(`${name} / spent_in_epoch unchanged by rejections`, SERVICE_PRICE.toString(), end?.spent_in_epoch ?? "no policy");
}
