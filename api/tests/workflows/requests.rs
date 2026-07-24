use super::*;

mod helpers;
pub(super) use helpers::create_owner_request;

const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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
