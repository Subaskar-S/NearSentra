mod helpers;

#[tokio::test]
async fn vault_deploys_and_initialises() -> testresult::TestResult<()> {
    let env = helpers::setup().await?;
    let status = env.vault_status().await?;
    assert!(!status.paused);
    assert_eq!(status.admin_account_id.as_str(), env.admin.id.as_str());
    assert_eq!(
        status.sentinel_account_id.as_ref().map(|a| a.as_str()),
        Some(env.sentinel.id.as_str())
    );
    assert!(env.agent_policy(&env.agent.id).await?.is_none());
    Ok(())
}
