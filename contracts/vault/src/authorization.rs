use crate::Vault;
use crate::errors::*;
use crate::events::{
    PendingExpiredData, TransferRequestedData, log_pending_expired, log_transfer_requested,
};
use crate::policy::{PENDING_TIMEOUT_BLOCKS, PendingTransfer};
use crate::spending::maybe_reset_epoch;
use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, Gas, Promise, env};

/// Gas reserved for `on_transfer_complete`. Measured at about 2 Tgas for a rollback with a
/// maximum-size policy (50 allowlist entries of 64 characters), so this leaves ample headroom.
const CALLBACK_GAS: Gas = Gas::from_tgas(5);

/// Log a `rejected` decision and abort the call (the panic reverts any state change).
fn reject(agent: &AccountId, recipient: &AccountId, amount: U128, reason: &str) -> ! {
    log_transfer_requested(&TransferRequestedData {
        agent_id: agent,
        recipient,
        amount,
        decision: "rejected",
        rejection_reason: Some(reason),
        transfer_id: None,
    });
    env::panic_str(reason)
}

pub fn request_transfer(vault: &mut Vault, recipient: AccountId, amount: U128) -> Promise {
    // The agent's identity is the immediate caller (predecessor), never the transaction signer.
    let caller = env::predecessor_account_id();
    let amount_u128 = amount.0;

    // ── Condition (a): vault not paused ───────────────────────────────────────
    if vault.paused {
        reject(&caller, &recipient, amount, ERR_VAULT_PAUSED);
    }

    // ── Condition (b): caller is registered ──────────────────────────────────
    let Some(policy) = vault.agents.get_mut(&caller) else {
        reject(&caller, &recipient, amount, ERR_AGENT_NOT_REGISTERED);
    };

    // ── Condition (c): agent active and not revoked ───────────────────────────
    if policy.revoked {
        reject(&caller, &recipient, amount, ERR_AGENT_REVOKED);
    }
    if !policy.active {
        reject(&caller, &recipient, amount, ERR_AGENT_PAUSED);
    }

    // ── Condition (d): policy not expired ────────────────────────────────────
    if policy
        .expiry_ns
        .is_some_and(|expiry| env::block_timestamp() >= expiry)
    {
        reject(&caller, &recipient, amount, ERR_POLICY_EXPIRED);
    }

    // ── Condition (e): recipient on allowlist ─────────────────────────────────
    if !policy.allowlist.contains(&recipient) {
        reject(&caller, &recipient, amount, ERR_RECIPIENT_NOT_ALLOWED);
    }

    // ── Condition (f): amount > 0 ─────────────────────────────────────────────
    if amount_u128 == 0 {
        reject(&caller, &recipient, amount, ERR_AMOUNT_ZERO);
    }

    // ── Condition (g): amount <= per_tx_limit ────────────────────────────────
    if amount_u128 > policy.per_tx_limit {
        reject(&caller, &recipient, amount, ERR_PER_TX_LIMIT);
    }

    // ── Condition (h): epoch reset then epoch limit check ────────────────────
    maybe_reset_epoch(&caller, policy);
    let Some(new_spent) = policy.spent_in_epoch.checked_add(amount_u128) else {
        reject(&caller, &recipient, amount, ERR_OVERFLOW);
    };
    if new_spent > policy.epoch_limit {
        reject(&caller, &recipient, amount, ERR_EPOCH_LIMIT);
    }

    // ── Condition (i): no other transfer of this agent is still in flight ────
    // Exactly one reservation per agent keeps the accounting trivially correct: every
    // callback maps to one stored reservation. A reservation whose callback never ran is
    // cleared after PENDING_TIMEOUT_BLOCKS and stays counted as spent (fail-closed).
    if let Some(stale) = policy.pending.as_ref() {
        if env::block_height().saturating_sub(stale.created_block) > PENDING_TIMEOUT_BLOCKS {
            log_pending_expired(&PendingExpiredData {
                agent_id: &caller,
                transfer_id: stale.id.into(),
                amount: stale.amount.into(),
            });
            policy.pending = None;
        } else {
            reject(&caller, &recipient, amount, ERR_TRANSFER_PENDING);
        }
    }

    // ── All checks passed: reserve the amount BEFORE the promise ─────────────
    let transfer_id = policy.next_transfer_id;
    policy.next_transfer_id = transfer_id
        .checked_add(1)
        .unwrap_or_else(|| env::panic_str(ERR_OVERFLOW));
    policy.spent_in_epoch = new_spent;
    policy.pending = Some(PendingTransfer {
        id: transfer_id,
        amount: amount_u128,
        epoch_start_ns: policy.epoch_start_ns,
        created_block: env::block_height(),
    });

    log_transfer_requested(&TransferRequestedData {
        agent_id: &caller,
        recipient: &recipient,
        amount,
        decision: "approved",
        rejection_reason: None,
        transfer_id: Some(U64(transfer_id)),
    });

    // ── Dispatch transfer and chain the settling callback ────────────────────
    Promise::new(recipient)
        .transfer(near_sdk::NearToken::from_yoctonear(amount_u128))
        .then(
            Vault::ext(env::current_account_id())
                // Reserve gas for the callback up front: if the caller attached too little,
                // this call fails and reverts (reservation included) instead of approving a
                // payout whose settlement could later run out of gas.
                .with_static_gas(CALLBACK_GAS)
                .on_transfer_complete(caller, U64(transfer_id)),
        )
}
