use super::*;

#[tokio::test]
async fn request_queue_enforces_section_visibility_order_search_and_stable_pagination() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = crate::db::scope_user_id_for_auth_identity("clerk", "queue_author");
    let invitee_id = crate::db::scope_user_id_for_auth_identity("clerk", "queue_invitee");
    for user in [
        test_user(&author_id, "queue-author", "queue-author@example.com"),
        test_user(&invitee_id, "queue-invitee", "queue-invitee@example.com"),
    ] {
        state.metadata.insert_user_for_tests(user).await.unwrap();
    }

    for id in ["req_draft_author", "req_draft_invited"] {
        create_public_request(&state, id, author_id.clone(), REQUEST_HEAD).await;
    }
    create_owner_request(&state, "req_ready_private", REQUEST_HEAD).await;
    create_owner_request(&state, "req_completed_private", REQUEST_HEAD).await;
    state
        .metadata
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_draft_invited".to_string(),
            actor_user_id: author_id.clone(),
            target_handle: "queue-invitee".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();

    create_public_request(&state, "req_ready_high", author_id.clone(), REQUEST_HEAD).await;
    ready_fixture(
        &state,
        "req_ready_high",
        25,
        30,
        1,
        "Needle title",
        "public ready body",
    )
    .await;
    create_public_request(&state, "req_ready_early", author_id.clone(), REQUEST_HEAD).await;
    ready_fixture(
        &state,
        "req_ready_early",
        10,
        10,
        2,
        "Early",
        "public ready body",
    )
    .await;
    create_public_request(&state, "req_ready_tie_a", author_id.clone(), REQUEST_HEAD).await;
    ready_fixture(
        &state,
        "req_ready_tie_a",
        10,
        20,
        3,
        "Tie A",
        "public ready body",
    )
    .await;
    create_public_request(&state, "req_ready_tie_b", author_id.clone(), REQUEST_HEAD).await;
    ready_fixture(
        &state,
        "req_ready_tie_b",
        10,
        20,
        4,
        "Tie B",
        "public ready body",
    )
    .await;
    ready_fixture(
        &state,
        "req_ready_private",
        0,
        5,
        5,
        "Private needle",
        "private ready needle",
    )
    .await;
    create_public_request(&state, "req_completed_old", author_id.clone(), REQUEST_HEAD).await;
    completed_fixture(
        &state,
        "req_completed_old",
        10,
        30,
        6,
        "Old public",
        "ordinary history",
    )
    .await;
    create_public_request(&state, "req_completed_new", author_id.clone(), REQUEST_HEAD).await;
    completed_fixture(
        &state,
        "req_completed_new",
        11,
        40,
        7,
        "New public",
        "needle history",
    )
    .await;
    completed_fixture(
        &state,
        "req_completed_private",
        12,
        50,
        8,
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

    for section in ["your_work", "ready", "completed"] {
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
            "/v1/repos/owner/repo/requests/queue?section=ready&limit=2",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&first), ["req_ready_high", "req_ready_early"]);
    let cursor = first["next_cursor"].as_str().unwrap();
    create_public_request(
        &state,
        "req_ready_new_priority",
        author_id.clone(),
        REQUEST_HEAD,
    )
    .await;
    ready_fixture(
        &state,
        "req_ready_new_priority",
        25,
        31,
        9,
        "New priority",
        "created after cursor",
    )
    .await;
    create_public_request(
        &state,
        "req_ready_new_tail",
        author_id.clone(),
        REQUEST_HEAD,
    )
    .await;
    ready_fixture(
        &state,
        "req_ready_new_tail",
        1,
        32,
        10,
        "New tail",
        "created after cursor",
    )
    .await;
    state
        .metadata
        .mutate_request_for_tests("req_ready_tie_a", |request| {
            request.ready_queue_version = Some(11);
            request.current_stake_credits = 25;
            request.ready_at_unix = Some(33);
            request.updated_at_unix = 33;
        })
        .await
        .unwrap();
    let second = response_json(
        api_request(
            app.clone(),
            "GET",
            &format!("/v1/repos/owner/repo/requests/queue?section=ready&limit=2&cursor={cursor}"),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(request_ids(&second), ["req_ready_tie_b"]);
    assert!(second["next_cursor"].is_null());

    let completed = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=completed",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        request_ids(&completed),
        ["req_completed_new", "req_completed_old"]
    );
    let maintainer_completed = response_json(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/queue?section=completed",
            Some(&bearer_header()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        request_ids(&maintainer_completed),
        [
            "req_completed_private",
            "req_completed_new",
            "req_completed_old"
        ]
    );

    for (section, expected) in [
        ("ready", vec!["req_ready_high"]),
        ("completed", vec!["req_completed_new"]),
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
        "/v1/repos/owner/repo/requests/queue?section=completed&cursor=v1:ready:1:25:30:req_ready_high".to_string(),
        "/v1/repos/owner/repo/requests/queue?section=ready&cursor=v1:ready:9223372036854775808:2147483648:1:req".to_string(),
        format!(
            "/v1/repos/owner/repo/requests/queue?section=ready&search={}",
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
    assert_eq!(public_repo["ready_for_review_count"], 6);
    assert!(public_repo.get("open_request_count").is_none());
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
    assert_eq!(maintainer_repo["ready_for_review_count"], 7);
}

async fn ready_fixture(
    state: &AppState,
    request_id: &str,
    stake: u32,
    ready_at_unix: u64,
    ready_queue_version: u64,
    title: &str,
    description: &str,
) {
    state
        .metadata
        .mutate_request_for_tests(request_id, |request| {
            request.title = title.to_string();
            request.description_markdown = description.to_string();
            request.state = RequestState::ReadyForReview;
            request.ready_queue_version = Some(ready_queue_version);
            request.current_stake_credits = stake;
            request.first_ready_at_unix = Some(ready_at_unix);
            request.ready_at_unix = Some(ready_at_unix);
            request.updated_at_unix = ready_at_unix;
        })
        .await
        .unwrap();
}

async fn completed_fixture(
    state: &AppState,
    request_id: &str,
    ready_at_unix: u64,
    completed_at_unix: u64,
    ready_queue_version: u64,
    title: &str,
    description: &str,
) {
    state
        .metadata
        .mutate_request_for_tests(request_id, |request| {
            request.title = title.to_string();
            request.description_markdown = description.to_string();
            request.state = RequestState::Completed;
            request.ready_queue_version = Some(ready_queue_version);
            request.current_stake_credits = 0;
            request.first_ready_at_unix = Some(ready_at_unix);
            request.ready_at_unix = None;
            request.completed_at_unix = Some(completed_at_unix);
            request.completed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = completed_at_unix;
        })
        .await
        .unwrap();
}
