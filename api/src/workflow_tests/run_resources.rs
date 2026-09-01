use super::*;
use scope_object_store::ObjectStore;

const WORKFLOW: &str = r#"
name: Test
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
        run: printf 'hello from runner\n'
"#;

fn workflow_named(name: &str) -> String {
    WORKFLOW.replacen("name: Test", &format!("name: {name}"), 1)
}

async fn state_with_pushed_workflow(label: &str) -> AppState {
    state_with_pushed_workflow_source(label, WORKFLOW).await
}

async fn state_with_pushed_workflow_source(label: &str, workflow: &str) -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), workflow).unwrap();
    run_git(Some(&source), &["add", "."], "stage workflow source").unwrap();
    commit_all(&source, "add workflow");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;
    state
}

async fn workflow_list_response(state: AppState) -> Response {
    router(state)
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
        .unwrap()
}

#[tokio::test]
async fn workflow_catalog_and_filtered_history_follow_current_main() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("run-history-pages");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), WORKFLOW).unwrap();
    run_git(Some(&source), &["add", "."], "stage workflow source").unwrap();
    commit_all(&source, "add workflow");
    let bare = clone_test_repo(&source, "run-history-pages-bare", true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let head = repo.git_head.as_ref().unwrap();
    state
        .test_object_store
        .delete(&scope_object_store::object_key(&head.manifest))
        .unwrap();
    for span in &repo.git_pack_spans {
        state
            .git_segment_store
            .delete_remote(&scope_git_storage::object_key(
                TEST_REPO_ID,
                &span.segment.segment_id,
            ))
            .await
            .unwrap();
    }
    let cache_path = state
        .repository_engine
        .repository_path(&test_repo_incarnation());
    if cache_path.exists() {
        fs::remove_dir_all(cache_path).unwrap();
    }
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

    let git_oid = git_head_oid(&source);
    let bundle_path = source.join("source.bundle");
    run_git(
        Some(&source),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create run history bundle",
    )
    .unwrap();
    let bundle = fs::read(bundle_path).unwrap();
    let mut created_run_ids = Vec::new();
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
        created_run_ids.push(
            response_json(created).await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
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
    assert_eq!(first_id, created_run_ids[1]);
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
    assert_eq!(second["runs"][0]["id"], created_run_ids[0]);
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

#[tokio::test]
async fn workflow_catalog_backfill_is_idempotent() {
    let state = state_with_pushed_workflow("workflow-catalog-backfill").await;
    assert_eq!(
        state.validate_repository_workflow_catalogs().await.unwrap(),
        1
    );
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    assert_eq!(
        workflow_list_response(state.clone()).await.status(),
        StatusCode::OK
    );
    state
        .metadata
        .repositories()
        .delete_repository_workflow_catalog_for_tests(TEST_REPO_ID)
        .await
        .unwrap();

    assert_eq!(
        state.backfill_repository_workflow_catalogs().await.unwrap(),
        1
    );
    assert_eq!(
        state.backfill_repository_workflow_catalogs().await.unwrap(),
        0
    );
    assert_eq!(workflow_list_response(state).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn maintenance_rejects_a_previously_captured_workflow_that_the_release_cannot_parse() {
    let legacy_workflow = WORKFLOW.replace(
        "caches: []",
        "caches:\n  - name: cargo\n    path: /scope/cache/cargo",
    );
    let state =
        state_with_pushed_workflow_source("workflow-catalog-release-validation", &legacy_workflow)
            .await;

    let error = state
        .validate_repository_workflow_catalogs()
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains(TEST_REPO_ID));
    assert!(error.contains("missing field `format`"));
}

#[tokio::test]
async fn repository_without_an_accepted_head_has_an_empty_workflow_catalog() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);

    let response = workflow_list_response(state).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["workflows"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn missing_or_inconsistent_workflow_catalog_fails_closed() {
    let missing = state_with_pushed_workflow("workflow-catalog-missing").await;
    missing
        .metadata
        .repositories()
        .delete_repository_workflow_catalog_for_tests(TEST_REPO_ID)
        .await
        .unwrap();
    assert_eq!(
        workflow_list_response(missing).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let wrong_source = state_with_pushed_workflow("workflow-catalog-wrong-source").await;
    wrong_source
        .metadata
        .repositories()
        .corrupt_repository_workflow_catalog_source_for_tests(
            TEST_REPO_ID,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .await
        .unwrap();
    assert_eq!(
        workflow_list_response(wrong_source).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let corrupt_file = state_with_pushed_workflow("workflow-catalog-corrupt-file").await;
    corrupt_file
        .metadata
        .repositories()
        .corrupt_repository_workflow_file_content_for_tests(
            TEST_REPO_ID,
            "/.scope/runs/test.yml",
            b"not the captured workflow".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(
        workflow_list_response(corrupt_file).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn direct_push_replaces_the_complete_workflow_catalog() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("workflow-catalog-replacement");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/old.yml"), workflow_named("Old")).unwrap();
    fs::write(
        source.join(".scope/runs/deleted.yml"),
        workflow_named("Deleted"),
    )
    .unwrap();
    run_git(Some(&source), &["add", "."], "stage initial workflows").unwrap();
    commit_all(&source, "add initial workflows");
    let first = clone_test_repo(&source, "workflow-catalog-replacement-first", true);
    apply_first_push_from_staging_repo(&state, &first, repo_config(Visibility::Public)).await;

    fs::rename(
        source.join(".scope/runs/old.yml"),
        source.join(".scope/runs/renamed.yml"),
    )
    .unwrap();
    fs::write(
        source.join(".scope/runs/renamed.yml"),
        workflow_named("Renamed and edited"),
    )
    .unwrap();
    fs::remove_file(source.join(".scope/runs/deleted.yml")).unwrap();
    fs::write(
        source.join(".scope/runs/added.yml"),
        workflow_named("Added"),
    )
    .unwrap();
    run_git(Some(&source), &["add", "-A"], "stage workflow replacement").unwrap();
    commit_all(&source, "replace workflows");
    let second = clone_test_repo(&source, "workflow-catalog-replacement-second", true);
    let current = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &second,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();
    update.base_git_manifest_ref =
        Some(Some(current.git_head.unwrap().manifest.content_ref.clone()));
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    persist_test_update(&state, update).await.unwrap();

    let response = workflow_list_response(state).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let workflows = body["workflows"].as_array().unwrap();
    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0]["key"], "added");
    assert_eq!(workflows[0]["name"], "Added");
    assert_eq!(workflows[1]["key"], "renamed");
    assert_eq!(workflows[1]["name"], "Renamed and edited");
}

#[tokio::test]
async fn workflow_catalog_failure_rolls_back_the_push_transaction() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("workflow-catalog-rollback");
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/test.yml"), WORKFLOW).unwrap();
    fs::write(source.join("README.md"), "before\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage rollback base").unwrap();
    commit_all(&source, "add rollback base");
    let first = clone_test_repo(&source, "workflow-catalog-rollback-first", true);
    apply_first_push_from_staging_repo(&state, &first, repo_config(Visibility::Public)).await;
    let before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    fs::write(source.join("README.md"), "after\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage rejected push").unwrap();
    commit_all(&source, "prepare rejected push");
    let second = clone_test_repo(&source, "workflow-catalog-rollback-second", true);
    let mut update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &second,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();
    update.base_git_manifest_ref = Some(Some(
        before
            .git_head
            .as_ref()
            .unwrap()
            .manifest
            .content_ref
            .clone(),
    ));
    update.workflow_catalog = scope_domain::runs::catalog::RepositoryWorkflowCatalog::captured(
        TEST_REPO_ID,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        before.record.change_version + 1,
        Vec::new(),
    )
    .unwrap();

    assert!(persist_test_update(&state, update).await.is_err());
    let after = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(after.record.change_version, before.record.change_version);
    assert_eq!(after.git_head, before.git_head);
    let response = workflow_list_response(state).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["workflows"][0]["key"], "test");
}
