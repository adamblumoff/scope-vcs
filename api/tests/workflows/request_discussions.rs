use super::*;
use tokio_stream::StreamExt;

#[tokio::test]
async fn threaded_discussion_http_workflow_preserves_activity_and_read_contracts() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(&bearer),
        Some(r#"{"name":"fix-parser-crash","audience":"Public"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let started = response_json(started).await;
    let request_id = started["request"]["id"].as_str().unwrap();
    let base = format!("/v1/repos/owner/repo/requests/{request_id}");

    let anonymous_create = api_request(
        app.clone(),
        "POST",
        &format!("{base}/timeline"),
        None,
        Some(r#"{"body_markdown":"No auth","client_discussion_id":"anonymous"}"#),
    )
    .await;
    assert_eq!(anonymous_create.status(), StatusCode::UNAUTHORIZED);

    let description = api_request(
        app.clone(),
        "PATCH",
        &format!("{base}/description"),
        Some(&bearer),
        Some(r###"{"description_markdown":"## Intent\nFix parser ownership."}"###),
    )
    .await;
    assert_eq!(description.status(), StatusCode::OK);
    let description = response_json(description).await;
    assert_eq!(
        description["request"]["description_markdown"],
        "## Intent\nFix parser ownership."
    );

    let events = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/events",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(events.status(), StatusCode::OK);
    let mut event_stream = events.into_body().into_data_stream();
    let connected = event_stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8(connected.to_vec())
            .unwrap()
            .contains("Connected")
    );

    let created = api_request(
        app.clone(),
        "POST",
        &format!("{base}/timeline"),
        Some(&bearer),
        Some(r#"{"body_markdown":"Who owns parser recovery?","client_discussion_id":"root-1"}"#),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created = response_json(created).await;
    let discussion_id = created["discussion"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["discussion"]["client_discussion_id"], "root-1");
    assert_eq!(created["discussion"]["unread_count"], 0);
    let targeted = tokio::time::timeout(std::time::Duration::from_secs(5), event_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let targeted = String::from_utf8(targeted.to_vec()).unwrap();
    assert!(targeted.contains("RequestTimelineChanged"));
    assert!(targeted.contains(&discussion_id));
    assert!(targeted.contains(request_id));

    let retried = api_request(
        app.clone(),
        "POST",
        &format!("{base}/timeline"),
        Some(&bearer),
        Some(r#"{"body_markdown":"Who owns parser recovery?","client_discussion_id":"root-1"}"#),
    )
    .await;
    assert_eq!(retried.status(), StatusCode::OK);
    let retried = response_json(retried).await;
    assert_eq!(retried["discussion"]["id"], discussion_id);

    let reply = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/replies"),
        Some(&bearer),
        Some(r#"{"body_markdown":"The parser module should own it.","client_reply_id":"reply-1","reply_to_reply_id":null}"#),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let reply = response_json(reply).await;
    assert_eq!(reply["discussion"]["reply_count"], 1);
    assert_eq!(reply["discussion"]["unread_count"], 0);

    let resolved = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/resolve"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(resolved.status(), StatusCode::OK);
    let resolved = response_json(resolved).await;
    assert_eq!(resolved["discussion"]["status"], "Resolved");
    assert_eq!(resolved["discussion"]["unread_count"], 0);

    let rejected_reply = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/replies"),
        Some(&bearer),
        Some(r#"{"body_markdown":"One more point.","client_reply_id":"reply-rejected","reply_to_reply_id":null}"#),
    )
    .await;
    assert_eq!(rejected_reply.status(), StatusCode::CONFLICT);

    let reopened = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/reopen-and-reply"),
        Some(&bearer),
        Some(r#"{"body_markdown":"One more point.","client_reply_id":"reply-2","reply_to_reply_id":null}"#),
    )
    .await;
    assert_eq!(reopened.status(), StatusCode::OK);
    let reopened = response_json(reopened).await;
    assert_eq!(reopened["discussion"]["status"], "Open");
    assert_eq!(reopened["discussion"]["reply_count"], 2);
    assert_eq!(reopened["discussion"]["unread_count"], 0);
    let through_position = reopened["reply"]["position"].as_u64().unwrap();

    let read = api_request(
        app.clone(),
        "PUT",
        &format!("{base}/threads/{discussion_id}/read"),
        Some(&bearer),
        Some(&format!(r#"{{"through_position":{through_position}}}"#)),
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(
        response_json(read).await["read_through_position"],
        through_position
    );

    let discussions = api_request(
        app.clone(),
        "GET",
        &format!("{base}/timeline?limit=25"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(discussions.status(), StatusCode::OK);
    let discussions = response_json(discussions).await;
    assert_eq!(discussions["discussions"].as_array().unwrap().len(), 1);
    assert_eq!(
        discussions["discussions"][0]["client_discussion_id"],
        "root-1"
    );
    assert_eq!(
        discussions["discussions"][0]["latest_replies"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let replies = api_request(
        app.clone(),
        "GET",
        &format!("{base}/threads/{discussion_id}/replies?limit=50"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(replies.status(), StatusCode::OK);
    assert_eq!(
        response_json(replies).await["replies"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let changes = api_request(
        app.clone(),
        "GET",
        &format!("{base}/timeline/changes?after=0&limit=100"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(changes.status(), StatusCode::OK);
    let changes = response_json(changes).await;
    assert_eq!(changes["discussions"].as_array().unwrap().len(), 1);
    assert_eq!(changes["discussions"][0]["client_discussion_id"], "root-1");
    assert_eq!(changes["has_more"], false);

    let activity = api_request(
        app.clone(),
        "GET",
        &format!("{base}/activity?after=0&limit=100"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(activity.status(), StatusCode::OK);
    let activity = response_json(activity).await;
    let kinds = activity["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            "Started",
            "DescriptionEdited",
            "DiscussionResolved",
            "DiscussionReopened",
        ]
    );

    let latest_activity = api_request(
        app,
        "GET",
        &format!("{base}/activity?latest=true&limit=2"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(latest_activity.status(), StatusCode::OK);
    let latest_activity = response_json(latest_activity).await;
    let kinds = latest_activity["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec!["DiscussionResolved", "DiscussionReopened"]);
}

#[tokio::test]
async fn request_activity_clamps_latest_and_after_pages_to_fifty_events() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let request_id = "req_bounded_activity";
    super::requests::create_owner_request(
        &state,
        request_id,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;
    for index in 0..52 {
        state
            .metadata
            .update_request_description(crate::domain::requests::UpdateRequestDescriptionInput {
                request_id: request_id.to_string(),
                actor_user_id: test_owner_id(),
                actor_can_edit_description: false,
                event_id: format!("event_description_{index}"),
                description_markdown: format!("description {index}"),
                now_unix: 10 + index,
            })
            .await
            .unwrap();
    }
    let app = router(state);
    let base = format!("/v1/repos/owner/repo/requests/{request_id}");

    for query in ["after=0&limit=1000", "latest=true&limit=1000"] {
        let response = api_request(
            app.clone(),
            "GET",
            &format!("{base}/activity?{query}"),
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        assert!(bytes.len() < 900 * 1024, "{} bytes", bytes.len());
        let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["events"].as_array().unwrap().len(), 50);
    }
}

#[tokio::test]
async fn timeline_cursor_is_stable_during_concurrent_thread_creation_and_changes() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let app = router(state);
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(&bearer),
        Some(r#"{"name":"stable-discussion-pages","audience":"Public"}"#),
    )
    .await;
    let started = response_json(started).await;
    let request_id = started["request"]["id"].as_str().unwrap();
    let base = format!("/v1/repos/owner/repo/requests/{request_id}");

    let mut discussion_ids = Vec::new();
    for index in 1..=4 {
        let created = api_request(
            app.clone(),
            "POST",
            &format!("{base}/timeline"),
            Some(&bearer),
            Some(&format!(
                r#"{{"body_markdown":"Root {index}","client_discussion_id":"root-{index}"}}"#
            )),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        discussion_ids.push(
            response_json(created).await["discussion"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    let first_page = api_request(
        app.clone(),
        "GET",
        &format!("{base}/timeline?limit=2"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = response_json(first_page).await;
    let cursor = first_page["next_cursor"].as_str().unwrap();
    assert_eq!(first_page["discussions"].as_array().unwrap().len(), 2);
    let first_page_ids = first_page["discussions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|discussion| discussion["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        first_page_ids,
        vec![discussion_ids[3].as_str(), discussion_ids[2].as_str()]
    );

    let concurrent_root = api_request(
        app.clone(),
        "POST",
        &format!("{base}/timeline"),
        Some(&bearer),
        Some(r#"{"body_markdown":"Concurrent root","client_discussion_id":"root-concurrent"}"#),
    )
    .await;
    assert_eq!(concurrent_root.status(), StatusCode::OK);
    let concurrent_root = response_json(concurrent_root).await;
    let concurrent_root_id = concurrent_root["discussion"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let oldest_id = &discussion_ids[0];
    let reply = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{oldest_id}/replies"),
        Some(&bearer),
        Some(
            r#"{"body_markdown":"Concurrent activity","client_reply_id":"concurrent-reply","reply_to_reply_id":null}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);

    let resolved_id = &discussion_ids[1];
    let resolved = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{resolved_id}/resolve"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(resolved.status(), StatusCode::OK);

    let second_page = api_request(
        app,
        "GET",
        &format!("{base}/timeline?limit=2&cursor={cursor}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page = response_json(second_page).await;
    let ids = second_page["discussions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|discussion| discussion["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![resolved_id.as_str(), oldest_id.as_str()]);
    let paged_ids = first_page_ids.into_iter().chain(ids).collect::<Vec<_>>();
    assert_eq!(
        paged_ids,
        discussion_ids
            .iter()
            .rev()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        paged_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        paged_ids.len()
    );
    assert!(!paged_ids.contains(&concurrent_root_id.as_str()));
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

#[tokio::test]
async fn discussion_changes_report_complete_pages_without_skipping_the_extra_row() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let app = router(state);
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(&bearer),
        Some(r#"{"name":"complete-discussion-changes","audience":"Public"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let started = response_json(started).await;
    let request_id = started["request"]["id"].as_str().unwrap();
    let base = format!("/v1/repos/owner/repo/requests/{request_id}");

    let mut positions = Vec::new();
    for index in 1..=3 {
        let created = api_request(
            app.clone(),
            "POST",
            &format!("{base}/timeline"),
            Some(&bearer),
            Some(&format!(
                r#"{{"body_markdown":"Root {index}","client_discussion_id":"root-{index}"}}"#
            )),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        positions.push(
            response_json(created).await["discussion"]["last_activity_position"]
                .as_u64()
                .unwrap(),
        );
    }

    let first_page = api_request(
        app.clone(),
        "GET",
        &format!("{base}/timeline/changes?after=0&limit=2"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = response_json(first_page).await;
    assert_eq!(first_page["discussions"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["through_position"], positions[1]);
    assert_eq!(first_page["has_more"], true);

    let second_page = api_request(
        app,
        "GET",
        &format!(
            "{base}/timeline/changes?after={}&limit=2",
            first_page["through_position"].as_u64().unwrap()
        ),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page = response_json(second_page).await;
    assert_eq!(second_page["discussions"].as_array().unwrap().len(), 1);
    assert_eq!(second_page["through_position"], positions[2]);
    assert_eq!(second_page["has_more"], false);
}
