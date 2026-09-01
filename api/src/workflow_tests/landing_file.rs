use super::*;
use scope_domain::landing_file::{RepositoryLandingFile, RepositoryLandingFileMutation};
use scope_object_store::ObjectStore;

#[tokio::test]
async fn readme_html_uses_postgres_when_git_cache_and_pack_objects_are_absent() {
    let state = test_state_with_repo();
    let source = temp_git_repo("landing-file-direct-read");
    let readme = "<!doctype html><h1>Direct from PostgreSQL</h1>\n";
    fs::write(source.join("README.html"), readme).unwrap();
    run_git(
        Some(&source),
        &["add", "README.html"],
        "stage repository landing file",
    )
    .unwrap();
    commit_all(&source, "add repository landing file");
    let bare = clone_test_repo(&source, "landing-file-direct-read-bare", true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;

    let rebuilt = state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            "landing-file-test",
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert_eq!(rebuilt.failed, 0);

    let path = ScopePath::parse("/README.html").unwrap();
    let captured = state
        .metadata
        .repositories()
        .repo_live_file_with_landing_content(TEST_REPO_OWNER, TEST_REPO_NAME, None, &path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        captured.landing_file.unwrap().content_bytes,
        readme.as_bytes()
    );
    state
        .metadata
        .repositories()
        .delete_repository_landing_file_for_tests(TEST_REPO_ID)
        .await
        .unwrap();
    assert_eq!(state.backfill_repository_landing_files().await.unwrap(), 1);
    assert_eq!(state.backfill_repository_landing_files().await.unwrap(), 0);

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

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files/content?path=README.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["path"], "/README.html");
    assert_eq!(body["content"]["kind"], "text");
    assert_eq!(body["content"]["text"], readme);
}

#[tokio::test]
async fn missing_landing_snapshot_does_not_fall_back_to_git() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    let readme = source_blob(&state, "<h1>legacy row without snapshot</h1>");
    let path = ScopePath::parse("/README.html").unwrap();
    repo.graph.commits[0].changes.push(FileChange {
        visibility: Visibility::Public,
        path: path.clone(),
        old_content: None,
        new_content: Some(readme.clone()),
    });
    repo.live_files.insert(path, readme);
    replace_test_repo(&state, repo).await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/files/content?path=README.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn landing_snapshot_failure_rolls_back_the_repository_transaction() {
    let state = test_state_with_readme().await;
    let before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let jobs_before = state
        .metadata
        .jobs()
        .outbox_job_counts_for_tests()
        .await
        .unwrap();
    let mut update = receive_pack_update(&state, vec![("/notes.md", Some("must roll back"))]);
    update.landing_file_mutation = RepositoryLandingFileMutation::Upsert(RepositoryLandingFile {
        oid: "invalid-landing-oid".to_string(),
        sha256: "0".repeat(64),
        size_bytes: 3,
        git_file_mode: "100644".to_string(),
        content_bytes: b"bad".to_vec(),
    });

    let error = persist_test_update(&state, update).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let after = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(after.record.change_version, before.record.change_version);
    assert_eq!(after.git_head, before.git_head);
    assert_eq!(after.git_pack_spans, before.git_pack_spans);
    assert!(
        !after
            .live_tree()
            .contains_key(&ScopePath::parse("/notes.md").unwrap())
    );
    assert_eq!(
        state
            .metadata
            .jobs()
            .outbox_job_counts_for_tests()
            .await
            .unwrap(),
        jobs_before
    );
}
