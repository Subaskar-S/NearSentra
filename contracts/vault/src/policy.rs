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
    /// Monotonic ID handed to the next approved transfer (never reused).
    pub next_transfer_id: u64,
    /// The single in-flight transfer, if any. Its amount is already counted in `spent_in_epoch`.
    pub pending: Option<PendingTransfer>,
}

/// A payout that has been approved and reserved but whose callback has not settled yet.
///
/// The callback receives only the transfer `id`; the amount and the epoch it was counted in are
/// read from this record, so a stale, duplicate or forged callback argument can never change the
/// counter by a value the vault did not itself reserve.
#[derive(BorshDeserialize, BorshSerialize, Clone)]
#[borsh(crate = "near_sdk::borsh")]
pub struct PendingTransfer {
    pub id: u64,
    pub amount: u128,
    /// `epoch_start_ns` of the epoch the amount was counted in.
    pub epoch_start_ns: u64,
    /// Block height at which the reservation was made (used to expire a stuck reservation).
    pub created_block: u64,
}

/// A reservation that has not settled after this many blocks is treated as settled-as-spent and
/// cleared, so a callback that could never run cannot lock an agent out forever. The amount stays
/// counted (fail-closed): the agent loses that budget for the current epoch, never gains any.
/// A normal callback settles within 2-3 blocks.
pub const PENDING_TIMEOUT_BLOCKS: u64 = 100;

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
    /// ID of the in-flight transfer, if any. While set, new requests are rejected.
    pub pending_transfer_id: Option<U64>,
    /// Amount reserved by the in-flight transfer (already included in `spent_in_epoch`).
    pub pending_amount: Option<U128>,
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
