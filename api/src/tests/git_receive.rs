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
            .map(String::as_str),
        Some("hello")
    );
}

#[test]
fn published_receive_pack_push_stages_from_seeded_git_repo() {
    let state = test_state_with_repo();
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let mut staged = catalog.clone();
        staged
            .repositories
            .insert(TEST_REPO_ID.to_string(), repo_with_readme());
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
    persist_receive_pack_update(&state, TEST_REPO_OWNER, TEST_REPO_NAME, update).unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let staged_update = repo.staged_update.as_ref().unwrap();
    assert_eq!(staged_update.changes.len(), 2);
    assert_eq!(
        live_tree(&repo)
            .get(&ScopePath::parse("/README.md").unwrap())
            .map(String::as_str),
        Some("hello")
    );

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
            .map(String::as_str),
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

    let public_projection = project_graph(&repo.policy, &repo.graph, &Principal::public());
    let public_git = build_virtual_git_projection(&public_projection);
    assert!(
        !public_git
            .blobs
            .iter()
            .any(|blob| blob.path == "/secret-plan.md")
    );
    let owner = Principal {
        id: repo.record.owner_user_id.clone(),
        kind: PrincipalKind::User,
    };
    let owner_projection = project_graph(&repo.policy, &repo.graph, &owner);
    assert!(
        build_virtual_git_projection(&owner_projection)
            .blobs
            .iter()
            .any(|blob| blob.path == "/secret-plan.md")
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
    let public_projection = project_graph(&repo.policy, &repo.graph, &Principal::public());
    let public_git = build_virtual_git_projection(&public_projection);
    assert!(
        !public_git
            .blobs
            .iter()
            .any(|blob| blob.path == "/private/new.txt")
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
                    r#"{"path":"/private/new.txt","visibility":"Public"}"#,
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

    let error = pending_import_from_staging_repo(&bare).unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    let _ = fs::remove_dir_all(&repo);
    let _ = fs::remove_dir_all(&bare);
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
