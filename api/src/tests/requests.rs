use super::*;
use crate::domain::requests::{
    CreditLedgerEntryKind, GrantUserCreditsInput, RecordRequestRevisionInput,
    RecordWorkingRequestUploadInput, canonical_request_ref,
};
use tokio_stream::StreamExt;

mod editors;
mod helpers;
use helpers::{
    create_owner_request, create_public_request, mark_working_request_uploaded,
    start_request_via_http, submit_request_via_http,
};

const PUBLIC_SUBJECT: &str = "public_requester";
const PUBLIC_EMAIL: &str = "public@example.com";
const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn public_submit_stakes_credits_and_uses_public_base() {
    let state = state_with_public_user();
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();

    let app = router(state.clone());
    let start = start_request_via_http(
        app.clone(),
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;
    assert_eq!(
        start["request"]["request_ref"],
        canonical_request_ref(start["request"]["id"].as_str().unwrap())
    );
    let hidden = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::OK);
    assert_eq!(
        response_json(hidden).await["requests"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    mark_working_request_uploaded(
        &state,
        start["request"]["id"].as_str().unwrap(),
        REQUEST_HEAD,
    )
    .await;

    let response = submit_request_via_http(
        app,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","stake_credits":10}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["title"], "Fix parser crash");
    assert_eq!(body["request"]["author_role"], "Public");
    assert_eq!(body["request"]["base_audience"], "Public");
    assert_eq!(body["request"]["stake_credits"], 10);
    assert_eq!(body["request"]["permissions"]["can_push_branch"], true);
    assert_eq!(body["request"]["mergeability"]["status"], "NotMaintainer");

    state
        .metadata
        .read(|catalog| {
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get(&public_user_id())
                    .unwrap()
                    .balance_credits,
                10
            );
            let stake_entries = catalog
                .credit_ledger_entries
                .values()
                .filter(|entry| entry.kind == CreditLedgerEntryKind::RequestStakeDebit)
                .count();
            assert_eq!(stake_entries, 1);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn owner_submit_uses_private_base_without_credit_stake() {
    let state = test_state_with_repo_with_readme();
    let app = router(state.clone());
    let start = start_request_via_http(app.clone(), &bearer_header()).await;
    mark_working_request_uploaded(
        &state,
        start["request"]["id"].as_str().unwrap(),
        REQUEST_HEAD,
    )
    .await;

    let response = submit_request_via_http(
        app,
        &bearer_header(),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["author_role"], "Owner");
    assert_eq!(body["request"]["base_audience"], "Private");
    assert_eq!(body["request"]["stake_credits"], 0);
    assert_eq!(body["request"]["permissions"]["can_merge"], true);

    state
        .metadata
        .read(|catalog| {
            assert!(catalog.credit_ledger_entries.is_empty());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn working_request_is_visible_but_not_maintainer_decision_ready() {
    let state = test_state_with_repo_with_readme();
    let app = router(state.clone());
    let start = start_request_via_http(app.clone(), &bearer_header()).await;

    assert_eq!(start["request"]["state"], "Working");
    assert_eq!(start["request"]["permissions"]["can_push_branch"], true);
    assert_eq!(
        start["request"]["permissions"]["can_mark_needs_response"],
        false
    );
    assert_eq!(start["request"]["permissions"]["can_resolve"], false);
    assert_eq!(start["request"]["permissions"]["can_merge"], false);
    assert_eq!(start["request"]["mergeability"]["status"], "NotReady");

    let resolve = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/owner/repo/requests/{}/resolve",
                    start["request"]["id"].as_str().unwrap()
                ))
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"disposition":"UsefulNotMerged","body":"Not ready."}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resolve.status(), StatusCode::CONFLICT);

    let merge = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/repos/owner/repo/requests/{}/merge",
                    start["request"]["id"].as_str().unwrap()
                ))
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"expected_main_oid":"{REQUEST_HEAD}","expected_head_oid":"{REQUEST_HEAD}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(merge.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn finalizing_without_uploaded_request_ref_does_not_stake_credits() {
    let state = state_with_public_user();
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
    let app = router(state.clone());
    let start = start_request_via_http(
        app.clone(),
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;

    let response = submit_request_via_http(
        app,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","stake_credits":10}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    state
        .metadata
        .read(|catalog| {
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get(&public_user_id())
                    .unwrap()
                    .balance_credits,
                20
            );
            assert!(
                catalog
                    .credit_ledger_entries
                    .values()
                    .all(|entry| { entry.kind != CreditLedgerEntryKind::RequestStakeDebit })
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn request_submit_publishes_summary_refresh_event() {
    let state = test_state_with_repo_with_readme();
    let app = router(state.clone());
    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/events")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let mut stream = events.into_body().into_data_stream();
    let initial = stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8(initial.to_vec())
            .unwrap()
            .contains(r#""reason":"connected""#)
    );

    let start = start_request_via_http(app.clone(), &bearer_header()).await;
    let started_event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let started_event = String::from_utf8(started_event.to_vec()).unwrap();
    assert!(started_event.contains(r#""reason":"request-started""#));

    mark_working_request_uploaded(
        &state,
        start["request"]["id"].as_str().unwrap(),
        REQUEST_HEAD,
    )
    .await;
    let submit = submit_request_via_http(
        app,
        &bearer_header(),
        start["request"]["id"].as_str().unwrap(),
        r#"{"head_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )
    .await;
    assert_eq!(submit.status(), StatusCode::OK);

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = String::from_utf8(event.to_vec()).unwrap();
    assert!(event.contains(r#""reason":"request-submitted""#));
    assert!(event.contains(r#""version":0"#));
}

#[tokio::test]
async fn public_readers_do_not_see_private_request_branches() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    create_owner_request(&state, "req_private", REQUEST_HEAD).await;
    let app = router(state);

    let public_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_response.status(), StatusCode::OK);
    let public_body = response_json(public_response).await;
    assert_eq!(public_body["requests"].as_array().unwrap().len(), 0);

    let public_summary_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(public_summary_response.status(), StatusCode::OK);
    let public_summary_body = response_json(public_summary_response).await;
    assert_eq!(public_summary_body["open_request_count"], 0);

    let owner_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/requests")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["requests"].as_array().unwrap().len(), 1);
    assert_eq!(owner_body["requests"][0]["base_audience"], "Private");
}

#[tokio::test]
async fn needs_response_respond_and_resolution_settle_public_stake() {
    let state = state_with_public_request().await;
    let app = router(state.clone());

    let needs_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/needs-response")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"Please add a repro."}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(needs_response.status(), StatusCode::OK);
    let body = response_json(needs_response).await;
    assert_eq!(body["request"]["state"], "NeedsResponse");

    let respond = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/respond")
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"Added."}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(respond.status(), StatusCode::OK);
    let body = response_json(respond).await;
    assert_eq!(body["request"]["state"], "Submitted");

    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let resolve = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public/resolve")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"disposition":"UsefulNotMerged","body":"Helpful, but not merging."}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resolve.status(), StatusCode::OK);
    let body = response_json(resolve).await;
    assert_eq!(body["request"]["state"], "Resolved");
    assert_eq!(body["request"]["settlement"]["refunded_credits"], 10);
    assert_eq!(body["request"]["settlement"]["reward_credits"], 2);
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.repo_id, TEST_REPO_ID);
    assert_eq!(event.reason, "request-resolved");
    assert_eq!(event.version, 0);

    state
        .metadata
        .read(|catalog| {
            assert_eq!(
                catalog
                    .user_credit_accounts
                    .get(&public_user_id())
                    .unwrap()
                    .balance_credits,
                22
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn clean_merge_applies_repo_update_and_resolves_as_accepted() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let main_repo = temp_git_repo("request-merge-main");
    write_file(&main_repo, "README.md", "hello\n");
    run_git(Some(&main_repo), &["add", "."], "stage main").unwrap();
    commit_all(&main_repo, "initial");
    let main_oid = git_head_oid(&main_repo);
    let main_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &main_repo, "refs/heads/main").unwrap();

    let request_repo = temp_git_repo("request-merge-branch");
    run_git(
        Some(&request_repo),
        &["pull", main_repo.to_str().unwrap(), "main"],
        "seed request branch",
    )
    .unwrap();
    write_file(&request_repo, "README.md", "hello from request\n");
    write_file(
        &request_repo,
        "scope-request.bundle.tmp",
        "valid repo file\n",
    );
    run_git(Some(&request_repo), &["add", "."], "stage request").unwrap();
    commit_all(&request_repo, "request change");
    let request_head = git_head_oid(&request_repo);
    run_git(
        Some(&request_repo),
        &[
            "update-ref",
            "refs/scope/requests/req_merge",
            request_head.as_str(),
        ],
        "create request ref",
    )
    .unwrap();
    let request_snapshot = git_snapshot_from_ref(
        &state,
        TEST_REPO_ID,
        &request_repo,
        "refs/scope/requests/req_merge",
    )
    .unwrap();

    {
        let mut repo = repo_with_readme();
        repo.git_snapshot = Some(main_snapshot);
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }
    create_owner_request(&state, "req_merge", &request_head).await;
    state
        .metadata
        .record_request_revision(RecordRequestRevisionInput {
            request_id: "req_merge".to_string(),
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request_head.clone()),
            new_head_oid: request_head.clone(),
            git_snapshot: Some(request_snapshot),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();
    let merge_worktree =
        receive_pack_staging_repo_path(&state, TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    if let Some(parent) = merge_worktree.parent() {
        let _ = fs::remove_dir_all(parent);
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_merge/merge")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"expected_main_oid":"{main_oid}","expected_head_oid":"{request_head}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["state"], "Resolved");
    assert_eq!(body["request"]["disposition"], "Accepted");

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live_tree = repo.live_tree();
    let readme = live_tree
        .get(&ScopePath::parse("/README.md").unwrap())
        .unwrap();
    let temp_named_file = live_tree
        .get(&ScopePath::parse("/scope-request.bundle.tmp").unwrap())
        .unwrap();
    assert_eq!(blob_content(readme), "hello from request\n");
    assert_eq!(blob_content(temp_named_file), "valid repo file\n");
    assert_ne!(repo.git_snapshot.unwrap().object_key, "");
    let _ = fs::remove_dir_all(main_repo);
    let _ = fs::remove_dir_all(request_repo);
}

#[tokio::test]
async fn public_request_merge_replays_public_delta_without_deleting_private_files() {
    let state = state_with_public_user();
    cache_test_jwks(&state);
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_public_merge_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();

    let raw_repo = temp_git_repo("public-request-raw-main");
    write_file(&raw_repo, "README.md", "hello\n");
    write_file(&raw_repo, ".gitignore", "ignored.txt\n");
    write_file(&raw_repo, "SECRET.md", "private\n");
    run_git(Some(&raw_repo), &["add", "."], "stage raw main").unwrap();
    commit_all(&raw_repo, "initial raw main");
    let raw_main_oid = git_head_oid(&raw_repo);
    let raw_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &raw_repo, "refs/heads/main").unwrap();

    {
        let mut repo = repo_with_public_readme_and_private_secret();
        repo.git_snapshot = Some(raw_snapshot);
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(&state, &public_projection).unwrap();
    let public_main_oid = git_stdout_text(
        &public_repo,
        &["rev-parse", "refs/heads/main"],
        "read public main",
    )
    .unwrap()
    .trim()
    .to_string();

    let request_repo = std::env::temp_dir().join(format!(
        "scope-vcs-public-request-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&request_repo);
    run_git(
        None,
        &[
            "clone",
            public_repo.to_str().unwrap(),
            request_repo.to_str().unwrap(),
        ],
        "clone public projection for request",
    )
    .unwrap();
    write_file(&request_repo, "README.md", "hello from public request\n");
    write_file(&request_repo, "ignored.txt", "tracked despite ignore\n");
    run_git(Some(&request_repo), &["add", "."], "stage public request").unwrap();
    run_git(
        Some(&request_repo),
        &["add", "-f", "ignored.txt"],
        "stage ignored public request file",
    )
    .unwrap();
    commit_all(&request_repo, "public request change");
    let request_head = git_head_oid(&request_repo);
    run_git(
        Some(&request_repo),
        &[
            "update-ref",
            "refs/scope/requests/req_public_merge",
            request_head.as_str(),
        ],
        "create public request ref",
    )
    .unwrap();
    let request_snapshot = git_snapshot_from_ref(
        &state,
        TEST_REPO_ID,
        &request_repo,
        "refs/scope/requests/req_public_merge",
    )
    .unwrap();

    create_public_request(
        &state,
        "req_public_merge",
        &public_main_oid,
        &request_head,
        "Public request merge",
        "ledger_public_merge_stake",
        "event_public_merge_created",
    )
    .await;
    state
        .metadata
        .record_request_revision(RecordRequestRevisionInput {
            request_id: "req_public_merge".to_string(),
            actor_user_id: public_user_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request_head.clone()),
            new_head_oid: request_head.clone(),
            git_snapshot: Some(request_snapshot),
            event_id: "event_public_merge_revision".to_string(),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_public_merge/merge")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"expected_main_oid":"{raw_main_oid}","expected_head_oid":"{request_head}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["request"]["state"], "Resolved");
    assert_eq!(body["request"]["disposition"], "Accepted");

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live_tree = repo.live_tree();
    let readme = live_tree
        .get(&ScopePath::parse("/README.md").unwrap())
        .unwrap();
    let secret = live_tree
        .get(&ScopePath::parse("/SECRET.md").unwrap())
        .unwrap();
    let ignored = live_tree
        .get(&ScopePath::parse("/ignored.txt").unwrap())
        .unwrap();
    assert_eq!(blob_content(readme), "hello from public request\n");
    assert_eq!(blob_content(ignored), "tracked despite ignore\n");
    assert_eq!(blob_content(secret), "private\n");
    let _ = fs::remove_dir_all(raw_repo);
    let _ = fs::remove_dir_all(request_repo);
}

#[tokio::test]
async fn public_request_merge_rejects_private_path_collision() {
    let state = state_with_public_user();
    cache_test_jwks(&state);
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_private_collision_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();

    let raw_repo = temp_git_repo("public-request-private-collision-raw");
    write_file(&raw_repo, "README.md", "hello\n");
    write_file(&raw_repo, "SECRET.md", "private\n");
    run_git(
        Some(&raw_repo),
        &["add", "."],
        "stage raw private collision main",
    )
    .unwrap();
    commit_all(&raw_repo, "initial raw main");
    let raw_main_oid = git_head_oid(&raw_repo);
    let raw_snapshot =
        git_snapshot_from_ref(&state, TEST_REPO_ID, &raw_repo, "refs/heads/main").unwrap();

    {
        let mut repo = repo_with_public_readme_and_private_secret();
        repo.git_snapshot = Some(raw_snapshot);
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let public_repo = projection_bare_repo_for_state(&state, &public_projection).unwrap();
    let public_main_oid = git_stdout_text(
        &public_repo,
        &["rev-parse", "refs/heads/main"],
        "read public main",
    )
    .unwrap()
    .trim()
    .to_string();

    let request_repo = std::env::temp_dir().join(format!(
        "scope-vcs-private-collision-request-{}-{}",
        std::process::id(),
        unix_now()
    ));
    let _ = fs::remove_dir_all(&request_repo);
    run_git(
        None,
        &[
            "clone",
            public_repo.to_str().unwrap(),
            request_repo.to_str().unwrap(),
        ],
        "clone public projection for private collision request",
    )
    .unwrap();
    write_file(&request_repo, "SECRET.md", "public overwrite attempt\n");
    run_git(
        Some(&request_repo),
        &["add", "."],
        "stage private collision request",
    )
    .unwrap();
    commit_all(&request_repo, "try private path collision");
    let request_head = git_head_oid(&request_repo);
    run_git(
        Some(&request_repo),
        &[
            "update-ref",
            "refs/scope/requests/req_private_collision",
            request_head.as_str(),
        ],
        "create private collision request ref",
    )
    .unwrap();
    let request_snapshot = git_snapshot_from_ref(
        &state,
        TEST_REPO_ID,
        &request_repo,
        "refs/scope/requests/req_private_collision",
    )
    .unwrap();

    create_public_request(
        &state,
        "req_private_collision",
        &public_main_oid,
        &request_head,
        "Private collision request",
        "ledger_private_collision_stake",
        "event_private_collision_created",
    )
    .await;
    state
        .metadata
        .record_request_revision(RecordRequestRevisionInput {
            request_id: "req_private_collision".to_string(),
            actor_user_id: public_user_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request_head.clone()),
            new_head_oid: request_head.clone(),
            git_snapshot: Some(request_snapshot),
            event_id: "event_private_collision_revision".to_string(),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/requests/req_private_collision/merge")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"expected_main_oid":"{raw_main_oid}","expected_head_oid":"{request_head}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let live_tree = repo.live_tree();
    let secret = live_tree
        .get(&ScopePath::parse("/SECRET.md").unwrap())
        .unwrap();
    assert_eq!(blob_content(secret), "private\n");
    state
        .metadata
        .read(|catalog| {
            let request = catalog.requests.get("req_private_collision").unwrap();
            assert_eq!(
                request.state,
                crate::domain::requests::RequestState::Submitted
            );
            Ok(())
        })
        .await
        .unwrap();
    let _ = fs::remove_dir_all(raw_repo);
    let _ = fs::remove_dir_all(request_repo);
}

fn state_with_public_user() -> AppState {
    let state = test_state_with_repo_with_readme();
    let public_user = UserAccount {
        id: public_user_id(),
        handle: "public".to_string(),
        email: PUBLIC_EMAIL.to_string(),
        email_verified: true,
    };
    state
        .metadata
        .update(|catalog| {
            catalog.users.insert(public_user.id.clone(), public_user);
            Ok(())
        })
        .unwrap();
    state
}

fn test_state_with_repo_with_readme() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .update(|catalog| {
            catalog
                .repositories
                .insert(TEST_REPO_ID.to_string(), repo_with_readme());
            Ok(())
        })
        .unwrap();
    state
}

async fn state_with_public_request() -> AppState {
    let state = state_with_public_user();
    state
        .metadata
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: public_user_id(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
    create_public_request(
        &state,
        "req_public",
        "base_main",
        REQUEST_HEAD,
        "Public request",
        "ledger_stake",
        "event_created",
    )
    .await;
    state
}

fn repo_with_public_readme_and_private_secret() -> StoredRepository {
    let mut repo = test_repo(&test_owner_id());
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: repo.record.owner_user_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "initial".to_string(),
        changes: vec![
            FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("hello\n")),
            },
            FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/.gitignore").unwrap(),
                old_content: None,
                new_content: Some(source_blob("ignored.txt\n")),
            },
            FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/SECRET.md").unwrap(),
                old_content: None,
                new_content: Some(source_blob("private\n")),
            },
        ],
    });
    repo
}

fn public_user_id() -> String {
    crate::db::scope_user_id_for_auth_identity("clerk", PUBLIC_SUBJECT)
}

fn write_file(repo: &FsPath, path: &str, content: &str) {
    fs::write(repo.join(path), content).unwrap();
}
