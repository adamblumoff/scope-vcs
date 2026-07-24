use super::*;
use crate::domain::repo_config::RepoConfigVisibilityRule;

async fn post(state: AppState, uri: &str, authorization: String, body: String) -> Response {
    router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn owner_post(state: AppState, uri: &str, body: String) -> Response {
    post(
        state,
        uri,
        bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
        body,
    )
    .await
}

async fn stored_config(state: &AppState) -> RepoConfig {
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .repo_config
}

fn bare_clone(source: &FsPath, label: &str) -> TempGitRepo {
    let bare = TempGitRepo(std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    )));
    let _ = fs::remove_dir_all(bare.as_ref());
    run_git(
        None,
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        "clone test bare repository",
    )
    .unwrap();
    bare
}

fn push_intent_request_json(head_oid: &str, config: RepoConfig) -> String {
    push_intent_request_json_with_base(
        head_oid,
        repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
        config,
    )
}

fn push_intent_request_json_with_base(
    head_oid: &str,
    base_config_hash: String,
    config: RepoConfig,
) -> String {
    serde_json::json!({
        "head_oid": head_oid,
        "base_config_hash": base_config_hash,
        "config": config,
    })
    .to_string()
}

fn readme_private_config() -> RepoConfig {
    let mut config = repo_config(Visibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: "/README.md".into(),
        visibility: ConfigVisibility::Private,
    });
    config
}

pub(super) async fn published_git_fixture(label: &str) -> (AppState, TempGitRepo, String) {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial");
    let bare = bare_clone(&source, &format!("{label}-bare"));
    let head = git_head_oid(&bare);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;
    (state, source, head)
}

#[tokio::test]
async fn create_push_intent_rejects_stale_local_config_base_hash() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.default_visibility = Visibility::Private;
            repo.policy = Policy::new(Visibility::Private);
            repo.repo_config = repo_config(Visibility::Private);
        })
        .await
        .unwrap();

    let response = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json_with_base(
            TEST_PUSH_HEAD_OID,
            repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
            readme_private_config(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"],
        "repo config changed since review; rerun scope review"
    );

    assert_eq!(
        stored_config(&state).await,
        repo_config(Visibility::Private)
    );
}

#[tokio::test]
async fn create_push_intent_rejects_oversized_config_for_git_header_transport() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut oversized_config = repo_config(Visibility::Public);
    oversized_config.visibility.rules = (0..300)
        .map(|index| RepoConfigVisibilityRule {
            path: format!("/private/path-{index}.txt"),
            visibility: ConfigVisibility::Private,
        })
        .collect::<Vec<_>>();

    let response = owner_post(
        state,
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json(TEST_PUSH_HEAD_OID, oversized_config),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response_json(response).await["error"]
            .as_str()
            .unwrap()
            .contains("repo config exceeds")
    );
}

#[tokio::test]
async fn create_push_intent_applies_config_when_reviewed_head_is_current() {
    let (state, _source, head_oid) = published_git_fixture("config-only-intent").await;
    let config = readme_private_config();
    let response = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json(&head_oid, config.clone()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(stored_config(&state).await, config);
}

#[tokio::test]
async fn create_push_intent_rejects_stale_config_only_review() {
    let (state, _source, head_oid) = published_git_fixture("stale-config-intent").await;
    let old_base_hash = repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap();
    let applied = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json_with_base(
            &head_oid,
            old_base_hash.clone(),
            repo_config(Visibility::Private),
        ),
    )
    .await;
    assert_eq!(applied.status(), StatusCode::OK);

    let stale = owner_post(
        state.clone(),
        "/v1/repos/owner/repo/push-intents",
        push_intent_request_json_with_base(&head_oid, old_base_hash, readme_private_config()),
    )
    .await;

    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stored_config(&state).await,
        repo_config(Visibility::Private)
    );
}

#[tokio::test]
async fn incremental_git_segment_restores_after_cache_loss() {
    let (state, source, _head_oid) = published_git_fixture("segment-restore").await;
    let first_snapshot = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .git_head
        .unwrap();

    fs::write(source.join("README.md"), "incremental content\n").unwrap();
    run_git(
        Some(&source),
        &["add", "README.md"],
        "add incremental update",
    )
    .unwrap();
    commit_all(&source, "incremental update");
    let expected_head = git_head_oid(&source);
    let bare = bare_clone(&source, "segment-restore-update-bare");
    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &bare,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();
    let snapshot = update.git_head.manifest.clone();
    persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .await
    .unwrap();

    let manifest_bytes =
        crate::object_store::source_blob_bytes(state.object_store.as_ref(), &snapshot).unwrap();
    let manifest = scope_core::git_segments::GitSegmentManifest::decode(&manifest_bytes).unwrap();
    assert_eq!(manifest.head_oid, expected_head);
    assert_eq!(manifest.previous, Some(first_snapshot.manifest));
    assert!(
        manifest
            .segment
            .object_key
            .starts_with("objects/git-segments/")
    );

    let restored = TempGitRepo(std::env::temp_dir().join(format!(
        "scope-vcs-segment-restore-{}-{}",
        std::process::id(),
        unix_now()
    )));
    crate::git::storage::restore_git_segments(&state, &snapshot, &restored).unwrap();
    assert_eq!(git_head_oid(&restored), expected_head);
}

#[test]
fn segment_restore_rejects_manifest_chains_over_the_limit() {
    let state = test_state_with_repo();
    let segment = scope_core::object_store::put_repo_object(
        state.object_store.as_ref(),
        TEST_REPO_ID,
        "git-segments",
        b"segment",
    )
    .unwrap();
    let mut snapshot = None;
    let max_chain_depth = scope_core::config::default_git_storage_limits().max_chain_depth();
    for index in 0..=max_chain_depth {
        let head_oid = format!("{index:040x}");
        let manifest = scope_core::git_segments::GitSegmentManifest::new(
            head_oid.clone(),
            snapshot,
            segment.clone(),
        );
        let mut stored_manifest = scope_core::object_store::put_repo_object(
            state.object_store.as_ref(),
            TEST_REPO_ID,
            "git-manifests",
            &manifest.encode().unwrap(),
        )
        .unwrap();
        stored_manifest.git_oid = head_oid;
        snapshot = Some(stored_manifest);
    }
    let restored = TempGitRepo(std::env::temp_dir().join(format!(
        "scope-vcs-segment-depth-{}-{}",
        std::process::id(),
        unix_now()
    )));

    let error =
        crate::git::storage::restore_git_segments(&state, snapshot.as_ref().unwrap(), &restored)
            .unwrap_err();

    assert_eq!(
        error.message(),
        format!(
            "Git segment chain exceeds maximum depth of {}",
            max_chain_depth
        )
    );
}

#[tokio::test]
async fn segment_creation_rejects_chain_at_limit_before_side_effects() {
    use crate::domain::store::{DEFAULT_GIT_FILE_MODE, GitHead, SourceBlob};
    use scope_core::git_segments::GitStorageLimits;

    let mut state = test_state_with_repo();
    let raw_store = Arc::new(MemoryObjectStore::new());
    let budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        git_storage_limits: GitStorageLimits::new(1024, 2).unwrap(),
        ..Default::default()
    }));
    state.object_store = Arc::new(BudgetedObjectStore::new(raw_store.clone(), budgets.clone()));
    state.runtime_budgets = budgets;
    state.test_object_store = raw_store.clone();
    let previous = GitHead {
        head_oid: TEST_PUSH_HEAD_OID.to_string(),
        segment_sequence: 2,
        change_version: 2,
        manifest: SourceBlob {
            object_key: "objects/git-manifests/previous".to_string(),
            sha256: "previous".to_string(),
            git_oid: TEST_PUSH_HEAD_OID.to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
    };
    let missing_repo = PathBuf::from("/scope-test-missing-git-repo");
    let object_count = raw_store.object_count();

    let error =
        match git_segment_manifest_from_repo(&state, TEST_REPO_ID, &missing_repo, Some(&previous))
            .await
        {
            Ok(_) => panic!("over-depth segment creation unexpectedly succeeded"),
            Err(error) => error,
        };

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error.message(),
        "Git segment chain has reached maximum depth of 2; retry after compaction"
    );
    assert_eq!(raw_store.object_count(), object_count);
}

#[tokio::test]
async fn content_push_rejects_stale_reviewed_config() {
    let (state, source, _head_oid) = published_git_fixture("stale-config-content-push").await;

    fs::write(source.join("README.md"), "content from old review\n").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add stale update").unwrap();
    commit_all(&source, "stale content update");
    let stale_bare = bare_clone(&source, "stale-config-content-update-bare");
    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &stale_bare,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();

    let newer_config = readme_private_config();
    state
        .metadata
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            crate::domain::reviewed_updates::apply_reviewed_config_to_repo(
                repo,
                crate::domain::reviewed_updates::ReviewedConfigUpdateInput {
                    author_id: test_owner_id(),
                    config: newer_config.clone(),
                },
            )
            .unwrap();
        })
        .await
        .unwrap();

    let error = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        update,
        &test_owner_id(),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.message(),
        "repo config changed since review; rerun scope push"
    );
    assert_eq!(stored_config(&state).await, newer_config);
}
