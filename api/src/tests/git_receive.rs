use super::*;

#[test]
fn receive_pack_stages_owner_update_without_changing_live_tree() {
    let mut repo = repo_with_readme();
    let staged = stage_receive_pack_update(
        &mut repo,
        receive_pack_update(vec![("/README.md", Some("staged readme"))]),
    )
    .unwrap()
    .unwrap();

    assert_eq!(staged.branch, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    assert!(repo.staged_update.is_some());
    assert_eq!(
        live_tree(&repo)
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );
}

#[test]
fn receive_pack_same_content_with_new_object_key_is_noop() {
    let mut repo = repo_with_readme();
    let readme = ScopePath::parse("/README.md").unwrap();
    let live = live_tree(&repo);
    let live_blob = live.get(&readme).unwrap();
    let update = receive_pack_update(vec![("/README.md", Some("hello"))]);
    let update_blob = update.changes[0].content.as_ref().unwrap();
    assert_ne!(live_blob.object_key, update_blob.object_key);

    let error = stage_receive_pack_update(&mut repo, update).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("did not change"));
}

#[test]
fn published_receive_pack_push_stages_from_seeded_git_repo() {
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
        "push staged update",
    )
    .unwrap();

    let update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        &test_owner_id(),
    )
    .unwrap();

    assert_eq!(update.branch, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    assert_eq!(update.message, "update from git");
    assert_eq!(update.changes.len(), 2);
    assert_eq!(update.uploaded_blobs.len(), 2);
    persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update).unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let staged_update = repo.staged_update.as_ref().unwrap();
    assert_eq!(staged_update.changes.len(), 2);
    assert_eq!(
        live_tree(&repo)
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("hello")
    );

    let _ = fs::remove_dir_all(&clone);
    let _ = fs::remove_dir_all(&staging_repo);
}

#[test]
fn published_receive_pack_rejects_non_fast_forward_push() {
    let state = test_state_with_repo();
    let staging_repo = ensure_published_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
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

#[test]
fn review_off_receive_pack_applies_immediately() {
    let mut repo = repo_with_readme();
    repo.settings.review_pushes_before_applying = false;

    let staged = stage_receive_pack_update(
        &mut repo,
        receive_pack_update(vec![("/README.md", Some("live now"))]),
    )
    .unwrap();

    assert!(staged.is_none());
    assert!(repo.staged_update.is_none());
    assert_eq!(
        live_tree(&repo)
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(blob_content)
            .as_deref(),
        Some("live now")
    );
}

#[test]
fn staged_new_private_file_stays_out_of_public_projection() {
    let mut repo = repo_with_readme();
    let mut staged = stage_receive_pack_update(
        &mut repo,
        receive_pack_update(vec![("/secret-plan.md", Some("private"))]),
    )
    .unwrap()
    .unwrap();
    staged.changes[0].visibility = Visibility::Private;

    apply_receive_pack_update(&mut repo, staged).unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &Principal::public(),
    );
    assert!(
        !public_projection
            .visible_paths()
            .contains(&"/secret-plan.md".to_string())
    );
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };
    let owner_projection =
        project_graph(&repo.policy, &repo.graph, &repo.visibility_events, &owner);
    assert!(
        owner_projection
            .visible_paths()
            .contains(&"/secret-plan.md".to_string())
    );
}

#[test]
fn staged_new_file_inherits_private_parent_visibility() {
    let mut repo = repo_with_readme();
    repo.policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/private").unwrap(),
            [repo.record.owner_user_id.clone()],
        ))
        .unwrap();

    let staged = stage_receive_pack_update(
        &mut repo,
        receive_pack_update(vec![("/private/new.txt", Some("private child"))]),
    )
    .unwrap()
    .unwrap();

    assert_eq!(staged.changes[0].visibility, Visibility::Private);
    apply_receive_pack_update(&mut repo, staged).unwrap();
    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &Principal::public(),
    );
    assert!(
        !public_projection
            .visible_paths()
            .contains(&"/private/new.txt".to_string())
    );
}
#[tokio::test]
async fn staged_visibility_route_rejects_public_child_under_private_parent() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/private").unwrap(),
                [repo.record.owner_user_id.clone()],
            ))
            .unwrap();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![("/private/new.txt", Some("private child"))]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/staged-update/files/visibility")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/private/new.txt"],"visibility":"Public"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let staged_update = repo.staged_update.as_ref().unwrap();
    assert_eq!(staged_update.changes[0].visibility, Visibility::Private);
}

#[tokio::test]
async fn staged_visibility_route_batches_multiple_paths() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        stage_receive_pack_update(
            &mut repo,
            receive_pack_update(vec![
                ("/README.md", Some("updated readme")),
                ("/notes.md", Some("new notes")),
            ]),
        )
        .unwrap();

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let app = router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/repos/owner/repo/staged-update/files/visibility")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"paths":["/README.md","/notes.md"],"visibility":"Private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let staged_update = repo.staged_update.as_ref().unwrap();
    assert!(
        staged_update
            .changes
            .iter()
            .all(|change| change.visibility == Visibility::Private)
    );
}

#[test]
fn applying_staged_public_to_private_update_removes_file_from_public_projection() {
    let mut repo = repo_with_readme();
    let mut staged = stage_receive_pack_update(
        &mut repo,
        receive_pack_update(vec![("/README.md", Some("private now"))]),
    )
    .unwrap()
    .unwrap();
    staged.changes[0].visibility = Visibility::Private;

    apply_receive_pack_update(&mut repo, staged).unwrap();

    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &Principal::public(),
    );
    assert!(projection.commits.is_empty());
    assert!(
        !projection
            .visible_paths()
            .contains(&"/README.md".to_string())
    );
}

#[test]
fn applying_staged_public_delete_marked_private_removes_file_from_public_projection() {
    let mut repo = repo_with_readme();
    let mut staged =
        stage_receive_pack_update(&mut repo, receive_pack_update(vec![("/README.md", None)]))
            .unwrap()
            .unwrap();
    staged.changes[0].visibility = Visibility::Private;

    apply_receive_pack_update(&mut repo, staged).unwrap();

    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &Principal::public(),
    );
    let last_commit = projection.commits.last().unwrap();
    assert!(
        last_commit
            .changes
            .iter()
            .any(|change| { change.path.as_str() == "/README.md" && change.new_content.is_none() })
    );
}

#[test]
fn applying_staged_private_delete_marked_public_stays_out_of_public_projection() {
    let mut repo = repo_with_readme();
    repo.graph.commits[0].changes[0].visibility = Visibility::Private;
    let owner_ids = repo_owner_ids(&repo);
    repo.policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
            owner_ids,
        ))
        .unwrap();
    let mut staged =
        stage_receive_pack_update(&mut repo, receive_pack_update(vec![("/README.md", None)]))
            .unwrap()
            .unwrap();
    staged.changes[0].visibility = Visibility::Public;

    apply_receive_pack_update(&mut repo, staged).unwrap();

    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        &Principal::public(),
    );
    assert!(
        projection
            .commits
            .iter()
            .flat_map(|commit| commit.changes.iter())
            .all(|change| change.path.as_str() != "/README.md")
    );
}

#[test]
fn receive_pack_rejects_non_default_branches_and_tags() {
    let mut repo = repo_with_readme();
    let mut feature = receive_pack_update(vec![("/README.md", Some("feature"))]);
    feature.branch = "refs/heads/feature".to_string();

    let error = stage_receive_pack_update(&mut repo, feature).unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut tag = receive_pack_update(vec![("/README.md", Some("tag"))]);
    tag.branch = "refs/tags/v1".to_string();

    let error = stage_receive_pack_update(&mut repo, tag).unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}

#[test]
fn pending_import_rejects_non_default_first_push_branch() {
    let repo = temp_git_repo("non-main-first-push");
    fs::write(repo.join("README.md"), "hello").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "add readme").unwrap();
    commit_all(&repo, "initial");
    run_git(
        Some(&repo),
        &["branch", "-m", DEFAULT_GIT_BRANCH, "master"],
        "rename first-push branch",
    )
    .unwrap();
    let bare = std::env::temp_dir().join(format!(
        "scope-vcs-non-main-first-push-bare-{}-{}",
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
        "clone first push bare repo",
    )
    .unwrap();

    let state = test_state_with_repo();
    let error = pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare)
        .unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&bare);
}

#[test]
fn published_receive_pack_staging_restores_accepted_git_head_from_bucket_snapshot() {
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
    let pending =
        pending_import_from_staging_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME, &bare).unwrap();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut staged = catalog.clone();
        let repo = staged.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.pending_import = Some(pending);
        preview_publish_import(repo).unwrap();
        *catalog = staged;
    }

    let restored = ensure_published_receive_pack_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
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

#[test]
fn applying_push_deletes_replaced_git_snapshot_bundle() {
    let state = test_state_with_repo();
    let old_snapshot = source_blob("old live git snapshot");
    let old_key = old_snapshot.object_key.clone();
    let update = receive_pack_update(vec![("/README.md", Some("updated"))]);
    let new_key = update.git_snapshot.object_key.clone();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.settings.review_pushes_before_applying = false;
        repo.git_snapshot = Some(old_snapshot);
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let persisted =
        persist_receive_pack_update_and_promote(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update)
            .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let store = MemoryObjectStore::new();
    assert!(!store.contains_key(&old_key));
    assert!(store.contains_key(&new_key));
}

#[test]
fn git_refs_ignore_remote_tracking_refs() {
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

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
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

#[test]
fn pending_import_marks_token_used_after_durable_state_update() {
    let state = test_state_with_repo();
    let secret = "scope_fp_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(FirstPushToken {
            token_hash: first_push_token_hash(secret),
            secret: Some(secret.to_string()),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
            expires_at_unix: unix_now() + FIRST_PUSH_TOKEN_TTL_SECS,
            used_at_unix: None,
        });
        repo.pending_import = None;
    }

    let import = PendingImport {
        default_branch: "main".to_string(),
        head_oid: "1111111111111111111111111111111111111111".to_string(),
        tree_oid: "2222222222222222222222222222222222222222".to_string(),
        imported_at_unix: unix_now(),
        git_snapshot: source_blob("manual pending git snapshot"),
        files: Vec::new(),
    };

    persist_pending_import(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &InitialPushCredential::FirstPushToken {
            secret: secret.to_string(),
        },
        import,
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::PendingPublish
    );
    assert_eq!(repo.pending_import.as_ref().unwrap().default_branch, "main");
    assert!(repo.first_push_token.unwrap().used_at_unix.is_some());

    let error = authorize_first_push(&state, TEST_REPO_OWNER, TEST_REPO_NAME, secret).unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
}

#[test]
fn pending_import_with_git_token_marks_first_push_token_used() {
    let state = test_state_with_repo();
    let first_secret = "scope_fp_test";
    let git_secret = "scope_git_test";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::PendingFirstPush;
        repo.first_push_token = Some(FirstPushToken {
            token_hash: first_push_token_hash(first_secret),
            secret: Some(first_secret.to_string()),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
            expires_at_unix: unix_now() + FIRST_PUSH_TOKEN_TTL_SECS,
            used_at_unix: None,
        });
        repo.git_push_token = Some(GitPushToken {
            token_hash: git_push_token_hash(git_secret),
            owner_user_id: repo.record.owner_user_id.clone(),
            created_at_unix: unix_now(),
        });
        repo.pending_import = None;
    }

    persist_pending_import(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &InitialPushCredential::GitPushToken {
            secret: git_secret.to_string(),
        },
        pending_import_fixture(Vec::new()),
    )
    .unwrap();

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::PendingPublish
    );
    assert!(repo.first_push_token.unwrap().used_at_unix.is_some());
}

#[test]
fn rollback_cleanup_keeps_blobs_still_referenced_by_catalog() {
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
        .unwrap();

    let store = MemoryObjectStore::new();
    assert!(store.contains_key(&live_blob.object_key));
    assert!(!store.contains_key(&unreferenced_blob.object_key));
}

#[test]
fn pending_source_blob_cleanup_drops_referenced_entries_after_scan() {
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

    drain_pending_source_blob_deletions(&state).unwrap();

    assert!(MemoryObjectStore::new().contains_key(&live_blob.object_key));
    assert!(
        lock_catalog(&state)
            .unwrap()
            .pending_source_blob_deletions
            .is_empty()
    );
}

#[test]
fn applied_push_survives_obsolete_snapshot_cleanup_failure() {
    let mut state = test_state_with_repo();
    state.object_store = Arc::new(DeleteFailsObjectStore);
    let old_snapshot = source_blob("old live git snapshot");
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut repo = repo_with_readme();
        repo.settings.review_pushes_before_applying = false;
        repo.git_snapshot = Some(old_snapshot);
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let persisted = persist_receive_pack_update_and_promote(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        receive_pack_update(vec![("/README.md", Some("cleanup failure still lands"))]),
    )
    .unwrap();

    assert_eq!(persisted, PersistedReceivePackUpdate::Applied);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let live = live_tree(&repo);
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
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), crate::error::ApiError> {
        let store = MemoryObjectStore::new();
        crate::object_store::ObjectStore::put(&store, key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, crate::error::ApiError> {
        let store = MemoryObjectStore::new();
        crate::object_store::ObjectStore::get(&store, key)
    }

    fn delete(&self, _key: &str) -> Result<(), crate::error::ApiError> {
        Err(crate::error::ApiError::service_unavailable("delete failed"))
    }
}
