use super::*;
use crate::domain::requests::{RequestActorRole, RequestAudience, StartRequestInput};

async fn repo_with_secret(state: &AppState, path: &str) {
    let mut repo = repo_with_readme(state);
    repo.policy
        .add_rule(VisibilityRule::private(ScopePath::parse(path).unwrap()))
        .unwrap();
    repo.graph.commits[0].changes.push(FileChange {
        visibility: Visibility::Private,
        path: ScopePath::parse(path).unwrap(),
        old_content: None,
        new_content: Some(source_blob(state, "owner only")),
    });
    replace_test_repo(state, repo).await;
}

fn auth_headers(value: impl AsRef<str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value.as_ref().parse().unwrap());
    headers
}

async fn cli_basic_headers(state: &AppState) -> HeaderMap {
    let grant = state
        .metadata
        .create_cli_exchange_grant(&test_user(
            test_owner_id(),
            TEST_REPO_OWNER,
            TEST_OWNER_EMAIL,
        ))
        .await
        .unwrap();
    let token = state
        .metadata
        .exchange_cli_grant(&grant.exchange_token)
        .await
        .unwrap()
        .session_token;
    auth_headers(format!("Basic {}", BASE64.encode(format!("scope:{token}"))))
}

#[tokio::test]
async fn permissioned_git_projection_accepts_basic_scope_cli_session_for_repo_owner() {
    let state = test_state_with_repo();
    repo_with_secret(&state, "/secret.txt").await;
    let headers = cli_basic_headers(&state).await;

    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ProjectionViewKey::Private);
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(paths.iter().any(|path| path == "/secret.txt"));
}

#[tokio::test]
async fn permissioned_git_projection_serves_public_view_without_target_repo_membership() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let clerk_id = "user_other_owner";
    let projection = git_projection_for_request(
        &state,
        &auth_headers(bearer_header_for(clerk_id, "other@example.com")),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    assert_eq!(projection.view_key, ProjectionViewKey::Public);
    assert!(
        projection
            .visible_paths()
            .iter()
            .any(|path| path == "/README.md")
    );
}

#[tokio::test]
async fn public_git_projection_ignores_credentials_and_omits_private_files() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    repo_with_secret(&state, "/owner-secret.txt").await;
    let projection = git_projection_for_request(
        &state,
        &auth_headers(bearer_header()),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Public,
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ProjectionViewKey::Public);
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(!paths.iter().any(|path| path == "/owner-secret.txt"));
}

#[tokio::test]
async fn permissioned_public_git_read_view_physically_excludes_private_objects() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    repo_with_secret(&state, "/owner-secret.txt").await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let private_oid = repo
        .graph
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .find(|change| change.path.as_str() == "/owner-secret.txt")
        .and_then(|change| change.new_content.as_ref())
        .unwrap()
        .git_oid
        .clone();
    let projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(&state, &projection).unwrap();
    let base_main_oid = git_stdout_text(
        &public_repo,
        &["rev-parse", "refs/heads/main"],
        "read public main",
    )
    .unwrap()
    .trim()
    .to_string();
    let reader_id = crate::db::scope_user_id_for_auth_identity("clerk", "public-reader");
    state
        .metadata
        .insert_user_for_tests(test_user(reader_id.clone(), "reader", "reader@example.com"))
        .await
        .unwrap();
    state
        .metadata
        .start_request(StartRequestInput {
            id: "req_public_read_view".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "public-fix".to_string(),
            author_user_id: reader_id,
            title: None,
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid,
            event_id: "event_req_public_read_view_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    let read_view = git_upload_pack_repo_for_request(
        &state,
        &auth_headers(bearer_header_for("public-reader", "reader@example.com")),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        GitRemoteMode::Permissioned,
    )
    .await
    .unwrap();
    let request_ref = git_stdout_text(
        &read_view,
        &["rev-parse", "refs/heads/public-fix"],
        "read named public request",
    )
    .unwrap();
    assert!(!request_ref.trim().is_empty());
    let private_object = run_git_output(
        Some(&read_view),
        &["cat-file", "-e", &private_oid],
        "probe private object",
    )
    .unwrap();
    assert!(!private_object.status.success());
}
