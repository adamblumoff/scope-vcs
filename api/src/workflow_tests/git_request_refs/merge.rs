use super::*;
use crate::use_cases::request_merge::{
    PreparedRequestMerge, persist_prepared_merge_for_tests, prepare_request_merge,
};
use std::time::Duration;

const MERGED_WORKFLOW: &str = r#"name: Merged checks
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
        run: printf 'merged workflow\n'
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merge_route_persists_git_content_once() {
    let (state, owner_source) = test_state_with_mergeable_request("request-merge-route").await;
    insert_member_user(&state).await;
    let (source, remote, _server, first_request_head) =
        request_checkout(&state, "request-http-merge").await;
    configure_bearer_header(
        &owner_source,
        &remote,
        &bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
    );
    fs::write(owner_source.join("README.md"), "upstream public change\n").unwrap();
    fs::create_dir_all(owner_source.join(".scope/runs")).unwrap();
    fs::write(owner_source.join(".scope/runs/merged.yml"), MERGED_WORKFLOW).unwrap();
    run_git(
        Some(&owner_source),
        &["add", "."],
        "stage public main advance",
    )
    .unwrap();
    commit_all(&owner_source, "advance public main");
    configure_push_intent_header(&state, &owner_source, &remote, &test_owner_id()).await;
    run_git(
        Some(&owner_source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "advance public main",
    )
    .unwrap();
    let public_remote = remote.replace("/git/permissioned/", "/git/public/");
    run_git(
        Some(&source),
        &[
            "fetch",
            &public_remote,
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "fetch updated public main",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-commit",
            "FETCH_HEAD",
        ],
        "merge updated public main into request",
    )
    .unwrap();
    commit_all(&source, "update request onto public main");
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "README.md",
        "request resolution\n",
        "resolve request against public main",
    )
    .unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request-second.txt",
        "second request commit\n",
        "second request change",
    )
    .unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "README.html",
        "<h1>Merged request landing page</h1>\n",
        "add request landing page",
    )
    .unwrap();
    let request_head = git_head_oid(&source);
    let app = router(state.clone());

    let submitted = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submitted.status(), StatusCode::OK);

    let merge_request = || {
        axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"
            ))
            .header(
                AUTHORIZATION,
                bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
            )
            .body(Body::empty())
            .unwrap()
    };
    let logical_commit_count_before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .graph
        .commits
        .len();
    let merged = app.clone().oneshot(merge_request()).await.unwrap();
    let merged_status = merged.status();
    let merged = response_json(merged).await;
    assert_eq!(merged_status, StatusCode::OK, "{merged}");
    assert_eq!(merged["request"]["state"], "Merged");
    assert_eq!(merged["request"]["merged_head_oid"], request_head);
    assert_ne!(merged["request"]["merged_main_oid"], request_head);
    let merged_main_oid = merged["request"]["merged_main_oid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        merged["request"]["mergeability"]["current_main_oid"],
        merged["request"]["merged_main_oid"]
    );
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("request resolution\n")
    );
    assert_eq!(
        live_file_content(&state, "/request.txt").await.as_deref(),
        Some("request branch content\n")
    );
    assert_eq!(
        live_file_content(&state, "/request-second.txt")
            .await
            .as_deref(),
        Some("second request commit\n")
    );
    let landing = state
        .metadata
        .repositories()
        .repo_live_file_with_landing_content(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            None,
            &ScopePath::parse("/README.html").unwrap(),
        )
        .await
        .unwrap()
        .unwrap()
        .landing_file
        .unwrap();
    assert_eq!(
        landing.content_bytes,
        b"<h1>Merged request landing page</h1>\n"
    );
    let workflows = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(scope_api_contract::routes::repo_run_workflows(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workflows.status(), StatusCode::OK);
    let workflows = response_json(workflows).await;
    assert_eq!(workflows["workflows"][0]["key"], "merged");
    assert_eq!(workflows["workflows"][0]["name"], "Merged checks");
    assert!(
        state
            .metadata
            .runs()
            .push_trigger_evaluation(TEST_REPO_ID, &merged_main_oid)
            .await
            .unwrap()
            .is_none(),
        "request merges must update the catalog without gaining push-trigger behavior"
    );

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let public_projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(
        &state,
        &repo.incarnation(),
        &public_projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
    .unwrap();
    assert_eq!(
        git_stdout_text(
            &public_repo,
            &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
            "read public main after request merge",
        )
        .unwrap()
        .trim(),
        request_head
    );
    run_git(
        Some(&public_repo),
        &[
            "merge-base",
            "--is-ancestor",
            &first_request_head,
            &request_head,
        ],
        "verify first contributor commit remains public ancestry",
    )
    .unwrap();
    run_git(
        Some(&public_repo),
        &["fsck", "--full"],
        "verify materialized public repository",
    )
    .unwrap();
    let public_history = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/history?audience=public"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_history.status(), StatusCode::NOT_IMPLEMENTED);
    let public_preview = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/projection-preview?audience=public"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_preview.status(), StatusCode::NOT_IMPLEMENTED);
    let committed_repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        committed_repo.graph.commits.len(),
        logical_commit_count_before + 1
    );
    match &committed_repo.graph.commits.last().unwrap().origin {
        LogicalCommitOrigin::PublicRequestMerge {
            request_id,
            request_head_oid,
            commits,
            ..
        } => {
            assert_eq!(request_id, REQUEST_ID);
            assert_eq!(request_head_oid, &request_head);
            assert_eq!(
                commits.last().map(|commit| &commit.oid),
                Some(&request_head)
            );
        }
        origin => panic!("expected public request merge origin, got {origin:?}"),
    }
    let replay = app.oneshot(merge_request()).await.unwrap();
    assert_eq!(replay.status(), StatusCode::CONFLICT);
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .graph
            .commits
            .len(),
        logical_commit_count_before + 1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_merge_rejects_path_made_private_after_request_push() {
    let state = test_state_with_mergeable_owner_public_request().await;
    insert_member_user(&state).await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "maintainer-public-policy-merge",
        TEST_CLERK_USER_ID,
        TEST_OWNER_EMAIL,
    )
    .await;
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "request branch content\n",
        "request change",
    )
    .unwrap();
    let app = router(state.clone());

    let submitted = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"
                ))
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let submitted_status = submitted.status();
    let submitted = response_json(submitted).await;
    assert_eq!(submitted_status, StatusCode::OK, "{submitted}");

    let private_path = ScopePath::parse("/request.txt").unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.repo_config.visibility.rules.push(
                scope_domain::repo_config::RepoConfigVisibilityRule {
                    path: private_path.as_str().to_string(),
                    visibility: ConfigVisibility::Private,
                },
            );
            repo.bump_change_version();
        })
        .await
        .unwrap();

    let merged = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let merged_status = merged.status();
    let merged = response_json(merged).await;
    assert_eq!(merged_status, StatusCode::CONFLICT, "{merged}");
    assert_eq!(live_file_content(&state, "/request.txt").await, None);
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.state(),
        RequestState::Open
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_merge_authorization_rejects_non_maintainer_before_content_persistence() {
    let (state, _source) = test_state_with_mergeable_request("locked-non-maintainer-state").await;
    let prepared = prepared_merge(&state, "locked-non-maintainer", &public_user_id()).await;
    let durable_refs = durable_content_refs(&prepared);
    let repo_before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    let error = persist_prepared_merge_for_tests(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        REQUEST_ID,
        &public_user_id(),
        prepared,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.public_message(), "repo maintainer required");
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.state(),
        RequestState::Open
    );
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .change_version,
        repo_before.record.change_version
    );
    assert_rollback_queued_and_fence_released(&state, &durable_refs).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_merge_rejects_stale_request_head_without_persisting_content() {
    let (state, _source) = test_state_with_mergeable_request("stale-request-head-state").await;
    insert_member_user(&state).await;
    let prepared = prepared_merge(&state, "stale-request-head", &member_user_id()).await;
    let durable_refs = durable_content_refs(&prepared);
    let repo_before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.head_oid = "ffffffffffffffffffffffffffffffffffffffff".to_string();
        })
        .await
        .unwrap();

    let error = persist_prepared_merge_for_tests(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        REQUEST_ID,
        &member_user_id(),
        prepared,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.public_message(),
        "request changed since merge was prepared; retry merge"
    );
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .change_version,
        repo_before.record.change_version
    );
    assert_rollback_queued_and_fence_released(&state, &durable_refs).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_merge_rejects_stale_repository_version_without_persisting_content() {
    let (state, _source) =
        test_state_with_mergeable_request("stale-repository-version-state").await;
    insert_member_user(&state).await;
    let prepared = prepared_merge(&state, "stale-repository-version", &member_user_id()).await;
    let durable_refs = durable_content_refs(&prepared);
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    let changed_repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    let error = persist_prepared_merge_for_tests(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        REQUEST_ID,
        &member_user_id(),
        prepared,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.public_message(),
        "repo changed since merge was prepared; retry merge"
    );
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .change_version,
        changed_repo.record.change_version
    );
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.state(),
        RequestState::Open
    );
    assert_rollback_queued_and_fence_released(&state, &durable_refs).await;
}

async fn prepared_merge(
    state: &AppState,
    label: &str,
    actor_user_id: &str,
) -> PreparedRequestMerge {
    let (_source, _remote, _server, _) = request_checkout(state, label).await;
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_can_submit: false,
            event_id: format!("event_submit_{label}"),
            now_unix: 5,
        })
        .await
        .unwrap();
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let request = stored_request(state, REQUEST_ID).await;
    prepare_request_merge(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        actor_user_id,
        &repo,
        &request,
    )
    .await
    .unwrap()
}

fn durable_content_refs(
    prepared: &PreparedRequestMerge,
) -> Vec<scope_domain::content_ref::ContentRef> {
    prepared
        .durable_objects()
        .iter()
        .map(|object| object.content_ref.clone())
        .collect()
}

async fn assert_rollback_queued_and_fence_released(
    state: &AppState,
    durable_refs: &[scope_domain::content_ref::ContentRef],
) {
    assert!(!durable_refs.is_empty());
    let queued = state
        .metadata
        .cleanup()
        .pending_source_blob_cleanups_for_tests()
        .await
        .unwrap()
        .into_iter()
        .map(|blob| blob.content_ref)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        durable_refs
            .iter()
            .all(|content_ref| queued.contains(content_ref))
    );

    let fence = tokio::time::timeout(
        Duration::from_secs(2),
        state.metadata.acquire_content_ref_fence(durable_refs),
    )
    .await
    .expect("failed merge must release its content fence")
    .unwrap();
    fence.release().await;
}
