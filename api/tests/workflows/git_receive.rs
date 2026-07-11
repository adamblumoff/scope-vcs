use super::*;

async fn repo_with_push_member(
    state: &AppState,
    member_id: &str,
    permissions: RepositoryMemberPermissions,
) {
    let mut repo = repo_with_readme(state);
    repo.members
        .push(test_repository_member(TEST_REPO_ID, member_id, permissions));
    replace_test_repo(state, repo).await;
}

fn commit_readme(repo: &FsPath, content: &str, message: &str) {
    fs::write(repo.join("README.md"), content).unwrap();
    run_git(Some(repo), &["add", "-A"], "stage readme change").unwrap();
    commit_all(repo, message);
}

#[tokio::test]
async fn published_receive_pack_push_applies_from_seeded_git_repo() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    repo.graph.commits[0].changes.push(FileChange {
        visibility: Visibility::Public,
        path: ScopePath::parse("/unchanged.md").unwrap(),
        old_content: None,
        new_content: Some(source_blob(&state, "already here")),
    });
    replace_test_repo(&state, repo).await;
    let staging_repo = published_staging_repo(&state).await;
    let clone = clone_test_repo(&staging_repo, "published-push-clone", false);
    fs::write(clone.join("README.md"), "staged readme").unwrap();
    fs::write(clone.join("notes.md"), "new notes").unwrap();
    run_git(Some(&clone), &["add", "-A"], "stage clone changes").unwrap();
    commit_all(&clone, "update from git");
    run_git(
        Some(&clone),
        &["push", "origin", DEFAULT_GIT_BRANCH],
        "push applied update",
    )
    .unwrap();

    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();

    assert_eq!(update.branch, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    assert_eq!(update.message, "update from git");
    persist_test_update(&state, update).await.unwrap();
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("staged readme")
    );
    assert_eq!(
        live_file_content(&state, "/notes.md").await.as_deref(),
        Some("new notes")
    );
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn published_receive_pack_rejects_non_fast_forward_push() {
    let state = test_state_with_repo();
    let staging_repo = published_staging_repo(&state).await;
    let clone = clone_test_repo(&staging_repo, "published-force-push-clone", false);

    commit_readme(&clone, "fast forward readme", "fast-forward update");
    run_git(
        Some(&clone),
        &["push", "origin", DEFAULT_GIT_BRANCH],
        "push fast-forward update",
    )
    .unwrap();
    let accepted_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "read accepted head",
    )
    .unwrap();

    run_git(
        Some(&clone),
        &["reset", "--hard", "HEAD~1"],
        "rewind clone before force push",
    )
    .unwrap();
    commit_readme(&clone, "rewritten readme", "rewritten update");
    let output = run_git_output(
        Some(&clone),
        &["push", "--force", "origin", DEFAULT_GIT_BRANCH],
        "force push rewritten update",
    )
    .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Scope rejects non-fast-forward pushes in v0")
    );
    let current_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "read current head",
    )
    .unwrap();
    assert_eq!(current_head, accepted_head);
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn push_only_member_can_apply_content_without_visibility_changes() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    repo_with_push_member(&state, member_id, member_permissions(true, false, false)).await;

    let persisted = persist_and_promote_test_update(
        &state,
        receive_pack_update(&state, vec![("/README.md", Some("hello\nextra line"))]),
        member_id,
    )
    .await
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello\nextra line")
    );
}

#[tokio::test]
async fn published_push_rechecks_member_permission_before_persisting() {
    let state = test_state_with_repo();
    let member_id = "user_removed_during_push";
    repo_with_push_member(&state, member_id, member_permissions(true, false, true)).await;
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| {
            repo.members.retain(|member| member.user_id != member_id);
        })
        .await
        .unwrap();

    let error = persist_and_promote_test_update(
        &state,
        receive_pack_update(&state, vec![("/README.md", Some("should not persist"))]),
        member_id,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn published_receive_pack_staging_restores_accepted_git_head_from_bucket_snapshot() {
    let state = test_state_with_repo();
    let source = temp_git_repo("snapshot-first-push");
    fs::write(source.join("README.md"), "hello from git").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial from git");
    let bare = clone_test_repo(&source, "snapshot-first-push-bare", true);
    let expected_head =
        git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "first push head").unwrap();
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;

    let restored = ensure_published_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
    .await
    .unwrap();
    let actual_head = git_stdout_text(
        &restored,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "restored head",
    )
    .unwrap();

    assert_eq!(actual_head, expected_head);
    let _ = fs::remove_dir_all(&restored);
}

#[tokio::test]
async fn applying_push_deletes_replaced_git_snapshot_bundle() {
    let state = test_state_with_repo();
    let old_snapshot = source_blob(&state, "old live git snapshot");
    let old_key = old_snapshot.object_key.clone();
    let update = receive_pack_update(&state, vec![("/README.md", Some("updated"))]);
    let new_key = update.git_snapshot.object_key.clone();
    let mut repo = repo_with_readme(&state);
    repo.git_snapshot = Some(old_snapshot);
    replace_test_repo(&state, repo).await;

    let persisted = persist_and_promote_test_update(&state, update, &test_owner_id())
        .await
        .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let store = &state.test_object_store;
    assert!(!store.contains_key(&old_key));
    assert!(store.contains_key(&new_key));
}

#[test]
fn bearer_token_ignores_removed_trusted_identity_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("x-scope-user-email", TEST_OWNER_EMAIL.parse().unwrap());
    headers.insert("x-scope-user-email-verified", "true".parse().unwrap());

    assert_eq!(bearer_token(&headers).unwrap(), None);
}

#[test]
fn bearer_token_rejects_non_bearer_authorization() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, "Basic abc".parse().unwrap());

    let error = bearer_token(&headers).unwrap_err();

    assert_eq!(error.kind, scope_core::error::ErrorKind::Unauthorized);
}

#[tokio::test]
async fn rollback_cleanup_keeps_blobs_still_referenced_by_catalog() {
    let state = test_state_with_repo();
    let live_blob = source_blob(&state, "hello");
    let unreferenced_blob = source_blob(&state, "rollback-only-content");
    let mut repo = repo_with_readme(&state);
    repo.graph.commits[0].changes[0].new_content = Some(live_blob.clone());
    replace_test_repo(&state, repo).await;

    delete_unreferenced_source_blobs(&state, &[live_blob.clone(), unreferenced_blob.clone()])
        .await
        .unwrap();

    let store = &state.test_object_store;
    assert!(store.contains_key(&live_blob.object_key));
    assert!(!store.contains_key(&unreferenced_blob.object_key));
}

#[tokio::test]
async fn pending_source_blob_cleanup_drops_referenced_entries_after_scan() {
    let state = test_state_with_repo();
    let live_blob = source_blob(&state, "referenced pending content");
    {
        let mut repo = repo_with_readme(&state);
        repo.graph.commits[0].changes[0].new_content = Some(live_blob.clone());
        replace_test_repo(&state, repo).await;
        state
            .metadata
            .queue_pending_source_blob_deletions(vec![live_blob.clone()])
            .await
            .unwrap();
    }

    drain_pending_source_blob_deletions(&state).await.unwrap();

    assert!(state.test_object_store.contains_key(&live_blob.object_key));
    assert!(
        state
            .metadata
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn applied_push_survives_obsolete_snapshot_cleanup_failure() {
    let mut state = test_state_with_repo();
    state.object_store = Arc::new(DeleteFailsObjectStore(state.test_object_store.clone()));
    let old_snapshot = source_blob(&state, "old live git snapshot");
    let mut repo = repo_with_readme(&state);
    repo.git_snapshot = Some(old_snapshot);
    replace_test_repo(&state, repo).await;

    let persisted = persist_and_promote_test_update(
        &state,
        receive_pack_update(
            &state,
            vec![("/README.md", Some("cleanup failure still lands"))],
        ),
        &test_owner_id(),
    )
    .await
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("cleanup failure still lands")
    );
    assert!(
        !state
            .metadata
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}

struct DeleteFailsObjectStore(Arc<MemoryObjectStore>);

impl crate::object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        crate::object_store::ObjectStore::put(self.0.as_ref(), key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        crate::object_store::ObjectStore::get(self.0.as_ref(), key)
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "delete failed",
        ))
    }
}
