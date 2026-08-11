use super::*;

const WORKFLOW: &str = r#"
name: Test
on:
  manual: true
runs-on: linux-box
caches: []
container:
  image: alpine:3.20
resources: { cpu: 1, memory: 1gb }
timeout: 5m
jobs:
  checks:
    steps:
      - name: Test
        run: printf 'hello from runner\n'
"#;

#[tokio::test]
async fn workflow_catalog_and_filtered_history_follow_current_main() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut repo = test_repo(&test_owner_id());
    repo.live_files.insert(
        ScopePath::parse("/.scope/runs/test.yml").unwrap(),
        source_blob(&state, WORKFLOW),
    );
    replace_test_repo(&state, repo).await;
    let app = router(state);

    let workflows = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(scope_api_contract::routes::repo_run_workflows(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workflows.status(), StatusCode::OK);
    let workflows = response_json(workflows).await;
    assert_eq!(workflows["workflows"][0]["key"], "test");
    assert_eq!(workflows["workflows"][0]["name"], "Test");
    assert_eq!(workflows["workflows"][0]["job_count"], 1);

    let source = temp_git_repo("run-history-pages");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), WORKFLOW).unwrap();
    run_git(Some(&source), &["add", "."], "stage run history source").unwrap();
    commit_all(&source, "run history source");
    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create run history bundle",
    )
    .unwrap();
    let bundle = fs::read(bundle_path).unwrap();
    for request_id in [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ] {
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "{}?workflow=test&git_oid={git_oid}&request_id={request_id}",
                        scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
                    ))
                    .header(AUTHORIZATION, bearer_header())
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(Body::from(bundle.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
    }

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?workflow=test&limit=1",
                    scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    assert_eq!(first["runs"].as_array().unwrap().len(), 1);
    let first_id = first["runs"][0]["id"].as_str().unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?workflow=test&limit=1&after={cursor}",
                    scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    assert_eq!(second["runs"].as_array().unwrap().len(), 1);
    assert_ne!(second["runs"][0]["id"], first_id);
    assert!(second["next_cursor"].is_null());

    let wrong_filter = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{}?workflow=missing&after={cursor}",
                    scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME)
                ))
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_filter.status(), StatusCode::BAD_REQUEST);
}
