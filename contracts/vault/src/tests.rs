use crate::Vault;
use crate::policy::{PolicyParams, PolicyUpdate};
use near_sdk::json_types::{U64, U128};
use near_sdk::test_utils::{VMContextBuilder, accounts};
use near_sdk::{AccountId, PromiseResult, test_vm_config, testing_env};

const START_TS: u64 = 1_000;
const EPOCH_NS: u64 = 1_000_000;

fn admin() -> AccountId {
    accounts(0)
}
fn sentinel() -> AccountId {
    accounts(1)
}
fn agent() -> AccountId {
    accounts(2)
}
fn recipient() -> AccountId {
    accounts(3)
}
fn other() -> AccountId {
    accounts(4)
}

fn ctx(caller: &AccountId, ts: u64) -> VMContextBuilder {
    let mut b = VMContextBuilder::new();
    b.current_account_id(accounts(5))
        .predecessor_account_id(caller.clone())
        .signer_account_id(caller.clone())
        .attached_deposit(near_sdk::NearToken::from_yoctonear(1))
        .block_timestamp(ts);
    b
}

fn set_caller(caller: &AccountId, ts: u64) {
    testing_env!(ctx(caller, ts).build());
}

fn params(per_tx: u128, epoch: u128) -> PolicyParams {
    PolicyParams {
        per_tx_limit: U128(per_tx),
        epoch_limit: U128(epoch),
        epoch_duration_ns: U64(EPOCH_NS),
        allowlist: vec![recipient()],
        expiry_ns: None,
    }
}

fn empty_update() -> PolicyUpdate {
    PolicyUpdate {
        per_tx_limit: None,
        epoch_limit: None,
        epoch_duration_ns: None,
        expiry_ns: None,
        allowlist_add: None,
        allowlist_remove: None,
    }
}

fn new_vault() -> Vault {
    set_caller(&admin(), START_TS);
    Vault::new(admin(), Some(sentinel()))
}

/// Vault with `agent()` registered (per_tx 10, epoch 100). Caller left as admin.
fn vault_with_agent() -> Vault {
    let mut v = new_vault();
    v.register_agent(agent(), params(10, 100));
    v
}

fn spent(v: &Vault) -> u128 {
    v.get_agent_policy(agent()).unwrap().spent_in_epoch.0
}

/// Approve a transfer and leave it pending (its callback has not run yet).
fn request(v: &mut Vault, amount: u128) {
    set_caller(&agent(), START_TS);
    let _ = v.request_transfer(recipient(), U128(amount));
}

/// Approve a transfer and settle it successfully, like a payout that went through.
fn transfer(v: &mut Vault, amount: u128) {
    request(v, amount);
    settle(v, PromiseResult::Successful(vec![]));
}

/// ID of the agent's in-flight reservation (panics if there is none).
fn pending_id(v: &Vault) -> u64 {
    v.agents
        .get(&agent())
        .and_then(|p| p.pending.as_ref().map(|t| t.id))
        .expect("agent has a pending reservation")
}

/// Run the callback of the agent's in-flight reservation.
fn settle(v: &mut Vault, result: PromiseResult) {
    let id = pending_id(v);
    run_callback(v, id, result);
}

/// Run `on_transfer_complete` for an explicit reservation ID, as the vault's own promise would.
fn run_callback(v: &mut Vault, id: u64, result: PromiseResult) {
    testing_env!(
        ctx(&accounts(5), START_TS).build(),
        test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![result]
    );
    v.on_transfer_complete(agent(), U64(id));
}

fn has_event(name: &str) -> bool {
    near_sdk::test_utils::get_logs()
        .iter()
        .any(|l| l.contains(&format!("\"event\":\"{name}\"")))
}

// ── init ─────────────────────────────────────────────────────────────────────

#[test]
fn init_defaults() {
    let v = new_vault();
    let s = v.get_vault_status();
    assert!(!s.paused);
    assert_eq!(s.admin_account_id, admin());
    assert_eq!(s.sentinel_account_id, Some(sentinel()));
}

#[test]
fn init_without_sentinel() {
    set_caller(&admin(), START_TS);
    let v = Vault::new(admin(), None);
    assert_eq!(v.get_vault_status().sentinel_account_id, None);
}

#[test]
fn init_admin_id_length_bounds() {
    // AccountId itself rejects ids outside 2..=64, so the contract's check
    // (defence in depth) is satisfied at both boundaries.
    set_caller(&admin(), START_TS);
    let min: AccountId = "ab".parse().unwrap();
    assert_eq!(
        Vault::new(min.clone(), None)
            .get_vault_status()
            .admin_account_id,
        min
    );
    let max: AccountId = "a".repeat(64).parse().unwrap();
    assert_eq!(
        Vault::new(max.clone(), None)
            .get_vault_status()
            .admin_account_id,
        max
    );
    assert!("a".parse::<AccountId>().is_err());
    assert!("a".repeat(65).parse::<AccountId>().is_err());
}

// ── register_agent ───────────────────────────────────────────────────────────

#[test]
fn register_agent_success_fields() {
    let v = vault_with_agent();
    let p = v.get_agent_policy(agent()).expect("registered");
    assert_eq!(p.version, 1);
    assert_eq!(p.spent_in_epoch.0, 0);
    assert!(p.active);
    assert!(!p.revoked);
    assert_eq!(p.per_tx_limit.0, 10);
    assert_eq!(p.epoch_limit.0, 100);
    assert_eq!(p.remaining_epoch_budget.0, 100);
    assert_eq!(p.epoch_start_ns.0, START_TS);
    assert_eq!(p.allowlist, vec![recipient()]);
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn register_agent_non_admin() {
    let mut v = new_vault();
    set_caller(&other(), START_TS);
    v.register_agent(agent(), params(10, 100));
}

#[test]
#[should_panic(expected = "ERR_DUPLICATE_AGENT")]
fn register_agent_duplicate() {
    let mut v = vault_with_agent();
    v.register_agent(agent(), params(10, 100));
}

#[test]
#[should_panic(expected = "ERR_TX_EXCEEDS_EPOCH")]
fn register_agent_per_tx_exceeds_epoch() {
    let mut v = new_vault();
    v.register_agent(agent(), params(101, 100));
}

#[test]
#[should_panic(expected = "ERR_ALLOWLIST_FULL")]
fn register_agent_allowlist_over_50() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.allowlist = (0..51)
        .map(|i| format!("r{i}.near").parse().unwrap())
        .collect();
    v.register_agent(agent(), p);
}

#[test]
fn register_agent_allowlist_exactly_50() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.allowlist = (0..50)
        .map(|i| format!("r{i}.near").parse().unwrap())
        .collect();
    v.register_agent(agent(), p);
    assert_eq!(v.get_agent_policy(agent()).unwrap().allowlist.len(), 50);
}

// ── update_agent_policy ──────────────────────────────────────────────────────

#[test]
fn update_policy_increments_version() {
    let mut v = vault_with_agent();
    let mut u = empty_update();
    u.per_tx_limit = Some(U128(20));
    v.update_agent_policy(agent(), u);
    let p = v.get_agent_policy(agent()).unwrap();
    assert_eq!(p.version, 2);
    assert_eq!(p.per_tx_limit.0, 20);
    v.update_agent_policy(agent(), empty_update());
    assert_eq!(v.get_agent_policy(agent()).unwrap().version, 3);
}

#[test]
#[should_panic(expected = "ERR_REVOKED_AGENT")]
fn update_policy_revoked_agent() {
    let mut v = vault_with_agent();
    v.revoke_agent(agent());
    v.update_agent_policy(agent(), empty_update());
}

#[test]
#[should_panic(expected = "ERR_AGENT_NOT_FOUND")]
fn update_policy_unknown_agent() {
    let mut v = new_vault();
    v.update_agent_policy(agent(), empty_update());
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn update_policy_non_admin() {
    let mut v = vault_with_agent();
    set_caller(&other(), START_TS);
    v.update_agent_policy(agent(), empty_update());
}

#[test]
fn update_policy_allowlist_add_remove_idempotent() {
    let mut v = vault_with_agent();
    let mut u = empty_update();
    u.allowlist_add = Some(vec![other(), other(), recipient()]);
    v.update_agent_policy(agent(), u);
    let list = v.get_agent_policy(agent()).unwrap().allowlist;
    assert_eq!(list.len(), 2);
    assert!(list.contains(&other()) && list.contains(&recipient()));

    let mut u = empty_update();
    u.allowlist_remove = Some(vec![other(), other(), accounts(5)]);
    v.update_agent_policy(agent(), u);
    assert_eq!(
        v.get_agent_policy(agent()).unwrap().allowlist,
        vec![recipient()]
    );
}

#[test]
fn update_policy_allowlist_capacity_50() {
    let mut v = vault_with_agent(); // 1 entry
    let mut u = empty_update();
    u.allowlist_add = Some(
        (0..49)
            .map(|i| format!("r{i}.near").parse().unwrap())
            .collect(),
    );
    v.update_agent_policy(agent(), u);
    assert_eq!(v.get_agent_policy(agent()).unwrap().allowlist.len(), 50);
    // Re-adding an existing entry at capacity is a no-op, not an error.
    let mut u = empty_update();
    u.allowlist_add = Some(vec![recipient()]);
    v.update_agent_policy(agent(), u);
    assert_eq!(v.get_agent_policy(agent()).unwrap().allowlist.len(), 50);
}

#[test]
#[should_panic(expected = "ERR_ALLOWLIST_FULL")]
fn update_policy_allowlist_over_capacity() {
    let mut v = vault_with_agent();
    let mut u = empty_update();
    u.allowlist_add = Some(
        (0..50)
            .map(|i| format!("r{i}.near").parse().unwrap())
            .collect(),
    );
    v.update_agent_policy(agent(), u);
}

// ── request_transfer ─────────────────────────────────────────────────────────

#[test]
fn transfer_approved_increments_spent_exactly() {
    let mut v = vault_with_agent();
    transfer(&mut v, 7);
    assert_eq!(spent(&v), 7);
    transfer(&mut v, 3);
    assert_eq!(spent(&v), 10);
    assert_eq!(
        v.get_agent_policy(agent())
            .unwrap()
            .remaining_epoch_budget
            .0,
        90
    );
}

#[test]
#[should_panic(expected = "ERR_VAULT_PAUSED")]
fn transfer_rejected_vault_paused() {
    let mut v = vault_with_agent();
    v.pause_vault();
    transfer(&mut v, 1);
}

#[test]
#[should_panic(expected = "ERR_AGENT_NOT_REGISTERED")]
fn transfer_rejected_unregistered() {
    let mut v = new_vault();
    transfer(&mut v, 1);
}

#[test]
#[should_panic(expected = "ERR_AGENT_PAUSED")]
fn transfer_rejected_agent_paused() {
    let mut v = vault_with_agent();
    v.pause_agent(agent());
    transfer(&mut v, 1);
}

#[test]
#[should_panic(expected = "ERR_AGENT_REVOKED")]
fn transfer_rejected_agent_revoked() {
    let mut v = vault_with_agent();
    v.revoke_agent(agent());
    transfer(&mut v, 1);
}

#[test]
#[should_panic(expected = "ERR_POLICY_EXPIRED")]
fn transfer_rejected_policy_expired() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.expiry_ns = Some(U64(START_TS + 10));
    v.register_agent(agent(), p);
    set_caller(&agent(), START_TS + 10); // expiry is inclusive
    let _ = v.request_transfer(recipient(), U128(1));
}

#[test]
#[should_panic(expected = "ERR_RECIPIENT_NOT_ALLOWED")]
fn transfer_rejected_recipient_not_allowed() {
    let mut v = vault_with_agent();
    set_caller(&agent(), START_TS);
    let _ = v.request_transfer(other(), U128(1));
}

#[test]
#[should_panic(expected = "ERR_AMOUNT_ZERO")]
fn transfer_rejected_zero_amount() {
    let mut v = vault_with_agent();
    transfer(&mut v, 0);
}

#[test]
#[should_panic(expected = "ERR_PER_TX_LIMIT")]
fn transfer_rejected_per_tx_limit() {
    let mut v = vault_with_agent();
    transfer(&mut v, 11);
}

#[test]
#[should_panic(expected = "ERR_EPOCH_LIMIT")]
fn transfer_rejected_epoch_limit() {
    let mut v = vault_with_agent();
    for _ in 0..10 {
        transfer(&mut v, 10);
    }
    assert_eq!(spent(&v), 100);
    transfer(&mut v, 1);
}

#[test]
fn transfer_exact_per_tx_limit_accepted() {
    let mut v = vault_with_agent();
    transfer(&mut v, 10);
    assert_eq!(spent(&v), 10);
}

#[test]
fn transfer_exact_epoch_limit_accepted() {
    let mut v = vault_with_agent();
    for _ in 0..10 {
        transfer(&mut v, 10);
    }
    assert_eq!(spent(&v), 100);
}

#[test]
fn transfer_not_before_expiry_accepted() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.expiry_ns = Some(U64(START_TS + 10));
    v.register_agent(agent(), p);
    set_caller(&agent(), START_TS + 9);
    let _ = v.request_transfer(recipient(), U128(1));
    assert_eq!(spent(&v), 1);
}

// ── epoch reset ──────────────────────────────────────────────────────────────

#[test]
fn epoch_resets_after_duration() {
    let mut v = vault_with_agent();
    transfer(&mut v, 10);
    assert_eq!(spent(&v), 10);

    // One ns before the boundary: still same epoch.
    set_caller(&agent(), START_TS + EPOCH_NS - 1);
    let _ = v.request_transfer(recipient(), U128(5));
    assert_eq!(spent(&v), 15);
    settle(&mut v, PromiseResult::Successful(vec![])); // one reservation at a time

    // At the boundary: counter resets, then new amount is applied.
    let now = START_TS + EPOCH_NS;
    set_caller(&agent(), now);
    let _ = v.request_transfer(recipient(), U128(4));
    let p = v.get_agent_policy(agent()).unwrap();
    assert_eq!(p.spent_in_epoch.0, 4);
    assert_eq!(p.epoch_start_ns.0, now);
}

#[test]
fn epoch_reset_allows_full_budget_again() {
    let mut v = vault_with_agent();
    for _ in 0..10 {
        transfer(&mut v, 10);
    }
    set_caller(&agent(), START_TS + EPOCH_NS + 1);
    let _ = v.request_transfer(recipient(), U128(10));
    assert_eq!(spent(&v), 10);
}

// ── on_transfer_complete ─────────────────────────────────────────────────────

#[test]
fn callback_failed_rolls_back() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    settle(&mut v, PromiseResult::Failed);
    assert_eq!(spent(&v), 0);
    assert!(has_event("transfer_rolled_back"));
}

#[test]
fn callback_success_keeps_spent_and_clears_the_reservation() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    settle(&mut v, PromiseResult::Successful(vec![]));
    assert_eq!(spent(&v), 5);
    assert!(!has_event("transfer_rolled_back"));
    assert!(
        v.get_agent_policy(agent())
            .unwrap()
            .pending_transfer_id
            .is_none()
    );
}

#[test]
fn callback_for_unknown_agent_is_noop() {
    let mut v = new_vault();
    run_callback(&mut v, 1, PromiseResult::Failed);
    assert!(v.get_agent_policy(agent()).is_none());
}

// ── pending-transfer reservations ────────────────────────────────────────────

#[test]
fn approved_request_records_a_reservation_visible_in_the_view() {
    let mut v = vault_with_agent();
    request(&mut v, 7);
    let view = v.get_agent_policy(agent()).unwrap();
    assert_eq!(view.pending_transfer_id, Some(U64(1)));
    assert_eq!(view.pending_amount, Some(U128(7)));
    assert_eq!(
        view.spent_in_epoch.0, 7,
        "the reserved amount is already counted"
    );
    let logs = near_sdk::test_utils::get_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("\"event\":\"transfer_requested\"")
                && l.contains("\"decision\":\"approved\"")
                && l.contains("\"transfer_id\":\"1\"")),
        "approved event carries the transfer id: {logs:?}"
    );
}

#[test]
#[should_panic(expected = "ERR_TRANSFER_PENDING")]
fn second_request_while_one_is_pending_is_rejected() {
    let mut v = vault_with_agent();
    request(&mut v, 3);
    request(&mut v, 4);
}

#[test]
fn rejected_request_while_pending_changes_nothing() {
    let mut v = vault_with_agent();
    request(&mut v, 3);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| request(&mut v, 4)));
    assert!(result.is_err());
    // A real chain reverts the state of a panicked call; in the unit harness the panic fires
    // before any mutation, so the counter and the reservation are exactly as before.
    assert_eq!(spent(&v), 3);
    assert_eq!(pending_id(&v), 1);
    assert_eq!(
        v.get_agent_policy(agent()).unwrap().pending_amount,
        Some(U128(3))
    );
}

#[test]
fn policy_violations_are_still_reported_while_a_transfer_is_pending() {
    let mut v = vault_with_agent();
    request(&mut v, 3);
    set_caller(&agent(), START_TS);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = v.request_transfer(other(), U128(1)); // not on the allowlist
    }));
    let msg = *result.unwrap_err().downcast::<String>().unwrap();
    assert!(msg.contains("ERR_RECIPIENT_NOT_ALLOWED"), "{msg}");
}

#[test]
fn next_request_is_allowed_after_settlement_and_ids_are_never_reused() {
    let mut v = vault_with_agent();
    request(&mut v, 2);
    assert_eq!(pending_id(&v), 1);
    settle(&mut v, PromiseResult::Failed); // rolled back, but the id is consumed
    request(&mut v, 2);
    assert_eq!(pending_id(&v), 2);
    settle(&mut v, PromiseResult::Successful(vec![]));
    request(&mut v, 2);
    assert_eq!(pending_id(&v), 3);
}

#[test]
fn duplicate_callback_does_not_refund_twice() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    run_callback(&mut v, 1, PromiseResult::Failed);
    assert_eq!(spent(&v), 0);

    // A different transfer is now counted and pending.
    request(&mut v, 3);
    assert_eq!(spent(&v), 3);

    // The first callback is replayed: it must not touch the new reservation or the counter.
    run_callback(&mut v, 1, PromiseResult::Failed);
    assert_eq!(
        spent(&v),
        3,
        "a replayed callback must not lower the counter"
    );
    assert_eq!(pending_id(&v), 2, "the new reservation is untouched");
    assert!(has_event("callback_ignored"));
    assert!(!has_event("transfer_rolled_back"));
}

#[test]
fn replayed_callback_after_settlement_is_ignored() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    run_callback(&mut v, 1, PromiseResult::Successful(vec![]));
    run_callback(&mut v, 1, PromiseResult::Failed); // forged or duplicated failure
    assert_eq!(
        spent(&v),
        5,
        "a settled transfer can never be refunded afterwards"
    );
    assert!(has_event("callback_ignored"));
}

#[test]
fn callback_with_an_unknown_id_is_ignored() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    run_callback(&mut v, 99, PromiseResult::Failed);
    assert_eq!(spent(&v), 5);
    assert_eq!(
        pending_id(&v),
        1,
        "the real reservation is still waiting for its callback"
    );
    assert!(has_event("callback_ignored"));
}

#[test]
fn failed_callback_after_the_epoch_changed_does_not_undercount() {
    let mut v = vault_with_agent();
    request(&mut v, 10); // reserved in epoch 1
    // Defence in depth: simulate the counter having been reset and re-used by a new epoch.
    {
        let p = v.agents.get_mut(&agent()).unwrap();
        p.epoch_start_ns += EPOCH_NS;
        p.spent_in_epoch = 4;
    }
    settle(&mut v, PromiseResult::Failed);
    assert_eq!(
        spent(&v),
        4,
        "an epoch-1 failure must not reduce epoch-2 spend"
    );
    assert!(
        v.get_agent_policy(agent())
            .unwrap()
            .pending_transfer_id
            .is_none(),
        "the reservation is cleared either way"
    );
}

#[test]
fn rollback_below_zero_leaves_the_counter_alone_and_logs_an_anomaly() {
    let mut v = vault_with_agent();
    request(&mut v, 5);
    v.agents.get_mut(&agent()).unwrap().spent_in_epoch = 2; // violates the invariant on purpose
    settle(&mut v, PromiseResult::Failed);
    assert_eq!(spent(&v), 2, "never lower the counter on a guess");
    assert!(has_event("accounting_anomaly"));
    assert!(!has_event("transfer_rolled_back"));
}

#[test]
fn stuck_reservation_expires_stays_counted_and_a_late_callback_is_ignored() {
    let mut v = vault_with_agent();
    request(&mut v, 6); // created at block 0
    assert_eq!(spent(&v), 6);

    // Still within the timeout: blocked.
    testing_env!(
        ctx(&agent(), START_TS)
            .block_height(crate::policy::PENDING_TIMEOUT_BLOCKS)
            .build()
    );
    let blocked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = v.request_transfer(recipient(), U128(1));
    }));
    assert!(blocked.is_err());

    // Past the timeout: the stale reservation is cleared (amount stays counted) and a new one made.
    testing_env!(
        ctx(&agent(), START_TS)
            .block_height(crate::policy::PENDING_TIMEOUT_BLOCKS + 1)
            .build()
    );
    let _ = v.request_transfer(recipient(), U128(1));
    assert!(has_event("pending_expired"));
    assert_eq!(
        spent(&v),
        7,
        "the expired amount stays counted (fail-closed)"
    );
    assert_eq!(pending_id(&v), 2);

    // The callback of the expired reservation finally arrives and fails: it must change nothing.
    run_callback(&mut v, 1, PromiseResult::Failed);
    assert_eq!(spent(&v), 7);
    assert_eq!(pending_id(&v), 2);
    assert!(has_event("callback_ignored"));
}

#[test]
fn delayed_failure_in_the_same_epoch_but_after_clock_expiry_still_refunds_once() {
    // The epoch elapsed on the clock but no request has reset it yet (the reset is lazy).
    // The refund is harmless: the next request resets the counter to zero anyway.
    let mut v = vault_with_agent();
    request(&mut v, 10);
    testing_env!(
        ctx(&accounts(5), START_TS + EPOCH_NS + 5).build(),
        test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![PromiseResult::Failed]
    );
    v.on_transfer_complete(agent(), U64(1));
    set_caller(&agent(), START_TS + EPOCH_NS + 6);
    let _ = v.request_transfer(recipient(), U128(4));
    assert_eq!(
        spent(&v),
        4,
        "the new epoch starts from zero plus the new request only"
    );
}

// ── pause / revoke / sentinel ────────────────────────────────────────────────

#[test]
fn pause_agent_by_admin() {
    let mut v = vault_with_agent();
    v.pause_agent(agent());
    assert!(!v.get_agent_policy(agent()).unwrap().active);
}

#[test]
fn pause_agent_by_sentinel() {
    let mut v = vault_with_agent();
    set_caller(&sentinel(), START_TS);
    v.pause_agent(agent());
    assert!(!v.get_agent_policy(agent()).unwrap().active);
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn pause_agent_by_other() {
    let mut v = vault_with_agent();
    set_caller(&other(), START_TS);
    v.pause_agent(agent());
}

#[test]
#[should_panic(expected = "ERR_AGENT_NOT_FOUND")]
fn pause_agent_unknown() {
    let mut v = new_vault();
    v.pause_agent(agent());
}

#[test]
fn revoke_agent_by_admin_is_permanent() {
    let mut v = vault_with_agent();
    v.revoke_agent(agent());
    let p = v.get_agent_policy(agent()).unwrap();
    assert!(p.revoked && !p.active);
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_revoke_agent() {
    let mut v = vault_with_agent();
    set_caller(&sentinel(), START_TS);
    v.revoke_agent(agent());
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_unpause_vault() {
    let mut v = new_vault();
    v.pause_vault();
    set_caller(&sentinel(), START_TS);
    v.unpause_vault();
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_pause_vault() {
    let mut v = new_vault();
    set_caller(&sentinel(), START_TS);
    v.pause_vault();
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_register_agent() {
    let mut v = new_vault();
    set_caller(&sentinel(), START_TS);
    v.register_agent(agent(), params(10, 100));
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_update_agent_policy() {
    let mut v = vault_with_agent();
    set_caller(&sentinel(), START_TS);
    v.update_agent_policy(agent(), empty_update());
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn sentinel_cannot_set_sentinel() {
    let mut v = new_vault();
    set_caller(&sentinel(), START_TS);
    v.set_sentinel(other());
}

#[test]
fn admin_can_set_sentinel() {
    let mut v = new_vault();
    v.set_sentinel(other());
    assert_eq!(v.get_vault_status().sentinel_account_id, Some(other()));
}

#[test]
#[should_panic(expected = "ERR_ONE_YOCTO_REQUIRED")]
fn admin_method_without_one_yocto_is_rejected() {
    let mut v = new_vault();
    testing_env!(
        ctx(&admin(), START_TS)
            .attached_deposit(near_sdk::NearToken::from_yoctonear(0))
            .build()
    );
    v.pause_vault();
}
#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn non_admin_sees_access_denied_before_deposit_check() {
    let mut v = new_vault();
    testing_env!(
        ctx(&other(), START_TS)
            .attached_deposit(near_sdk::NearToken::from_yoctonear(0))
            .build()
    );
    v.pause_vault();
}
#[test]
fn pause_and_unpause_vault_by_admin() {
    let mut v = new_vault();
    v.pause_vault();
    assert!(v.get_vault_status().paused);
    v.unpause_vault();
    assert!(!v.get_vault_status().paused);
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn pause_vault_by_other() {
    let mut v = new_vault();
    set_caller(&other(), START_TS);
    v.pause_vault();
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn unpause_vault_by_other() {
    let mut v = new_vault();
    v.pause_vault();
    set_caller(&other(), START_TS);
    v.unpause_vault();
}

// ── fixes: unpause_agent, cross-epoch rollback, expiry, allowlist, events ────

#[test]
fn unpause_agent_by_admin_restores_agent() {
    let mut v = vault_with_agent();
    v.pause_agent(agent());
    assert!(!v.get_agent_policy(agent()).unwrap().active);
    set_caller(&admin(), START_TS);
    v.unpause_agent(agent());
    assert!(v.get_agent_policy(agent()).unwrap().active);
    transfer(&mut v, 1);
    assert_eq!(spent(&v), 1);
}

#[test]
#[should_panic(expected = "ERR_ACCESS_DENIED")]
fn unpause_agent_by_sentinel_denied() {
    let mut v = vault_with_agent();
    v.pause_agent(agent());
    set_caller(&sentinel(), START_TS);
    v.unpause_agent(agent());
}

#[test]
#[should_panic(expected = "ERR_REVOKED_AGENT")]
fn unpause_agent_revoked_is_permanent() {
    let mut v = vault_with_agent();
    v.revoke_agent(agent());
    v.unpause_agent(agent());
}

#[test]
#[should_panic(expected = "ERR_AGENT_NOT_PAUSED")]
fn unpause_agent_not_paused() {
    let mut v = vault_with_agent();
    v.unpause_agent(agent());
}

#[test]
#[should_panic(expected = "ERR_AGENT_NOT_FOUND")]
fn unpause_agent_unknown() {
    let mut v = new_vault();
    v.unpause_agent(agent());
}

#[test]
fn epoch_rollover_cannot_happen_while_a_transfer_is_pending() {
    // With one reservation per agent, a request that would reset the epoch is rejected while
    // another is in flight, and the rejection reverts the reset. So the counter can never be
    // reset underneath a pending transfer.
    let mut v = vault_with_agent();
    request(&mut v, 10); // reserved in epoch 1

    set_caller(&agent(), START_TS + EPOCH_NS);
    let blocked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = v.request_transfer(recipient(), U128(4));
    }));
    let msg = *blocked.unwrap_err().downcast::<String>().unwrap();
    assert!(msg.contains("ERR_TRANSFER_PENDING"), "{msg}");
    // (On chain the panic also reverts the lazy epoch reset that ran just before the check;
    // the sandbox test `epoch_rollover_with_a_pending_transfer` proves that end to end.)
}

#[test]
fn rollover_after_expiry_then_late_failure_does_not_undercount_the_new_epoch() {
    let mut v = vault_with_agent();
    request(&mut v, 10); // epoch 1, reservation 1 at block 0

    // Much later (after the timeout and an epoch boundary) the agent asks again.
    testing_env!(
        ctx(&agent(), START_TS + EPOCH_NS + 1)
            .block_height(crate::policy::PENDING_TIMEOUT_BLOCKS + 1)
            .build()
    );
    let _ = v.request_transfer(recipient(), U128(4));
    assert_eq!(
        spent(&v),
        4,
        "new epoch: counter was reset, only the new request counts"
    );

    // The epoch-1 payout is reported failed very late: it must NOT reduce epoch-2 spend.
    run_callback(&mut v, 1, PromiseResult::Failed);
    assert_eq!(spent(&v), 4);
    assert_eq!(pending_id(&v), 2);
}

#[test]
fn zero_amount_rejection_emits_event() {
    let mut v = vault_with_agent();
    set_caller(&agent(), START_TS);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = v.request_transfer(recipient(), U128(0));
    }));
    assert!(r.is_err());
    let logs = near_sdk::test_utils::get_logs();
    assert!(
        logs.iter().any(|l| l.starts_with("EVENT_JSON:")
            && l.contains("\"decision\":\"rejected\"")
            && l.contains("ERR_AMOUNT_ZERO")),
        "logs: {logs:?}"
    );
}

#[test]
fn approved_transfer_event_uses_string_amounts() {
    let mut v = vault_with_agent();
    request(&mut v, 7); // `transfer` also settles, which starts a fresh log buffer
    let logs = near_sdk::test_utils::get_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("\"event\":\"transfer_requested\"")
                && l.contains("\"amount\":\"7\"")
                && l.contains("\"decision\":\"approved\"")),
        "logs: {logs:?}"
    );
}

#[test]
fn event_timestamps_are_json_strings() {
    let mut v = vault_with_agent();
    v.pause_agent(agent());
    let logs = near_sdk::test_utils::get_logs();
    assert!(
        logs.iter().any(|l| l.contains("\"event\":\"agent_paused\"")
            && l.contains(&format!("\"timestamp_ns\":\"{START_TS}\""))),
        "logs: {logs:?}"
    );
}

#[test]
fn register_dedupes_allowlist() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.allowlist = vec![recipient(), recipient(), other(), recipient()];
    v.register_agent(agent(), p);
    assert_eq!(
        v.get_agent_policy(agent()).unwrap().allowlist,
        vec![recipient(), other()]
    );
}

#[test]
#[should_panic(expected = "ERR_EXPIRY_IN_PAST")]
fn register_expiry_in_past() {
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.expiry_ns = Some(U64(START_TS));
    v.register_agent(agent(), p);
}

#[test]
#[should_panic(expected = "ERR_EXPIRY_IN_PAST")]
fn update_expiry_in_past() {
    let mut v = vault_with_agent();
    let mut u = empty_update();
    u.expiry_ns = Some(Some(U64(START_TS - 1)));
    v.update_agent_policy(agent(), u);
}

#[test]
fn update_json_null_expiry_clears_and_missing_leaves_unchanged() {
    use near_sdk::serde_json::from_str;
    let mut v = new_vault();
    let mut p = params(10, 100);
    p.expiry_ns = Some(U64(START_TS + 500));
    v.register_agent(agent(), p);

    // Missing key: unchanged.
    let u: PolicyUpdate = from_str("{}").unwrap();
    assert!(u.expiry_ns.is_none());
    v.update_agent_policy(agent(), u);
    assert_eq!(
        v.get_agent_policy(agent()).unwrap().expiry_ns,
        Some(U64(START_TS + 500))
    );

    // Explicit null: cleared.
    let u: PolicyUpdate = from_str(r#"{"expiry_ns": null}"#).unwrap();
    assert!(matches!(u.expiry_ns, Some(None)));
    v.update_agent_policy(agent(), u);
    assert_eq!(v.get_agent_policy(agent()).unwrap().expiry_ns, None);

    // Number string: set.
    let u: PolicyUpdate = from_str(r#"{"expiry_ns": "9999"}"#).unwrap();
    v.update_agent_policy(agent(), u);
    assert_eq!(
        v.get_agent_policy(agent()).unwrap().expiry_ns,
        Some(U64(9999))
    );
}

#[test]
fn view_reports_reset_budget_once_epoch_elapsed() {
    let mut v = vault_with_agent();
    transfer(&mut v, 10);
    assert_eq!(
        v.get_agent_policy(agent())
            .unwrap()
            .remaining_epoch_budget
            .0,
        90
    );
    set_caller(&other(), START_TS + EPOCH_NS);
    let p = v.get_agent_policy(agent()).unwrap();
    assert_eq!(p.spent_in_epoch.0, 0);
    assert_eq!(p.remaining_epoch_budget.0, 100);
}
#[test]
#[should_panic(expected = "ERR_OVERFLOW")]
fn transfer_counter_overflow_is_rejected_not_wrapped() {
    let mut v = new_vault();
    // Limits at the top of the u128 range: two transfers of 2^127 would wrap the counter.
    let half: u128 = 1 << 127;
    v.register_agent(agent(), params(u128::MAX, u128::MAX));
    testing_env!(
        ctx(&agent(), START_TS)
            .account_balance(near_sdk::NearToken::from_yoctonear(half + 10))
            .build()
    );
    let _ = v.request_transfer(recipient(), U128(half));
    let _ = v.request_transfer(recipient(), U128(half));
}
