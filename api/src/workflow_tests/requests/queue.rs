use super::*;

#[tokio::test]
async fn request_queue_enforces_section_visibility_order_search_and_stable_pagination() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "queue_author");
    let invitee_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "queue_invitee");
    for user in [
        test_user(&author_id, "queue-author", "queue-author@example.com"),
        test_user(&invitee_id, "queue-invitee", "queue-invitee@example.com"),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }

    for id in ["req_draft_author", "req_draft_invited"] {
        create_public_request(&state, id, author_id.clone(), REQUEST_HEAD).await;
    }
    create_owner_request(&state, "req_open_private", REQUEST_HEAD).await;
    create_owner_request(&state, "req_closed_private", REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_draft_invited".to_string(),
            actor_user_id: author_id.clone(),
            target_handle: "queue-invitee".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();

    create_public_request(&state, "req_open_high", author_id.clone(), REQUEST_HEAD).await;
    open_fixture(
        &state,
        "req_open_high",
        30,
        "Needle title",
        "public open body",
    )
    .await;
    create_public_request(&state, "req_open_early", author_id.clone(), REQUEST_HEAD).await;
    open_fixture(&state, "req_open_early", 10, "Early", "public open body").await;
    create_public_request(&state, "req_open_tie_a", author_id.clone(), REQUEST_HEAD).await;
    open_fixture(&state, "req_open_tie_a", 20, "Tie A", "public open body").await;
    create_public_request(&state, "req_open_tie_b", author_id.clone(), REQUEST_HEAD).await;
    open_fixture(&state, "req_open_tie_b", 20, "Tie B", "public open body").await;
    open_fixture(
        &state,
        "req_open_private",
        5,
        "Private needle",
        "private open needle",
    )
    .await;
    create_public_request(&state, "req_closed_old", author_id.clone(), REQUEST_HEAD).await;
    closed_fixture(
        &state,
        "req_closed_old",
        10,
        30,
        "Old public",
        "ordinary history",
    )
    .await;
    create_public_request(&state, "req_closed_new", author_id.clone(), REQUEST_HEAD).await;
    closed_fixture(
        &state,
        "req_closed_new",
        11,
        40,
        "New public",
        "needle history",
    )
    .await;
    closed_fixture(
        &state,
        "req_closed_private",
        12,
        50,
        "Private history",
        "needle history",
    )
    .await;
    create_public_request(&state, "req_closed_draft", author_id.clone(), REQUEST_HEAD).await;

    let app = router(state.clone());
    let author = bearer_header_for("queue_author", "queue-author@example.com");
    let invitee = bearer_header_for("queue_invitee", "queue-invitee@example.com");
    assert_eq!(
        api_request(
            app.clone(),
            "DELETE",
            "/v1/repos/owner/repo/requests/req_closed_draft",
            Some(&author),
            None,
        )
        .await
        .status(),
        StatusCode::OK
    );

    for section in ["your_work", "open", "closed"] {
        let response = api_request(
            app.clone(),
            "GET",
            &format!("/v1/repos/owner/repo/requests/queue?section={section}"),
            None,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        if section == "your_work" {
            assert!(request_ids(&response_json(response).await).is_empty());
        }
    }

    let author_work = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=your_work",
            Some(&author),
            None,
        )
        .await,
    )
    .await;
    assert!(request_ids(&author_work).contains(&"req_draft_author"));
    assert!(!request_ids(&author_work).contains(&"req_closed_draft"));
    let invitee_work = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=your_work",
            Some(&invitee),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&invitee_work), ["req_draft_invited"]);
    let maintainer_work = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=your_work",
            Some(&bearer_header()),
            None,
        )
        .await,
    )
    .await;
    assert!(!request_ids(&maintainer_work).contains(&"req_draft_author"));

    let first = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=open&limit=2",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&first), ["req_open_early", "req_open_tie_a"]);
    let cursor = first["next_cursor"].as_str().unwrap();
    create_public_request(
        &state,
        "req_open_new_priority",
        author_id.clone(),
        REQUEST_HEAD,
    )
    .await;
    open_fixture(
        &state,
        "req_open_new_priority",
        31,
        "New priority",
        "created after cursor",
    )
    .await;
    create_public_request(&state, "req_open_new_tail", author_id.clone(), REQUEST_HEAD).await;
    open_fixture(
        &state,
        "req_open_new_tail",
        32,
        "New tail",
        "created after cursor",
    )
    .await;
    let second = response_json(
        api_request(
            app.clone(),
            "GET",
            &format!("/v1/repos/owner/repo/requests/queue?section=open&limit=2&cursor={cursor}"),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&second), ["req_open_tie_b", "req_open_high"]);
    assert!(second["next_cursor"].is_string());

    let closed = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=closed",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&closed), ["req_closed_new", "req_closed_old"]);
    let maintainer_closed = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=closed",
            Some(&bearer_header()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        request_ids(&maintainer_closed),
        ["req_closed_private", "req_closed_new", "req_closed_old"]
    );

    for (section, expected) in [
        ("open", vec!["req_open_high"]),
        ("closed", vec!["req_closed_new"]),
    ] {
        let searched = response_json(
            api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/queue?section={section}&search=needle"),
                Some(&bearer_header()),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(request_ids(&searched), expected);
    }

    for uri in [
        "/v1/repos/owner/repo/requests/queue?section=closed&cursor=v1:open:1:25:30:req_open_high".to_string(),
        "/v1/repos/owner/repo/requests/queue?section=open&cursor=v1:open:9223372036854775808:2147483648:1:req".to_string(),
        format!(
            "/v1/repos/owner/repo/requests/queue?section=open&search={}",
            "a".repeat(201)
        ),
        "/v1/repos/owner/repo/requests/queue?section=your_work&search=needle".to_string(),
    ] {
        assert_eq!(
            api_request(app.clone(), "GET", &uri, Some(&bearer_header()), None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    let public_repo =
        response_json(api_request(app.clone(), "GET", "/v1/repos/owner/repo", None, None).await)
            .await;
    assert_eq!(public_repo["open_request_count"], 6);
    let maintainer_repo = response_json(
        api_request(
            app,
            "GET",
            "/v1/repos/owner/repo",
            Some(&bearer_header()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(maintainer_repo["open_request_count"], 7);
}

async fn open_fixture(
    state: &AppState,
    request_id: &str,
    submitted_at_unix: u64,
    title: &str,
    description: &str,
) {
    state
        .metadata
        .requests()
        .mutate_request_for_tests(request_id, |request| {
            request.title = title.to_string();
            request.description_markdown = description.to_string();
            request.submitted_at_unix = Some(submitted_at_unix);
            request.updated_at_unix = submitted_at_unix;
        })
        .await
        .unwrap();
}

async fn closed_fixture(
    state: &AppState,
    request_id: &str,
    submitted_at_unix: u64,
    closed_at_unix: u64,
    title: &str,
    description: &str,
) {
    state
        .metadata
        .requests()
        .mutate_request_for_tests(request_id, |request| {
            request.title = title.to_string();
            request.description_markdown = description.to_string();
            request.submitted_at_unix = Some(submitted_at_unix);
            request.closed_at_unix = Some(closed_at_unix);
            request.closed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = closed_at_unix;
        })
        .await
        .unwrap();
}
