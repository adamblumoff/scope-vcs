use super::*;
use crate::object_store::ObjectStore;
use std::{
    process::Command,
    time::{Duration, Instant},
};

fn budgeted_store(config: RuntimeBudgetConfig) -> (Arc<MemoryObjectStore>, BudgetedObjectStore) {
    let raw = Arc::new(MemoryObjectStore::new());
    let store =
        BudgetedObjectStore::new(raw.clone(), Arc::new(RuntimeBudgets::from_config(config)));
    (raw, store)
}

#[tokio::test]
async fn receive_pack_capacity_exhaustion_returns_backpressure() {
    let state = state_with_budget_config(RuntimeBudgetConfig {
        receive_pack_concurrency: 0,
        ..Default::default()
    });
    let secret = "scope_git_budget_test";
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.git_push_token = Some(GitPushToken {
                token_hash: git_push_token_hash(secret),
                owner_user_id: repo.record.owner_user_id.clone(),
                created_at_unix: unix_now(),
            });
        })
        .await
        .unwrap();

    let push_intent = create_test_push_intent(&state, &test_owner_id(), TEST_PUSH_HEAD_OID).await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/permissioned/owner/repo/info/refs?service=git-receive-pack")
                .header(
                    AUTHORIZATION,
                    format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
                )
                .header("x-scope-push-intent", push_intent)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response_json(response).await;
    assert_eq!(
        body["error"],
        "Git receive-pack capacity is exhausted; retry later"
    );
}

#[tokio::test]
async fn upload_pack_capacity_exhaustion_happens_before_projection_build() {
    let state = state_with_budget_config(RuntimeBudgetConfig {
        upload_pack_concurrency: 0,
        projection_build_concurrency: 0,
        ..Default::default()
    });

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/git/public/owner/repo/git-upload-pack")
                .header(CONTENT_TYPE, "application/x-git-upload-pack-request")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("Git upload-pack capacity is exhausted; retry later"));
    assert!(!body.contains("Git projection build capacity"));
}

#[tokio::test]
async fn object_store_capacity_exhaustion_returns_backpressure() {
    let (_, store) = budgeted_store(RuntimeBudgetConfig {
        object_store_concurrency: 0,
        ..Default::default()
    });

    let error = store.get("tests/budget/backpressure").unwrap_err();

    assert_eq!(error.kind, scope_core::error::ErrorKind::TooManyRequests);
    assert_eq!(
        error.message,
        "object store read capacity is exhausted; retry later"
    );
}

#[tokio::test]
async fn object_store_readiness_bypasses_operation_capacity() {
    let (_, store) = budgeted_store(RuntimeBudgetConfig {
        object_store_concurrency: 0,
        ..Default::default()
    });

    store.readiness_check().unwrap();
}

#[test]
fn object_store_size_limits_cover_writes_and_reads() {
    let key = "tests/budget/read-too-large";
    let (raw, store) = budgeted_store(RuntimeBudgetConfig {
        object_store_max_bytes: 4,
        ..Default::default()
    });
    raw.put(key, b"12345").unwrap();
    for error in [
        store
            .put("tests/budget/write-too-large", b"12345")
            .unwrap_err(),
        store.get(key).unwrap_err(),
    ] {
        assert_eq!(error.kind, scope_core::error::ErrorKind::PayloadTooLarge);
        assert!(error.message.contains("exceeds 4 bytes"));
    }
}

#[test]
fn git_command_timeout_covers_blocked_stdin_write() {
    let mut command = Command::new("sh");
    command.arg("-c").arg("sleep 5");
    let input = vec![b'x'; 8 * 1024 * 1024];
    let started_at = Instant::now();

    let error =
        git_command_output_with_timeout(&mut command, Some(input), Duration::from_millis(250))
            .unwrap_err();

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(error.message().contains("timed out"));
    assert!(started_at.elapsed() < Duration::from_secs(2));
}

#[test]
fn git_command_broken_pipe_preserves_child_failure() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("printf 'real git failure' >&2; exit 42");
    let input = vec![b'x'; 8 * 1024 * 1024];

    let error = git_command_output_with_timeout(&mut command, Some(input), Duration::from_secs(2))
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.message(), "real git failure");
}

#[test]
fn git_command_drains_stderr_after_diagnostic_cap() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("set -e; dd if=/dev/zero bs=1024 count=20 >&2 2>/dev/null; printf ok");

    let output =
        git_command_output_with_timeout(&mut command, None, Duration::from_secs(2)).unwrap();

    assert_eq!(output, b"ok");
}

fn state_with_budget_config(config: RuntimeBudgetConfig) -> AppState {
    let mut state = test_state_with_repo();
    let budgets = Arc::new(RuntimeBudgets::from_config(config));
    state.runtime_budgets = budgets.clone();
    state.object_store = Arc::new(BudgetedObjectStore::new(
        Arc::new(MemoryObjectStore::new()),
        budgets,
    ));
    state
}
