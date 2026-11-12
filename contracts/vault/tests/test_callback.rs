//! Task 6.6: counter rollback when the transfer promise fails.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;

#[tokio::test]
async fn failed_transfer_rolls_back_counter_and_emits_event() -> testresult::TestResult<()> {
    let env = setup().await?;
    // Allowlist an account that does not exist: the Transfer receipt will fail.
    let ghost: near_api::AccountId = "ghost.sandbox".parse()?;
    env.call_ok(
        &env.admin,
        "register_agent",
        register_args(
            &env.agent.id,
            NEAR,
            3 * NEAR,
            HOUR_NS,
            &[&env.recipient.id, &ghost],
            None,
        ),
    )
    .await;

    // One good transfer so the counter is non-zero before the failing one.
    assert!(
        env.transfer(&env.agent, &env.recipient.id, NEAR / 2)
            .await
            .is_success()
    );
    let before = env.spent(&env.agent.id).await;
    assert_eq!(before, NEAR / 2);

    let res = env.transfer(&env.agent, &ghost, NEAR / 4).await;

    // The request was approved, the transfer receipt failed, the callback rolled back.
    let approved = events_named(&res, "transfer_requested");
    assert_eq!(approved[0]["data"]["decision"], "approved");
    assert!(
        !res.logs().is_empty() && !events_named(&res, "transfer_rolled_back").is_empty(),
        "expected transfer_rolled_back; logs: {:?}",
        res.logs()
    );
    let rb = events_named(&res, "transfer_rolled_back");
    assert_eq!(rb[0]["data"]["amount"], (NEAR / 4).to_string());
    assert_eq!(rb[0]["data"]["agent_id"], env.agent.id.to_string());

    assert_eq!(
        env.spent(&env.agent.id).await,
        before,
        "counter must be restored"
    );

    // Budget is genuinely available again: spend the remaining epoch budget.
    let remaining = env
        .agent_policy(&env.agent.id)
        .await?
        .unwrap()
        .remaining_epoch_budget
        .0;
    assert_eq!(remaining, 3 * NEAR - before);
    assert!(
        env.transfer(&env.agent, &env.recipient.id, NEAR)
            .await
            .is_success()
    );
    Ok(())
}

#[tokio::test]
async fn successful_transfer_emits_no_rollback() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env.transfer(&env.agent, &env.recipient.id, NEAR / 2).await;
    assert!(res.is_success());
    assert!(events_named(&res, "transfer_rolled_back").is_empty());
    assert_eq!(env.spent(&env.agent.id).await, NEAR / 2);
    Ok(())
}

#[tokio::test]
async fn direct_callback_invocation_is_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert!(
        env.transfer(&env.agent, &env.recipient.id, NEAR)
            .await
            .is_success()
    );
    // Even the admin cannot call the private callback to reset the counter.
    let res = env
        .call(
            &env.admin,
            "on_transfer_complete",
            json!({"agent_id": env.agent.id, "amount": NEAR.to_string(), "epoch_start_ns": "0"}),
        )
        .await?;
    assert!(res.is_failure());
    assert_eq!(env.spent(&env.agent.id).await, NEAR);
    Ok(())
}

/// Gas for the rollback callback is reserved up front: a call that cannot afford it fails and
/// reverts (counter included) instead of approving a payout whose rollback might not run.
#[tokio::test]
async fn too_little_gas_fails_cleanly_without_touching_the_counter() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = near_api::Contract(env.vault.clone())
        .call_function(
            "request_transfer",
            json!({ "recipient": env.recipient.id, "amount": (NEAR / 2).to_string() }),
        )
        .transaction()
        .gas(near_api::NearGas::from_tgas(6))
        .with_signer(env.agent.id.clone(), env.agent.signer.clone())
        .send_to(&env.network)
        .await?;
    assert!(
        res.is_failure(),
        "6 Tgas cannot cover the 5 Tgas reserve plus execution"
    );
    assert_eq!(env.spent(&env.agent.id).await, 0);
    // With a normal amount of gas the same request succeeds.
    assert!(
        env.transfer(&env.agent, &env.recipient.id, NEAR / 2)
            .await
            .is_success()
    );
    assert_eq!(env.spent(&env.agent.id).await, NEAR / 2);
    Ok(())
}
