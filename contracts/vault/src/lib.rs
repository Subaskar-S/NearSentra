use near_sdk::json_types::{U64, U128};
use near_sdk::store::LookupMap;
use near_sdk::{AccountId, PanicOnDefault, Promise, near};

pub mod errors;
pub mod events;
pub mod policy;

pub mod authorization;
pub mod identity;
pub mod revocation;
pub mod spending;

#[cfg(test)]
mod tests;

use errors::ERR_INVALID_ADMIN_ID;
use policy::{PolicyParams, PolicyUpdate, PolicyView, StorageKey, VaultStatus};

/// NearSentra vault — the final enforcement authority for all agent fund transfers.
#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Vault {
    pub admin: AccountId,
    pub sentinel: Option<AccountId>,
    pub paused: bool,
    pub agents: LookupMap<AccountId, policy::Policy>,
}

#[near]
impl Vault {
    /// Initialize the vault exactly once.
    #[init]
    pub fn new(admin_account_id: AccountId, sentinel_account_id: Option<AccountId>) -> Self {
        // Validate admin account ID: 2-64 characters
        let s = admin_account_id.as_str();
        near_sdk::require!(s.len() >= 2 && s.len() <= 64, ERR_INVALID_ADMIN_ID);
        Self {
            admin: admin_account_id,
            sentinel: sentinel_account_id,
            paused: false,
            agents: LookupMap::new(StorageKey::Agents),
        }
    }

    // ── Admin: sentinel management ────────────────────────────────────────────

    #[payable]
    pub fn set_sentinel(&mut self, sentinel_account_id: AccountId) {
        revocation::require_admin(self);
        self.sentinel = Some(sentinel_account_id);
    }

    // ── View methods ──────────────────────────────────────────────────────────

    pub fn get_agent_policy(&self, agent_id: AccountId) -> Option<PolicyView> {
        self.agents.get(&agent_id).map(|p| {
            // Report the budget as the next transfer would see it: if the epoch has
            // already elapsed, the counter is treated as reset.
            let now = near_sdk::env::block_timestamp();
            let elapsed = now.saturating_sub(p.epoch_start_ns) >= p.epoch_duration_ns;
            let spent = if elapsed { 0 } else { p.spent_in_epoch };
            let remaining = p.epoch_limit.saturating_sub(spent);
            PolicyView {
                agent_id: agent_id.clone(),
                active: p.active,
                revoked: p.revoked,
                per_tx_limit: p.per_tx_limit.into(),
                epoch_limit: p.epoch_limit.into(),
                epoch_duration_ns: p.epoch_duration_ns.into(),
                epoch_start_ns: p.epoch_start_ns.into(),
                spent_in_epoch: spent.into(),
                remaining_epoch_budget: remaining.into(),
                allowlist: p.allowlist.clone(),
                expiry_ns: p.expiry_ns.map(Into::into),
                version: p.version,
            }
        })
    }

    pub fn get_vault_status(&self) -> VaultStatus {
        VaultStatus {
            paused: self.paused,
            admin_account_id: self.admin.clone(),
            sentinel_account_id: self.sentinel.clone(),
        }
    }

    // ── Agent management ──────────────────────────────────────────────────────

    #[payable]
    pub fn register_agent(&mut self, agent_id: AccountId, params: PolicyParams) {
        identity::register_agent(self, agent_id, params);
    }

    #[payable]
    pub fn update_agent_policy(&mut self, agent_id: AccountId, updates: PolicyUpdate) {
        identity::update_agent_policy(self, agent_id, updates);
    }

    // ── Transfer ──────────────────────────────────────────────────────────────

    pub fn request_transfer(&mut self, recipient: AccountId, amount: U128) -> Promise {
        authorization::request_transfer(self, recipient, amount)
    }

    #[private]
    pub fn on_transfer_complete(&mut self, agent_id: AccountId, amount: U128, epoch_start_ns: U64) {
        spending::on_transfer_complete(self, agent_id, amount, epoch_start_ns);
    }

    // ── Suspension / revocation ───────────────────────────────────────────────

    pub fn pause_agent(&mut self, agent_id: AccountId) {
        revocation::pause_agent(self, agent_id);
    }

    /// Admin-only: re-activate a paused (not revoked) agent.
    #[payable]
    pub fn unpause_agent(&mut self, agent_id: AccountId) {
        revocation::unpause_agent(self, agent_id);
    }

    #[payable]
    pub fn revoke_agent(&mut self, agent_id: AccountId) {
        revocation::revoke_agent(self, agent_id);
    }

    #[payable]
    pub fn pause_vault(&mut self) {
        revocation::pause_vault(self);
    }

    #[payable]
    pub fn unpause_vault(&mut self) {
        revocation::unpause_vault(self);
    }
}
