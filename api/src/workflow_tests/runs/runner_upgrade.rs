use super::*;

#[tokio::test]
async fn owned_runner_upgrade_rotates_machine_credentials_over_http() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state.clone());
    let registered = app
        .clone()
        .oneshot(json_request(
            "POST",
            scope_api_contract::routes::RUNNERS,
            Some(bearer_header()),
            &RegisterRunnerRequest {
                owner: TEST_REPO_OWNER.to_string(),
                repo: TEST_REPO_NAME.to_string(),
                name: "upgrade-box".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs: RunnerMaxConcurrentJobs::new(2).unwrap(),
            },
        ))
        .await
        .unwrap();
    let registered = response_json(registered).await;
    assert_eq!(registered["runner"]["max_concurrent_jobs"], 2);
    let runner_id = registered["runner"]["id"].as_str().unwrap();
    let old_secret = registered["secret"].as_str().unwrap();

    let unauthorized = app
        .clone()
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::runner_upgrade(runner_id),
            None,
            &UpgradeRunnerRegistrationRequest {
                version: "2.0.0".to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs: RunnerMaxConcurrentJobs::new(2).unwrap(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let cache_ack = app
        .clone()
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::attempt_cache_finalization("attempt-missing"),
            None,
            &AttemptCacheFinalizationRequest {
                outcome: AttemptCacheFinalizationOutcome::Succeeded,
            },
        ))
        .await
        .unwrap();
    assert_eq!(cache_ack.status(), StatusCode::UNAUTHORIZED);

    let upgraded = app
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::runner_upgrade(runner_id),
            Some(bearer_header()),
            &UpgradeRunnerRegistrationRequest {
                version: "2.0.0".to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs: RunnerMaxConcurrentJobs::new(3).unwrap(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(upgraded.status(), StatusCode::OK);
    let upgraded = response_json(upgraded).await;
    let new_secret = upgraded["secret"].as_str().unwrap();
    assert_ne!(new_secret, old_secret);
    assert_eq!(upgraded["runner"]["version"], "2.0.0");
    assert_eq!(upgraded["runner"]["max_concurrent_jobs"], 3);
    assert!(
        state
            .metadata
            .runs()
            .authenticate_runner(&machine_token_hash(old_secret), unix_now())
            .await
            .is_err()
    );
    assert_eq!(
        state
            .metadata
            .runs()
            .authenticate_runner(&machine_token_hash(new_secret), unix_now())
            .await
            .unwrap()
            .id,
        runner_id
    );
}
