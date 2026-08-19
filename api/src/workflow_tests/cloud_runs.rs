use super::*;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use scope_api_contract::{
    AttemptHeartbeatRequest, AttemptHeartbeatResponse, ClaimRuntimeResponse,
    CompleteAttemptStepRequest, StepConclusionRequest,
};
use scope_cache_contract::SignedCacheGrantClaims;

const WORKFLOW: &str = r#"
name: Cloud protocol
on:
  manual: true
caches: []
container:
  image: alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 5m
jobs:
  checks:
    steps:
      - name: Test
        run: printf 'hello from Scope Cloud\n'
"#;

#[tokio::test]
async fn cloud_runtime_claim_is_one_use_and_completes_the_job() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state.clone());
    let source = temp_git_repo("cloud-runtime-protocol");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), WORKFLOW).unwrap();
    run_git(Some(&source), &["add", "."], "stage cloud run source").unwrap();
    commit_all(&source, "cloud run source");
    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create cloud run bundle",
    )
    .unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "{}?workflow=test&git_oid={git_oid}&request_id=11111111111111111111111111111111",
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

    let offer = state
        .metadata
        .runs()
        .next_dispatchable_job()
        .await
        .unwrap()
        .unwrap();
    let attempt_id = "attempt_cloud_protocol";
    let bootstrap_token = format!("scope_bootstrap_{}", "test-token");
    state
        .metadata
        .runs()
        .dispatch_job(
            &offer.run.id,
            offer.job.key.as_str(),
            attempt_id,
            &machine_token_hash(&bootstrap_token),
            "test-runtime",
            unix_now(),
            unix_now() + 900,
        )
        .await
        .unwrap();

    let claim_request = || {
        Request::builder()
            .method("POST")
            .uri(scope_api_contract::routes::attempt_claim(attempt_id))
            .header(AUTHORIZATION, format!("Bearer {bootstrap_token}"))
            .body(Body::empty())
            .unwrap()
    };
    let claimed = app.clone().oneshot(claim_request()).await.unwrap();
    assert_eq!(claimed.status(), StatusCode::OK);
    let claim: ClaimRuntimeResponse = serde_json::from_value(response_json(claimed).await).unwrap();
    assert!(claim.attempt_token.starts_with("scope_attempt_"));
    assert_eq!(
        cache_grant_claims(&claim.cache_grant).expires_at_unix,
        claim.lease_expires_at_unix
    );
    assert_eq!(
        claim.job.pinned_container_image,
        "alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    let replayed = app.clone().oneshot(claim_request()).await.unwrap();
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

    let attempt_auth = format!("Bearer {}", claim.attempt_token);
    let heartbeat = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(scope_api_contract::routes::attempt_heartbeat(attempt_id))
                .header(AUTHORIZATION, &attempt_auth)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&AttemptHeartbeatRequest {}).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(heartbeat.status(), StatusCode::OK);
    let heartbeat: AttemptHeartbeatResponse =
        serde_json::from_value(response_json(heartbeat).await).unwrap();
    assert_eq!(
        cache_grant_claims(&heartbeat.cache_grant).expires_at_unix,
        heartbeat.status.lease_expires_at_unix
    );

    let started = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(scope_api_contract::routes::attempt_step_start(
                    attempt_id, 0,
                ))
                .header(AUTHORIZATION, &attempt_auth)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);

    let completed_step = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(scope_api_contract::routes::attempt_step_complete(
                    attempt_id, 0,
                ))
                .header(AUTHORIZATION, &attempt_auth)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CompleteAttemptStepRequest {
                        conclusion: StepConclusionRequest::Succeeded,
                        logs_truncated: false,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(completed_step.status(), StatusCode::OK);
    assert_eq!(response_json(completed_step).await["state"], "running");

    let completed_attempt = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(scope_api_contract::routes::attempt_complete(attempt_id))
                .header(AUTHORIZATION, &attempt_auth)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&scope_api_contract::CompleteAttemptRequest {
                        conclusion: scope_api_contract::AttemptConclusionRequest::Succeeded,
                        logs_truncated: false,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(completed_attempt.status(), StatusCode::OK);
    assert_eq!(response_json(completed_attempt).await["state"], "succeeded");
}

fn cache_grant_claims(token: &str) -> SignedCacheGrantClaims {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    decode::<SignedCacheGrantClaims>(
        token,
        &DecodingKey::from_ed_pem(crate::cache_grants::TEST_PUBLIC_KEY.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap()
    .claims
}
