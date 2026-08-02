use super::*;

mod helpers;
mod queue;
pub(super) use helpers::{create_owner_request, create_public_request, rebuild_request_projection};

use scope_domain::requests::RequestState;
use scope_postgres::db::AddRequestInviteeCommand;

const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn request_reads_do_not_consume_git_projection_capacity() {
    let mut state = test_state_with_readme().await;
    cache_test_jwks(&state);
    state.runtime_budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        projection_build_concurrency: 0,
        ..Default::default()
    }));
    let app = router(state.clone());

    for uri in [
        "/v1/repos/owner/repo/requests",
        "/v1/repos/owner/repo/requests/queue?section=ready",
    ] {
        let response = api_request(app.clone(), "GET", uri, None, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(request_ids(&response_json(response).await).is_empty());
    }

    rebuild_request_projection(&state).await;
    create_owner_request(&state, "req_metadata_head", REQUEST_HEAD).await;
    let ready = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_metadata_head/ready",
        Some(&bearer_header()),
        Some("{}"),
    )
    .await;
    assert_eq!(ready.status(), StatusCode::OK);

    let queue = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests/queue?section=ready",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(queue.status(), StatusCode::OK);
    let queue = response_json(queue).await;
    assert_eq!(request_ids(&queue), ["req_metadata_head"]);
    let current_main_oid = queue["requests"][0]["mergeability"]["current_main_oid"]
        .as_str()
        .unwrap();
    assert_eq!(current_main_oid.len(), 40);
    assert!(
        current_main_oid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
}

#[tokio::test]
async fn native_private_request_reads_use_the_persisted_git_head() {
    let (mut state, _source, head_oid) =
        super::push_intent_completion::published_git_fixture("request-metadata-native").await;
    create_owner_request(&state, "req_native_head", REQUEST_HEAD).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let raw_cache = raw_git_cache_path(&state, &repo.git_head.as_ref().unwrap().manifest).unwrap();
    if raw_cache.exists() {
        fs::remove_dir_all(raw_cache).unwrap();
    }
    state.runtime_budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        projection_build_concurrency: 0,
        ..Default::default()
    }));

    let response = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/requests/req_native_head",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["request"]["mergeability"]["current_main_oid"],
        head_oid
    );
}

#[tokio::test]
async fn private_request_bases_select_the_native_private_head() {
    let (state, _source, head_oid) =
        super::push_intent_completion::published_git_fixture("request-private-base").await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();

    assert_eq!(
        crate::http::requests::current_main_oid_for_audience(
            &state,
            &repo,
            scope_domain::requests::RequestAudience::Private,
        )
        .await
        .unwrap(),
        Some(head_oid)
    );
}

#[tokio::test]
async fn stale_request_projection_identity_returns_the_rebuilding_response() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    create_owner_request(&state, "req_stale_projection", REQUEST_HEAD).await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.change_version += 1;
        })
        .await
        .unwrap();
    let app = router(state);

    let stale = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_stale_projection",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response_json(stale).await["error"],
        "repository projection is rebuilding; retry shortly"
    );

    for uri in [
        "/v1/repos/owner/repo/requests?cursor=v1:zzzz",
        "/v1/repos/owner/repo/requests/queue?section=ready",
    ] {
        let empty = api_request(app.clone(), "GET", uri, Some(&bearer_header()), None).await;
        assert_eq!(empty.status(), StatusCode::OK);
        assert!(request_ids(&response_json(empty).await).is_empty());
    }
}

#[tokio::test]
async fn public_readers_do_not_see_private_request_branches() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    create_owner_request(&state, "req_private", REQUEST_HEAD).await;
    let app = router(state);

    let public_response = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests",
        None,
        None,
    )
    .await;

    assert_eq!(public_response.status(), StatusCode::OK);
    let public_body = response_json(public_response).await;
    assert_eq!(public_body["requests"].as_array().unwrap().len(), 0);
    assert!(public_body["next_cursor"].is_null());

    let owner_response = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests",
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["requests"].as_array().unwrap().len(), 1);
    assert_eq!(owner_body["requests"][0]["audience"], "Private");
    assert!(
        owner_body["requests"][0]
            .get("description_markdown")
            .is_none()
    );
}

#[tokio::test]
async fn request_list_rejects_malformed_cursors() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let response = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/requests?cursor=not-versioned",
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn request_list_pages_one_hundred_and_one_visible_rows_without_overlap() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    for index in 0..=100 {
        create_owner_request(
            &state,
            &format!("req_page_{index:03}"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .await;
    }
    let app = router(state);

    let anonymous = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests?limit=1000",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    let anonymous = response_json(anonymous).await;
    assert_eq!(anonymous["requests"].as_array().unwrap().len(), 0);
    assert!(anonymous["next_cursor"].is_null());

    let first = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests?limit=1000",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let first_requests = first["requests"].as_array().unwrap();
    assert_eq!(first_requests.len(), 100);
    assert_eq!(first_requests.first().unwrap()["id"], "req_page_000");
    assert_eq!(first_requests.last().unwrap()["id"], "req_page_099");
    let cursor = first["next_cursor"].as_str().unwrap();

    let second = api_request(
        app,
        "GET",
        &format!("/v1/repos/owner/repo/requests?limit=1000&cursor={cursor}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    assert_eq!(second["requests"].as_array().unwrap().len(), 1);
    assert_eq!(second["requests"][0]["id"], "req_page_100");
    assert!(second["next_cursor"].is_null());
}

#[tokio::test]
async fn maintainer_request_lifecycle_routes_delegate_to_atomic_commands() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    create_owner_request(&state, "req_lifecycle", REQUEST_HEAD).await;
    let app = router(state);
    let bearer = bearer_header();

    let ready = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/ready",
        Some(&bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(ready.status(), StatusCode::OK);
    let ready = response_json(ready).await;
    assert_eq!(ready["request"]["state"], "ReadyForReview");
    assert_eq!(ready["request"]["permissions"]["can_request_changes"], true);

    let held = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_lifecycle/hold",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(held.status(), StatusCode::OK);
    assert!(response_json(held).await["request"]["held_at_unix"].is_number());

    let held_author_exit = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/working",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(held_author_exit.status(), StatusCode::CONFLICT);

    let released = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_lifecycle/hold",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(released.status(), StatusCode::OK);
    assert!(response_json(released).await["request"]["held_at_unix"].is_null());

    let working = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/working",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(working.status(), StatusCode::OK);
    assert_eq!(response_json(working).await["request"]["state"], "Working");

    let ready_again = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/ready",
        Some(&bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(ready_again.status(), StatusCode::OK);
    let changes = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/request-changes",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(changes.status(), StatusCode::OK);
    assert_eq!(response_json(changes).await["request"]["state"], "Working");

    let ready_for_assessment = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/ready",
        Some(&bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(ready_for_assessment.status(), StatusCode::OK);
    let assessed = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_lifecycle/assessment",
        Some(&bearer),
        Some(r#"{"outcome":"Neutral"}"#),
    )
    .await;
    assert_eq!(assessed.status(), StatusCode::OK);
    let assessed = response_json(assessed).await;
    assert_eq!(assessed["request"]["state"], "Completed");
    assert_eq!(assessed["request"]["assessment_outcome"], "Neutral");
}

#[tokio::test]
async fn request_reads_apply_one_viewer_aware_policy_across_lists_and_exact_surfaces() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_author");
    let invitee_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_invitee");
    let unrelated_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_unrelated");
    for user in [
        test_user(&author_id, "request-author", "request-author@example.com"),
        test_user(
            &invitee_id,
            "request-invitee",
            "request-invitee@example.com",
        ),
        test_user(
            &unrelated_id,
            "request-unrelated",
            "request-unrelated@example.com",
        ),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }

    create_public_request(&state, "req_never", author_id.clone(), REQUEST_HEAD).await;
    create_public_request(&state, "req_previous", author_id.clone(), REQUEST_HEAD).await;
    create_public_request(&state, "req_ready_public", author_id.clone(), REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_previous", |request| {
            request.first_ready_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_ready_public", |request| {
            request.state = RequestState::ReadyForReview;
            request.first_ready_at_unix = Some(4);
            request.ready_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    create_owner_request(&state, "req_private_matrix", REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_never".to_string(),
            actor_user_id: author_id.clone(),
            target_handle: "request-invitee".to_string(),
            now_unix: 5,
        })
        .await
        .unwrap();

    let app = router(state);
    let anonymous_list = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&anonymous_list), vec!["req_ready_public"]);
    assert_eq!(anonymous_list["requests"][0]["ready_at_unix"], 4);
    assert!(anonymous_list["requests"][0]["held_at_unix"].is_null());

    let unrelated = bearer_header_for("request_unrelated", "request-unrelated@example.com");
    for suffix in ["req_never", "req_never/timeline", "req_never/activity"] {
        assert_eq!(
            api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/{suffix}"),
                Some(&unrelated),
                None,
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    for request_id in ["req_previous", "req_ready_public"] {
        assert_eq!(
            api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/{request_id}"),
                None,
                None,
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/req_private_matrix",
            None,
            None,
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    let invitee = bearer_header_for("request_invitee", "request-invitee@example.com");
    let invitee_list = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests",
            Some(&invitee),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        request_ids(&invitee_list),
        vec!["req_never", "req_ready_public"]
    );
    let invitee_detail = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_never",
        Some(&invitee),
        None,
    )
    .await;
    assert_eq!(invitee_detail.status(), StatusCode::OK);
    let invitee_detail = response_json(invitee_detail).await;
    assert_eq!(
        invitee_detail["request"]["invitees"][0]["user"]["handle"],
        "request-invitee"
    );
    assert_eq!(
        invitee_detail["request"]["permissions"]["can_push_branch"],
        true
    );
    assert_eq!(
        invitee_detail["request"]["permissions"]["can_view_activity"],
        true
    );
    assert_eq!(
        invitee_detail["request"]["permissions"]["can_open_discussion"],
        true
    );
    assert_eq!(
        invitee_detail["request"]["permissions"]["can_edit_identity"],
        false
    );
    assert_eq!(
        invitee_detail["request"]["permissions"]["can_manage_invitees"],
        false
    );

    let maintainer_list = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests",
            Some(&bearer_header()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        request_ids(&maintainer_list),
        vec!["req_private_matrix", "req_ready_public"]
    );
    for request_id in ["req_previous", "req_private_matrix"] {
        assert_eq!(
            api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/{request_id}"),
                Some(&bearer_header()),
                None,
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    for suffix in ["req_never", "req_never/timeline", "req_never/activity"] {
        assert_eq!(
            api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/{suffix}"),
                Some(&bearer_header()),
                None,
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        api_request(
            app.clone(),
            "PATCH",
            "/v1/repos/owner/repo/requests/req_never",
            Some(&bearer_header()),
            Some(r#"{"title":"Maintainer must not see this"}"#),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api_request(
            app,
            "POST",
            "/v1/repos/owner/repo/requests/req_never/timeline",
            Some(&bearer_header()),
            Some(
                r#"{"body_markdown":"Maintainer must not see this","client_discussion_id":"hidden"}"#,
            ),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn invitee_routes_enforce_exact_handles_roles_leave_and_private_exclusion() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_author");
    let invitee_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_target");
    let other_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_other");
    for user in [
        test_user(&author_id, "invite-author", "invite-author@example.com"),
        test_user(&invitee_id, "invite-target", "invite-target@example.com"),
        test_user(&other_id, "invite-other", "invite-other@example.com"),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }
    create_public_request(&state, "req_invites", author_id.clone(), REQUEST_HEAD).await;
    create_public_request(&state, "req_held_invites", author_id.clone(), REQUEST_HEAD).await;
    create_owner_request(&state, "req_private_invites", REQUEST_HEAD).await;
    let app = router(state.clone());
    let author = bearer_header_for("invite_author", "invite-author@example.com");
    let invitee = bearer_header_for("invite_target", "invite-target@example.com");

    let wrong_case = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&author),
        Some(r#"{"handle":"Invite-Target"}"#),
    )
    .await;
    assert_eq!(wrong_case.status(), StatusCode::NOT_FOUND);

    let added = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&author),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(added.status(), StatusCode::OK);
    let added = response_json(added).await;
    assert_eq!(added["invitee"]["user"]["handle"], "invite-target");
    assert_eq!(added["request"]["invitees"].as_array().unwrap().len(), 1);
    let held_added = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_held_invites/invitees",
        Some(&author),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(held_added.status(), StatusCode::OK);

    let ready = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_held_invites/ready",
        Some(&author),
        Some("{}"),
    )
    .await;
    assert_eq!(ready.status(), StatusCode::OK);
    let ready = response_json(ready).await;
    assert_eq!(ready["request"]["state"], "ReadyForReview");

    let held = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_held_invites/hold",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(held.status(), StatusCode::OK);
    let held = response_json(held).await;
    assert_eq!(held["request"]["permissions"]["can_leave_request"], false);

    let held_leave = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_held_invites/invitees/me",
        Some(&invitee),
        None,
    )
    .await;
    assert_eq!(held_leave.status(), StatusCode::FORBIDDEN);

    let released = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_held_invites/hold",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(released.status(), StatusCode::OK);

    let invitee_manage = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&invitee),
        Some(r#"{"handle":"invite-other"}"#),
    )
    .await;
    assert_eq!(invitee_manage.status(), StatusCode::FORBIDDEN);

    let left = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_invites/invitees/me",
        Some(&invitee),
        None,
    )
    .await;
    assert_eq!(left.status(), StatusCode::OK);
    assert_eq!(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/req_invites",
            Some(&invitee),
            None,
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_invites", |request| {
            request.first_ready_at_unix = Some(5);
            request.updated_at_unix = 5;
        })
        .await
        .unwrap();

    let maintainer_add = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(maintainer_add.status(), StatusCode::OK);
    let maintainer_remove = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(maintainer_remove.status(), StatusCode::OK);

    let private_add = api_request(
        app,
        "PUT",
        "/v1/repos/owner/repo/requests/req_private_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(private_add.status(), StatusCode::CONFLICT);
}

fn request_ids(body: &serde_json::Value) -> Vec<&str> {
    body["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| request["id"].as_str().unwrap())
        .collect()
}

async fn api_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    let body = match body {
        Some(json) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(request.body(body).unwrap()).await.unwrap()
}
