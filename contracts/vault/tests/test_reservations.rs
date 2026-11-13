//! End-to-end tests of the single-pending-transfer reservation model against a real sandbox
//! chain: concurrent requests, epoch rollover with a failing payout, reservation IDs, callback
//! gas headroom, caller identity and a compromised agent key.
mod helpers;

use futures::future::join_all;
use helpers::*;
use near_sdk::serde_json::json;
use testresult::TestResult;

const EPOCH_NS: u64 = 6_000_000_000; // 6 seconds

fn ghost() -> near_api::AccountId {
    "ghost.sandbox".parse().expect("valid account id")
}

/// Many requests in flight at once (rotating scoped keys, so no nonce collisions) must never
/// push the agent past its epoch budget, and the counter must equal what was really paid out.
#[tokio::test]
async fn concurrent_requests_never_exceed_the_epoch_budget() -> TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await; // 1 NEAR per tx, 3 NEAR per epoch
    let pool = env.scoped_key_pool(&env.agent, 8).await?;
    let before = env.balance(&env.recipient.id).await?.as_yoctonear();

    let results =
        join_all((0..8).map(|_| env.transfer_as(&pool, &env.agent.id, &env.recipient.id, NEAR)))
            .await;

    let (mut approved, mut pending, mut epoch_limit) = (0u128, 0u32, 0u32);
    for res in results {
        let res = res?;
        if res.is_success() {
            approved += 1;
        } else {
            let text = error_text(&res);
            if text.contains("ERR_TRANSFER_PENDING") {
                pending += 1;
            } else if text.contains("ERR_EPOCH_LIMIT") {
                epoch_limit += 1;
            } else {
                panic!("unexpected failure: {text}");
            }
        }
    }
    println!("approved={approved} rejected_pending={pending} rejected_epoch_limit={epoch_limit}");

    assert_eq!(approved + u128::from(pending) + u128::from(epoch_limit), 8);
    assert!((1..=3).contains(&approved), "approved = {approved}");

    let view = env.agent_policy(&env.agent.id).await?.expect("registered");
    assert_eq!(
        view.spent_in_epoch.0,
        approved * NEAR,
        "counter == what was paid"
    );
    assert!(
        view.spent_in_epoch.0 <= 3 * NEAR,
        "never above the epoch limit"
    );
    assert!(
        view.pending_transfer_id.is_none(),
        "every reservation settled"
    );
    let after = env.balance(&env.recipient.id).await?.as_yoctonear();
    assert_eq!(
        after - before,
        approved * NEAR,
        "recipient got exactly the approved total"
    );
    Ok(())
}

/// Same race with a budget that only fits some of the requests: pending rejections and epoch
/// limit rejections must account for every request, none may slip through.
#[tokio::test]
async fn concurrent_requests_with_a_failing_payout_restore_the_counter() -> TestResult<()> {
    let env = setup().await?;
    let ghost = ghost();
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
    let pool = env.scoped_key_pool(&env.agent, 6).await?;
    let before = env.balance(&env.recipient.id).await?.as_yoctonear();

    // Alternate real and failing recipients, all at once.
    let results = join_all((0..6).map(|i| {
        let to = if i % 2 == 0 {
            &env.recipient.id
        } else {
            &ghost
        };
        env.transfer_as(&pool, &env.agent.id, to, NEAR / 2)
    }))
    .await;
    for res in results {
        let res = res?;
        if res.is_failure() {
            let text = error_text(&res);
            assert!(
                text.contains("ERR_TRANSFER_PENDING") || text.contains("ERR_EPOCH_LIMIT"),
                "unexpected failure: {text}"
            );
        }
    }

    let view = env.agent_policy(&env.agent.id).await?.expect("registered");
    let paid = env.balance(&env.recipient.id).await?.as_yoctonear() - before;
    assert!(view.pending_transfer_id.is_none());
    assert_eq!(
        view.spent_in_epoch.0, paid,
        "after rollbacks the counter equals exactly what reached the recipient"
    );
    assert!(view.spent_in_epoch.0 <= 3 * NEAR);
    Ok(())
}

/// A payout that fails right after an epoch boundary is refunded against the new epoch only,
/// and the budget of the new epoch is fully usable afterwards.
#[tokio::test]
async fn failed_payout_after_epoch_rollover_refunds_only_the_new_epoch() -> TestResult<()> {
    let env = setup().await?;
    let ghost = ghost();
    env.call_ok(
        &env.admin,
        "register_agent",
        register_args(
            &env.agent.id,
            NEAR,
            3 * NEAR,
            EPOCH_NS,
            &[&env.recipient.id, &ghost],
            None,
        ),
    )
    .await;
    for _ in 0..3 {
        assert!(
            env.transfer(&env.agent, &env.recipient.id, NEAR)
                .await
                .is_success()
        );
    }
    assert_eq!(env.spent(&env.agent.id).await, 3 * NEAR);
    let epoch_start = env
        .agent_policy(&env.agent.id)
        .await?
        .unwrap()
        .epoch_start_ns
        .0;

    env.wait_until_chain_time(epoch_start + EPOCH_NS + 1)
        .await?;

    // New epoch: the request resets the counter, reserves 1 NEAR, the payout fails, the refund
    // brings the counter back to 0 (not below it, and not to the old epoch's 3 NEAR).
    let res = env.transfer(&env.agent, &ghost, NEAR).await;
    assert_eq!(
        events_named(&res, "epoch_reset").len(),
        1,
        "{:?}",
        res.logs()
    );
    let rolled_back = events_named(&res, "transfer_rolled_back");
    assert_eq!(rolled_back.len(), 1, "{:?}", res.logs());
    assert_eq!(rolled_back[0]["data"]["amount"], NEAR.to_string());
    assert_eq!(env.spent(&env.agent.id).await, 0);

    // The whole new-epoch budget is usable, and exactly that.
    for _ in 0..3 {
        assert!(
            env.transfer(&env.agent, &env.recipient.id, NEAR)
                .await
                .is_success()
        );
    }
    assert_rejected(
        &env.transfer(&env.agent, &env.recipient.id, 1).await,
        vault::errors::ERR_EPOCH_LIMIT,
    );
    Ok(())
}

/// Reservation IDs are monotonic, appear on the approved event and the rollback event, and the
/// reservation is cleared once the callback ran.
#[tokio::test]
async fn reservation_ids_are_monotonic_and_cleared_after_settlement() -> TestResult<()> {
    let env = setup().await?;
    let ghost = ghost();
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

    let r1 = env.transfer(&env.agent, &env.recipient.id, NEAR / 4).await;
    let r2 = env.transfer(&env.agent, &env.recipient.id, NEAR / 4).await;
    let r3 = env.transfer(&env.agent, &ghost, NEAR / 4).await; // payout fails
    let id = |r: &near_api::types::transaction::result::TransactionResult| {
        events_named(r, "transfer_requested")[0]["data"]["transfer_id"].clone()
    };
    assert_eq!(id(&r1), "1");
    assert_eq!(id(&r2), "2");
    assert_eq!(id(&r3), "3");
    assert_eq!(
        events_named(&r3, "transfer_rolled_back")[0]["data"]["transfer_id"],
        "3"
    );

    let view = env.agent_policy(&env.agent.id).await?.unwrap();
    assert!(view.pending_transfer_id.is_none() && view.pending_amount.is_none());
    assert_eq!(
        view.spent_in_epoch.0,
        NEAR / 2,
        "only the two real payouts remain counted"
    );

    // A rejected request carries no transfer id.
    let rejected = env.transfer(&env.agent, &env.attacker.id, 1).await;
    assert!(
        events_named(&rejected, "transfer_requested")[0]["data"]
            .get("transfer_id")
            .is_none()
    );
    Ok(())
}

/// Nobody, not even the admin, can settle a reservation by calling the callback directly, and
/// a forged call changes nothing.
#[tokio::test]
async fn nobody_can_forge_a_callback() -> TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await;
    assert!(
        env.transfer(&env.agent, &env.recipient.id, NEAR)
            .await
            .is_success()
    );
    let args = json!({ "agent_id": env.agent.id, "transfer_id": "1" });
    for caller in [&env.admin, &env.attacker, &env.agent, &env.sentinel] {
        let res = env
            .call(caller, "on_transfer_complete", args.clone())
            .await?;
        assert!(
            res.is_failure(),
            "{} must not be able to call the callback",
            caller.id
        );
        assert!(error_text(&res).contains("Method on_transfer_complete is private"));
    }
    assert_eq!(env.spent(&env.agent.id).await, NEAR);
    Ok(())
}

/// The rollback callback must fit well inside its 5 Tgas reserve even for the largest policy
/// (50 allowlist entries of 64 characters): otherwise a failed payout could never be refunded.
#[tokio::test]
async fn callback_gas_with_a_maximum_size_policy_stays_under_the_reserve() -> TestResult<()> {
    let env = setup().await?;
    let ghost = ghost();
    let filler: Vec<near_api::AccountId> = (0..48)
        .map(|i| {
            format!("{}{i:02}.sandbox", "x".repeat(54))
                .parse()
                .expect("valid")
        })
        .collect();
    assert!(filler.iter().all(|a| a.as_str().len() == 64));
    let mut allowlist: Vec<&near_api::AccountId> = vec![&env.recipient.id, &ghost];
    allowlist.extend(filler.iter());
    assert_eq!(allowlist.len(), 50);
    env.call_ok(
        &env.admin,
        "register_agent",
        register_args(&env.agent.id, NEAR, 3 * NEAR, HOUR_NS, &allowlist, None),
    )
    .await;

    // The payout to a non-existent account fails, so the callback takes its most expensive path.
    let res = env.transfer(&env.agent, &ghost, NEAR / 10).await;
    assert!(res.is_success(), "{res:?}");
    let near_api::types::transaction::result::TransactionResult::Full(full) = &res else {
        panic!("expected a final result, got a pending one");
    };
    let rollback_gas = full
        .receipt_outcomes()
        .iter()
        .find(|o| o.logs.iter().any(|l| l.contains("transfer_rolled_back")))
        .expect("rollback receipt")
        .gas_burnt
        .as_gas();
    println!(
        "rollback callback gas burnt = {:.3} Tgas (reserve 5 Tgas)",
        rollback_gas as f64 / 1e12
    );
    assert!(
        rollback_gas < 4_000_000_000_000,
        "rollback callback burnt {} Tgas, too close to the 5 Tgas reserve",
        rollback_gas as f64 / 1e12
    );
    Ok(())
}

/// Someone who holds an agent's scoped key (or any account at all) gets exactly the agent's
/// policy and nothing else: limits are enforced by the contract, the key can only call
/// `request_transfer` on the vault, and the key cannot move money any other way.
#[tokio::test]
async fn compromised_agent_key_cannot_exceed_its_policy() -> TestResult<()> {
    let env = setup().await?;
    env.register_default_agent().await; // 1 NEAR per tx, 3 NEAR per epoch, recipient only
    let stolen = env.scoped_key_pool(&env.agent, 1).await?;
    let id = &env.agent.id;

    assert_rejected(
        &env.transfer_as(&stolen, id, &env.recipient.id, 2 * NEAR)
            .await?,
        vault::errors::ERR_PER_TX_LIMIT,
    );
    assert_rejected(
        &env.transfer_as(&stolen, id, &env.attacker.id, NEAR / 2)
            .await?,
        vault::errors::ERR_RECIPIENT_NOT_ALLOWED,
    );
    for _ in 0..3 {
        assert!(
            env.transfer_as(&stolen, id, &env.recipient.id, NEAR)
                .await?
                .is_success()
        );
    }
    assert_rejected(
        &env.transfer_as(&stolen, id, &env.recipient.id, 1).await?,
        vault::errors::ERR_EPOCH_LIMIT,
    );
    assert_eq!(env.spent(id).await, 3 * NEAR);

    // The key itself is limited by the network: other methods, other receivers and deposits are
    // refused before they reach a block.
    let pause = near_api::Contract(env.vault.clone())
        .call_function("pause_vault", ())
        .transaction()
        .gas(DEFAULT_GAS)
        .with_signer(id.clone(), stolen.clone())
        .send_to(&env.network)
        .await;
    assert!(
        pause.is_err(),
        "pause_vault must be refused for a scoped key: {pause:?}"
    );
    let other_receiver = near_api::Contract(env.recipient.id.clone())
        .call_function("request_transfer", ())
        .transaction()
        .gas(DEFAULT_GAS)
        .with_signer(id.clone(), stolen.clone())
        .send_to(&env.network)
        .await;
    assert!(
        other_receiver.is_err(),
        "other receivers must be refused: {other_receiver:?}"
    );
    let with_deposit = near_api::Contract(env.vault.clone())
        .call_function(
            "request_transfer",
            json!({"recipient": env.recipient.id, "amount": "1"}),
        )
        .transaction()
        .gas(DEFAULT_GAS)
        .deposit(near_api::NearToken::from_yoctonear(1))
        .with_signer(id.clone(), stolen.clone())
        .send_to(&env.network)
        .await;
    assert!(
        with_deposit.is_err(),
        "a deposit must be refused for a scoped key: {with_deposit:?}"
    );

    let status = env.vault_status().await?;
    assert!(!status.paused, "the vault must not have been paused");
    Ok(())
}

/// The agent is whoever calls: requests are attributed to the caller only, other agents'
/// budgets are untouched and an unregistered account is refused.
#[tokio::test]
async fn requests_are_attributed_to_the_caller_only() -> TestResult<()> {
    let env = setup().await?;
    let agent2 = create_actor(&env.sandbox, "agent2", ACCOUNT_BALANCE).await?;
    env.register_default_agent().await;
    env.call_ok(
        &env.admin,
        "register_agent",
        register_args(
            &agent2.id,
            NEAR,
            3 * NEAR,
            HOUR_NS,
            &[&env.recipient.id],
            None,
        ),
    )
    .await;

    assert!(
        env.transfer(&agent2, &env.recipient.id, NEAR)
            .await
            .is_success()
    );
    assert_eq!(env.spent(&agent2.id).await, NEAR);
    assert_eq!(
        env.spent(&env.agent.id).await,
        0,
        "agent 1 was not charged for agent 2's request"
    );

    let res = env.transfer(&env.attacker, &env.recipient.id, 1).await;
    assert_rejected(&res, vault::errors::ERR_AGENT_NOT_REGISTERED);
    assert_eq!(env.spent(&env.agent.id).await, 0);
    assert_eq!(env.spent(&agent2.id).await, NEAR);
    Ok(())
}
