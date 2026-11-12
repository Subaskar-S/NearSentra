use near_sdk::AccountId;
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::json_types::{U64, U128};
use near_sdk::near;
use near_sdk::serde::Deserialize;

/// Storage key enum — unique variant per collection to prevent key collisions.
/// IMPORTANT: Never reorder or delete variants after first deployment.
#[derive(BorshSerialize, near_sdk::BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
pub enum StorageKey {
    Agents,
}

/// Per-agent policy stored on-chain (Borsh only — not exposed as JSON directly).
#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Policy {
    pub active: bool,
    pub revoked: bool,
    pub per_tx_limit: u128,
    pub epoch_limit: u128,
    pub epoch_duration_ns: u64,
    pub epoch_start_ns: u64,
    pub spent_in_epoch: u128,
    pub allowlist: Vec<AccountId>,
    pub expiry_ns: Option<u64>,
    pub version: u32,
}

/// JSON view returned to callers — includes computed remaining budget.
#[near(serializers = [json])]
pub struct PolicyView {
    pub agent_id: AccountId,
    pub active: bool,
    pub revoked: bool,
    pub per_tx_limit: U128,
    pub epoch_limit: U128,
    pub epoch_duration_ns: U64,
    pub epoch_start_ns: U64,
    pub spent_in_epoch: U128,
    pub remaining_epoch_budget: U128,
    pub allowlist: Vec<AccountId>,
    pub expiry_ns: Option<U64>,
    pub version: u32,
}

/// Parameters for registering a new agent.
#[near(serializers = [json])]
pub struct PolicyParams {
    pub per_tx_limit: U128,
    pub epoch_limit: U128,
    pub epoch_duration_ns: U64,
    pub allowlist: Vec<AccountId>,
    pub expiry_ns: Option<U64>,
}

/// Fields that can be updated via update_agent_policy.
#[near(serializers = [json])]
pub struct PolicyUpdate {
    pub per_tx_limit: Option<U128>,
    pub epoch_limit: Option<U128>,
    pub epoch_duration_ns: Option<U64>,
    /// Absent / not provided: leave unchanged. `null`: clear the expiry.
    /// A number string: set the expiry.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub expiry_ns: Option<Option<U64>>,
    pub allowlist_add: Option<Vec<AccountId>>,
    pub allowlist_remove: Option<Vec<AccountId>>,
}

/// Distinguishes a missing field (`None`, via `#[serde(default)]`) from an explicit
/// JSON `null` (`Some(None)`) so that `update_agent_policy` can clear an expiry.
fn deserialize_double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: near_sdk::serde::Deserializer<'de>,
    T: near_sdk::serde::Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Return type for get_vault_status.
#[near(serializers = [json])]
pub struct VaultStatus {
    pub paused: bool,
    pub admin_account_id: AccountId,
    pub sentinel_account_id: Option<AccountId>,
}
