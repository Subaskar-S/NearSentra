//! Task 6.3: agent registration and policy management.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

#[tokio::test]
async fn register_stores_exact_policy() -> testresult::TestResult<()> {
    let env = setup().await?;
    let expiry = env.chain_now_ns().await + 10 * HOUR_NS;
    env.register_agent_with(NEAR, 3 * NEAR, HOUR_NS, Some(expiry))
        .await;

    let p = env.agent_policy(&env.agent.id).await?.expect("registered");
    assert_eq!(p.agent_id.as_str(), env.agent.id.as_str());
    assert!(p.active && !p.revoked);
    assert_eq!(p.per_tx_limit.0, NEAR);
    assert_eq!(p.epoch_limit.0, 3 * NEAR);
    assert_eq!(p.epoch_duration_ns.0, HOUR_NS);
    assert_eq!(p.spent_in_epoch.0, 0);
    assert_eq!(p.remaining_epoch_budget.0, 3 * NEAR);
    assert_eq!(p.expiry_ns.map(|e| e.0), Some(expiry));
    assert_eq!(p.version, 1);
    assert_eq!(p.allowlist.len(), 1);
    assert_eq!(p.allowlist[0].as_str(), env.recipient.id.as_str());
    Ok(())
}

#[tokio::test]
async fn duplicate_registration_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env
        .call(
            &env.admin,
            "register_agent",
            register_args(&env.agent.id, NEAR, 3 * NEAR, HOUR_NS, &[], None),
        )
        .await?;
    assert_rejected(&res, ERR_DUPLICATE_AGENT);
    // Original policy untouched.
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn invalid_policy_params_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    let a = &env.agent.id;
    let cases = [
        (
            register_args(a, 4 * NEAR, 3 * NEAR, HOUR_NS, &[], None),
            ERR_TX_EXCEEDS_EPOCH,
        ),
        (
            register_args(a, 0, 3 * NEAR, HOUR_NS, &[], None),
            ERR_AMOUNT_ZERO,
        ),
        (
            register_args(a, NEAR, 0, HOUR_NS, &[], None),
            ERR_AMOUNT_ZERO,
        ),
        (
            register_args(a, NEAR, 3 * NEAR, 0, &[], None),
            ERR_EPOCH_DURATION_ZERO,
        ),
        (
            register_args(a, NEAR, 3 * NEAR, HOUR_NS, &[], Some(1)),
            ERR_EXPIRY_IN_PAST,
        ),
    ];
    for (args, err) in cases {
        let res = env.call(&env.admin, "register_agent", args).await?;
        assert_rejected(&res, err);
    }
    assert!(env.agent_policy(a).await?.is_none());
    Ok(())
}

fn many_accounts(n: usize) -> Vec<near_sdk::AccountId> {
    (0..n)
        .map(|i| format!("r{i}.sandbox").parse().unwrap())
        .collect()
}

#[tokio::test]
async fn allowlist_dedupe_and_capacity() -> testresult::TestResult<()> {
    let env = setup().await?;
    // Duplicates collapse on registration.
    env.call_ok(
        &env.admin,
        "register_agent",
        json!({"agent_id": env.agent.id, "params": {
            "per_tx_limit": "10", "epoch_limit": "100", "epoch_duration_ns": HOUR_NS.to_string(),
            "allowlist": [env.recipient.id, env.recipient.id], "expiry_ns": null }}),
    )
    .await;
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        1
    );

    // Idempotent add / remove.
    let add = json!({"agent_id": env.agent.id, "updates": {"allowlist_add": [env.recipient.id]}});
    env.call_ok(&env.admin, "update_agent_policy", add).await;
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        1
    );
    let rm_absent =
        json!({"agent_id": env.agent.id, "updates": {"allowlist_remove": [env.attacker.id]}});
    env.call_ok(&env.admin, "update_agent_policy", rm_absent)
        .await;
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        1
    );

    // Fill to the 50-entry cap (49 more), then the 51st is rejected.
    let fill = many_accounts(49);
    env.call_ok(
        &env.admin,
        "update_agent_policy",
        json!({"agent_id": env.agent.id, "updates": {"allowlist_add": fill}}),
    )
    .await;
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        50
    );
    let res = env
        .call(
            &env.admin,
            "update_agent_policy",
            json!({"agent_id": env.agent.id, "updates": {"allowlist_add": [env.attacker.id]}}),
        )
        .await?;
    assert_rejected(&res, ERR_ALLOWLIST_FULL);
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        50
    );

    // Removal frees a slot.
    env.call_ok(
        &env.admin,
        "update_agent_policy",
        json!({"agent_id": env.agent.id, "updates": {"allowlist_remove": [env.recipient.id]}}),
    )
    .await;
    assert_eq!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .allowlist
            .len(),
        49
    );
    Ok(())
}

#[tokio::test]
async fn register_with_51_entries_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    let res = env
        .call(
            &env.admin,
            "register_agent",
            json!({"agent_id": env.agent.id, "params": {
                "per_tx_limit": "10", "epoch_limit": "100",
                "epoch_duration_ns": HOUR_NS.to_string(),
                "allowlist": many_accounts(51), "expiry_ns": null }}),
        )
        .await?;
    assert_rejected(&res, ERR_ALLOWLIST_FULL);
    Ok(())
}

#[tokio::test]
async fn update_increments_version_and_applies_fields() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    env.call_ok(
        &env.admin,
        "update_agent_policy",
        json!({"agent_id": env.agent.id, "updates": {
            "per_tx_limit": (2 * NEAR).to_string(), "epoch_limit": (5 * NEAR).to_string() }}),
    )
    .await;
    let p = env.agent_policy(&env.agent.id).await?.unwrap();
    assert_eq!(p.version, 2);
    assert_eq!(p.per_tx_limit.0, 2 * NEAR);
    assert_eq!(p.epoch_limit.0, 5 * NEAR);

    // per_tx > epoch after update is rejected and nothing changes.
    let res = env
        .call(
            &env.admin,
            "update_agent_policy",
            json!({"agent_id": env.agent.id, "updates": {"per_tx_limit": (9 * NEAR).to_string()}}),
        )
        .await?;
    assert_rejected(&res, ERR_TX_EXCEEDS_EPOCH);
    assert_eq!(env.agent_policy(&env.agent.id).await?.unwrap().version, 2);
    Ok(())
}

#[tokio::test]
async fn update_unknown_and_revoked_agents() -> testresult::TestResult<()> {
    let env = setup().await?;
    let upd = |a: &near_api::AccountId| json!({"agent_id": a, "updates": {"per_tx_limit": "1"}});
    let res = env
        .call(&env.admin, "update_agent_policy", upd(&env.agent.id))
        .await?;
    assert_rejected(&res, ERR_AGENT_NOT_FOUND);

    env.register_default_agent().await;
    env.call_ok(
        &env.admin,
        "revoke_agent",
        json!({"agent_id": env.agent.id}),
    )
    .await;
    let res = env
        .call(&env.admin, "update_agent_policy", upd(&env.agent.id))
        .await?;
    assert_rejected(&res, ERR_REVOKED_AGENT);
    Ok(())
}

#[tokio::test]
async fn expiry_can_be_set_and_cleared() -> testresult::TestResult<()> {
    let env = setup().await?;
    let expiry = env.chain_now_ns().await + 10 * HOUR_NS;
    env.register_agent_with(NEAR, 3 * NEAR, HOUR_NS, Some(expiry))
        .await;

    // null clears it.
    env.call_ok(
        &env.admin,
        "update_agent_policy",
        json!({"agent_id": env.agent.id, "updates": {"expiry_ns": null}}),
    )
    .await;
    assert!(
        env.agent_policy(&env.agent.id)
            .await?
            .unwrap()
            .expiry_ns
            .is_none()
    );

    // A past expiry is rejected.
    let res = env
        .call(
            &env.admin,
            "update_agent_policy",
            json!({"agent_id": env.agent.id, "updates": {"expiry_ns": "1"}}),
        )
        .await?;
    assert_rejected(&res, ERR_EXPIRY_IN_PAST);
    Ok(())
}

#[tokio::test]
async fn unknown_agent_view_is_none() -> testresult::TestResult<()> {
    let env = setup().await?;
    assert!(env.agent_policy(&env.attacker.id).await?.is_none());
    Ok(())
}
