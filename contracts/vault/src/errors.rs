// Access control
pub const ERR_ACCESS_DENIED: &str = "ERR_ACCESS_DENIED: caller is not authorized";
pub const ERR_ONE_YOCTO_REQUIRED: &str = "ERR_ONE_YOCTO_REQUIRED: attach exactly 1 yoctoNEAR (forces a full-access key and wallet confirmation)";
pub const ERR_INVALID_ADMIN_ID: &str = "ERR_INVALID_ADMIN_ID: admin account ID is invalid";

// Agent registration
pub const ERR_AGENT_NOT_FOUND: &str = "ERR_AGENT_NOT_FOUND: agent is not registered";
pub const ERR_DUPLICATE_AGENT: &str = "ERR_DUPLICATE_AGENT: agent is already registered";
pub const ERR_REVOKED_AGENT: &str = "ERR_REVOKED_AGENT: agent has been permanently revoked";

// Policy validation
pub const ERR_TX_EXCEEDS_EPOCH: &str =
    "ERR_TX_EXCEEDS_EPOCH: per-transaction limit exceeds epoch limit";
pub const ERR_ALLOWLIST_FULL: &str =
    "ERR_ALLOWLIST_FULL: allowlist has reached the maximum of 50 entries";
pub const ERR_EPOCH_DURATION_ZERO: &str =
    "ERR_EPOCH_DURATION_ZERO: epoch duration must be greater than 0";
pub const ERR_EXPIRY_IN_PAST: &str = "ERR_EXPIRY_IN_PAST: policy expiry must be in the future";

// Transfer authorization
pub const ERR_VAULT_PAUSED: &str = "ERR_VAULT_PAUSED: vault is globally paused";
pub const ERR_AGENT_NOT_REGISTERED: &str =
    "ERR_AGENT_NOT_REGISTERED: caller is not a registered agent";
pub const ERR_AGENT_PAUSED: &str = "ERR_AGENT_PAUSED: agent is paused";
pub const ERR_AGENT_NOT_PAUSED: &str = "ERR_AGENT_NOT_PAUSED: agent is not paused";
pub const ERR_AGENT_REVOKED: &str = "ERR_AGENT_REVOKED: agent has been permanently revoked";
pub const ERR_POLICY_EXPIRED: &str = "ERR_POLICY_EXPIRED: agent policy has expired";
pub const ERR_RECIPIENT_NOT_ALLOWED: &str =
    "ERR_RECIPIENT_NOT_ALLOWED: recipient is not on the allowlist";
pub const ERR_AMOUNT_ZERO: &str = "ERR_AMOUNT_ZERO: transfer amount must be greater than zero";
pub const ERR_PER_TX_LIMIT: &str = "ERR_PER_TX_LIMIT: amount exceeds per-transaction limit";
pub const ERR_EPOCH_LIMIT: &str = "ERR_EPOCH_LIMIT: amount would exceed epoch spending limit";

// Arithmetic
pub const ERR_OVERFLOW: &str = "ERR_OVERFLOW: arithmetic overflow";

// Sentinel
pub const ERR_SENTINEL_NOT_SET: &str = "ERR_SENTINEL_NOT_SET: no sentinel account configured";
