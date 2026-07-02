use super::*;
use crate::object_store::ObjectStore;
use std::{
    process::Command,
    time::{Duration, Instant},
};

#[tokio::test]
async fn receive_pack_capacity_exhaustion_returns_backpressure() {
    let state = state_with_budget_config(RuntimeBudgetConfig {
        receive_pack_concurrency: 0,
        ..Default::default()
    });
    let secret = "scope_git_budget_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/owner/repo/info/refs?service=git-receive-pack")
                .header(
                    AUTHORIZATION,
                    format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
                )
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
                .uri("/git/owner/repo/git-upload-pack")
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

#[test]
fn object_store_capacity_exhaustion_returns_backpressure() {
    let raw = Arc::new(MemoryObjectStore::new());
    let store = BudgetedObjectStore::new(
        raw,
        Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            object_store_concurrency: 0,
            ..Default::default()
        })),
    );

    let error = store.get("tests/budget/backpressure").unwrap_err();

    assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        error.message,
        "object store read capacity is exhausted; retry later"
    );
}

#[test]
fn object_store_readiness_bypasses_operation_capacity() {
    let store = BudgetedObjectStore::new(
        Arc::new(MemoryObjectStore::new()),
        Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            object_store_concurrency: 0,
            ..Default::default()
        })),
    );

    store.readiness_check().unwrap();
}

#[test]
fn object_store_write_size_limit_returns_payload_too_large() {
    let store = BudgetedObjectStore::new(
        Arc::new(MemoryObjectStore::new()),
        Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            object_store_max_bytes: 4,
            ..Default::default()
        })),
    );

    let error = store
        .put("tests/budget/write-too-large", b"12345")
        .unwrap_err();

    assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error.message.contains("exceeds 4 bytes"));
}

#[test]
fn object_store_read_size_limit_returns_payload_too_large() {
    let key = "tests/budget/read-too-large";
    let raw = Arc::new(MemoryObjectStore::new());
    raw.put(key, b"12345").unwrap();
    let store = BudgetedObjectStore::new(
        raw,
        Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            object_store_max_bytes: 4,
            ..Default::default()
        })),
    );

    let error = store.get(key).unwrap_err();

    assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error.message.contains("exceeds 4 bytes"));
}

#[test]
fn git_command_timeout_returns_service_unavailable() {
    let mut command = Command::new("sh");
    command.arg("-c").arg("sleep 2");

    let error =
        git_command_output_with_timeout(&mut command, None, Duration::from_millis(25)).unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(error.message.contains("timed out"));
}

#[test]
fn git_command_timeout_covers_blocked_stdin_write() {
    let mut command = Command::new("sh");
    command.arg("-c").arg("sleep 5");
    let input = vec![b'x'; 8 * 1024 * 1024];
    let started_at = Instant::now();

    let error =
        git_command_output_with_timeout(&mut command, Some(input), Duration::from_millis(25))
            .unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(error.message.contains("timed out"));
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

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.message, "real git failure");
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
