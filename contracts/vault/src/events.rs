use near_sdk::json_types::{U64, U128};
use near_sdk::serde::Serialize;
use near_sdk::{AccountId, env};

const EVENT_STANDARD: &str = "nearsentra";
const EVENT_STANDARD_VERSION: &str = "1.0.0";

/// NEP-297 envelope — logged as "EVENT_JSON:{...}"
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct NearEvent<'a, T: Serialize> {
    standard: &'a str,
    version: &'a str,
    event: &'a str,
    data: &'a T,
}

fn log_event<T: Serialize>(event_name: &str, data: &T) {
    let event = NearEvent {
        standard: EVENT_STANDARD,
        version: EVENT_STANDARD_VERSION,
        event: event_name,
        data,
    };
    let json = near_sdk::serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
    env::log_str(&format!("EVENT_JSON:{}", json));
}

// ── Event data structs ────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TransferRequestedData<'a> {
    pub agent_id: &'a AccountId,
    pub recipient: &'a AccountId,
    pub amount: U128,
    pub decision: &'a str, // "approved" or "rejected"
    pub rejection_reason: Option<&'a str>,
    /// ID of the reservation, present only on approved requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<U64>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct EpochResetData<'a> {
    pub agent_id: &'a AccountId,
    pub previous_epoch_spend: U128,
    pub new_epoch_start_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AgentPausedData<'a> {
    pub agent_id: &'a AccountId,
    pub caller: &'a AccountId,
    pub timestamp_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AgentUnpausedData<'a> {
    pub agent_id: &'a AccountId,
    pub caller: &'a AccountId,
    pub timestamp_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AgentRevokedData<'a> {
    pub agent_id: &'a AccountId,
    pub caller: &'a AccountId,
    pub timestamp_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct VaultPausedData<'a> {
    pub caller: &'a AccountId,
    pub timestamp_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct VaultUnpausedData<'a> {
    pub caller: &'a AccountId,
    pub timestamp_ns: U64,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TransferRolledBackData<'a> {
    pub agent_id: &'a AccountId,
    pub amount: U128,
    pub transfer_id: U64,
}

/// A reservation that never settled was cleared after the timeout; its amount stays counted.
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PendingExpiredData<'a> {
    pub agent_id: &'a AccountId,
    pub transfer_id: U64,
    pub amount: U128,
}

/// A callback arrived for a reservation that is not (or no longer) the agent's pending transfer.
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct CallbackIgnoredData<'a> {
    pub agent_id: &'a AccountId,
    pub transfer_id: U64,
    pub reason: &'a str,
}

/// A rollback would have taken the counter below zero. The counter is left untouched (fail-closed).
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AccountingAnomalyData<'a> {
    pub agent_id: &'a AccountId,
    pub transfer_id: U64,
    pub amount: U128,
    pub spent_in_epoch: U128,
}

// ── Public log functions ──────────────────────────────────────────────────────

pub fn log_transfer_requested(data: &TransferRequestedData) {
    log_event("transfer_requested", data);
}

pub fn log_epoch_reset(data: &EpochResetData) {
    log_event("epoch_reset", data);
}

pub fn log_agent_paused(data: &AgentPausedData) {
    log_event("agent_paused", data);
}

pub fn log_agent_unpaused(data: &AgentUnpausedData) {
    log_event("agent_unpaused", data);
}

pub fn log_agent_revoked(data: &AgentRevokedData) {
    log_event("agent_revoked", data);
}

pub fn log_vault_paused(data: &VaultPausedData) {
    log_event("vault_paused", data);
}

pub fn log_vault_unpaused(data: &VaultUnpausedData) {
    log_event("vault_unpaused", data);
}

pub fn log_transfer_rolled_back(data: &TransferRolledBackData) {
    log_event("transfer_rolled_back", data);
}

pub fn log_pending_expired(data: &PendingExpiredData) {
    log_event("pending_expired", data);
}

pub fn log_callback_ignored(data: &CallbackIgnoredData) {
    log_event("callback_ignored", data);
}

pub fn log_accounting_anomaly(data: &AccountingAnomalyData) {
    log_event("accounting_anomaly", data);
}
