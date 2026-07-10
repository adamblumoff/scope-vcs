use super::*;
use crate::domain::requests::{
    RecordWorkingRequestUploadInput, RequestActorRole, RequestBaseAudience, StartRequestInput,
    SubmitRequestInput, canonical_request_ref,
};

#[tokio::test]
async fn create_repo_route_creates_user_and_lists_repo() {
    let state = test_state_with_jwks();
    let app = router(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Scope_App"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["repo"]["id"], "owner/scope_app");
    assert_eq!(body["repo"]["owner_handle"], "owner");
    assert_eq!(body["repo"]["lifecycle_state"], "Unpublished");
    assert_eq!(body["repo"]["default_visibility"], "Private");
    assert_eq!(body["repo"]["access"]["actor"], "Owner");
    assert_eq!(body["repo"]["open_request_count"], 0);
    assert_eq!(
        body["repo"]["request_permissions"]["uses_credit_stake"],
        false
    );
    assert_eq!(
        body["init"]["git_remote_url"],
        "http://localhost:8080/git/permissioned/owner/scope_app"
    );
    assert_eq!(body["init"]["remote_name"], "scope");
    assert_eq!(body["init"]["push_branch"], DEFAULT_GIT_BRANCH);
    let secret = body["init"]["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    assert_eq!(body["init"]["token"]["status"], "Active");
    let push_secret = body["init"]["push_token"]["secret"].as_str().unwrap();
    assert!(push_secret.starts_with("scope_git_"));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "owner/scope_app");
    assert_eq!(body[0]["lifecycle_state"], "Unpublished");
    assert_eq!(body[0]["access"]["actor"], "Owner");

    let catalog = lock_catalog(&state).unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert_eq!(catalog.repositories.len(), 1);
    let repo = catalog.repositories.get("owner/scope_app").unwrap();
    let token = repo.first_push_token.as_ref().unwrap();
    assert_ne!(token.token_hash, secret);
    assert!(token.secret.is_none());
    assert!(token.token_hash.starts_with("sha256:"));
    assert_eq!(token.owner_user_id, test_owner_id());
    assert_eq!(
        token.expires_at_unix - token.created_at_unix,
        FIRST_PUSH_TOKEN_TTL_SECS
    );
    let push_token = repo.git_push_token.as_ref().unwrap();
    assert_ne!(push_token.token_hash, push_secret);
    assert!(push_token.token_hash.starts_with("sha256:"));
    assert_eq!(push_token.owner_user_id, test_owner_id());
}

#[tokio::test]
async fn list_repos_returns_request_summary_fields() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);

    let app = router(state);
    let summary_response = app
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

    assert_eq!(summary_response.status(), StatusCode::OK);
    let summary_body = response_json(summary_response).await;
    assert_eq!(summary_body["id"], TEST_REPO_ID);
    assert_eq!(summary_body["access"]["actor"], "Public");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body[0]["id"], TEST_REPO_ID);
    assert_eq!(body[0]["lifecycle_state"], "Published");
    assert_eq!(body[0]["open_request_count"], 0);
    assert_eq!(body[0]["request_permissions"]["uses_credit_stake"], false);
}

#[tokio::test]
async fn get_repo_route_returns_owner_summary() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], TEST_REPO_ID);
    assert_eq!(body["owner_handle"], TEST_REPO_OWNER);
    assert_eq!(body["name"], TEST_REPO_NAME);
    assert_eq!(body["access"]["actor"], "Owner");
    assert_eq!(body["change_version"], 1);
}

#[tokio::test]
async fn get_repo_route_hides_change_version_from_public_reader() {
    let state = test_state_with_repo();
    {
        let mut repo = repo_with_readme();
        repo.bump_change_version();
        repo.bump_change_version();
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["access"]["actor"], "Public");
    assert_eq!(body["change_version"], 0);
}

#[tokio::test]
async fn list_repos_route_requires_sign_in() {
    let response = router(test_state_with_jwks())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn member_management_hides_private_repo_from_unrelated_users() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    {
        let mut repo = repo_with_readme();
        repo.record.default_visibility = Visibility::Private;
        repo.policy = Policy::new(Visibility::Private);
        repo.graph.commits[0].changes[0].visibility = Visibility::Private;

        let mut catalog = lock_catalog(&state).unwrap();
        catalog.repositories.insert(TEST_REPO_ID.to_string(), repo);
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos/owner/repo/members")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_other", "other@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn accept_invite_returns_open_request_count() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .start_request(StartRequestInput {
            id: "req_invite_count".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            author_user_id: test_owner_id(),
            title: "Open owner request".to_string(),
            author_role: RequestActorRole::Owner,
            base_audience: RequestBaseAudience::Private,
            target_branch: DEFAULT_GIT_BRANCH.to_string(),
            request_ref: canonical_request_ref("req_invite_count"),
            base_main_oid: "base_main".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    state
        .metadata
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: "req_invite_count".to_string(),
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            git_snapshot: source_blob("invite count request"),
            now_unix: 3,
        })
        .await
        .unwrap();
    state
        .metadata
        .submit_request(SubmitRequestInput {
            request_id: "req_invite_count".to_string(),
            actor_user_id: test_owner_id(),
            expected_head_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_invite_count_created".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let app = router(state);
    let invited_email = "invitee@example.com";
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/repos/owner/repo/invites")
                .header(AUTHORIZATION, bearer_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{
                        "email":"{invited_email}",
                        "permissions":{{
                            "can_push":false,
                            "can_change_file_visibility":false,
                            "can_apply_changes":false
                        }}
                    }}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = response_json(create_response).await;
    let invite_url = create_body["invite_url"].as_str().unwrap();
    let token = invite_url.rsplit('/').next().unwrap();

    let accept_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/repository-invites/{token}/accept"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_invitee", invited_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(accept_response.status(), StatusCode::OK);
    let body = response_json(accept_response).await;
    assert_eq!(body["repo"]["access"]["actor"], "Member");
    assert_eq!(body["repo"]["open_request_count"], 1);
}

#[tokio::test]
async fn accept_expired_invite_persists_expired_state() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "expired-invite-token";
    let token_hash = repository_invite_token_hash(token);
    let invited_email = "invitee@example.com";
    let expires_at_unix = unix_now().saturating_sub(1);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.invitations.push(RepositoryInvite {
            id: "invite_1".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            invited_email: invited_email.to_string(),
            invited_email_normalized: crate::domain::store::normalize_repository_invite_email(
                invited_email,
            ),
            permissions: RepositoryMemberPermissions::default(),
            invited_by_user_id: test_owner_id(),
            state: RepositoryInviteState::Pending,
            token_hash: token_hash.clone(),
            created_at_unix: expires_at_unix.saturating_sub(100),
            updated_at_unix: expires_at_unix.saturating_sub(100),
            expires_at_unix,
            accepted_by_user_id: None,
            accepted_at_unix: None,
            revoked_at_unix: None,
        });
    }

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/repository-invites/{token}/accept"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_invitee", invited_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let invite = repo
        .invitations
        .iter()
        .find(|invite| invite.token_hash == token_hash)
        .unwrap();
    assert_eq!(invite.state, RepositoryInviteState::Expired);
}

#[tokio::test]
async fn accept_invite_uses_current_clerk_email_for_existing_identity() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state.clone());
    let invited_email = "invitee@example.com";
    let token = "stale-email-invite-token";
    let token_hash = repository_invite_token_hash(token);
    let invitee_user_id = crate::db::scope_user_id_for_auth_identity("clerk", "user_invitee");

    let bootstrap_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/session")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_invitee", invited_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bootstrap_response.status(), StatusCode::OK);

    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.invitations.push(RepositoryInvite {
            id: "invite_stale_email".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            invited_email: invited_email.to_string(),
            invited_email_normalized: crate::domain::store::normalize_repository_invite_email(
                invited_email,
            ),
            permissions: RepositoryMemberPermissions::default(),
            invited_by_user_id: test_owner_id(),
            state: RepositoryInviteState::Pending,
            token_hash: token_hash.clone(),
            created_at_unix: unix_now(),
            updated_at_unix: unix_now(),
            expires_at_unix: unix_now() + 3600,
            accepted_by_user_id: None,
            accepted_at_unix: None,
            revoked_at_unix: None,
        });
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/repository-invites/{token}/accept"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_invitee", "renamed@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let invite = repo
        .invitations
        .iter()
        .find(|invite| invite.token_hash == token_hash)
        .unwrap();
    assert_eq!(invite.state, RepositoryInviteState::Pending);
    assert!(repo.member_for_user(&invitee_user_id).is_none());
}

#[tokio::test]
async fn owner_can_revoke_pending_invite_before_acceptance() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "revoked-invite-token";
    let token_hash = repository_invite_token_hash(token);
    let invited_email = "invitee@example.com";
    {
        let mut catalog = lock_catalog(&state).unwrap();
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.invitations.push(RepositoryInvite {
            id: "invite_revoke".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            invited_email: invited_email.to_string(),
            invited_email_normalized: crate::domain::store::normalize_repository_invite_email(
                invited_email,
            ),
            permissions: RepositoryMemberPermissions::default(),
            invited_by_user_id: test_owner_id(),
            state: RepositoryInviteState::Pending,
            token_hash: token_hash.clone(),
            created_at_unix: unix_now(),
            updated_at_unix: unix_now(),
            expires_at_unix: unix_now() + 600,
            accepted_by_user_id: None,
            accepted_at_unix: None,
            revoked_at_unix: None,
        });
    }

    let app = router(state.clone());
    let revoke_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/repos/owner/repo/invites/invite_revoke")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(revoke_response.status(), StatusCode::OK);
    let body = response_json(revoke_response).await;
    assert_eq!(body["state"], "Revoked");

    let accept_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/repository-invites/{token}/accept"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_invitee", invited_email),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(accept_response.status(), StatusCode::CONFLICT);
    let catalog = lock_catalog(&state).unwrap();
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    let invite = repo
        .invitations
        .iter()
        .find(|invite| invite.id == "invite_revoke")
        .unwrap();
    assert_eq!(invite.state, RepositoryInviteState::Revoked);
    assert!(invite.revoked_at_unix.is_some());
}

#[tokio::test]
async fn list_repos_route_hides_pending_repo_from_reader_member() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let reader_identity = ClerkIdentity {
        user_id: "user_reader".to_string(),
        email: Some("reader@example.com".to_string()),
        email_verified: true,
    };
    let reader_id = crate::db::scope_user_id_for_auth_identity("clerk", &reader_identity.user_id);
    {
        let mut catalog = lock_catalog(&state).unwrap();
        catalog.users.insert(
            reader_id.clone(),
            UserAccount {
                id: reader_id.clone(),
                handle: "reader".to_string(),
                email: "reader@example.com".to_string(),
                email_verified: true,
            },
        );
        let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
        repo.record.publication_state = RepoPublicationState::Unpublished;
        repo.members.push(test_repository_member(
            TEST_REPO_ID,
            reader_id,
            RepositoryMemberPermissions::default(),
        ));
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/repos")
                .header(
                    AUTHORIZATION,
                    bearer_header_for("user_reader", "reader@example.com"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}
