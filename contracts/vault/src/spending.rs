use crate::Vault;
use crate::errors::ERR_EPOCH_DURATION_ZERO;
use crate::events::{
    EpochResetData, TransferRolledBackData, log_epoch_reset, log_transfer_rolled_back,
};
use crate::policy::Policy;
use near_sdk::json_types::{U64, U128};
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

/// Callback — rolls back epoch counter if the transfer promise failed.
/// Uses saturating_sub to avoid erasing concurrent successful transactions.
///
/// `epoch_start_ns` is the epoch the amount was counted in. If the agent's
/// epoch has since rolled over, the counter was already reset to zero and the
/// failed amount no longer sits in the current counter, so nothing is
/// subtracted (otherwise new-epoch spend would be under-counted).
pub fn on_transfer_complete(
    vault: &mut Vault,
    agent_id: AccountId,
    amount: U128,
    epoch_start_ns: U64,
) {
    // A Transfer promise returns no data, so max_len = 0 is sufficient.
    // Only a genuine Failed result triggers rollback.
    let failed = matches!(
        env::promise_result_checked(0, 0),
        Err(near_sdk::PromiseError::Failed)
    );
    if !failed {
        return;
    }
    if let Some(policy) = vault.agents.get_mut(&agent_id) {
        if policy.epoch_start_ns != epoch_start_ns.0 {
            return;
        }
        policy.spent_in_epoch = policy.spent_in_epoch.saturating_sub(amount.0);
        log_transfer_rolled_back(&TransferRolledBackData {
            agent_id: &agent_id,
            amount,
        });
    }
}
