use super::*;

fn with_test_repo(state: &AppState, configure: impl FnOnce(&mut StoredRepository)) {
    let mut repo = test_repo(&test_owner_id());
    configure(&mut repo);
    let mut catalog = lock_catalog(state).unwrap();
    catalog.repositories.insert(TEST_REPO_ID.into(), repo);
}

fn commit(
    id: &str,
    parent: Option<&str>,
    message: &str,
    changes: Vec<FileChange>,
) -> LogicalCommit {
    LogicalCommit {
        id: id.to_string(),
        parent_ids: parent.into_iter().map(str::to_string).collect(),
        author_id: test_owner_id(),
        author_visibility: AuthorVisibility::Visible,
        message: message.to_string(),
        changes,
    }
}

fn change(
    visibility: Visibility,
    path: &str,
    old_content: Option<crate::domain::store::SourceBlob>,
    new_content: Option<crate::domain::store::SourceBlob>,
) -> FileChange {
    FileChange {
        visibility,
        path: ScopePath::parse(path).unwrap(),
        old_content,
        new_content,
    }
}

async fn get(state: AppState, uri: &str, authorization: Option<&str>) -> Response {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn projection_repo(
    label: &str,
    projection: &crate::domain::projection::Projection,
) -> (TempGitRepo, PathBuf) {
    let cache = TempGitRepo(std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    )));
    let _ = fs::remove_dir_all(cache.as_ref());
    ensure_private_dir(cache.as_ref()).unwrap();
    let repo = projection_bare_repo(&MemoryObjectStore::new(), cache.as_ref(), projection).unwrap();
    (cache, repo)
}

#[tokio::test]
async fn published_default_private_repo_serves_public_file_subset() {
    let state = test_state_with_repo();
    with_test_repo(&state, |repo| {
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "initial",
            vec![
                change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(source_blob("hello")),
                ),
                change(
                    Visibility::Private,
                    "/secret.txt",
                    None,
                    Some(source_blob("secret")),
                ),
            ],
        ));
    });

    let response = get(state, "/v1/repos/owner/repo/files", None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["path"], "/README.md");
}

#[tokio::test]
async fn public_file_content_uses_the_projected_blob() {
    let state = test_state_with_repo();
    with_test_repo(&state, |repo| {
        let public_blob = source_blob("public readme");
        repo.graph.commits.extend([
            commit(
                "rv1",
                None,
                "public version",
                vec![change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(public_blob.clone()),
                )],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "private draft",
                vec![change(
                    Visibility::Private,
                    "/README.md",
                    Some(public_blob),
                    Some(source_blob("private draft")),
                )],
            ),
        ]);
    });

    let response = get(
        state,
        "/v1/repos/owner/repo/files/content?path=README.md",
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["path"], "/README.md");
    assert_eq!(body["content"]["kind"], "text");
    assert_eq!(body["content"]["text"], "public readme");
}

#[tokio::test]
async fn file_content_rejects_empty_path() {
    let response = get(
        test_state_with_repo(),
        "/v1/repos/owner/repo/files/content?path=",
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn published_repo_projection_preview_serves_public_file_subset() {
    let state = test_state_with_repo();
    with_test_repo(&state, |repo| {
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.extend([
            commit(
                "rv1",
                None,
                "initial",
                vec![
                    change(
                        Visibility::Public,
                        "/README.md",
                        None,
                        Some(source_blob("hello")),
                    ),
                    change(
                        Visibility::Private,
                        "/secret.txt",
                        None,
                        Some(source_blob("secret")),
                    ),
                ],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "private notes",
                vec![change(
                    Visibility::Private,
                    "/notes/private.md",
                    None,
                    Some(source_blob("private notes")),
                )],
            ),
        ]);
    });

    cache_test_jwks(&state);
    let public_response = get(
        state.clone(),
        "/v1/repos/owner/repo/projection-preview?audience=public",
        None,
    )
    .await;

    assert_eq!(public_response.status(), StatusCode::OK);
    let public_body = response_json(public_response).await;
    assert_eq!(public_body["audience"], "public");
    assert_eq!(public_body["source"], "live");
    assert_eq!(public_body["summary"]["visible_files"], 1);
    assert_eq!(public_body["summary"]["hidden_files"], 0);
    assert_eq!(public_body["summary"]["hidden_commits"], 0);
    assert_eq!(public_body["files"][0]["path"], "/README.md");

    let owner_response = get(
        state,
        "/v1/repos/owner/repo/projection-preview?audience=public",
        Some(&bearer_header()),
    )
    .await;

    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["summary"]["visible_files"], 1);
    assert_eq!(owner_body["summary"]["hidden_files"], 2);
    assert_eq!(owner_body["summary"]["hidden_commits"], 1);
}

#[tokio::test]
async fn owner_projection_preview_labels_mixed_visibility_commit() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    with_test_repo(&state, |repo| {
        repo.policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/secret.txt").unwrap(),
            ))
            .unwrap();
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "mixed visibility",
            vec![
                change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(source_blob("hello")),
                ),
                change(
                    Visibility::Private,
                    "/secret.txt",
                    None,
                    Some(source_blob("secret")),
                ),
            ],
        ));
    });

    let response = get(
        state,
        "/v1/repos/owner/repo/projection-preview?audience=private",
        Some(&bearer_header()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["commits"][0]["visibility"], "Mixed");
}

#[tokio::test]
async fn owner_public_projection_preview_counts_visibility_transition_hidden_commits() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    with_test_repo(&state, |repo| {
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/notes.md").unwrap(),
            ))
            .unwrap();
        let private_blob = source_blob("private draft");
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "private draft",
            vec![change(
                Visibility::Private,
                "/notes.md",
                None,
                Some(private_blob.clone()),
            )],
        ));
        repo.visibility_events.push(VisibilityEvent {
            id: "vis_1".to_string(),
            after_commit_id: Some("rv1".to_string()),
            source_commit_id: None,
            author_id: test_owner_id(),
            path: ScopePath::parse("/notes.md").unwrap(),
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(private_blob),
        });
    });

    let response = get(
        state,
        "/v1/repos/owner/repo/projection-preview?audience=public",
        Some(&bearer_header()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["summary"]["visible_commits"], 1);
    assert_eq!(body["summary"]["hidden_commits"], 1);
    assert_eq!(body["commits"][0]["logical_commit_id"], "vis_1");
    assert_eq!(body["commits"][0]["visibility"], "FullyPublic");
}

#[tokio::test]
async fn published_default_private_repo_without_public_files_stays_hidden() {
    let state = test_state_with_repo();
    with_test_repo(&state, |repo| {
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.graph.commits.push(commit(
            "rv1",
            None,
            "initial",
            vec![change(
                Visibility::Private,
                "/secret.txt",
                None,
                Some(source_blob("secret")),
            )],
        ));
    });

    let response = get(state, "/v1/repos/owner/repo/files", None).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logged_in_non_member_reads_empty_public_repo_as_public() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let other_auth = bearer_header_for("user_other", "other@example.com");

    let repo_response = get(state.clone(), "/v1/repos/owner/repo", Some(&other_auth)).await;

    assert_eq!(repo_response.status(), StatusCode::OK);
    let body = response_json(repo_response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["access"]["actor"], "Public");
    assert_eq!(body["change_version"], 0);

    let files_response = get(state, "/v1/repos/owner/repo/files", Some(&other_auth)).await;

    assert_eq!(files_response.status(), StatusCode::OK);
    assert!(
        response_json(files_response)
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deleted_public_file_keeps_public_history_readable_with_empty_tree() {
    let state = test_state_with_repo();
    with_test_repo(&state, |repo| {
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.policy
            .add_rule(VisibilityRule::public(
                ScopePath::parse("/README.md").unwrap(),
            ))
            .unwrap();
        let readme_blob = source_blob("hello");
        repo.graph.commits.extend([
            commit(
                "rv1",
                None,
                "initial",
                vec![change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(readme_blob.clone()),
                )],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "delete public file",
                vec![change(
                    Visibility::Public,
                    "/README.md",
                    Some(readme_blob),
                    None,
                )],
            ),
        ]);
    });

    let response = get(state, "/v1/repos/owner/repo/files", None).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response_json(response).await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn anonymous_request_uses_public_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let principal = principal_for_scope_user(&repo, None);

    assert_eq!(principal, Principal::public());
}

#[tokio::test]
async fn scope_owner_uses_repo_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let user = UserAccount {
        id: test_owner_id(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
    };
    let principal = principal_for_scope_user(&repo, Some(&user));

    assert_eq!(principal.id, test_owner_id());
    assert_eq!(principal.kind, PrincipalKind::User);
}

#[tokio::test]
async fn non_member_scope_user_uses_public_principal() {
    let state = test_state_with_repo();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let user = UserAccount {
        id: "scope_usr_other".to_string(),
        handle: "other".to_string(),
        email: "other@example.com".to_string(),
        email_verified: true,
    };
    let principal = principal_for_scope_user(&repo, Some(&user));

    assert_eq!(principal, Principal::public());
}

#[tokio::test]
async fn unreadable_repo_is_hidden_from_public_requests() {
    let state = test_state_with_repo();
    let mut repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .clone();
    repo.record.publication_state = RepoPublicationState::Unpublished;

    let error = ensure_repo_read(&state, &repo, &Principal::public()).unwrap_err();

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[test]
fn git_projection_cache_omits_private_files_for_public_clone() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/secret.txt").unwrap(),
        ))
        .unwrap();
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![commit(
            "rv1",
            None,
            "initial",
            vec![
                change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(source_blob("hello")),
                ),
                change(
                    Visibility::Private,
                    "/secret.txt",
                    None,
                    Some(source_blob("nope")),
                ),
            ],
        )],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let (_cache, repo_path) = projection_repo("git-cache-test", &projection);
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", "--name-only", DEFAULT_GIT_BRANCH],
        "list cached projection",
    )
    .unwrap();

    assert!(tree.contains("README.md"));
    assert!(!tree.contains("secret.txt"));
}

#[test]
fn git_projection_cache_preserves_executable_file_mode() {
    let policy = Policy::new(Visibility::Public);
    let mut script = source_blob("#!/bin/sh\necho hi\n");
    script.git_file_mode = EXECUTABLE_GIT_FILE_MODE.to_string();
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![commit(
            "rv1",
            None,
            "initial",
            vec![change(Visibility::Public, "/bin/run", None, Some(script))],
        )],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let (_cache, repo_path) = projection_repo("git-mode-cache-test", &projection);
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", DEFAULT_GIT_BRANCH],
        "list cached projection modes",
    )
    .unwrap();

    assert!(tree.contains("100755 blob"));
    assert!(tree.contains("bin/run"));
}

#[test]
fn public_git_projection_starts_at_private_to_public_transition() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let private_blob = source_blob("private draft");
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![
            commit(
                "rv1",
                None,
                "private draft",
                vec![change(
                    Visibility::Private,
                    "/notes.md",
                    None,
                    Some(private_blob.clone()),
                )],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "public release",
                vec![change(
                    Visibility::Public,
                    "/notes.md",
                    Some(private_blob),
                    Some(source_blob("public release")),
                )],
            ),
        ],
    };
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);
    let (_cache, repo_path) = projection_repo("git-transition-test", &projection);
    let history = git_stdout_text(
        &repo_path,
        &["log", "--all", "-p", "--", "notes.md"],
        "read projected Git history",
    )
    .unwrap();

    assert!(!history.contains("private draft"));
}

#[test]
fn public_git_projection_keeps_history_after_public_to_private_transition() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    let public_blob = source_blob("public readme");
    let graph = SourceGraph {
        repo_id: TEST_REPO_ID.to_string(),
        commits: vec![
            commit(
                "rv1",
                None,
                "public readme",
                vec![change(
                    Visibility::Public,
                    "/README.md",
                    None,
                    Some(public_blob.clone()),
                )],
            ),
            commit(
                "rv2",
                Some("rv1"),
                "private readme",
                vec![change(
                    Visibility::Private,
                    "/README.md",
                    Some(public_blob),
                    Some(source_blob("private readme")),
                )],
            ),
        ],
    };
    let projection = project_graph(
        &policy,
        &graph,
        &[VisibilityEvent {
            id: "vis_1".to_string(),
            after_commit_id: Some("rv1".to_string()),
            source_commit_id: Some("rv2".to_string()),
            author_id: TEST_REPO_OWNER.to_string(),
            path: ScopePath::parse("/README.md").unwrap(),
            old_visibility: Visibility::Public,
            new_visibility: Visibility::Private,
            current_content: Some(source_blob("private readme")),
        }],
        ProjectionViewKey::Public,
    );
    let (_cache, repo_path) = projection_repo("git-public-to-private-test", &projection);
    let history = git_stdout_text(
        &repo_path,
        &["log", "--all", "--format=%B", "--", "README.md"],
        "read projected Git history",
    )
    .unwrap();
    let tree = git_stdout_text(
        &repo_path,
        &["ls-tree", "-r", "--name-only", DEFAULT_GIT_BRANCH],
        "list projected Git tree",
    )
    .unwrap();

    assert!(history.contains("public readme"));
    assert!(!history.contains("private readme"));
    assert!(!tree.contains("README.md"));
}
