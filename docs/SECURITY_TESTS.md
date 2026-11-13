# Vault Contract Audit Findings

Scope: `contracts/vault/src` (near-sdk 5.30.1). Method: the checklist from the NEAR contract audit skill
(`.agents/skills/near-contract-audit`, 14 detectors), applied by manual review of every source file, backed by
`cargo clippy --all-targets -- -D warnings`, the unit tests and the sandbox integration tests. No third-party
Rust vulnerability scanner was run; the review is by hand.

Result: **0 unresolved high-severity findings.** One high-severity item (yocto-attach) was found and fixed during
this review; one medium item (callback gas) was hardened.

## Checklist

| # | Detector | Severity | Status | Evidence |
|---|---|---|---|---|
| 1 | `non-private-callback` | High | PASS | `on_transfer_complete` is `#[private]` (`lib.rs`). `test_callback.rs::direct_callback_invocation_is_rejected`, `test_access_control.rs::callback_is_private`. |
| 2 | `reentrancy` | High | PASS | `spent_in_epoch` is raised before the payout promise is created and rolled back in the callback only on failure (`authorization.rs`, `spending.rs`). `test_callback.rs::failed_transfer_rolls_back_counter_and_emits_event`. |
| 3 | `incorrect-argument-or-return-types` | High | PASS | All `u128`/`u64` values in the public interface are `U128`/`U64` (decimal strings): `PolicyParams`, `PolicyUpdate`, `PolicyView`, event data, `request_transfer`, callback. The only plain integer is `version: u32`. `src/tests.rs::approved_transfer_event_uses_string_amounts`, `::event_timestamps_are_json_strings`. |
| 4 | `unsaved-changes` | High | PASS | Policies are changed through `store::LookupMap::get_mut` and `insert` inside `&mut self` methods; the collection flushes when the contract value is dropped at the end of the call. Every integration test reads the changed state back through a separate view call. |
| 5 | `owner-check` | High | PASS | See the access-control table below. |
| 6 | `yocto-attach` | High | **FOUND, FIXED**, then PASS | See finding F1. |
| 7 | `storage-collision` | High | PASS | One collection (`agents`) with prefix `StorageKey::Agents` (`BorshStorageKey` enum). No other prefixed collection exists. |
| 8 | `required-initialization-macro` | High | PASS | `new` is `#[init]`; `Vault` derives `PanicOnDefault`. `test_init.rs::reinit_is_rejected_and_state_unchanged`, `::methods_before_init_fail`, `::invalid_admin_id_rejected_at_init`. |
| 9 | `gas-griefing` | Medium | PASS | No unbounded loops. The only loops walk an allowlist capped at 50 entries (`register_agent` dedupe, `update_agent_policy`, `request_transfer` lookup). `test_registration.rs::allowlist_dedupe_and_capacity`, `::register_with_51_entries_rejected`. |
| 10 | `insecure-random` | Medium | PASS (not applicable) | The contract uses no randomness. |
| 11 | `prepaid-gas` | Medium | **HARDENED**, PASS | See finding F2. |
| 12 | `cover-storage-cost` | Low | PASS (accepted) | Storage growth happens only in admin-only `register_agent`, paid from the vault's balance; callers cannot make the vault store data. The runtime refuses the registration if the vault cannot cover the storage stake. |
| 13 | `unsafe-math` | Low | PASS | `checked_add` for the spend counter (`ERR_OVERFLOW`), `checked_sub` for rollback (`saturating_sub` only in views), `saturating_add` for `version`, `overflow-checks = true` in the release profile. `src/tests.rs::transfer_counter_overflow_is_rejected_not_wrapped`. |
| 14 | `float-math` | Low | PASS | No `f32`/`f64` anywhere in `src`. |

### Access-control table (detector 5)

| Method | Who may call | Check | Test |
|---|---|---|---|
| `new` | once, at deployment | `#[init]` | `test_init.rs` |
| `set_sentinel`, `register_agent`, `update_agent_policy`, `revoke_agent`, `unpause_agent`, `pause_vault`, `unpause_vault` | admin, with exactly 1 yoctoNEAR | `require_admin` (`revocation.rs`) | `test_access_control.rs::attacker_denied_on_every_admin_only_method`, `::sentinel_denied_on_every_admin_only_method`, `::admin_methods_require_exactly_one_yocto` |
| `pause_agent` | admin or the stored Sentinel | `require_admin_or_sentinel` | `test_access_control.rs::sentinel_can_pause_agent_but_not_unpause_vault`, `::attacker_cannot_pause_agent`, `::no_sentinel_configured_only_admin_can_pause` |
| `request_transfer` | any registered, active, non-revoked agent (authorized by `predecessor_account_id`) | 8 conditions in `authorization.rs` | `test_authorization.rs` (one test per condition) |
| `on_transfer_complete` | the contract itself | `#[private]` | `test_callback.rs::direct_callback_invocation_is_rejected` |
| `get_agent_policy`, `get_vault_status` | anyone (views) | read-only | `test_registration.rs::unknown_agent_view_is_none` |

## Findings

### F1. Admin methods callable with a function-call key (yocto-attach) — High — fixed

- **Location:** `identity.rs`, `revocation.rs`, `lib.rs::set_sentinel`.
- **Description:** The admin methods checked only `predecessor_account_id == admin`. A function-call access key added to
  the admin account (for a script or an app) could therefore register agents, change allowlists, revoke agents or
  unpause the vault without a wallet confirmation.
- **Resolution:** Every admin method is `#[payable]` and calls `require_admin`, which checks the admin first (everyone else
  still sees `ERR_ACCESS_DENIED`) and then requires exactly 1 yoctoNEAR (`ERR_ONE_YOCTO_REQUIRED`). Only full-access keys can
  attach a deposit. The dashboard and the simulation attach 1 yoctoNEAR.
- **Deliberate exceptions:** `request_transfer` and `pause_agent` do not require a deposit, because agents and the Sentinel
  are designed to run on function-call keys restricted to those single methods.
- **Verification:** `test_access_control.rs::admin_methods_require_exactly_one_yocto` (0 and 2 yocto rejected for all seven
  methods, 1 yocto accepted); `src/tests.rs::admin_method_without_one_yocto_is_rejected`,
  `::non_admin_sees_access_denied_before_deposit_check`.

### F2. Callback gas was not reserved (prepaid-gas) — Medium — hardened

- **Location:** `authorization.rs::request_transfer`.
- **Description:** The rollback callback was scheduled without a gas reservation. With a very small gas attachment the payout
  could be approved and the callback could run out of gas, which only ever leaves the counter too high (fail-safe) but
  would block part of an agent's budget until the next epoch.
- **Resolution:** The callback is scheduled with `with_static_gas(5 Tgas)`. If the attached gas cannot cover it, the call
  fails at promise creation and the whole transaction reverts, counter included.
- **Verification:** `test_callback.rs::too_little_gas_fails_cleanly_without_touching_the_counter`; the rollback test still passes
  with the reservation.

## Additional manual review

| Topic | Result |
|---|---|
| Order of checks and state change | All 8 authorization conditions run before any state change; every rejection logs a `rejected` event and then panics (state reverts, the log persists). Verified in the sandbox. |
| Rollback correctness | One reservation per agent with a monotonic transfer ID. `on_transfer_complete(agent_id, transfer_id)` reads the amount and epoch from the stored reservation, ignores unknown or already-settled IDs, uses `checked_sub` and skips the refund when the epoch has rolled over, so it cannot erase concurrent spend, under-count a new epoch or refund twice. |
| Paused vs revoked | Revocation is permanent: `update_agent_policy` and `unpause_agent` both refuse a revoked agent. The Sentinel cannot unpause agents or the vault. |
| Events | `EVENT_JSON:` logs follow NEP-297 with standard `nearsentra`; numbers are strings. Only the contract can emit logs under its account. |
| Policy validation | Zero limits and zero durations rejected, `per_tx_limit <= epoch_limit` re-checked on update, expiry must be in the future, allowlist deduplicated and capped at 50. |
| Panic strings | Panics are `ERR_X: message`; the Gateway and tests match on the `ERR_X` code. |

## Accepted limitations (not vulnerabilities)

Listed in `docs/THREAT_MODEL.md` section 3: fixed-window boundary burst, no vault-balance pre-check, no admin rotation or
timelock, no deposit method, block-time timestamps.

## How to reproduce

```bash
make test    # 79 unit tests and 52 sandbox integration tests
make lint    # clippy -D warnings, tsc --noEmit, ruff
```
