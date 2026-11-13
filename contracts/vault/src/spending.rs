use crate::Vault;
use crate::errors::ERR_EPOCH_DURATION_ZERO;
use crate::events::{
    AccountingAnomalyData, CallbackIgnoredData, EpochResetData, TransferRolledBackData,
    log_accounting_anomaly, log_callback_ignored, log_epoch_reset, log_transfer_rolled_back,
};
use crate::policy::Policy;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, env};

/// Check if the epoch has expired; reset counter if so.
/// Must be called BEFORE the epoch limit check.
pub fn maybe_reset_epoch(agent_id: &AccountId, policy: &mut Policy) {
    near_sdk::require!(policy.epoch_duration_ns >= 1, ERR_EPOCH_DURATION_ZERO);
    let now = env::block_timestamp();
    if now.saturating_sub(policy.epoch_start_ns) >= policy.epoch_duration_ns {
        let prev_spend = policy.spent_in_epoch;
        policy.spent_in_epoch = 0;
        policy.epoch_start_ns = now;
        log_epoch_reset(&EpochResetData {
            agent_id,
            previous_epoch_spend: prev_spend.into(),
            new_epoch_start_ns: now.into(),
        });
    }
}

/// Callback: settles the agent's single pending reservation `transfer_id`.
///
/// * The reservation is looked up by ID. A callback for any other ID (stale, duplicate or
///   superseded after a timeout) changes nothing and only logs `callback_ignored`.
/// * The amount and the epoch it was counted in come from the stored reservation, never from
///   the callback arguments.
/// * A failed payout is refunded only if the agent is still in the epoch the amount was counted
///   in. After an epoch rollover the counter was already reset to zero, so subtracting would
///   under-count the new epoch.
/// * If the refund would take the counter below zero the invariant is broken. The counter is left
///   as it is (never lowered on a guess) and `accounting_anomaly` is logged.
pub fn on_transfer_complete(vault: &mut Vault, agent_id: AccountId, transfer_id: U64) {
    // A Transfer promise returns no data, so max_len = 0 is sufficient.
    // Only a genuine Failed result triggers rollback.
    let failed = matches!(
        env::promise_result_checked(0, 0),
        Err(near_sdk::PromiseError::Failed)
    );
    let Some(policy) = vault.agents.get_mut(&agent_id) else {
        return;
    };

    let is_current = policy
        .pending
        .as_ref()
        .is_some_and(|p| p.id == transfer_id.0);
    if !is_current {
        let reason = if policy.pending.is_some() {
            "superseded"
        } else {
            "already_settled"
        };
        log_callback_ignored(&CallbackIgnoredData {
            agent_id: &agent_id,
            transfer_id,
            reason,
        });
        return;
    }
    let reservation = policy
        .pending
        .take()
        .unwrap_or_else(|| env::panic_str("pending reservation vanished"));

    if !failed {
        return;
    }
    if policy.epoch_start_ns != reservation.epoch_start_ns {
        return;
    }
    match policy.spent_in_epoch.checked_sub(reservation.amount) {
        Some(remaining) => {
            policy.spent_in_epoch = remaining;
            log_transfer_rolled_back(&TransferRolledBackData {
                agent_id: &agent_id,
                amount: reservation.amount.into(),
                transfer_id,
            });
        }
        None => log_accounting_anomaly(&AccountingAnomalyData {
            agent_id: &agent_id,
            transfer_id,
            amount: reservation.amount.into(),
            spent_in_epoch: policy.spent_in_epoch.into(),
        }),
    }
}
