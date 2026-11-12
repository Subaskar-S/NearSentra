# NearSentra Vault contract

Rust contract (near-sdk 5) that holds funds and decides every agent payment. See the root `README.md` for the whole project
and `docs/SECURITY_TESTS.md` for the audit.

## Layout

| File | Purpose |
|---|---|
| `src/lib.rs` | `Vault` state, `new`, views, method wiring |
| `src/authorization.rs` | `request_transfer` and its 8 checks |
| `src/spending.rs` | epoch reset and the `#[private]` rollback callback |
| `src/identity.rs` | `register_agent`, `update_agent_policy` |
| `src/revocation.rs` | pause/unpause/revoke, global pause, admin gate |
| `src/policy.rs`, `events.rs`, `errors.rs` | data types, NEP-297 events, `ERR_*` constants |
| `tests/` | sandbox integration tests (real NEAR sandbox) |
| `examples/localnet.rs` | launches the local network used by `make sandbox-deploy` |

## Commands (from this directory)

```bash
cargo test                                   # unit + sandbox integration tests
cargo clippy --all-targets -- -D warnings    # lint
cargo near build non-reproducible-wasm       # build the contract
```

A plain `cargo build` is blocked by near-sdk for native targets; use `cargo near build`.
