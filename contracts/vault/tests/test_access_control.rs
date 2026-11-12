//! Task 6.2 (part 2): access control on every privileged method.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

/// (method, args) for every admin-only entry point.
fn admin_only_calls(env: &TestEnv) -> Vec<(&'static str, near_sdk::serde_json::Value)> {
    vec![
        (
            "register_agent",
            register_args(&env.agent.id, NEAR, 3 * NEAR, HOUR_NS, &[], None),
        ),
        (
            "update_agent_policy",
            json!({ "agent_id": env.agent.id, "updates": { "per_tx_limit": "1" } }),
        ),
        ("revoke_agent", json!({ "agent_id": env.agent.id })),
        ("unpause_agent", json!({ "agent_id": env.agent.id })),
        ("pause_vault", json!({})),
        ("unpause_vault", json!({})),
        (
            "set_sentinel",
            json!({ "sentinel_account_id": env.attacker.id }),
        ),
    ]
}

#[tokio::test]
async fn attacker_denied_on_every_admin_only_method() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    for (method, args) in admin_only_calls(&env) {
        let res = env.call(&env.attacker, method, args).await?;
        assert_rejected(&res, ERR_ACCESS_DENIED);
    }
    // Nothing changed.
    let st = env.vault_status().await?;
    assert!(!st.paused);
    assert_eq!(
        st.sentinel_account_id.as_ref().map(|a| a.as_str()),
        Some(env.sentinel.id.as_str())
    );
    let p = env.agent_policy(&env.agent.id).await?.unwrap();
    assert!(p.active && !p.revoked);
    assert_eq!(p.version, 1);
    Ok(())
}

#[tokio::test]
async fn sentinel_denied_on_every_admin_only_method() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    for (method, args) in admin_only_calls(&env) {
        let res = env.call(&env.sentinel, method, args).await?;
        assert_rejected(&res, ERR_ACCESS_DENIED);
    }
    Ok(())
}

#[tokio::test]
async fn sentinel_can_pause_agent_but_not_unpause_vault() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;

    env.call_ok(
        &env.sentinel,
        "pause_agent",
        json!({ "agent_id": env.agent.id }),
    )
    .await;
    assert!(!env.agent_policy(&env.agent.id).await?.unwrap().active);

    // Admin pauses the whole vault; the sentinel must not be able to lift it.
    env.call_ok(&env.admin, "pause_vault", json!({})).await;
    let res = env.call(&env.sentinel, "unpause_vault", json!({})).await?;
    assert_rejected(&res, ERR_ACCESS_DENIED);
    assert!(env.vault_status().await?.paused);

    env.call_ok(&env.admin, "unpause_vault", json!({})).await;
    assert!(!env.vault_status().await?.paused);
    Ok(())
}

#[tokio::test]
async fn attacker_cannot_pause_agent() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env
        .call(
            &env.attacker,
            "pause_agent",
            json!({ "agent_id": env.agent.id }),
        )
        .await?;
    assert_rejected(&res, ERR_ACCESS_DENIED);
    assert!(env.agent_policy(&env.agent.id).await?.unwrap().active);
    Ok(())
}

#[tokio::test]
async fn no_sentinel_configured_only_admin_can_pause() -> testresult::TestResult<()> {
    let env = setup_with(false).await?;
    env.register_default_agent().await;
    let res = env
        .call(
            &env.sentinel,
            "pause_agent",
            json!({ "agent_id": env.agent.id }),
        )
        .await?;
    assert_rejected(&res, ERR_ACCESS_DENIED);
    env.call_ok(
        &env.admin,
        "pause_agent",
        json!({ "agent_id": env.agent.id }),
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn admin_can_rotate_sentinel() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    env.call_ok(
        &env.admin,
        "set_sentinel",
        json!({ "sentinel_account_id": env.attacker.id }),
    )
    .await;
    // Old sentinel loses power, new one gains it.
    let res = env
        .call(
            &env.sentinel,
            "pause_agent",
            json!({ "agent_id": env.agent.id }),
        )
        .await?;
    assert_rejected(&res, ERR_ACCESS_DENIED);
    env.call_ok(
        &env.attacker,
        "pause_agent",
        json!({ "agent_id": env.agent.id }),
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn callback_is_private() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env
        .call(
            &env.attacker,
            "on_transfer_complete",
            json!({ "agent_id": env.agent.id, "amount": "1", "epoch_start_ns": "0" }),
        )
        .await?;
    assert!(res.is_failure());
    assert!(error_text(&res).contains("Method on_transfer_complete is private"));
    Ok(())
}

/// Admin methods need exactly 1 yoctoNEAR, which only a full-access key can attach.
/// A function-call key on the admin account (no deposit possible) therefore cannot use them.
#[tokio::test]
async fn admin_methods_require_exactly_one_yocto() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    for (method, args) in admin_only_calls(&env) {
        for deposit in [0u128, 2] {
            let res = env
                .call_with_deposit(
                    &env.admin,
                    method,
                    args.clone(),
                    near_api::NearToken::from_yoctonear(deposit),
                )
                .await?;
            assert_rejected(&res, ERR_ONE_YOCTO_REQUIRED);
        }
    }
    // Nothing changed, and the admin still works with exactly 1 yocto.
    assert!(!env.vault_status().await?.paused);
    env.call_ok(&env.admin, "pause_vault", json!({})).await;
    assert!(env.vault_status().await?.paused);
    Ok(())
}
