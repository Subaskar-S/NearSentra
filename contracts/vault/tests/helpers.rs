//! Shared sandbox integration-test harness for the Vault contract.
//!
//! Usage: add `mod helpers;` to a test file, then
//! `let env = helpers::setup().await?;`.
#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use near_api::types::transaction::actions::{AccessKeyPermission, FunctionCallPermission};
use near_api::types::transaction::result::TransactionResult;
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use near_sdk::serde::Serialize;
use near_sdk::serde_json::{self, json};
use testresult::TestResult;
use vault::policy::{PolicyView, VaultStatus};

/// Default gas attached to state-changing calls (300 Tgas is the NEAR max per tx).
pub const DEFAULT_GAS: NearGas = NearGas::from_tgas(100);
/// Vault methods that require the admin and exactly 1 yoctoNEAR attached.
pub const ADMIN_METHODS: [&str; 7] = [
    "register_agent",
    "update_agent_policy",
    "revoke_agent",
    "unpause_agent",
    "pause_vault",
    "unpause_vault",
    "set_sentinel",
];
/// NEAR balance given to each user sub-account.
pub const ACCOUNT_BALANCE: NearToken = NearToken::from_near(10);
/// NEAR balance the vault contract account starts with (pays out transfers).
pub const VAULT_BALANCE: NearToken = NearToken::from_near(100);

/// An account together with the signer holding its own full-access key.
#[derive(Clone)]
pub struct Actor {
    pub id: AccountId,
    pub signer: Arc<Signer>,
    /// `ed25519:...` secret key string (sandbox-only keys; used by the localnet launcher).
    pub secret_key: String,
}

/// Everything a test needs: the running sandbox, network config, the deployed
/// vault, and pre-funded actors. Keep `sandbox` alive for the whole test.
pub struct TestEnv {
    pub sandbox: near_sandbox::Sandbox,
    pub network: NetworkConfig,
    pub vault: AccountId,
    pub admin: Actor,
    pub agent: Actor,
    pub sentinel: Actor,
    pub recipient: Actor,
    pub attacker: Actor,
}

/// Build the contract wasm once per test binary and return its bytes.
pub fn contract_wasm() -> TestResult<Vec<u8>> {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    if let Some(w) = WASM.get() {
        return Ok(w.clone());
    }
    let path = cargo_near_build::build_with_cli(Default::default())?;
    let bytes = std::fs::read(path)?;
    Ok(WASM.get_or_init(|| bytes).clone())
}

/// Create `<name>.sandbox` funded with `balance` and controlled by a fresh key.
pub async fn create_actor(
    sandbox: &near_sandbox::Sandbox,
    name: &str,
    balance: NearToken,
) -> TestResult<Actor> {
    let id: AccountId = format!("{name}.sandbox").parse()?;
    let secret = near_api::signer::generate_secret_key()?;
    let public_key = secret.public_key();
    sandbox
        .create_account(id.as_str().parse::<near_sdk::AccountId>()?)
        .initial_balance(balance)
        .public_key(public_key.to_string())
        .send()
        .await?;
    let secret_key = secret.to_string();
    let signer = Signer::from_secret_key(secret)?;
    Ok(Actor {
        id,
        signer,
        secret_key,
    })
}

/// Start an isolated sandbox, deploy the vault, call `new` with the generated
/// admin and sentinel, and create all actors.
pub async fn setup() -> TestResult<TestEnv> {
    setup_with(true).await
}

/// Like [`setup`], but `with_sentinel = false` initialises with no sentinel.
pub async fn setup_with(with_sentinel: bool) -> TestResult<TestEnv> {
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);

    let admin = create_actor(&sandbox, "admin", ACCOUNT_BALANCE).await?;
    let agent = create_actor(&sandbox, "agent", ACCOUNT_BALANCE).await?;
    let sentinel = create_actor(&sandbox, "sentinel", ACCOUNT_BALANCE).await?;
    let recipient = create_actor(&sandbox, "recipient", ACCOUNT_BALANCE).await?;
    let attacker = create_actor(&sandbox, "attacker", ACCOUNT_BALANCE).await?;

    // Vault contract account (funded; uses the default genesis key to deploy).
    let vault_id: AccountId = "vault.sandbox".parse()?;
    sandbox
        .create_account(vault_id.as_str().parse::<near_sdk::AccountId>()?)
        .initial_balance(VAULT_BALANCE)
        .send()
        .await?;
    let genesis_signer = Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?,
    )?;

    Contract::deploy(vault_id.clone())
        .use_code(contract_wasm()?)
        .with_init_call(
            "new",
            json!({
                "admin_account_id": admin.id,
                "sentinel_account_id": with_sentinel.then(|| sentinel.id.clone()),
            }),
        )?
        .with_signer(genesis_signer)
        .send_to(&network)
        .await?
        .assert_success();

    Ok(TestEnv {
        sandbox,
        network,
        vault: vault_id,
        admin,
        agent,
        sentinel,
        recipient,
        attacker,
    })
}

impl TestEnv {
    /// Call a vault method as `caller` with no deposit; returns the full result
    /// (check `.is_success()` / `.is_failure()` or use [`error_text`]).
    pub async fn call<A: Serialize>(
        &self,
        caller: &Actor,
        method: &str,
        args: A,
    ) -> TestResult<TransactionResult> {
        // Admin methods require exactly 1 yoctoNEAR (full-access key); everything else none.
        let deposit = if ADMIN_METHODS.contains(&method) {
            NearToken::from_yoctonear(1)
        } else {
            NearToken::from_yoctonear(0)
        };
        self.call_with_deposit(caller, method, args, deposit).await
    }

    /// Same as [`call`](Self::call) with an attached deposit.
    pub async fn call_with_deposit<A: Serialize>(
        &self,
        caller: &Actor,
        method: &str,
        args: A,
        deposit: NearToken,
    ) -> TestResult<TransactionResult> {
        let res = Contract(self.vault.clone())
            .call_function(method, args)
            .transaction()
            .gas(DEFAULT_GAS)
            .deposit(deposit)
            .with_signer(caller.id.clone(), caller.signer.clone())
            .send_to(&self.network)
            .await?;
        Ok(res)
    }

    /// Call and assert the whole transaction (incl. callbacks) succeeded.
    pub async fn call_ok<A: Serialize>(&self, caller: &Actor, method: &str, args: A) {
        let res = self.call(caller, method, args).await.expect("send failed");
        assert!(res.is_success(), "{method} should succeed: {res:?}");
    }

    /// Call and assert failure; returns the failure text for substring checks.
    pub async fn call_err<A: Serialize>(&self, caller: &Actor, method: &str, args: A) -> String {
        let res = self.call(caller, method, args).await.expect("send failed");
        assert!(res.is_failure(), "{method} should fail but succeeded");
        error_text(&res)
    }

    /// View `get_vault_status`.
    pub async fn vault_status(&self) -> TestResult<VaultStatus> {
        Ok(Contract(self.vault.clone())
            .call_function("get_vault_status", ())
            .read_only()
            .fetch_from(&self.network)
            .await?
            .data)
    }

    /// View `get_agent_policy` (None if the agent is not registered).
    pub async fn agent_policy(&self, agent: &AccountId) -> TestResult<Option<PolicyView>> {
        Ok(Contract(self.vault.clone())
            .call_function("get_agent_policy", json!({ "agent_id": agent }))
            .read_only()
            .fetch_from(&self.network)
            .await?
            .data)
    }

    /// Total NEAR balance of an account.
    pub async fn balance(&self, id: &AccountId) -> TestResult<NearToken> {
        Ok(near_api::Tokens::account(id.clone())
            .near_balance()
            .fetch_from(&self.network)
            .await?
            .total)
    }
}

/// Full debug rendering of a result, including every receipt failure. Panic
/// messages from contract `require!`/`env::panic_str` appear in this text, so
/// tests can do `assert!(error_text(&res).contains(ERR_...))`.
pub fn error_text(res: &TransactionResult) -> String {
    format!("{res:?}")
}

/// 1 NEAR in yoctoNEAR.
pub const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
/// One hour in nanoseconds (default epoch length for tests).
pub const HOUR_NS: u64 = 3_600 * 1_000_000_000;

/// Parse every `EVENT_JSON:` log of a result into JSON values.
pub fn events(res: &TransactionResult) -> Vec<serde_json::Value> {
    res.logs()
        .into_iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .filter_map(|j| serde_json::from_str(j).ok())
        .collect()
}

/// Events with the given `event` name.
pub fn events_named(res: &TransactionResult, name: &str) -> Vec<serde_json::Value> {
    events(res)
        .into_iter()
        .filter(|e| e["event"] == name)
        .collect()
}

/// Args for `register_agent`.
pub fn register_args(
    agent: &AccountId,
    per_tx: u128,
    epoch: u128,
    duration_ns: u64,
    allowlist: &[&AccountId],
    expiry_ns: Option<u64>,
) -> serde_json::Value {
    json!({
        "agent_id": agent,
        "params": {
            "per_tx_limit": per_tx.to_string(),
            "epoch_limit": epoch.to_string(),
            "epoch_duration_ns": duration_ns.to_string(),
            "allowlist": allowlist,
            "expiry_ns": expiry_ns.map(|e| e.to_string()),
        }
    })
}

impl TestEnv {
    /// Register `self.agent` with per-tx 1 NEAR, epoch 3 NEAR, 1 hour epoch,
    /// and `self.recipient` allowlisted.
    pub async fn register_default_agent(&self) {
        self.register_agent_with(NEAR, 3 * NEAR, HOUR_NS, None)
            .await;
    }

    /// Register `self.agent` with custom limits; `self.recipient` allowlisted.
    pub async fn register_agent_with(
        &self,
        per_tx: u128,
        epoch: u128,
        duration_ns: u64,
        expiry_ns: Option<u64>,
    ) {
        self.call_ok(
            &self.admin,
            "register_agent",
            register_args(
                &self.agent.id,
                per_tx,
                epoch,
                duration_ns,
                &[&self.recipient.id],
                expiry_ns,
            ),
        )
        .await;
    }

    /// `request_transfer` as `caller` to `recipient` for `amount` yoctoNEAR.
    pub async fn transfer(
        &self,
        caller: &Actor,
        recipient: &AccountId,
        amount: u128,
    ) -> TransactionResult {
        self.call(
            caller,
            "request_transfer",
            json!({ "recipient": recipient, "amount": amount.to_string() }),
        )
        .await
        .expect("send failed")
    }

    /// Current `spent_in_epoch` of an agent (stored counter as the view reports it).
    pub async fn spent(&self, agent: &AccountId) -> u128 {
        self.agent_policy(agent)
            .await
            .expect("view")
            .expect("registered")
            .spent_in_epoch
            .0
    }

    /// Give `owner` a pool of `n` scoped keys (function-call keys limited to `request_transfer`
    /// on the vault, exactly the shape the gateway and the local network use) and return a signer
    /// that rotates over them. Rotating keys lets many transactions be in flight at once without
    /// colliding on a nonce, which is what real concurrent agent traffic looks like.
    pub async fn scoped_key_pool(&self, owner: &Actor, n: usize) -> TestResult<Arc<Signer>> {
        let mut pool: Option<Arc<Signer>> = None;
        for _ in 0..n {
            let secret = near_api::signer::generate_secret_key()?;
            Account(owner.id.clone())
                .add_key(
                    AccessKeyPermission::FunctionCall(FunctionCallPermission {
                        allowance: Some(NearToken::from_near(5)),
                        receiver_id: self.vault.to_string(),
                        method_names: vec!["request_transfer".to_string()],
                    }),
                    secret.public_key(),
                )
                .with_signer(owner.signer.clone())
                .send_to(&self.network)
                .await?
                .assert_success();
            match &pool {
                None => pool = Some(Signer::from_secret_key(secret)?),
                Some(signer) => signer.add_secret_key_to_pool(secret).await?,
            }
        }
        pool.ok_or_else(|| "scoped_key_pool needs n >= 1".into())
    }

    /// `request_transfer` signed by `signer` (typically a [`scoped_key_pool`](Self::scoped_key_pool))
    /// on behalf of the account `agent`.
    pub async fn transfer_as(
        &self,
        signer: &Arc<Signer>,
        agent: &AccountId,
        recipient: &AccountId,
        amount: u128,
    ) -> TestResult<TransactionResult> {
        Ok(Contract(self.vault.clone())
            .call_function(
                "request_transfer",
                json!({ "recipient": recipient, "amount": amount.to_string() }),
            )
            .transaction()
            .gas(DEFAULT_GAS)
            .with_signer(agent.clone(), signer.clone())
            .send_to(&self.network)
            .await?)
    }

    /// Advance the sandbox until the chain clock reaches `target_ns`.
    pub async fn wait_until_chain_time(&self, target_ns: u64) -> TestResult<()> {
        for _ in 0..120 {
            if self.chain_now_ns().await >= target_ns {
                return Ok(());
            }
            self.sandbox.fast_forward(20).await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Err("chain clock never reached target".into())
    }

    /// Latest block timestamp in nanoseconds.
    pub async fn chain_now_ns(&self) -> u64 {
        near_api::Chain::block()
            .fetch_from(&self.network)
            .await
            .expect("block")
            .header
            .timestamp
    }
}

/// Assert a result failed and its text contains the `ERR_*` identifier of `expected`
/// (the part before the first ':').
pub fn assert_rejected(res: &TransactionResult, expected: &str) {
    assert!(
        res.is_failure(),
        "expected failure with {expected}, got success"
    );
    let code = expected.split(':').next().unwrap();
    let text = error_text(res);
    assert!(text.contains(code), "expected {code} in: {text}");
}

/// Convenience for building JSON args from serializable values.
pub fn to_json<T: Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).expect("serialize")
}
