
# NearSentra Threat Model

NearSentra puts a policy-enforcing vault between an autonomous AI agent and funds. This document lists
the attack paths we considered, which layer stops each one, and which test or simulation shows it.

## 1. Authority and layers

**The NEAR vault contract is the final enforcement authority for every financial decision.** Every other
layer can be bypassed, crashed, or compromised without letting an agent exceed its on-chain policy.

| Layer | Role | Kind of enforcement |
|---|---|---|
| Vault contract (`contracts/vault`) | Evaluates all 8 authorization conditions, holds the funds, keeps the spend counters | **Definitive** |
| NEAR access keys | Agent keys can call only `request_transfer`; the Sentinel key can call only `pause_agent` | **Definitive** (enforced by the protocol, set at deployment) |
| Gateway (`services/gateway`) | Validates input, signs with scoped agent keys, reports the on-chain decision | Defense-in-depth |
| Sentinel (`services/sentinel`) | Detects request floods and pauses the agent | Defense-in-depth |
| Dashboard (`frontend`) | Shows state, lets the admin pause/revoke through a wallet | Defense-in-depth (operator visibility) |
| Operational practice | Key custody, admin account choice, monitoring | Defense-in-depth |

Trust assumptions: the NEAR network is honest-majority and its block timestamps are not attacker controlled
beyond normal validator clock skew; the admin account's full-access key is held by a human (ideally a hardware
wallet or multisig account); the vault is funded with an amount the operator is willing to put at risk.

### Contract rules that most scenarios rely on

- Identity: one NEAR account per agent; the contract authorizes by `predecessor_account_id`.
- `request_transfer` checks, in order: vault not paused, caller registered, agent not revoked/paused, policy not expired,
  recipient on allowlist, amount > 0, amount ≤ per-transaction limit, then epoch reset and `spent + amount ≤ epoch limit`
  (checked arithmetic). A rejection logs a `transfer_requested` event with `decision: "rejected"` before it panics.
- The counter is incremented **before** the payout promise and rolled back by a `#[private]` callback only if the payout failed.
- Admin methods (`register_agent`, `update_agent_policy`, `revoke_agent`, `unpause_agent`, `pause_vault`, `unpause_vault`,
  `set_sentinel`) require the admin **and exactly 1 yoctoNEAR**, which only a full-access key can attach.
  `pause_agent` is the only method the Sentinel may call; it never needs a deposit.

## 2. Scenarios

Test references use `contracts/vault/tests/<file>::<test>` for sandbox integration tests,
`contracts/vault/src/tests.rs::<test>` for unit tests, and `simulation scenario N` for `make simulate`.

---

### T1. Prompt injection changes the recipient or amount

- **Preconditions:** The agent's LLM reads attacker-controlled text (web page, email, token metadata).
- **Attacker Action:** Injects instructions so the agent requests a payment to the attacker's account or for a huge amount.
- **Expected System Behavior:** The vault rejects any recipient not on the agent's allowlist (`ERR_RECIPIENT_NOT_ALLOWED`) and any amount over the per-transaction or epoch limit. The agent can still be tricked into paying an allowlisted recipient within its limits.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::recipient_not_allowlisted_rejected`, `::per_tx_limit_rejected_and_boundary_accepted`; simulation scenario 2 (rogue agent).
- **Residual Risk:** Accepted. Loss is bounded by the epoch limit and the allowlist; operators should keep both small and allowlist only recipients they trust.

### T2. Compromised agent credential

- **Preconditions:** An attacker obtains an agent's private key (leaked from the gateway host or agent runtime).
- **Attacker Action:** Signs transactions as the agent, calls `request_transfer`, or tries other vault methods.
- **Expected System Behavior:** The key is a function-call key limited to `request_transfer` on the vault, so the network refuses every other method. `request_transfer` is bound by the same policy as the legitimate agent. The admin can pause or revoke the agent; the Sentinel pauses it on a flood.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::attacker_cannot_spend_via_agent_policy`; `test_suspension.rs::revoke_is_permanent`; simulation scenario 5 (revoked agent).
- **Residual Risk:** Accepted. Maximum loss is one epoch limit to allowlisted recipients before a human revokes. The key restriction is created by deployment tooling (`examples/localnet.rs` does this); an operator who issues a full-access key to an agent removes that protection.

### T3. Runaway agent transaction loop

- **Preconditions:** A buggy or hijacked agent submits requests in a tight loop.
- **Attacker Action:** Floods `request_transfer` within one epoch.
- **Expected System Behavior:** The epoch limit stops spending once reached. The Sentinel counts requests (approved and rejected) per agent per window and calls `pause_agent` when the threshold is exceeded; later requests fail with `ERR_AGENT_PAUSED`.
- **Enforcement Layer:** contract (spend cap and pause), sentinel (detection)
- **Test or Simulation Reference:** simulation scenario 3 (runaway agent, live Sentinel); `services/sentinel/tests/test_monitor.py::test_velocity_pauses_agent_and_records_alert`; `test_detector.py::test_velocity_triggers_when_count_exceeds_threshold`; `test_suspension.rs::admin_and_sentinel_pause_block_transfers_and_admin_can_unpause`.
- **Residual Risk:** Mitigated. The Sentinel reacts after a few blocks, so a burst can complete before the pause lands, but never beyond the epoch limit.

### T4. Unauthorized recipient

- **Preconditions:** Agent is registered with an allowlist.
- **Attacker Action:** Requests a transfer to an account that is not on the allowlist, or asks the gateway to do so.
- **Expected System Behavior:** The vault panics with `ERR_RECIPIENT_NOT_ALLOWED` and no funds move. The gateway forwards the request anyway (its preflight is advisory) and reports `decision: "rejected"`.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::recipient_not_allowlisted_rejected`; `services/gateway/test/routes.test.ts` "still forwards requests the preflight thinks will fail"; simulation scenario 2.
- **Residual Risk:** Mitigated.

### T5. Per-transaction limit bypass

- **Preconditions:** Agent has a per-transaction limit smaller than its epoch limit.
- **Attacker Action:** Requests an amount above the per-transaction limit, or exactly at and just above the boundary.
- **Expected System Behavior:** `amount > per_tx_limit` panics with `ERR_PER_TX_LIMIT`; an amount exactly equal is accepted. Registration and updates reject `per_tx_limit > epoch_limit`, zero limits, and zero durations.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::per_tx_limit_rejected_and_boundary_accepted`; `src/tests.rs::transfer_rejected_per_tx_limit`, `::transfer_exact_per_tx_limit_accepted`; simulation scenario 2.
- **Residual Risk:** Mitigated.

### T6. Epoch-budget bypass

- **Preconditions:** Agent has an epoch limit.
- **Attacker Action:** Splits a large payment into many allowed ones; tries to reset the counter by causing failed payouts; or times spending at an epoch boundary.
- **Expected System Behavior:** Splitting is limited by the cumulative counter (`ERR_EPOCH_LIMIT`). Each agent has at most one reserved transfer in flight (a second request gets `ERR_TRANSFER_PENDING`). The callback refunds the amount stored in that reservation, never an amount passed as an argument, and only if the agent is still in the epoch it was counted in, so failing payouts cannot erase other spend. Fixed windows allow up to two epoch limits in a short span if an agent spends fully just before and just after a reset.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::epoch_limit_rejected_when_cumulative_exceeds`; `test_epoch.rs::epoch_resets_after_duration_and_emits_event`, `::no_reset_within_epoch`; `src/tests.rs::second_request_while_one_is_pending_is_rejected`, `::rollover_after_expiry_then_late_failure_does_not_undercount_the_new_epoch`; `test_reservations.rs::concurrent_requests_never_exceed_the_epoch_budget`, `::failed_payout_after_epoch_rollover_refunds_only_the_new_epoch`.
- **Residual Risk:** Accepted. The boundary burst is inherent to fixed epochs; operators who care should choose a per-transaction limit and epoch limit whose double is tolerable.

### T7. Replay of a previously authorized request

- **Preconditions:** An attacker sees an agent's earlier `request_transfer` transaction.
- **Attacker Action:** Rebroadcasts the same signed transaction, or has the agent/gateway repeat the same request.
- **Expected System Behavior:** NEAR rejects a rebroadcast signed transaction (nonce already used, block hash expires). A new, identical request from a holder of the key is a new transaction and is evaluated again, consuming budget each time. The gateway therefore treats `(agent_id, request_id)` as an idempotency key: the first request is processed, and any repeat with the same recipient and amount (including concurrent ones) gets the stored response with an `idempotent-replay: true` header and submits nothing. Reusing a `request_id` with a different recipient or amount returns HTTP 409 `request_id_conflict`. Results are remembered once a transaction may have reached the chain (including 502 submission or outcome errors, so a retry cannot double pay); results where nothing was submitted (503 preflight failure, 422 no key) are not, so the agent can retry them. To retry on purpose after an ambiguous 502, use a new `request_id`.
- **Enforcement Layer:** gateway (idempotency), contract (budget caps), NEAR protocol (nonces)
- **Test or Simulation Reference:** `services/gateway/test/routes.test.ts` "POST /transfer idempotency"; `services/gateway/test/idempotency.test.ts`; protocol nonce behaviour is not re-tested here; repeated spend is bounded by `test_authorization.rs::epoch_limit_rejected_when_cumulative_exceeds`.
- **Residual Risk:** Reduced. The idempotency store is in memory, per gateway process, with a 24 hour TTL and 10,000 entry cap. It is lost on restart and not shared between gateway instances, so a retry after a restart can still be charged against the budget twice. The contract's limits remain the final authority.

### T8. Timestamp or boundary manipulation

- **Preconditions:** Policy has an expiry and/or an epoch duration.
- **Attacker Action:** Tries to spend right at expiry or the epoch boundary, or influence block time.
- **Expected System Behavior:** The contract uses `env::block_timestamp()`, which the agent cannot set. A policy is expired once `now >= expiry`; an epoch resets when `now - start >= duration`. Registration and update reject an expiry in the past.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_authorization.rs::expired_policy_rejected`; `src/tests.rs::transfer_not_before_expiry_accepted`, `::epoch_resets_after_duration`, `::register_expiry_in_past`; `test_epoch.rs`.
- **Residual Risk:** Accepted. Validators control block time within the protocol's tolerance (seconds), so boundaries are only accurate to that.

### T9. Arithmetic overflow

- **Preconditions:** Policy limits are near `u128::MAX`.
- **Attacker Action:** Requests amounts that wrap the spend counter.
- **Expected System Behavior:** `spent + amount` uses `checked_add`; overflow is rejected with `ERR_OVERFLOW` and logged as a rejection. The release profile sets `overflow-checks = true`. Rollback uses `checked_sub`: a rollback that would go below zero leaves the counter untouched and logs `accounting_anomaly`. JSON amounts are `U128` strings, so no precision is lost.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `src/tests.rs::transfer_counter_overflow_is_rejected_not_wrapped`, `::rollback_below_zero_leaves_the_counter_alone_and_logs_an_anomaly`; gateway `schema.test.ts` "accepts the amount bounds 1 and 2^128-1".
- **Residual Risk:** Mitigated.

### T10. Sentinel compromise

- **Preconditions:** An attacker holds the Sentinel's runtime key, or even the Sentinel account's full-access key.
- **Attacker Action:** Calls `request_transfer`, `update_agent_policy`, `unpause_vault`, or `pause_agent` for every agent.
- **Expected System Behavior:** The runtime key is limited by the network to `pause_agent` on the vault. The Sentinel account is not a registered agent (`ERR_AGENT_NOT_REGISTERED`) and is not the admin (`ERR_ACCESS_DENIED`), so even its full-access key cannot move funds, change policy, or unpause. The Sentinel service refuses to start pausing with a key broader than `pause_agent`.
- **Enforcement Layer:** contract, NEAR access keys
- **Test or Simulation Reference:** simulation scenario 4; `test_access_control.rs::sentinel_denied_on_every_admin_only_method`, `::sentinel_can_pause_agent_but_not_unpause_vault`; `services/sentinel/tests/test_pauser.py::test_scope_verification_refuses_broader_keys`.
- **Residual Risk:** Accepted. A compromised Sentinel can pause every agent (denial of service); the admin recovers with `unpause_agent` and `set_sentinel`.

### T11. Gateway compromise

- **Preconditions:** An attacker controls the gateway process or host and reads its environment.
- **Attacker Action:** Uses the agent keys it holds to submit transfers, or alters requests and responses.
- **Expected System Behavior:** The gateway holds only agent function-call keys limited to `request_transfer`; it holds no admin or Sentinel key. Anything it submits is evaluated by the vault under each agent's policy, so the damage is the same as T2 for each agent it holds a key for. A dishonest gateway could misreport a decision to a caller, but the chain record and events are authoritative.
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `services/gateway/test/client.test.ts` "rejects a missing or malformed key without echoing it"; `routes.test.ts` "unknown agent key => 422 and nothing submitted"; `test_authorization.rs::attacker_cannot_spend_via_agent_policy`.
- **Residual Risk:** Accepted. Keep the gateway on a trusted host, bound to `127.0.0.1` (the default), and give each agent only the budget it needs.

### T12. Admin key compromise

- **Preconditions:** An attacker holds the admin account's full-access key.
- **Attacker Action:** Registers an attacker-controlled agent or allowlists their own account, then drains the vault within the new limits; or unpauses the vault.
- **Expected System Behavior:** The contract treats the admin as fully trusted and does not stop this. A leaked function-call key on the admin account cannot call admin methods, because they need an attached 1 yoctoNEAR deposit.
- **Enforcement Layer:** operational
- **Test or Simulation Reference:** `test_access_control.rs::admin_methods_require_exactly_one_yocto`, `::attacker_denied_on_every_admin_only_method`; `src/tests.rs::admin_method_without_one_yocto_is_rejected`; full admin compromise is manual (not testable at the contract level).
- **Residual Risk:** Transferred. Use a hardware-wallet-backed or multisig admin account; the vault has no timelock or admin rotation method, so a compromised admin key is not recoverable on the same vault.

### T13. Frontend compromise or malicious UI

- **Preconditions:** The dashboard is served from a compromised host or a lookalike site.
- **Attacker Action:** Shows false state, or asks the admin to sign a different call than the one displayed (for example `unpause_vault` instead of `pause_agent`).
- **Expected System Behavior:** The dashboard stores no keys and signs nothing; the wallet signs. Admin methods require 1 yoctoNEAR, so the wallet must show a full-access confirmation with the method name and arguments. The contract re-checks the caller on every call. The dashboard never applies optimistic state; it re-reads the chain after a call.
- **Enforcement Layer:** contract, operational (wallet confirmation)
- **Test or Simulation Reference:** manual: inspect `frontend/src` (no key handling) and `frontend/src/pages/AdminControls.tsx` (wallet-only signing, deposit `1`); `test_access_control.rs::admin_methods_require_exactly_one_yocto`.
- **Residual Risk:** Transferred. The admin must read the wallet prompt; a malicious UI cannot sign without the wallet.

### T14. RPC failure or stale state

- **Preconditions:** The RPC node is down, lagging, or lying.
- **Attacker Action:** Withholds blocks or serves old state so the Sentinel or gateway acts on stale data.
- **Expected System Behavior:** The contract executes against on-chain state, not any RPC view. The gateway returns 503 if its preflight view fails and 502 if submission or the outcome is unavailable; it never invents a decision. The Sentinel flags a stall longer than 30 seconds as an error, keeps retrying, and treats events seen after an outage as alert-only (no pauses). The dashboard keeps showing the last data with an error banner and the last update time.
- **Enforcement Layer:** contract, gateway, sentinel
- **Test or Simulation Reference:** `services/gateway/test/routes.test.ts` "preflight RPC failure => 503 and nothing submitted", "submission failure => 502 with no fabricated outcome"; `services/sentinel/tests/test_monitor.py::test_outage_over_30s_logs_error_retries_fast_and_recovers_without_pausing`, `::test_replayed_events_after_outage_never_pause`.
- **Residual Risk:** Accepted. During an outage the Sentinel cannot pause floods; the contract limits still apply. A malicious RPC could hide events from the Sentinel and dashboard; use a trusted RPC.

### T15. Failed async transfer and accounting consistency

- **Preconditions:** The vault approves a transfer whose payout later fails (for example the recipient account does not exist).
- **Attacker Action:** Crafts failing payouts to leave the counter wrong in either direction.
- **Expected System Behavior:** The counter is raised before the promise. The `#[private]` callback `on_transfer_complete` checks the promise result, looks up the agent's single reservation by transfer ID, ignores any other ID (`callback_ignored`, so a duplicate or stale callback changes nothing), refunds the stored amount with `checked_sub` only if the agent's epoch has not rolled over since, and emits `transfer_rolled_back`. Nobody can call the callback directly. A reservation whose callback never ran is cleared after 100 blocks and stays counted as spent (fail-closed, `pending_expired`).
- **Enforcement Layer:** contract
- **Test or Simulation Reference:** `test_callback.rs::failed_transfer_rolls_back_counter_and_emits_event`, `::successful_transfer_emits_no_rollback`, `::direct_callback_invocation_is_rejected`; `src/tests.rs::callback_failed_rolls_back`, `::duplicate_callback_does_not_refund_twice`; `test_reservations.rs::nobody_can_forge_a_callback`, `::callback_gas_with_a_maximum_size_policy_stays_under_the_reserve`.
- **Residual Risk:** Mitigated. Until the callback runs (one or two blocks), the counter is conservatively high and the agent cannot start another transfer (it retries after `ERR_TRANSFER_PENDING`).

### T16. DoS through excessive requests or storage growth

- **Preconditions:** Anyone can send transactions to the vault.
- **Attacker Action:** Spams `request_transfer` from many accounts; registers/updates to grow storage; or triggers the Sentinel to pause legitimate agents with a fast legitimate burst.
- **Expected System Behavior:** Per-request storage does not grow: counters are updated in place, and only the admin can add agents. Allowlists are capped at 50 entries (the linear scan is bounded). Spam from unregistered accounts is rejected at the second check and costs the attacker gas. The Sentinel does not pause unregistered accounts. A legitimate agent that exceeds the velocity threshold is paused by design and must be unpaused by the admin.
- **Enforcement Layer:** contract, sentinel
- **Test or Simulation Reference:** `test_registration.rs::allowlist_dedupe_and_capacity`, `::register_with_51_entries_rejected`; `test_authorization.rs::unregistered_caller_rejected`; `services/sentinel/tests/test_monitor.py::test_unregistered_account_is_not_paused`.
- **Residual Risk:** Accepted. Request spam can still add load to the Sentinel's RPC polling, and the Sentinel pausing a busy agent is an availability trade-off. Choose the velocity threshold above the agent's normal rate.

### T17. Agent-to-agent payment: seller overcharge, redirected payment, or impersonation

- **Preconditions:** A buyer agent (ResearchAgent) is registered with a per-transaction limit equal to the agreed service price, an epoch budget, and only the seller agent (AnalysisAgent) allowlisted.
- **Attacker Action:** The seller raises its price after the deal; a prompt injection tells the buyer to pay a stranger instead; the buyer is told to buy the service again past its budget; a third party signs a payment with its own key to spend the buyer's budget; or the buyer keeps paying after the admin paused it.
- **Expected System Behavior:** The vault rejects each request with its own code: `ERR_PER_TX_LIMIT`, `ERR_RECIPIENT_NOT_ALLOWED`, `ERR_EPOCH_LIMIT`, `ERR_AGENT_NOT_REGISTERED` (identity is the on-chain caller, never a request field) and `ERR_AGENT_PAUSED`. No balance or spent counter changes. The agreed payment is approved, reserved before the payout and settled once in the callback, so the seller receives exactly the price.
- **Enforcement Layer:** contract. The Gateway only forwards the request and reports the contract's decision.
- **Test or Simulation Reference:** `test_agent_to_agent.rs::research_agent_pays_analysis_agent_under_policy`; simulation scenario 6 and `make demo-agent-payment` (same steps through the Gateway).
- **Residual Risk:** Accepted. The vault limits how much and to whom an agent pays, but it cannot check that the seller delivered the service, and there is no escrow or refund. A dishonest allowlisted seller can keep one payment within the limits. Allowlist only sellers you trust and keep the per-transaction limit close to the price.

---

## 3. Known limitations

- Fixed-window epochs allow a boundary burst of up to two epoch limits (T6).
- The vault does not check its own balance before approving; a payout that exceeds the balance fails and rolls back (T15).
- There is no deposit method; the vault is funded by plain transfers to its account.
- The admin cannot be rotated and there is no timelock (T12).
- Event timestamps come from block time (T8).
- Agent and Sentinel keys are restricted by access-key permissions that deployment tooling must set; the contract cannot see which key signed.
- The function-call key allowance pays for gas and runs out; renew it before it does.
- The Sentinel's chain polling has been exercised against the local sandbox only, not against a public network with several shards.
