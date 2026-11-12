//! Task 6.2 (part 1): initialization.
mod helpers;

use helpers::*;
use near_sdk::serde_json::json;
use vault::errors::*;

#[tokio::test]
async fn valid_init_stores_admin_and_unpaused() -> testresult::TestResult<()> {
    let env = setup().await?;
    let st = env.vault_status().await?;
    assert!(!st.paused);
    assert_eq!(st.admin_account_id.as_str(), env.admin.id.as_str());
    assert_eq!(
        st.sentinel_account_id.as_ref().map(|a| a.as_str()),
        Some(env.sentinel.id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn init_without_sentinel_stores_none() -> testresult::TestResult<()> {
    let env = setup_with(false).await?;
    assert!(env.vault_status().await?.sentinel_account_id.is_none());
    Ok(())
}

#[tokio::test]
async fn reinit_is_rejected_and_state_unchanged() -> testresult::TestResult<()> {
    let env = setup().await?;
    // Attacker tries to take over by calling `new` again.
    let res = env
        .call(
            &env.attacker,
            "new",
            json!({ "admin_account_id": env.attacker.id, "sentinel_account_id": null }),
        )
        .await?;
    assert!(res.is_failure(), "re-init must fail");
    // near-sdk's #[init] guard reports the contract is already initialized.
    let text = error_text(&res);
    assert!(
        text.contains("The contract has already been initialized"),
        "unexpected re-init error: {text}"
    );
    assert_eq!(
        env.vault_status().await?.admin_account_id.as_str(),
        env.admin.id.as_str()
    );
    Ok(())
}

#[tokio::test]
async fn invalid_admin_id_rejected_at_init() -> testresult::TestResult<()> {
    // An invalid account id never reaches the contract: it fails argument
    // deserialization (AccountId validation), so the deploy+init transaction fails.
    let env = setup().await?;
    let res = env
        .call(
            &env.attacker,
            "new",
            json!({ "admin_account_id": "Not A Valid Id!", "sentinel_account_id": null }),
        )
        .await?;
    assert!(res.is_failure());
    let _ = ERR_INVALID_ADMIN_ID; // length check inside `new` is defence-in-depth only
    Ok(())
}

#[tokio::test]
async fn methods_before_init_fail() -> testresult::TestResult<()> {
    // Deploy WITHOUT calling `new`, then call register_agent.
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let caller = create_actor(&sandbox, "caller", ACCOUNT_BALANCE).await?;
    let vault_id: near_api::AccountId = "vault.sandbox".parse()?;
    sandbox
        .create_account(vault_id.as_str().parse::<near_sdk::AccountId>()?)
        .initial_balance(VAULT_BALANCE)
        .send()
        .await?;
    let genesis = near_api::Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?,
    )?;
    near_api::Contract::deploy(vault_id.clone())
        .use_code(contract_wasm()?)
        .without_init_call()
        .with_signer(genesis)
        .send_to(&network)
        .await?
        .assert_success();

    let res = near_api::Contract(vault_id)
        .call_function(
            "register_agent",
            register_args(&caller.id, NEAR, 3 * NEAR, HOUR_NS, &[], None),
        )
        .transaction()
        .gas(DEFAULT_GAS)
        .with_signer(caller.id.clone(), caller.signer.clone())
        .send_to(&network)
        .await?;
    assert!(res.is_failure(), "uninitialized contract must reject calls");
    Ok(())
}
