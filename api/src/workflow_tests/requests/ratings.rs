use super::*;

#[tokio::test]
async fn terminal_participants_rate_each_other_once_and_reasons_follow_request_visibility() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "rating_author");
    let stranger_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "rating_stranger");
    for user in [
        test_user(&author_id, "rating-author", "rating-author@example.com"),
        test_user(
            &stranger_id,
            "rating-stranger",
            "rating-stranger@example.com",
        ),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }
    create_public_request(&state, "req_ratings", author_id.clone(), REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_ratings", |request| {
            request.submitted_at_unix = Some(4);
            request.closed_at_unix = Some(5);
            request.closed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = 5;
        })
        .await
        .unwrap();
    let app = router(state);
    let author = bearer_header_for("rating_author", "rating-author@example.com");
    let stranger = bearer_header_for("rating_stranger", "rating-stranger@example.com");
    let uri = "/v1/repos/owner/repo/requests/req_ratings/ratings";

    let anonymous = response_json(api_request(app.clone(), "GET", uri, None, None).await).await;
    assert!(anonymous["ratings"].as_array().unwrap().is_empty());
    assert!(anonymous["eligible_subject"].is_null());

    assert_eq!(
        api_request(
            app.clone(),
            "POST",
            uri,
            Some(&stranger),
            Some(r#"{"score":5,"reason":"Not my request"}"#),
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );

    let author_rating = response_json(
        api_request(
            app.clone(),
            "POST",
            uri,
            Some(&author),
            Some(r#"{"score":5,"reason":"  Fast and clear review  "}"#),
        )
        .await,
    )
    .await;
    assert_eq!(author_rating["subject"]["id"], test_owner_id());
    assert_eq!(author_rating["reason"], "Fast and clear review");
    assert_eq!(
        api_request(
            app.clone(),
            "POST",
            uri,
            Some(&author),
            Some(r#"{"score":4,"reason":"Second rating"}"#),
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );

    let owner_rating = response_json(
        api_request(
            app.clone(),
            "POST",
            uri,
            Some(&bearer_header()),
            Some(r#"{"score":4,"reason":"Useful contribution"}"#),
        )
        .await,
    )
    .await;
    assert_eq!(owner_rating["subject"]["id"], author_id);

    let public = response_json(api_request(app, "GET", uri, None, None).await).await;
    assert_eq!(public["ratings"].as_array().unwrap().len(), 2);
    assert!(
        public["ratings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rating| rating["reason"] == "Fast and clear review")
    );
    assert!(public["eligible_subject"].is_null());
}
