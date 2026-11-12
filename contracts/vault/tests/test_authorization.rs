//! Task 6.4: transfer authorization, every approval and rejection path.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

#[tokio::test]
async fn approved_transfer_moves_funds_and_emits_event() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let before = env.balance(&env.recipient.id).await?;

    let amount = NEAR / 2;
    let res = env.transfer(&env.agent, &env.recipient.id, amount).await;
    assert!(res.is_success(), "{res:?}");

    assert_eq!(env.spent(&env.agent.id).await, amount);
    let after = env.balance(&env.recipient.id).await?;
    assert_eq!(after.as_yoctonear() - before.as_yoctonear(), amount);

    let ev = events_named(&res, "transfer_requested");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["standard"], "nearsentra");
    assert_eq!(ev[0]["data"]["decision"], "approved");
    assert_eq!(ev[0]["data"]["amount"], amount.to_string());
    assert_eq!(ev[0]["data"]["agent_id"], env.agent.id.to_string());
    assert_eq!(ev[0]["data"]["recipient"], env.recipient.id.to_string());
    assert!(ev[0]["data"]["rejection_reason"].is_null());
    Ok(())
}

/// Assert a rejection: exact error, a `rejected` event in the failed receipt's
/// logs (verifies logs persist on failure), and no change to `spent_in_epoch`.
async fn assert_rejection(
    env: &TestEnv,
    caller: &Actor,
    recipient: &near_api::AccountId,
    amount: u128,
    err: &str,
    expect_event: bool,
) {
    let spent_before = env.spent(&env.agent.id).await;
    let res = env.transfer(caller, recipient, amount).await;
    assert_rejected(&res, err);
    if expect_event {
        let ev = events_named(&res, "transfer_requested");
        assert_eq!(
            ev.len(),
            1,
            "expected one rejection event for {err}: {:?}",
            res.logs()
        );
        assert_eq!(ev[0]["data"]["decision"], "rejected");
        assert!(
            ev[0]["data"]["rejection_reason"]
                .as_str()
                .unwrap()
                .starts_with(err.split(':').next().unwrap())
        );
    }
    assert_eq!(
        env.spent(&env.agent.id).await,
        spent_before,
        "{err} mutated state"
    );
}

#[tokio::test]
async fn vault_paused_rejects() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    env.call_ok(&env.admin, "pause_vault", json!({})).await;
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        1,
        ERR_VAULT_PAUSED,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn unregistered_caller_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env.transfer(&env.attacker, &env.recipient.id, 1).await;
    assert_rejected(&res, ERR_AGENT_NOT_REGISTERED);
    assert_eq!(events_named(&res, "transfer_requested").len(), 1);
    Ok(())
}

#[tokio::test]
async fn paused_agent_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    env.call_ok(&env.admin, "pause_agent", json!({"agent_id": env.agent.id}))
        .await;
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        1,
        ERR_AGENT_PAUSED,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn revoked_agent_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    env.call_ok(
        &env.admin,
        "revoke_agent",
        json!({"agent_id": env.agent.id}),
    )
    .await;
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        1,
        ERR_AGENT_REVOKED,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn expired_policy_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    let expiry = env.chain_now_ns().await + 4_000_000_000; // ~4s from now
    env.register_agent_with(NEAR, 3 * NEAR, HOUR_NS, Some(expiry))
        .await;
    // Wait until the chain clock passes the expiry.
    for _ in 0..60 {
        if env.chain_now_ns().await >= expiry {
            break;
        }
        env.sandbox.fast_forward(50).await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(
        env.chain_now_ns().await >= expiry,
        "chain clock never reached expiry"
    );
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        1,
        ERR_POLICY_EXPIRED,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn recipient_not_allowlisted_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert_rejection(
        &env,
        &env.agent,
        &env.attacker.id,
        1,
        ERR_RECIPIENT_NOT_ALLOWED,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn zero_amount_rejected_with_event() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        0,
        ERR_AMOUNT_ZERO,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn per_tx_limit_rejected_and_boundary_accepted() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        NEAR + 1,
        ERR_PER_TX_LIMIT,
        true,
    )
    .await;
    // Exactly the limit is fine.
    let res = env.transfer(&env.agent, &env.recipient.id, NEAR).await;
    assert!(res.is_success(), "{res:?}");
    assert_eq!(env.spent(&env.agent.id).await, NEAR);
    Ok(())
}

#[tokio::test]
async fn epoch_limit_rejected_when_cumulative_exceeds() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await; // per-tx 1 NEAR, epoch 3 NEAR
    for _ in 0..3 {
        let res = env.transfer(&env.agent, &env.recipient.id, NEAR).await;
        assert!(res.is_success(), "{res:?}");
    }
    assert_eq!(env.spent(&env.agent.id).await, 3 * NEAR);
    assert_rejection(
        &env,
        &env.agent,
        &env.recipient.id,
        1,
        ERR_EPOCH_LIMIT,
        true,
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn attacker_cannot_spend_via_agent_policy() -> testresult::TestResult<()> {
    // Identity is the predecessor account: an attacker cannot claim the agent's budget.
    let env = setup().await?;
    env.register_default_agent().await;
    let before = env.balance(&env.attacker.id).await?;
    let res = env.transfer(&env.attacker, &env.attacker.id, 1).await;
    assert_rejected(&res, ERR_AGENT_NOT_REGISTERED);
    let after = env.balance(&env.attacker.id).await?;
    assert!(after.as_yoctonear() <= before.as_yoctonear());
    Ok(())
}
