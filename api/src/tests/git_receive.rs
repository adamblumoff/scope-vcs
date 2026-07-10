use super::*;

#[tokio::test]
async fn receive_pack_same_content_with_new_object_key_is_noop() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let readme = ScopePath::parse("/README.md").unwrap();
    let live = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .live_tree();
    let live_blob = live.get(&readme).unwrap();
    let update = receive_pack_update(vec![("/README.md", Some("hello"))]);
    let update_blob = update.changes[0].content.as_ref().unwrap();
    assert_ne!(live_blob.object_key, update_blob.object_key);

    let error = persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("did not change"));
}

#[tokio::test]
async fn receive_pack_same_content_with_new_mode_applies_mode_change() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
    }
    let readme = ScopePath::parse("/README.md").unwrap();
    let mut executable_blob = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .live_tree()
        .get(&readme)
        .unwrap()
        .clone();
    executable_blob.git_file_mode = EXECUTABLE_GIT_FILE_MODE.to_string();

    persist_receive_pack_update(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        ReceivePackUpdate {
            branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            head_oid: TEST_PUSH_HEAD_OID.to_string(),
            base_git_snapshot_key: None,
            author_id: test_owner_id(),
            message: "chmod readme".to_string(),
            git_snapshot: source_blob("test chmod git snapshot"),
            uploaded_blobs: vec![executable_blob.clone()],
            previous_config: None,
            base_config_hash: repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
            config: repo_config(Visibility::Public),
            changes: vec![ReceivePackFileChange {
                path: readme.clone(),
                content: Some(executable_blob),
            }],
        },
    )
    .await
    .unwrap();

    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .live_tree()
            .get(&readme)
            .unwrap()
            .git_file_mode,
        EXECUTABLE_GIT_FILE_MODE
    );
}

#[tokio::test]
async fn published_receive_pack_push_applies_from_seeded_git_repo() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut staged = catalog.clone();
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes.push(FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/unchanged.md").unwrap(),
            old_content: None,
            new_content: Some(source_blob("already here")),
        });
        staged.repositories.insert(TEST_REPO_ID.to_string(), repo);
        *catalog = staged;
    }
    let staging_repo = ensure_published_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
    .await
    .unwrap();
    let clone = std::env::temp_dir().join(format!(
        "scope-vcs-published-push-clone-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&clone);
    run_git(
        None,
        &[
            "clone",
            staging_repo.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
        "clone published staging repo",
    )
    .unwrap();
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
    assert_eq!(update.changes.len(), 2);
    assert_eq!(update.uploaded_blobs.len(), 2);
    persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update)
        .await
        .unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("staged readme")
    );
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/notes.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("new notes")
    );

    let _ = fs::remove_dir_all(&clone);
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn receive_pack_apply_rejects_stale_reviewed_base() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_snapshot = Some(source_blob("current git snapshot"));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut update = receive_pack_update(vec![("/README.md", Some("stale review update"))]);
    update.base_git_snapshot_key = Some(Some("stale-snapshot-key".to_string()));

    let error = persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(
        error.message,
        "repo changed since push was reviewed; rerun scope push"
    );
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn receive_pack_apply_rejects_reviewed_empty_base_after_snapshot_appears() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_snapshot = Some(source_blob("first applied git snapshot"));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    let mut update = receive_pack_update(vec![("/README.md", Some("second first push"))]);
    update.base_git_snapshot_key = Some(None);

    let error = persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update)
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(
        error.message,
        "repo changed since push was reviewed; rerun scope push"
    );
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn published_receive_pack_rejects_non_fast_forward_push() {
    let state = test_state_with_repo();
    let staging_repo = ensure_published_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
    .await
    .unwrap();
    let clone = std::env::temp_dir().join(format!(
        "scope-vcs-published-force-push-clone-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&clone);
    run_git(
        None,
        &[
            "clone",
            staging_repo.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
        "clone published staging repo",
    )
    .unwrap();

    fs::write(clone.join("README.md"), "fast forward readme").unwrap();
    run_git(Some(&clone), &["add", "-A"], "stage fast-forward change").unwrap();
    commit_all(&clone, "fast-forward update");
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
    fs::write(clone.join("README.md"), "rewritten readme").unwrap();
    run_git(Some(&clone), &["add", "-A"], "stage rewritten change").unwrap();
    commit_all(&clone, "rewritten update");
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

    let _ = fs::remove_dir_all(&clone);
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn push_only_member_can_apply_content_without_visibility_changes() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, false),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        receive_pack_update(vec![("/README.md", Some("hello\nextra line"))]),
        member_id,
    )
    .await
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello\nextra line")
    );
}

#[tokio::test]
async fn published_push_rechecks_member_permission_before_persisting() {
    let state = test_state_with_repo();
    let member_id = "user_removed_during_push";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            member_id,
            member_permissions(true, false, true),
        ));
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.members.retain(|member| member.user_id != member_id);
    }

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        receive_pack_update(vec![("/README.md", Some("should not persist"))]),
        member_id,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status, StatusCode::FORBIDDEN);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        repo.live_tree()
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn receive_pack_rejects_non_default_branches_and_tags() {
    let state = test_state_with_repo();
    let mut feature = receive_pack_update(vec![("/README.md", Some("feature"))]);
    feature.branch = "refs/heads/feature".to_string();

    let error = persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, feature)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut tag = receive_pack_update(vec![("/README.md", Some("tag"))]);
    tag.branch = "refs/tags/v1".to_string();

    let error = persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, tag)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn published_receive_pack_staging_restores_accepted_git_head_from_bucket_snapshot() {
    let state = test_state_with_repo();
    let source = temp_git_repo("snapshot-first-push");
    fs::write(source.join("README.md"), "hello from git").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial from git");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-snapshot-first-push-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone first push bare repo",
    )
    .unwrap();
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
    let _ = fs::remove_dir_all(&source);
    let _ = fs::remove_dir_all(&bare);
    let _ = fs::remove_dir_all(&restored);
}

#[tokio::test]
async fn applying_push_deletes_replaced_git_snapshot_bundle() {
    let state = test_state_with_repo();
    let old_snapshot = source_blob("old live git snapshot");
    let old_key = old_snapshot.object_key.clone();
    let update = receive_pack_update(vec![("/README.md", Some("updated"))]);
    let new_key = update.git_snapshot.object_key.clone();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_snapshot = Some(old_snapshot);
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .await
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let store = MemoryObjectStore::new();
    assert!(!store.contains_key(&old_key));
    assert!(store.contains_key(&new_key));
}

#[tokio::test]
async fn git_refs_ignore_remote_tracking_refs() {
    let repo = temp_git_repo("pushed-refs");
    fs::write(repo.join("README.md"), "hello").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
    commit_all(&repo, "initial");
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-pushed-refs-bare-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&bare);
    run_git(
        None,
        &[
            "clone",
            "--bare",
            repo.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone pushed refs bare repo",
    )
    .unwrap();
    run_git(
        Some(&bare),
        &["update-ref", "refs/remotes/origin/main", DEFAULT_GIT_BRANCH],
        "create remote tracking ref",
    )
    .unwrap();

    let refs = git_refs(&bare).unwrap();

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&bare);
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

#[test]
fn first_push_token_accepts_bearer_and_basic_password() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer scope_fp_secret".parse().unwrap());
    assert_eq!(
        first_push_token_from_headers(&headers).unwrap(),
        "scope_fp_secret"
    );

    let encoded = BASE64.encode("scope:scope_fp_secret");
    headers.insert(AUTHORIZATION, format!("Basic {encoded}").parse().unwrap());
    assert_eq!(
        first_push_token_from_headers(&headers).unwrap(),
        "scope_fp_secret"
    );
}

#[tokio::test]
async fn rollback_cleanup_keeps_blobs_still_referenced_by_catalog() {
    let state = test_state_with_repo();
    let live_blob = source_blob("hello");
    let unreferenced_blob = source_blob("rollback-only-content");
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes[0].new_content = Some(live_blob.clone());
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    delete_unreferenced_source_blobs(&state, &[live_blob.clone(), unreferenced_blob.clone()])
        .await
        .unwrap();

    let store = MemoryObjectStore::new();
    assert!(store.contains_key(&live_blob.object_key));
    assert!(!store.contains_key(&unreferenced_blob.object_key));
}

#[tokio::test]
async fn pending_source_blob_cleanup_drops_referenced_entries_after_scan() {
    let state = test_state_with_repo();
    let live_blob = source_blob("referenced pending content");
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.graph.commits[0].changes[0].new_content = Some(live_blob.clone());
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
        catalog
            .pending_source_blob_deletions
            .push(live_blob.clone());
    }

    drain_pending_source_blob_deletions(&state).await.unwrap();

    assert!(MemoryObjectStore::new().contains_key(&live_blob.object_key));
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
}

#[tokio::test]
async fn applied_push_survives_obsolete_snapshot_cleanup_failure() {
    let mut state = test_state_with_repo();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let old_snapshot = source_blob("old live git snapshot");
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.git_snapshot = Some(old_snapshot);
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        receive_pack_update(vec![("/README.md", Some("cleanup failure still lands"))]),
        &test_owner_id(),
    )
    .await
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live = repo.live_tree();
    let readme = live.get(&ScopePath::parse("/README.md").unwrap()).unwrap();
    assert_eq!(blob_content(readme), "cleanup failure still lands");
    assert!(
        !lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
}

struct DeleteFailsObjectStore;

impl crate::object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), scope_core::error::ApiError> {
        let store = MemoryObjectStore::new();
        crate::object_store::ObjectStore::put(&store, key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_core::error::ApiError> {
        let store = MemoryObjectStore::new();
        crate::object_store::ObjectStore::get(&store, key)
    }

    fn delete(&self, _key: &str) -> Result<(), scope_core::error::ApiError> {
        Err(scope_core::error::ApiError::service_unavailable(
            "delete failed",
        ))
    }
}
