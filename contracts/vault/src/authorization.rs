use crate::Vault;
use crate::errors::*;
use crate::events::{TransferRequestedData, log_transfer_requested};
use crate::spending::maybe_reset_epoch;
use near_sdk::json_types::U128;
use near_sdk::{AccountId, Gas, Promise, env};

/// Gas reserved for `on_transfer_complete` (it reads and writes one map entry and logs once).
const CALLBACK_GAS: Gas = Gas::from_tgas(5);

pub fn request_transfer(vault: &mut Vault, recipient: AccountId, amount: U128) -> Promise {
    let caller = env::predecessor_account_id();
    let amount_u128 = amount.0;

    // ── Condition (a): vault not paused ───────────────────────────────────────
    if vault.paused {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_VAULT_PAUSED),
        });
        env::panic_str(ERR_VAULT_PAUSED);
    }

    // ── Condition (b): caller is registered ──────────────────────────────────
    if !vault.agents.contains_key(&caller) {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_AGENT_NOT_REGISTERED),
        });
        env::panic_str(ERR_AGENT_NOT_REGISTERED);
    }

    let policy = vault.agents.get_mut(&caller).unwrap();

    // ── Condition (c): agent active and not revoked ───────────────────────────
    if policy.revoked {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_AGENT_REVOKED),
        });
        env::panic_str(ERR_AGENT_REVOKED);
    }
    if !policy.active {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_AGENT_PAUSED),
        });
        env::panic_str(ERR_AGENT_PAUSED);
    }

    // ── Condition (d): policy not expired ────────────────────────────────────
    let expired = policy
        .expiry_ns
        .is_some_and(|expiry| env::block_timestamp() >= expiry);
    if expired {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_POLICY_EXPIRED),
        });
        env::panic_str(ERR_POLICY_EXPIRED);
    }

    // ── Condition (e): recipient on allowlist ─────────────────────────────────
    if !policy.allowlist.contains(&recipient) {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_RECIPIENT_NOT_ALLOWED),
        });
        env::panic_str(ERR_RECIPIENT_NOT_ALLOWED);
    }

    // ── Condition (f): amount > 0 ─────────────────────────────────────────────
    if amount_u128 == 0 {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_AMOUNT_ZERO),
        });
        env::panic_str(ERR_AMOUNT_ZERO);
    }

    // ── Condition (g): amount <= per_tx_limit ────────────────────────────────
    if amount_u128 > policy.per_tx_limit {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_PER_TX_LIMIT),
        });
        env::panic_str(ERR_PER_TX_LIMIT);
    }

    // ── Condition (h): epoch reset then epoch limit check ────────────────────
    maybe_reset_epoch(&caller, policy);
    let Some(new_spent) = policy.spent_in_epoch.checked_add(amount_u128) else {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_OVERFLOW),
        });
        env::panic_str(ERR_OVERFLOW);
    };
    if new_spent > policy.epoch_limit {
        log_transfer_requested(&TransferRequestedData {
            agent_id: &caller,
            recipient: &recipient,
            amount,
            decision: "rejected",
            rejection_reason: Some(ERR_EPOCH_LIMIT),
        });
        env::panic_str(ERR_EPOCH_LIMIT);
    }

    // ── All checks passed: increment counter BEFORE promise ──────────────────
    policy.spent_in_epoch = new_spent;
    let epoch_start_ns = policy.epoch_start_ns;

    log_transfer_requested(&TransferRequestedData {
        agent_id: &caller,
        recipient: &recipient,
        amount,
        decision: "approved",
        rejection_reason: None,
    });

    // ── Dispatch transfer and chain callback for rollback on failure ──────────
    Promise::new(recipient)
        .transfer(near_sdk::NearToken::from_yoctonear(amount_u128))
        .then(
            Vault::ext(env::current_account_id())
                // Reserve gas for the rollback callback up front: if the caller attached too
                // little, this call fails and reverts (counter included) instead of approving
                // a payout whose rollback could later run out of gas.
                .with_static_gas(CALLBACK_GAS)
                .on_transfer_complete(caller, amount, epoch_start_ns.into()),
        )
}
