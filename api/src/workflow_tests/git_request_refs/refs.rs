use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permissioned_clone_fetches_named_public_requests_without_joining() {
    let state = test_state_with_request().await;
    let (_author_checkout, permissioned_remote, _server, _request_head) =
        request_checkout(&state, "published-request-clone-source").await;
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            actor_is_author: false,
            actor_can_submit: false,
            event_id: "event_published_clone_submitted".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
    insert_public_contributor(&state).await;
    let checkout = checkout_dir("named-request-clone");
    clone_with_bearer(
        &permissioned_remote,
        &checkout,
        &bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
        "clone all public request refs",
    );

    let request_head = git_stdout_text(
        &checkout,
        &["rev-parse", "refs/remotes/origin/request-branch"],
        "read fetched request ref",
    )
    .unwrap();
    assert_eq!(
        request_head.trim(),
        stored_request(&state, REQUEST_ID).await.head_oid
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_public_request_remains_fetchable_as_read_only_history() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.submitted_at_unix = Some(3);
            request.closed_at_unix = Some(3);
            request.closed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = 3;
        })
        .await
        .unwrap();
    let (origin, _server) = spawn_test_server(&state).await;
    let checkout = checkout_dir("closed-named-request-clone");
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    clone_with_bearer(
        &permissioned_remote,
        &checkout,
        &bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
        "clone closed public request ref",
    );
    configure_bearer_header(
        &checkout,
        &permissioned_remote,
        &bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
    );

    assert!(
        git_stdout_text(
            &checkout,
            &["rev-parse", "refs/remotes/origin/request-branch"],
            "read closed request ref",
        )
        .is_ok()
    );
    fs::write(checkout.join("closed.txt"), "closed request edit\n").unwrap();
    run_git(Some(&checkout), &["add", "closed.txt"], "stage closed edit").unwrap();
    commit_all(&checkout, "closed request edit");
    let output = run_git_output(
        Some(&checkout),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "reject closed request push",
    )
    .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn public_request_receive_pack_requires_current_repo_read() {
    let state = test_state_with_request().await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.policy = Policy::new(Visibility::Private);
            repo.graph.commits.clear();
        })
        .await
        .unwrap();
    let headers = authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));

    let error = receive_pack_access(&state, &headers, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_request_ref_push_replaces_snapshot_without_touching_main() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, first_request_head) =
        request_checkout(&state, "request-ref-push").await;
    let before_event_count = request_event_count(&state).await;
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content v2\n",
        "request change v2",
    )
    .unwrap();
    let request_head = git_head_oid(&source);

    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
    assert_eq!(live_file_content(&state, "/request.txt").await, None);
    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.head_oid, request_head);
    source_blob_bytes(
        state.object_store.as_ref(),
        request.git_snapshot.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(request_event_count(&state).await, before_event_count + 1);
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, &test_repo_incarnation());
    let stored_head = git_stdout_text(&store_repo, &["rev-parse", REQUEST_REF], "read request ref")
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(stored_head, request_head);
    run_git(
        Some(&store_repo),
        &["update-ref", REQUEST_REF, &first_request_head],
        "simulate stale request ref cache",
    )
    .unwrap();
    let staging_repo = assert_restored_request_head(&state, &request_head).await;
    let _ = fs::remove_dir_all(staging_repo);
    fs::remove_dir_all(&store_repo).unwrap();
    let staging_repo = assert_restored_request_head(&state, &request_head).await;
    let _ = fs::remove_dir_all(staging_repo);
}

#[tokio::test]
async fn request_ref_completion_cannot_cross_repository_recreation() {
    let state = test_state_with_request().await;
    let predecessor_request = stored_request(&state, REQUEST_ID).await;
    let preparation = git_receive_use_case::prepare(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        ReceivePackAccess::RequestContributor {
            author_id: public_user_id(),
            incarnation: test_repo_incarnation(),
        },
        false,
    )
    .await
    .unwrap();
    let staging_repo = preparation.staging_repo.clone();
    let predecessor_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", REQUEST_REF],
        "read predecessor request ref",
    )
    .unwrap()
    .trim()
    .to_string();
    let treeish = format!("{predecessor_head}^{{tree}}");
    let tree = git_stdout_text(
        &staging_repo,
        &["rev-parse", &treeish],
        "read predecessor request tree",
    )
    .unwrap()
    .trim()
    .to_string();
    let delayed_head = git_stdout_text(
        &staging_repo,
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit-tree",
            &tree,
            "-p",
            &predecessor_head,
            "-m",
            "delayed predecessor request update",
        ],
        "create delayed predecessor request commit",
    )
    .unwrap()
    .trim()
    .to_string();
    run_git(
        Some(&staging_repo),
        &["update-ref", REQUEST_REF, &delayed_head],
        "stage delayed predecessor request update",
    )
    .unwrap();

    let mut recreated = test_repo(&test_owner_id());
    recreated.record.incarnation_id = "repoi_recreated_request_ref".to_string();
    state
        .metadata
        .repositories()
        .recreate_repository_for_tests(recreated)
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .insert_request_for_tests(predecessor_request.clone())
        .await
        .unwrap();

    let error = git_receive_use_case::complete(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        preparation,
        std::time::Duration::ZERO,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert!(
        error
            .public_message()
            .contains("changed after receive-pack")
    );
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.head_oid,
        predecessor_request.head_oid
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_request_snapshot_put_rolls_back_the_request_ref_cache() {
    let mut state = test_state_with_request().await;
    let (source, _permissioned_remote, server, accepted_head) =
        request_checkout(&state, "request-ref-put-failure").await;
    drop(server);
    state.object_store = Arc::new(PutFailsObjectStore {
        readable: state.test_object_store.clone(),
    });
    let (origin, _server) = spawn_test_server(&state).await;
    let permissioned_remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(
        &source,
        &permissioned_remote,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    );
    fs::write(source.join("request.txt"), "rejected request content\n").unwrap();
    run_git(
        Some(&source),
        &["add", "request.txt"],
        "stage rejected request edit",
    )
    .unwrap();
    commit_all(&source, "rejected request edit");

    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "reject request snapshot storage failure",
    )
    .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.head_oid,
        accepted_head
    );
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, &test_repo_incarnation());
    assert_eq!(
        git_stdout_text(
            &store_repo,
            &["rev-parse", REQUEST_REF],
            "read rolled-back request ref"
        )
        .unwrap()
        .trim(),
        accepted_head
    );
}

struct PutFailsObjectStore {
    readable: Arc<MemoryObjectStore>,
}

impl scope_object_store::ObjectStore for PutFailsObjectStore {
    fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::service_unavailable(
            "object PUT failed for test",
        ))
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_object_store::ObjectStoreError> {
        scope_object_store::ObjectStore::get(self.readable.as_ref(), key)
    }

    fn delete(&self, key: &str) -> Result<(), scope_object_store::ObjectStoreError> {
        scope_object_store::ObjectStore::delete(self.readable.as_ref(), key)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_push_records_revision_activity_without_touching_main() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server, _) =
        request_checkout(&state, "request-ref-revision").await;
    let before = stored_request(&state, REQUEST_ID).await;
    let before_event_count = request_event_count(&state).await;
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    fs::write(
        source.join("later-private.txt"),
        "public while the revision is created\n",
    )
    .unwrap();
    push_change(
        &source,
        &permissioned_remote,
        REQUEST_REF,
        "request.txt",
        "request branch content after feedback\n",
        "respond with revision",
    )
    .unwrap();

    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.state(), RequestState::Draft);
    assert_eq!(request.head_oid, git_head_oid(&source));
    assert_eq!(request.activity_version, before.activity_version + 1);
    assert_eq!(request_event_count(&state).await, before_event_count + 1);
    assert!(events.try_recv().is_ok());

    let app = router(state.clone());
    let revisions = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/changes"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revisions.status(), StatusCode::OK);
    let revisions = response_json(revisions).await;
    assert_eq!(revisions["has_earlier_revisions"], false);
    let revisions = revisions["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0]["commits"][0]["message"], "request change");
    let latest = revisions.last().unwrap();
    assert_eq!(latest["inspection"], "Complete");
    assert_eq!(latest["old_head_oid"], serde_json::Value::Null);
    assert_eq!(latest["new_head_oid"], request.head_oid);
    assert_eq!(latest["commits"][0]["message"], "respond with revision");
    assert!(
        latest["commits"][0]["parent_oids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(latest["commits"][0]["change_count"], 2);
    assert_eq!(latest["commits"][0]["files"].as_array().unwrap().len(), 2);
    assert!(latest["commits"][0]["authored_at_unix"].as_u64().unwrap() > 0);

    let revision_id = latest["id"].as_str().unwrap();
    let commit_oid = latest["commits"][0]["oid"].as_str().unwrap();
    cache_test_jwks(&state);
    let abbreviated_anchor = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"body_markdown":"Ambiguous anchor","client_discussion_id":"abbreviated-anchor","anchor":{{"revision_id":"{revision_id}","commit_oid":"{}","path":null}}}}"#,
                    &commit_oid[..8],
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(abbreviated_anchor.status(), StatusCode::BAD_REQUEST);

    assert!(
        latest["commits"][0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "request.txt")
    );

    let anchored = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"body_markdown":"Review this parser change","client_discussion_id":"anchored-review","anchor":{{"revision_id":"{revision_id}","commit_oid":"{commit_oid}","path":"request.txt"}}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anchored.status(), StatusCode::OK);
    let anchored = response_json(anchored).await;
    assert_eq!(anchored["discussion"]["anchor"]["revision_id"], revision_id);
    assert_eq!(
        anchored["discussion"]["anchor"]["revision_position"],
        latest["position"]
    );
    assert_eq!(anchored["discussion"]["anchor"]["commit_oid"], commit_oid);
    assert_eq!(anchored["discussion"]["anchor"]["path"], "/request.txt");
    let anchored_id = anchored["discussion"]["id"].as_str().unwrap();
    let focused_discussion = public_get_json(
        &app,
        format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline?discussion={anchored_id}"
        ),
    )
    .await;
    assert_eq!(focused_discussion["discussions"][0]["id"], anchored_id);

    let discussions = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(discussions.status(), StatusCode::OK);
    let discussions = response_json(discussions).await;
    assert_eq!(discussions["discussions"].as_array().unwrap().len(), 1);
    assert_eq!(
        discussions["discussions"][0]["client_discussion_id"],
        "anchored-review"
    );
    let filtered_discussions = public_get_json(
        &app,
        format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline?revision={revision_id}&commit={commit_oid}"
        ),
    )
    .await;
    assert_eq!(
        filtered_discussions["discussions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        filtered_discussions["discussions"][0]["client_discussion_id"],
        "anchored-review"
    );
    let unrelated_discussions = public_get_json(
        &app,
        format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline?revision=missing-revision&commit={commit_oid}"
        ),
    )
    .await;
    assert!(
        unrelated_discussions["discussions"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let private_path = ScopePath::parse("/later-private.txt").unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.policy
                .add_rule(VisibilityRule::private(private_path.clone()))
                .unwrap();
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
    let redacted_revisions = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/changes"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(redacted_revisions.status(), StatusCode::OK);
    let redacted_revisions = response_json(redacted_revisions).await;
    let redacted_revision = redacted_revisions["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|revision| revision["id"] == revision_id)
        .unwrap();
    assert!(redacted_revision["commits"].as_array().unwrap().is_empty());
    assert_eq!(redacted_revision["new_head_oid"], serde_json::Value::Null);
    let redacted_discussion = public_get_json(
        &app,
        format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline?discussion={anchored_id}"
        ),
    )
    .await;
    assert_eq!(
        redacted_discussion["discussions"][0]["anchor"]["revision_id"],
        revision_id
    );
    assert_eq!(
        redacted_discussion["discussions"][0]["anchor"]["revision_position"],
        latest["position"]
    );
    assert_eq!(
        redacted_discussion["discussions"][0]["anchor"]["commit_oid"],
        serde_json::Value::Null
    );
    assert_eq!(
        redacted_discussion["discussions"][0]["anchor"]["path"],
        serde_json::Value::Null
    );
    let revision = state
        .metadata
        .requests()
        .request_revision(REQUEST_ID, revision_id)
        .await
        .unwrap()
        .unwrap();
    state
        .object_store
        .delete(&scope_object_store::object_key(&revision.git_snapshot))
        .unwrap();
    let unavailable_anchor = public_get_json(
        &app,
        format!(
            "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/timeline?discussion={anchored_id}"
        ),
    )
    .await;
    assert_eq!(
        unavailable_anchor["discussions"][0]["anchor"]["commit_oid"],
        serde_json::Value::Null
    );
    assert_eq!(
        unavailable_anchor["discussions"][0]["anchor"]["path"],
        serde_json::Value::Null
    );

    let resolved = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{REQUEST_ID}/threads/{anchored_id}/resolve"
                ))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let resolved = response_json(resolved).await;
    assert_eq!(resolved["discussion"]["status"], "Resolved");
    assert_eq!(
        resolved["discussion"]["anchor"]["commit_oid"],
        serde_json::Value::Null
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_invitee_can_push_request_ref_but_uninvited_maintainer_cannot() {
    for (label, subject, email, path, is_invitee, push_allowed) in [
        (
            "request-ref-contributor-push",
            CONTRIBUTOR_SUBJECT,
            CONTRIBUTOR_EMAIL,
            "contributor.txt",
            true,
            true,
        ),
        (
            "request-ref-maintainer-push",
            MEMBER_SUBJECT,
            MEMBER_EMAIL,
            "maintainer.txt",
            false,
            false,
        ),
    ] {
        let state = test_state_with_request().await;
        if is_invitee {
            insert_public_contributor(&state).await;
            state
                .metadata
                .requests()
                .add_request_invitee(AddRequestInviteeCommand {
                    request_id: REQUEST_ID.to_string(),
                    actor_user_id: public_user_id(),
                    target_handle: "contributor".to_string(),
                    now_unix: 3,
                })
                .await
                .unwrap();
        } else {
            insert_member_user(&state).await;
        }
        let (source, remote, _server) = request_push_checkout(&state, label, subject, email).await;
        if !is_invitee {
            configure_push_intent_header(&state, &source, &remote, &member_user_id()).await;
        }
        let before_event_count = request_event_count(&state).await;
        let pushed = push_change(
            &source,
            &remote,
            REQUEST_REF,
            path,
            "request branch content\n",
            "request change",
        );
        if push_allowed {
            pushed.unwrap();
            let request = stored_request(&state, REQUEST_ID).await;
            assert_eq!(request.head_oid, git_head_oid(&source));
            assert!(request.git_snapshot.is_some());
            assert_eq!(request_event_count(&state).await, before_event_count + 1);
        } else {
            pushed.unwrap_err();
            assert_request_branch_unchanged(&state).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_history_unrelated_to_recorded_base() {
    let state = test_state_with_request().await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.author_user_id = test_owner_id();
            request.author_role = RequestActorRole::Owner;
            request.audience = RequestAudience::Private;
        })
        .await
        .unwrap();
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-ref-unrelated-history",
        TEST_CLERK_USER_ID,
        TEST_OWNER_EMAIL,
    )
    .await;
    run_git(
        Some(&source),
        &["checkout", "--orphan", "unrelated-request"],
        "create unrelated request history",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["rm", "-rf", "."],
        "clear unrelated request tree",
    )
    .unwrap();
    fs::write(source.join("unrelated.txt"), "unrelated history\n").unwrap();
    run_git(
        Some(&source),
        &["add", "-A"],
        "add unrelated request changes",
    )
    .unwrap();
    commit_all(&source, "unrelated request change");
    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push unrelated request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_ref_push_rejects_unsupported_tree_entries() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-ref-invalid-tree",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    let commit = git_head_oid(&source);
    run_git(
        Some(&source),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor/submodule"),
        ],
        "add request gitlink",
    )
    .unwrap();
    commit_all(&source, "invalid request tree");
    let output = run_git_output(
        Some(&source),
        &["push", &permissioned_remote, &format!("HEAD:{REQUEST_REF}")],
        "push invalid request ref",
    )
    .unwrap();

    assert!(!output.status.success());
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_request_author_cannot_push_main() {
    let state = test_state_with_request().await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "request-main-rejected",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    let output = push_change(
        &source,
        &permissioned_remote,
        "main",
        "README.md",
        "public main write\n",
        "try main",
    )
    .unwrap_err();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Scope contributors cannot update main")
    );
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
    assert_request_branch_unchanged(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_actor_cannot_push_private_request_after_membership_loss() {
    let state = test_state_with_request().await;
    insert_private_request_for_public_user(&state).await;
    let (source, permissioned_remote, _server) = request_push_checkout(
        &state,
        "private-request-rejected",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    push_change(
        &source,
        &permissioned_remote,
        PRIVATE_REQUEST_REF,
        "private-request.txt",
        "private request write\n",
        "try private request",
    )
    .unwrap_err();

    assert_eq!(
        stored_request(&state, PRIVATE_REQUEST_ID).await.head_oid,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}
