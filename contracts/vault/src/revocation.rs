use crate::Vault;
use crate::errors::*;
use crate::events::{
    AgentPausedData, AgentRevokedData, AgentUnpausedData, VaultPausedData, VaultUnpausedData,
    log_agent_paused, log_agent_revoked, log_agent_unpaused, log_vault_paused, log_vault_unpaused,
};
use near_sdk::{AccountId, env, require};

/// Admin-only gate for every state-changing admin method.
///
/// The caller must be the admin (checked first, so everyone else sees
/// `ERR_ACCESS_DENIED`) and must attach exactly 1 yoctoNEAR. Only full-access keys
/// can attach a deposit, so a leaked function-call key on the admin account cannot
/// change policy, revoke agents or move the pause switches.
pub fn require_admin(vault: &Vault) {
    require!(
        env::predecessor_account_id() == vault.admin,
        ERR_ACCESS_DENIED
    );
    require!(
        env::attached_deposit() == near_sdk::NearToken::from_yoctonear(1),
        ERR_ONE_YOCTO_REQUIRED
    );
}

fn require_admin_or_sentinel(vault: &Vault) {
    let caller = env::predecessor_account_id();
    let is_admin = caller == vault.admin;
    let is_sentinel = vault.sentinel.as_ref() == Some(&caller);
    require!(is_admin || is_sentinel, ERR_ACCESS_DENIED);
}

pub fn pause_agent(vault: &mut Vault, agent_id: AccountId) {
    require_admin_or_sentinel(vault);

    let policy = vault
        .agents
        .get_mut(&agent_id)
        .unwrap_or_else(|| env::panic_str(ERR_AGENT_NOT_FOUND));
    policy.active = false;

    log_agent_paused(&AgentPausedData {
        agent_id: &agent_id,
        caller: &env::predecessor_account_id(),
        timestamp_ns: env::block_timestamp().into(),
    });
}

/// Admin-only: re-activate an agent previously paused (by admin or sentinel).
/// Revoked agents can never be re-activated.
pub fn unpause_agent(vault: &mut Vault, agent_id: AccountId) {
    require_admin(vault);

    let policy = vault
        .agents
        .get_mut(&agent_id)
        .unwrap_or_else(|| env::panic_str(ERR_AGENT_NOT_FOUND));
    require!(!policy.revoked, ERR_REVOKED_AGENT);
    require!(!policy.active, ERR_AGENT_NOT_PAUSED);
    policy.active = true;

    log_agent_unpaused(&AgentUnpausedData {
        agent_id: &agent_id,
        caller: &env::predecessor_account_id(),
        timestamp_ns: env::block_timestamp().into(),
    });
}

pub fn revoke_agent(vault: &mut Vault, agent_id: AccountId) {
    require_admin(vault);

    let policy = vault
        .agents
        .get_mut(&agent_id)
        .unwrap_or_else(|| env::panic_str(ERR_AGENT_NOT_FOUND));
    policy.revoked = true;
    policy.active = false;

    log_agent_revoked(&AgentRevokedData {
        agent_id: &agent_id,
        caller: &env::predecessor_account_id(),
        timestamp_ns: env::block_timestamp().into(),
    });
}

pub fn pause_vault(vault: &mut Vault) {
    require_admin(vault);
    vault.paused = true;
    log_vault_paused(&VaultPausedData {
        caller: &env::predecessor_account_id(),
        timestamp_ns: env::block_timestamp().into(),
    });
}

pub fn unpause_vault(vault: &mut Vault) {
    require_admin(vault);
    vault.paused = false;
    log_vault_unpaused(&VaultUnpausedData {
        caller: &env::predecessor_account_id(),
        timestamp_ns: env::block_timestamp().into(),
    });
}
