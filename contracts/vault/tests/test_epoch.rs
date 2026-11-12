//! Task 6.5 (part 1): epoch reset against a real chain clock.
mod helpers;

use helpers::*;
use std::time::Duration;

const EPOCH_NS: u64 = 6_000_000_000; // 6 seconds

/// Advance the sandbox until the chain clock reaches `target_ns`.
async fn wait_until_chain_time(env: &TestEnv, target_ns: u64) -> testresult::TestResult<()> {
    for _ in 0..120 {
        if env.chain_now_ns().await >= target_ns {
            return Ok(());
        }
        env.sandbox.fast_forward(20).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("chain clock never reached target".into())
}

#[tokio::test]
async fn epoch_resets_after_duration_and_emits_event() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_agent_with(NEAR, 3 * NEAR, EPOCH_NS, None)
        .await;

    // Spend the whole budget; the next transfer is rejected.
    for _ in 0..3 {
        assert!(
            env.transfer(&env.agent, &env.recipient.id, NEAR)
                .await
                .is_success()
        );
    }
    let epoch_start = env
        .agent_policy(&env.agent.id)
        .await?
        .unwrap()
        .epoch_start_ns
        .0;
    assert_eq!(env.spent(&env.agent.id).await, 3 * NEAR);
    let res = env.transfer(&env.agent, &env.recipient.id, 1).await;
    assert_rejected(&res, vault::errors::ERR_EPOCH_LIMIT);

    // Cross the epoch boundary.
    wait_until_chain_time(&env, epoch_start + EPOCH_NS + 1).await?;

    // The view already reports the fresh budget.
    let view = env.agent_policy(&env.agent.id).await?.unwrap();
    assert_eq!(view.spent_in_epoch.0, 0);
    assert_eq!(view.remaining_epoch_budget.0, 3 * NEAR);

    // The next transfer resets stored state and logs an epoch_reset event.
    let res = env.transfer(&env.agent, &env.recipient.id, NEAR).await;
    assert!(res.is_success(), "{res:?}");
    let resets = events_named(&res, "epoch_reset");
    assert_eq!(resets.len(), 1, "{:?}", res.logs());
    assert_eq!(
        resets[0]["data"]["previous_epoch_spend"],
        (3 * NEAR).to_string()
    );
    assert_eq!(resets[0]["data"]["agent_id"], env.agent.id.to_string());

    let after = env.agent_policy(&env.agent.id).await?.unwrap();
    assert_eq!(after.spent_in_epoch.0, NEAR);
    assert!(after.epoch_start_ns.0 >= epoch_start + EPOCH_NS);
    Ok(())
}

#[tokio::test]
async fn no_reset_within_epoch() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await; // 1 hour epoch
    let r1 = env.transfer(&env.agent, &env.recipient.id, NEAR).await;
    let r2 = env.transfer(&env.agent, &env.recipient.id, NEAR).await;
    assert!(events_named(&r1, "epoch_reset").is_empty());
    assert!(events_named(&r2, "epoch_reset").is_empty());
    assert_eq!(env.spent(&env.agent.id).await, 2 * NEAR);
    Ok(())
}
