//! Task 6.5 (part 2): suspension, revocation and global pause.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

#[tokio::test]
async fn admin_and_sentinel_pause_block_transfers_and_admin_can_unpause()
-> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert!(
        env.transfer(&env.agent, &env.recipient.id, 1)
            .await
            .is_success()
    );

    // Sentinel pauses.
    let res = env
        .call(
            &env.sentinel,
            "pause_agent",
            json!({"agent_id": env.agent.id}),
        )
        .await?;
    assert!(res.is_success());
    let ev = events_named(&res, "agent_paused");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["data"]["caller"], env.sentinel.id.to_string());
    assert!(
        ev[0]["data"]["timestamp_ns"].is_string(),
        "timestamps must be strings"
    );
    assert!(!env.agent_policy(&env.agent.id).await?.unwrap().active);
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        ERR_AGENT_PAUSED,
    );

    // Admin can unpause; sentinel cannot.
    let res = env
        .call(
            &env.sentinel,
            "unpause_agent",
            json!({"agent_id": env.agent.id}),
        )
        .await?;
    assert_rejected(&res, ERR_ACCESS_DENIED);
    let res = env
        .call(
            &env.admin,
            "unpause_agent",
            json!({"agent_id": env.agent.id}),
        )
        .await?;
    assert!(res.is_success());
    assert_eq!(events_named(&res, "agent_unpaused").len(), 1);
    assert!(
        env.transfer(&env.agent, &env.recipient.id, 1)
            .await
            .is_success()
    );

    // Admin pauses too.
    let res = env
        .call(&env.admin, "pause_agent", json!({"agent_id": env.agent.id}))
        .await?;
    assert!(res.is_success());
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        ERR_AGENT_PAUSED,
    );
    Ok(())
}

#[tokio::test]
async fn revoke_is_permanent() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    let res = env
        .call(
            &env.admin,
            "revoke_agent",
            json!({"agent_id": env.agent.id}),
        )
        .await?;
    assert!(res.is_success());
    let ev = events_named(&res, "agent_revoked");
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["data"]["caller"], env.admin.id.to_string());

    let p = env.agent_policy(&env.agent.id).await?.unwrap();
    assert!(p.revoked && !p.active);
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        ERR_AGENT_REVOKED,
    );

    // Neither unpause nor update nor re-register can bring it back.
    let res = env
        .call(
            &env.admin,
            "unpause_agent",
            json!({"agent_id": env.agent.id}),
        )
        .await?;
    assert_rejected(&res, ERR_REVOKED_AGENT);
    let res = env
        .call(
            &env.admin,
            "register_agent",
            register_args(
                &env.agent.id,
                NEAR,
                3 * NEAR,
                HOUR_NS,
                &[&env.recipient.id],
                None,
            ),
        )
        .await?;
    assert_rejected(&res, ERR_DUPLICATE_AGENT);
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        ERR_AGENT_REVOKED,
    );

    // Sentinel pausing a revoked agent must not revive it.
    env.call_ok(
        &env.sentinel,
        "pause_agent",
        json!({"agent_id": env.agent.id}),
    )
    .await;
    assert!(env.agent_policy(&env.agent.id).await?.unwrap().revoked);
    Ok(())
}

#[tokio::test]
async fn pause_vault_blocks_all_agents_and_unpause_restores() -> testresult::TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;

    let res = env.call(&env.admin, "pause_vault", json!({})).await?;
    assert!(res.is_success());
    assert_eq!(events_named(&res, "vault_paused").len(), 1);
    assert!(env.vault_status().await?.paused);
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        ERR_VAULT_PAUSED,
    );

    let res = env.call(&env.admin, "unpause_vault", json!({})).await?;
    assert!(res.is_success());
    assert_eq!(events_named(&res, "vault_unpaused").len(), 1);
    assert!(!env.vault_status().await?.paused);
    assert!(
        env.transfer(&env.agent, &env.recipient.id, 1)
            .await
            .is_success()
    );
    Ok(())
}

#[tokio::test]
async fn pause_unknown_agent_rejected() -> testresult::TestResult<()> {
    let env = setup().await?;
    for (caller, m) in [
        (&env.admin, "pause_agent"),
        (&env.admin, "revoke_agent"),
        (&env.admin, "unpause_agent"),
        (&env.sentinel, "pause_agent"),
    ] {
        let res = env
            .call(caller, m, json!({"agent_id": env.agent.id}))
            .await?;
        assert_rejected(&res, ERR_AGENT_NOT_FOUND);
    }
    Ok(())
}
