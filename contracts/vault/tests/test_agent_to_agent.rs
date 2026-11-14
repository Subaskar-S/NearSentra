//! Agent-to-agent payment: ResearchAgent buys a 2 NEAR service from AnalysisAgent.
//!
//! `env.agent` plays ResearchAgent (a registered agent) and `env.recipient` plays AnalysisAgent (the
//! seller). The vault is the only authority: every rejection leaves balances and the spent counter
//! untouched, and every approval settles exactly once.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

const PRICE: u128 = 2 * NEAR;

#[tokio::test]
async fn research_agent_pays_analysis_agent_under_policy() -> testresult::TestResult<()> {
    let env = setup().await?;
    let research = &env.agent;
    let analysis = &env.recipient;
    // Per-tx 2 NEAR (the service price), 3 NEAR budget per epoch, only AnalysisAgent allowlisted.
    env.register_agent_with(PRICE, 3 * NEAR, HOUR_NS, None)
        .await;
    let seller_start = env.balance(&analysis.id).await?.as_yoctonear();

    // 1. The agreed 2 NEAR payment is approved and settles exactly once.
    let paid = env.transfer(research, &analysis.id, PRICE).await;
    assert!(paid.is_success(), "{paid:?}");
    let ev = events_named(&paid, "transfer_requested");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["data"]["decision"], "approved");
    assert_eq!(ev[0]["data"]["agent_id"], research.id.to_string());
    assert_eq!(ev[0]["data"]["recipient"], analysis.id.to_string());
    assert_eq!(ev[0]["data"]["amount"], PRICE.to_string());

    let seller_paid = env.balance(&analysis.id).await?.as_yoctonear();
    assert_eq!(
        seller_paid - seller_start,
        PRICE,
        "seller must receive exactly the price"
    );
    let policy = env.agent_policy(&research.id).await?.expect("registered");
    assert_eq!(policy.spent_in_epoch.0, PRICE);
    assert!(
        policy.pending_transfer_id.is_none() && policy.pending_amount.is_none(),
        "the reservation must be settled once the callback has run"
    );

    // 2. The seller raises its price above the per-transaction limit: rejected, nothing moves.
    let overcharge = env.transfer(research, &analysis.id, PRICE + NEAR / 2).await;
    assert_rejected(&overcharge, ERR_PER_TX_LIMIT);
    let ev = events_named(&overcharge, "transfer_requested");
    assert_eq!(ev[0]["data"]["decision"], "rejected");

    // 3. A second 2 NEAR purchase would pass the 3 NEAR epoch budget: rejected.
    let over_budget = env.transfer(research, &analysis.id, PRICE).await;
    assert_rejected(&over_budget, ERR_EPOCH_LIMIT);

    // 4. A prompt-injected redirect to a stranger is rejected (recipient not allowlisted).
    let redirect = env.transfer(research, &env.attacker.id, NEAR / 2).await;
    assert_rejected(&redirect, ERR_RECIPIENT_NOT_ALLOWED);

    // 5. Someone else cannot spend ResearchAgent's budget: identity is the on-chain caller.
    let impostor = env.transfer(&env.attacker, &analysis.id, PRICE).await;
    assert_rejected(&impostor, ERR_AGENT_NOT_REGISTERED);

    // None of the rejections changed any balance or counter.
    assert_eq!(env.balance(&analysis.id).await?.as_yoctonear(), seller_paid);
    assert_eq!(env.spent(&research.id).await, PRICE);

    // 6. Once the admin pauses ResearchAgent, even an in-policy payment is rejected.
    env.call_ok(
        &env.admin,
        "pause_agent",
        json!({ "agent_id": research.id }),
    )
    .await;
    let paused = env.transfer(research, &analysis.id, NEAR / 10).await;
    assert_rejected(&paused, ERR_AGENT_PAUSED);
    assert_eq!(env.balance(&analysis.id).await?.as_yoctonear(), seller_paid);
    assert_eq!(env.spent(&research.id).await, PRICE);
    Ok(())
}
