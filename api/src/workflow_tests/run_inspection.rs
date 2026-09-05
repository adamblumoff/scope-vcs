use super::*;
use scope_domain::runs::{
    log::RunLogChunk,
    run::Run,
    source::{RunSource, RunTrigger},
    step::StepConclusion,
};

const INSPECTION_WORKFLOW: &str = r#"
name: Inspection
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
        run: printf 'inspect run\n'
"#;

struct InspectableRun {
    state: AppState,
    run_id: String,
    attempt_id: String,
    first_log_position: u64,
    second_log_position: u64,
}

async fn inspectable_run(logs_truncated: bool) -> InspectableRun {
    inspectable_run_with_long_logs(logs_truncated, false).await
}

async fn inspectable_run_with_long_logs(logs_truncated: bool, long: bool) -> InspectableRun {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/inspection.yml",
        INSPECTION_WORKFLOW.as_bytes(),
    )
    .unwrap()
    .into_revision(TEST_REPO_ID.to_string())
    .unwrap();
    let run_id = format!(
        "run_inspection_{}",
        if logs_truncated { "cut" } else { "full" }
    );
    let mut source = scope_object_store::content_object_for_bytes(
        ContentObjectKind::GitBundle,
        b"inspection bundle",
    );
    source.git_oid = "b".repeat(40);
    let run = Run::new(
        run_id.clone(),
        format!("manual:{run_id}"),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(test_owner_id()),
        RunSource::ephemeral_git_bundle(source).unwrap(),
        1,
    )
    .unwrap();
    state
        .metadata
        .runs()
        .enqueue_run(run, revision)
        .await
        .unwrap();

    let attempt_id = format!("attempt_{run_id}");
    let bootstrap_token_hash = "c".repeat(64);
    let attempt_token_hash = "d".repeat(64);
    state
        .metadata
        .runs()
        .dispatch_job(
            &run_id,
            "checks",
            &attempt_id,
            &bootstrap_token_hash,
            "test-runtime",
            2,
            100,
        )
        .await
        .unwrap();
    state
        .metadata
        .runs()
        .claim_runtime(
            &attempt_id,
            &bootstrap_token_hash,
            &attempt_token_hash,
            3,
            100,
        )
        .await
        .unwrap();
    state
        .metadata
        .runs()
        .start_attempt_step(&attempt_id, &attempt_token_hash, 0, 4)
        .await
        .unwrap();
    let first = state
        .metadata
        .runs()
        .append_attempt_log(
            RunLogChunk::new(&attempt_id, 0, 1, "first\n", 5).unwrap(),
            &attempt_token_hash,
            5,
        )
        .await
        .unwrap();
    let second = state
        .metadata
        .runs()
        .append_attempt_log(
            RunLogChunk::new(&attempt_id, 0, 2, "second\n", 6).unwrap(),
            &attempt_token_hash,
            6,
        )
        .await
        .unwrap();
    if long {
        for sequence in 3..=18 {
            state
                .metadata
                .runs()
                .append_attempt_log(
                    RunLogChunk::new(&attempt_id, 0, sequence, "x".repeat(64 * 1024), 6).unwrap(),
                    &attempt_token_hash,
                    6,
                )
                .await
                .unwrap();
        }
        let retry = state
            .metadata
            .runs()
            .append_attempt_log(
                RunLogChunk::new(&attempt_id, 0, 1, "first\n", 6).unwrap(),
                &attempt_token_hash,
                6,
            )
            .await
            .unwrap();
        assert!(!retry.appended);
        assert_eq!(retry.log.position, first.log.position);
        for (sequence, text) in [(1, "different"), (20, "gap")] {
            let error = state
                .metadata
                .runs()
                .append_attempt_log(
                    RunLogChunk::new(&attempt_id, 0, sequence, text, 6).unwrap(),
                    &attempt_token_hash,
                    6,
                )
                .await
                .unwrap_err();
            assert_eq!(
                error.kind,
                scope_postgres::error::PostgresErrorKind::Conflict
            );
        }
    }
    state
        .metadata
        .runs()
        .complete_attempt_step(
            &attempt_id,
            &attempt_token_hash,
            0,
            StepConclusion::Succeeded,
            logs_truncated,
            7,
        )
        .await
        .unwrap();

    InspectableRun {
        state,
        run_id,
        attempt_id,
        first_log_position: first.log.position,
        second_log_position: second.log.position,
    }
}

async fn get_run(state: AppState, owner: &str, repo: &str, run_id: &str, auth: String) -> Response {
    router(state)
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run(owner, repo, run_id))
                .header(AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn run_inspection_allows_the_repository_owner() {
    let fixture = inspectable_run(false).await;
    let response = get_run(
        fixture.state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        bearer_header(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn run_inspection_allows_a_repository_member() {
    let fixture = inspectable_run(false).await;
    let member_subject = "user_run_member";
    let member_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", member_subject);
    fixture
        .state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            member_id.clone(),
            "run-member",
            "run-member@example.com",
        ))
        .await
        .unwrap();
    fixture
        .state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();

    let response = get_run(
        fixture.state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        bearer_header_for(member_subject, "run-member@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn run_inspection_denies_a_public_actor() {
    let fixture = inspectable_run(false).await;
    let public_subject = "user_run_public";
    let public_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", public_subject);
    fixture
        .state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_id, "run-public", "run-public@example.com"))
        .await
        .unwrap();

    let response = get_run(
        fixture.state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        bearer_header_for(public_subject, "run-public@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn run_inspection_hides_a_run_from_another_repository() {
    let fixture = inspectable_run(false).await;
    let mut other = test_repo(&test_owner_id());
    other.record.id = "owner/other".to_string();
    other.record.incarnation_id = "repoi_owner_other".to_string();
    other.record.name = "other".to_string();
    other.graph.repo_id = other.record.id.clone();
    fixture
        .state
        .metadata
        .repositories()
        .replace_repository_for_tests(other)
        .await
        .unwrap();

    let response = get_run(
        fixture.state,
        TEST_REPO_OWNER,
        "other",
        &fixture.run_id,
        bearer_header(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_detail_reconstructs_jobs_attempts_and_steps() {
    let fixture = inspectable_run(false).await;
    let response = router(fixture.state)
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_detail(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    &fixture.run_id,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["run"]["id"], fixture.run_id);
    assert_eq!(body["jobs"][0]["job"]["key"], "checks");
    assert_eq!(body["jobs"][0]["job"]["started_at_unix"], 4);
    assert_eq!(body["jobs"][0]["attempts"][0]["id"], fixture.attempt_id);
    assert_eq!(body["jobs"][0]["attempts"][0]["steps"][0]["name"], "Test");
    assert_eq!(
        body["jobs"][0]["attempts"][0]["steps"][0]["command"],
        "printf 'inspect run\\n'"
    );
}

#[tokio::test]
async fn step_log_inspection_applies_the_cursor_and_preserves_truncation() {
    let fixture = inspectable_run(true).await;
    let response = router(fixture.state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?after={}",
                    scope_api_contract::routes::repo_run_step_logs(
                        TEST_REPO_OWNER,
                        TEST_REPO_NAME,
                        &fixture.run_id,
                        &fixture.attempt_id,
                        0,
                    ),
                    fixture.first_log_position
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["logs"].as_array().unwrap().len(), 1);
    assert_eq!(body["logs"][0]["text"], "second\n");
    assert_eq!(body["next_after"], fixture.second_log_position);
    assert_eq!(body["logs_truncated"], true);
}

#[tokio::test]
async fn long_step_logs_open_at_the_tail_and_page_back_without_gaps() {
    let fixture = inspectable_run_with_long_logs(false, true).await;
    let path = scope_api_contract::routes::repo_run_step_logs(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        &fixture.attempt_id,
        0,
    );
    let app = router(fixture.state);
    let mut query = String::new();
    let mut sequences = Vec::new();
    loop {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{path}{query}"))
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let logs = body["logs"].as_array().unwrap();
        assert!(
            logs.iter()
                .map(|log| log["text"].as_str().unwrap().len())
                .sum::<usize>()
                <= 512 * 1024
        );
        if query.is_empty() {
            assert_eq!(logs.first().unwrap()["sequence"], 11);
            assert_eq!(logs.last().unwrap()["sequence"], 18);
            assert_eq!(body["has_more"], false);
        }
        let page_sequences = logs
            .iter()
            .map(|log| log["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert!(page_sequences.windows(2).all(|pair| pair[0] < pair[1]));
        sequences.extend(page_sequences);
        if body["has_earlier"] == false {
            break;
        }
        query = format!(
            "?before={}",
            logs.first().unwrap()["position"].as_u64().unwrap()
        );
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=18).collect::<Vec<_>>());
}

#[tokio::test]
async fn step_log_inspection_rejects_conflicting_cursors() {
    let fixture = inspectable_run(false).await;
    let path = scope_api_contract::routes::repo_run_step_logs(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
        &fixture.attempt_id,
        0,
    );
    let response = router(fixture.state)
        .oneshot(
            Request::builder()
                .uri(format!("{path}?before=9&after=1"))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
