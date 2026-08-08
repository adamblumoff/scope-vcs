use super::*;
use futures_util::StreamExt;
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptCacheFinalizationOutcome, AttemptCacheFinalizationRequest,
    CompleteAttemptStepRequest, PinAttemptContainerImageRequest, RegisterRunnerRequest,
    StepConclusionRequest, UpgradeRunnerRegistrationRequest,
};
use scope_domain::runs::runner::{
    RUNNER_PROTOCOL_VERSION, RunnerCapabilities, RunnerMaxConcurrentJobs,
};
use std::time::Duration;

const WORKFLOW: &str = r#"
name: Test
on:
  manual: true
runs-on: linux-box
caches: []
container:
  image: alpine:3.20
timeout: 5m
jobs:
  checks:
    steps:
      - name: Test
        run: printf 'hello from runner\n'
"#;

const MULTI_JOB_WORKFLOW: &str = r#"
name: Parallel checks
on:
  manual: true
runs-on: any
caches: []
container:
  image: alpine:3.20
timeout: 5m
jobs:
  backend:
    steps:
      - { name: Backend, run: "true" }
  web:
    steps:
      - { name: Web, run: "true" }
  integration:
    needs: [backend, web]
    steps:
      - { name: Integration, run: "true" }
"#;

#[tokio::test]
async fn run_detail_exposes_jobs_in_workflow_order_with_independent_state() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);
    let source = temp_git_repo("multi-job-run-detail");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/parallel.yml"), MULTI_JOB_WORKFLOW).unwrap();
    run_git(Some(&source), &["add", "."], "stage multi-job run source").unwrap();
    commit_all(&source, "multi-job run source");
    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create multi-job run bundle",
    )
    .unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "{}?workflow=parallel&git_oid={git_oid}&request_id=22222222222222222222222222222222",
                    scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
                ))
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(fs::read(bundle_path).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let run_id = response_json(created).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let detail = app
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_detail(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = response_json(detail).await;
    let jobs = detail["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0]["job"]["key"], "backend");
    assert_eq!(jobs[0]["job"]["state"], "queued");
    assert_eq!(jobs[1]["job"]["key"], "integration");
    assert_eq!(jobs[1]["job"]["state"], "blocked");
    assert_eq!(jobs[2]["job"]["key"], "web");
    assert_eq!(jobs[2]["job"]["state"], "queued");
    assert!(
        jobs.iter()
            .all(|job| job["attempts"].as_array().unwrap().is_empty())
    );
}

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

#[tokio::test]
async fn push_trigger_evaluation_is_queryable_by_the_accepted_head() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let mut update = receive_pack_update(&state, vec![("/README.md", Some("triggered"))]);
    update.push_trigger_input.as_mut().unwrap().workflows.push(
        scope_domain::runs::trigger::PushWorkflowFile::new(
            "/.scope/runs/test.yml",
            br#"
name: Push Test
on: { push: true }
runs-on: any
caches: []
container: { image: alpine:3.20 }
timeout: 1m
jobs:
  checks:
    steps:
      - { name: Test, run: "true" }
"#
            .to_vec(),
        )
        .unwrap(),
    );
    let head_oid = update.head_oid.clone();
    let persisted = persist_test_update(&state, update).await.unwrap();
    assert_eq!(persisted.git_head.head_oid, head_oid);
    state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            "push-trigger-api-test",
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_push_trigger_evaluation(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &head_oid,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let evaluation: scope_api_contract::PushTriggerEvaluationResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        evaluation.state,
        scope_domain::runs::trigger::PushTriggerEvaluationState::Succeeded
    );
    assert_eq!(evaluation.head_oid, head_oid);
    assert_eq!(evaluation.checks.len(), 1);
    assert_eq!(evaluation.checks[0].run.git_oid, evaluation.head_oid);
    assert_eq!(
        evaluation.checks[0].run.state,
        scope_domain::runs::run::RunState::Queued
    );
}

#[tokio::test]
async fn manual_run_protocol_crosses_human_runner_and_attempt_credentials() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);

    let registered = app
        .clone()
        .oneshot(json_request(
            "POST",
            scope_api_contract::routes::RUNNERS,
            Some(bearer_header()),
            &RegisterRunnerRequest {
                owner: TEST_REPO_OWNER.to_string(),
                repo: TEST_REPO_NAME.to_string(),
                name: "linux-box".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs: RunnerMaxConcurrentJobs::new(2).unwrap(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(registered.status(), StatusCode::OK);
    let registered = response_json(registered).await;
    let runner_secret = registered["secret"].as_str().unwrap().to_string();

    let source = temp_git_repo("manual-run-protocol");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), WORKFLOW).unwrap();
    fs::write(source.join("hello.txt"), "hello").unwrap();
    run_git(Some(&source), &["add", "."], "stage run source").unwrap();
    commit_all(&source, "run source");
    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create run bundle",
    )
    .unwrap();
    let bundle = fs::read(bundle_path).unwrap();
    let create_uri = format!(
        "{}?workflow=test&git_oid={git_oid}&request_id=11111111111111111111111111111111",
        scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
    );
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(create_uri)
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(bundle.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created = response_json(created).await;
    assert_eq!(created["runner_selection"]["kind"], "named");
    assert_eq!(created["runner_selection"]["name"], "linux-box");

    let polled = app
        .clone()
        .oneshot(machine_request(
            "POST",
            scope_api_contract::routes::RUNNER_POLL,
            &runner_secret,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(polled.status(), StatusCode::OK);
    let polled = response_json(polled).await;
    let run_id = polled["run"]["run_id"].as_str().unwrap().to_string();

    let claimed = app
        .clone()
        .oneshot(machine_request(
            "POST",
            &scope_api_contract::routes::runner_claim(&run_id, "checks"),
            &runner_secret,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(claimed.status(), StatusCode::OK);
    let claimed = response_json(claimed).await;
    let attempt_id = claimed["attempt_id"].as_str().unwrap().to_string();
    let attempt_token = claimed["attempt_token"].as_str().unwrap().to_string();
    assert_eq!(claimed["job"]["git_oid"], git_oid);
    assert_eq!(claimed["job"]["job_key"], "checks");
    assert_eq!(claimed["job"]["workflow_path"], "/.scope/runs/test.yml");
    assert_eq!(claimed["job"]["definition"]["id"], "checks");

    let source_response = app
        .clone()
        .oneshot(machine_request(
            "GET",
            &scope_api_contract::routes::attempt_source(&attempt_id),
            &attempt_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(source_response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(source_response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
        bundle
    );

    let pinned_image = format!("docker.io/library/alpine@sha256:{}", "a".repeat(64));
    let pinned = app
        .clone()
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::attempt_container_image(&attempt_id),
            Some(format!("Bearer {attempt_token}")),
            &PinAttemptContainerImageRequest {
                image: pinned_image.clone(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(pinned.status(), StatusCode::OK);
    assert_eq!(response_json(pinned).await["image"], pinned_image);

    let started = app
        .clone()
        .oneshot(machine_request(
            "POST",
            &scope_api_contract::routes::attempt_step_start(&attempt_id, 0),
            &attempt_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(
        started.status(),
        StatusCode::OK,
        "{:?}",
        response_json(started).await
    );

    let logged = app
        .clone()
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::attempt_logs(&attempt_id),
            Some(format!("Bearer {attempt_token}")),
            &AppendAttemptLogRequest {
                step_index: 0,
                sequence: 1,
                text: "hello from runner\n".to_string(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(logged.status(), StatusCode::OK);

    let events_uri = format!(
        "{}?after=0",
        scope_api_contract::routes::repo_run_events(TEST_REPO_OWNER, TEST_REPO_NAME, &run_id)
    );
    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(events_uri)
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    assert_eq!(events.headers()[CONTENT_TYPE], "text/event-stream");
    let mut event_stream = events.into_body().into_data_stream();
    let mut event_bytes = Vec::new();
    while !String::from_utf8_lossy(&event_bytes).contains("\"state\":\"running\"") {
        let chunk = tokio::time::timeout(Duration::from_secs(2), event_stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        event_bytes.extend_from_slice(&chunk);
    }
    let events = String::from_utf8(event_bytes).unwrap();
    assert!(events.contains("\"text\":\"hello from runner\\n\""));
    assert!(events.contains("\"state\":\"running\""));

    for sequence in 2..=130 {
        let logged = app
            .clone()
            .oneshot(json_request(
                "POST",
                &scope_api_contract::routes::attempt_logs(&attempt_id),
                Some(format!("Bearer {attempt_token}")),
                &AppendAttemptLogRequest {
                    step_index: 0,
                    sequence,
                    text: format!("chunk-{sequence}\n"),
                },
            ))
            .await
            .unwrap();
        assert_eq!(logged.status(), StatusCode::OK);
    }

    let completed = app
        .clone()
        .oneshot(json_request(
            "POST",
            &scope_api_contract::routes::attempt_step_complete(&attempt_id, 0),
            Some(format!("Bearer {attempt_token}")),
            &CompleteAttemptStepRequest {
                conclusion: StepConclusionRequest::Succeeded,
            },
        ))
        .await
        .unwrap();
    assert_eq!(completed.status(), StatusCode::OK);

    let operations = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_operations(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(operations.status(), StatusCode::OK);
    let operations = response_json(operations).await;
    assert_eq!(operations["runs"][0]["id"], run_id);
    assert_eq!(operations["runs"][0]["state"], "succeeded");
    assert_eq!(operations["runs"][0]["runner_selection"]["kind"], "named");
    assert_eq!(
        operations["runs"][0]["runner_selection"]["name"],
        "linux-box"
    );
    assert_eq!(operations["runs"][0]["can_retry"], true);
    assert_eq!(operations["runners"][0]["name"], "linux-box");
    assert_eq!(operations["runners"][0]["state"], "online");
    let operations_json = serde_json::to_string(&operations).unwrap();
    assert!(!operations_json.contains(&runner_secret));
    for forbidden_field in [
        "secret",
        "owner_user_id",
        "capabilities",
        "object_key",
        "token",
    ] {
        assert!(!operations_json.contains(forbidden_field));
    }

    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_detail(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = response_json(detail).await;
    assert_eq!(detail["run"]["id"], run_id);
    assert_eq!(detail["run"]["runner_selection"]["kind"], "named");
    assert_eq!(detail["run"]["runner_selection"]["name"], "linux-box");
    assert!(detail["run"].get("attempt_number").is_none());
    assert_eq!(detail["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(detail["jobs"][0]["job"]["key"], "checks");
    assert_eq!(detail["jobs"][0]["job"]["state"], "succeeded");
    assert_eq!(detail["jobs"][0]["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(detail["jobs"][0]["attempts"][0]["id"], attempt_id);
    assert_eq!(detail["jobs"][0]["attempts"][0]["runner_name"], "linux-box");
    assert_eq!(detail["jobs"][0]["attempts"][0]["state"], "succeeded");
    assert_eq!(detail["jobs"][0]["attempts"][0]["steps"][0]["name"], "Test");
    assert_eq!(
        detail["jobs"][0]["attempts"][0]["steps"][0]["state"],
        "succeeded"
    );
    assert_eq!(detail["jobs"][0]["attempts"][0]["steps"][0]["exit_code"], 0);

    let step_logs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?after=0",
                    scope_api_contract::routes::repo_run_step_logs(
                        TEST_REPO_OWNER,
                        TEST_REPO_NAME,
                        &run_id,
                        &attempt_id,
                        0,
                    )
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(step_logs.status(), StatusCode::OK);
    let step_logs = response_json(step_logs).await;
    assert_eq!(step_logs["logs"].as_array().unwrap().len(), 128);
    assert_eq!(step_logs["logs"][0]["text"], "hello from runner\n");
    assert_eq!(step_logs["logs"][127]["text"], "chunk-128\n");
    assert_eq!(step_logs["next_after"], 128);
    assert_eq!(step_logs["logs_truncated"], false);

    let remaining_logs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?after=128",
                    scope_api_contract::routes::repo_run_step_logs(
                        TEST_REPO_OWNER,
                        TEST_REPO_NAME,
                        &run_id,
                        &attempt_id,
                        0,
                    )
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let remaining_logs = response_json(remaining_logs).await;
    assert_eq!(remaining_logs["logs"].as_array().unwrap().len(), 2);
    assert_eq!(remaining_logs["logs"][0]["text"], "chunk-129\n");
    assert_eq!(remaining_logs["logs"][1]["text"], "chunk-130\n");
    assert_eq!(remaining_logs["next_after"], 130);

    let wrong_step = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_step_logs(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                    &attempt_id,
                    1,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_step.status(), StatusCode::NOT_FOUND);

    let anonymous_operations = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_operations(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous_operations.status(), StatusCode::UNAUTHORIZED);

    let unrelated_operations = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_operations(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for("runs_unrelated", "runs-unrelated@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unrelated_operations.status(), StatusCode::FORBIDDEN);

    let terminal_events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?after=0",
                    scope_api_contract::routes::repo_run_events(
                        TEST_REPO_OWNER,
                        TEST_REPO_NAME,
                        &run_id
                    )
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let terminal_events = to_bytes(terminal_events.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let terminal_events = String::from_utf8(terminal_events.to_vec()).unwrap();
    assert!(terminal_events.contains("\"text\":\"chunk-130\\n\""));
    assert!(terminal_events.contains("\"job_key\":\"checks\""));
    assert!(terminal_events.contains("\"state\":\"succeeded\""));
    assert!(terminal_events.contains("\"logs_truncated\":false"));

    let retried = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(scope_api_contract::routes::repo_run_retry(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::OK);
    let offered_again = app
        .clone()
        .oneshot(machine_request(
            "POST",
            scope_api_contract::routes::RUNNER_POLL,
            &runner_secret,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(offered_again.status(), StatusCode::OK);
    let claimed_again = app
        .clone()
        .oneshot(machine_request(
            "POST",
            &scope_api_contract::routes::runner_claim(&run_id, "checks"),
            &runner_secret,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(claimed_again.status(), StatusCode::OK);
    let newer_attempt_id = response_json(claimed_again).await["attempt_id"]
        .as_str()
        .unwrap()
        .to_string();

    let retried_detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_detail(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let retried_detail = response_json(retried_detail).await;
    assert_eq!(
        retried_detail["jobs"][0]["attempts"][0]["id"],
        newer_attempt_id
    );
    assert_eq!(retried_detail["jobs"][0]["attempts"][1]["id"], attempt_id);
    let retried_detail_json = serde_json::to_string(&retried_detail).unwrap();
    assert!(!retried_detail_json.contains("\"number\""));
    assert!(!retried_detail_json.contains("attempt_number"));

    let human_cannot_use_attempt_token = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &run_id,
                ))
                .header(AUTHORIZATION, format!("Bearer {attempt_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        human_cannot_use_attempt_token.status(),
        StatusCode::UNAUTHORIZED
    );

    let runner_cannot_read_without_attempt_token = app
        .clone()
        .oneshot(machine_request(
            "GET",
            &scope_api_contract::routes::attempt_source(&attempt_id),
            &runner_secret,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(
        runner_cannot_read_without_attempt_token.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn unused_runner_registration_can_be_rolled_back() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);

    let registered = app
        .clone()
        .oneshot(json_request(
            "POST",
            scope_api_contract::routes::RUNNERS,
            Some(bearer_header()),
            &RegisterRunnerRequest {
                owner: TEST_REPO_OWNER.to_string(),
                repo: TEST_REPO_NAME.to_string(),
                name: "rollback-box".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                capabilities: RunnerCapabilities::v1(),
                max_concurrent_jobs: RunnerMaxConcurrentJobs::new(1).unwrap(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(registered.status(), StatusCode::OK);
    let runner_id = response_json(registered).await["runner"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(scope_api_contract::routes::runner(&runner_id))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let missing = app
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::runner(&runner_id))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manual_run_rejects_oversized_workflow_before_parsing() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state);
    let source = temp_git_repo("oversized-run-workflow");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(
        source.join(".scope/runs/oversized.yml"),
        vec![b'x'; scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES + 1],
    )
    .unwrap();
    run_git(Some(&source), &["add", "."], "stage oversized workflow").unwrap();
    commit_all(&source, "oversized workflow");
    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create oversized workflow bundle",
    )
    .unwrap();
    let create_uri = format!(
        "{}?workflow=oversized&git_oid={git_oid}&request_id=22222222222222222222222222222222",
        scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(create_uri)
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(fs::read(bundle_path).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response_json(response).await["message"]
            .as_str()
            .unwrap()
            .contains("workflow definition exceeds")
    );
}

fn json_request<T: serde::Serialize>(
    method: &str,
    uri: &str,
    authorization: Option<String>,
    body: &T,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    request
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

fn machine_request(method: &str, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(body)
        .unwrap()
}
