use super::*;

#[tokio::test]
async fn setup_and_git_credential_reset_routes_are_gone() {
    let app = router(test_state_with_repo());

    for uri in [
        "/v1/repos/owner/repo/setup",
        "/v1/repos/owner/repo/setup-token",
        "/v1/repos/owner/repo/git-credential",
        "/v1/repos/owner/repo/files/visibility",
        "/v1/repos/owner/repo/settings",
        "/v1/repos/owner/repo/pending-import",
        "/v1/repos/owner/repo/review/file-diff",
        "/v1/repos/owner/repo/publish",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(AUTHORIZATION, bearer_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}
