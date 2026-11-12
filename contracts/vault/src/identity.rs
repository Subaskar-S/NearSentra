use crate::Vault;
use crate::errors::*;
use crate::policy::{Policy, PolicyParams, PolicyUpdate};
use near_sdk::{AccountId, env, require};

pub fn register_agent(vault: &mut Vault, agent_id: AccountId, params: PolicyParams) {
    crate::revocation::require_admin(vault);
    require!(!vault.agents.contains_key(&agent_id), ERR_DUPLICATE_AGENT);

    let per_tx: u128 = params.per_tx_limit.0;
    let epoch: u128 = params.epoch_limit.0;
    let duration: u64 = params.epoch_duration_ns.0;

    require!(per_tx >= 1, ERR_AMOUNT_ZERO);
    require!(epoch >= 1, ERR_AMOUNT_ZERO);
    require!(duration >= 1, ERR_EPOCH_DURATION_ZERO);
    require!(per_tx <= epoch, ERR_TX_EXCEEDS_EPOCH);
    if let Some(expiry) = params.expiry_ns {
        require!(expiry.0 > env::block_timestamp(), ERR_EXPIRY_IN_PAST);
    }

    // De-duplicate the allowlist while preserving order, then enforce the cap.
    let mut allowlist: Vec<AccountId> = Vec::with_capacity(params.allowlist.len());
    for addr in params.allowlist {
        if !allowlist.contains(&addr) {
            allowlist.push(addr);
        }
    }
    require!(allowlist.len() <= 50, ERR_ALLOWLIST_FULL);

    let policy = Policy {
        active: true,
        revoked: false,
        per_tx_limit: per_tx,
        epoch_limit: epoch,
        epoch_duration_ns: duration,
        epoch_start_ns: env::block_timestamp(),
        spent_in_epoch: 0,
        allowlist,
        expiry_ns: params.expiry_ns.map(|v| v.0),
        version: 1,
    };
    vault.agents.insert(agent_id, policy);
}

pub fn update_agent_policy(vault: &mut Vault, agent_id: AccountId, updates: PolicyUpdate) {
    crate::revocation::require_admin(vault);

    let policy = vault
        .agents
        .get_mut(&agent_id)
        .unwrap_or_else(|| near_sdk::env::panic_str(ERR_AGENT_NOT_FOUND));

    require!(!policy.revoked, ERR_REVOKED_AGENT);

    if let Some(v) = updates.per_tx_limit {
        require!(v.0 >= 1, ERR_AMOUNT_ZERO);
        policy.per_tx_limit = v.0;
    }
    if let Some(v) = updates.epoch_limit {
        require!(v.0 >= 1, ERR_AMOUNT_ZERO);
        policy.epoch_limit = v.0;
    }
    if let Some(v) = updates.epoch_duration_ns {
        require!(v.0 >= 1, ERR_EPOCH_DURATION_ZERO);
        policy.epoch_duration_ns = v.0;
    }
    if let Some(v) = updates.expiry_ns {
        // `Some(None)` clears the expiry; `Some(Some(t))` sets it (must be in the future).
        if let Some(t) = v {
            require!(t.0 > env::block_timestamp(), ERR_EXPIRY_IN_PAST);
        }
        policy.expiry_ns = v.map(|u| u.0);
    }

    // Allowlist operations (idempotent)
    if let Some(to_add) = updates.allowlist_add {
        for addr in to_add {
            if !policy.allowlist.contains(&addr) {
                require!(policy.allowlist.len() < 50, ERR_ALLOWLIST_FULL);
                policy.allowlist.push(addr);
            }
        }
    }
    if let Some(to_remove) = updates.allowlist_remove {
        for addr in to_remove {
            policy.allowlist.retain(|a| a != &addr);
        }
    }

    // Re-validate consistency
    require!(
        policy.per_tx_limit <= policy.epoch_limit,
        ERR_TX_EXCEEDS_EPOCH
    );

    policy.version = policy.version.saturating_add(1);
}
