use super::*;

#[test]
fn shoo_token_verifies_issuer_audience_signature_expiration_and_pairwise_sub() {
    let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
    let identity = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap();

    assert_eq!(identity.pairwise_sub, TEST_PAIRWISE_SUB);
    assert_eq!(identity.email.as_deref(), Some(TEST_OWNER_EMAIL));
    assert!(identity.email_verified);
}

#[test]
fn shoo_token_rejects_wrong_audience() {
    let jwt = token("origin:https://other.example", TEST_PAIRWISE_SUB, true);
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn shoo_token_requires_issuer_and_audience_claims() {
    let jwt = token_without_origin_claims();
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn shoo_token_requires_pairwise_sub() {
    let jwt = token("origin:http://localhost:3000", "", true);
    let error = verify_shoo_token(
        &jwt,
        &test_jwks(),
        SHOO_ISSUER,
        "origin:http://localhost:3000",
    )
    .unwrap_err();

    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
}

#[test]
fn verified_email_reuses_account_when_pairwise_sub_changes() {
    let state = AppState::test_state();
    let first_identity = ShooIdentity {
        pairwise_sub: "shoo-v1".to_string(),
        email: Some("Owner@Example.com".to_string()),
        email_verified: true,
    };
    let changed_identity = ShooIdentity {
        pairwise_sub: "shoo-v2".to_string(),
        email: Some(" owner@example.com ".to_string()),
        email_verified: true,
    };

    let first = ensure_user_for_identity(&state, &first_identity).unwrap();
    let changed = ensure_user_for_identity(&state, &changed_identity).unwrap();

    assert_eq!(changed.id, first.id);
    assert_eq!(changed.handle, "owner");
    assert_eq!(changed.email, TEST_OWNER_EMAIL);

    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert!(catalog.users.contains_key(&first.id));
}

#[test]
fn unverified_email_does_not_merge_into_verified_account() {
    let state = AppState::test_state();
    let verified_identity = ShooIdentity {
        pairwise_sub: "verified-shoo-user".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    let unverified_identity = ShooIdentity {
        pairwise_sub: "unverified-shoo-user".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: false,
    };

    let verified = ensure_user_for_identity(&state, &verified_identity).unwrap();
    let unverified = ensure_user_for_identity(&state, &unverified_identity).unwrap();

    assert_ne!(unverified.id, verified.id);
    assert!(!unverified.email_verified);

    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 2);
    drop(catalog);

    let later_verified_identity = ShooIdentity {
        pairwise_sub: "unverified-shoo-user".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    let merged = ensure_user_for_identity(&state, &later_verified_identity).unwrap();

    assert_eq!(merged.id, verified.id);
    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert!(catalog.users.contains_key(&verified.id));
    assert!(!catalog.users.contains_key(&unverified.id));
}

#[test]
fn verified_email_sign_in_collapses_existing_duplicate_user() {
    let canonical_identity = ShooIdentity {
        pairwise_sub: "canonical-shoo-user".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    let duplicate_identity = ShooIdentity {
        pairwise_sub: "duplicate-shoo-user".to_string(),
        email: Some(TEST_OWNER_EMAIL.to_string()),
        email_verified: true,
    };
    let canonical_id = identity_user_id(&canonical_identity);
    let duplicate_id = identity_user_id(&duplicate_identity);
    let canonical = UserAccount {
        id: canonical_id.clone(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let duplicate = UserAccount {
        id: duplicate_id.clone(),
        handle: format!("{TEST_REPO_OWNER}-2"),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let mut repo = test_repo(&canonical_id);
    repo.memberships.push(RepoMembership {
        repo_id: TEST_REPO_ID.to_string(),
        user_id: duplicate_id.clone(),
        role: RepoRole::Writer,
    });
    repo.graph.commits.push(LogicalCommit {
        id: "rv_duplicate".to_string(),
        parent_ids: Vec::new(),
        author_id: duplicate_id.clone(),
        author_visibility: AuthorVisibility::Visible,
        message: "duplicate user commit".to_string(),
        mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
        changes: Vec::new(),
    });
    let state = test_state_with_metadata(crate::db::MetadataStore::memory(AppCatalog {
        users: BTreeMap::from([
            (canonical_id.clone(), canonical),
            (duplicate_id.clone(), duplicate),
        ]),
        repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
        pending_repo_storage_deletions: Vec::new(),
        pending_source_blob_deletions: Vec::new(),
    }));

    let user = ensure_user_for_identity(&state, &duplicate_identity).unwrap();

    assert_eq!(user.id, canonical_id);
    let catalog = state.metadata.test_catalog().unwrap();
    assert_eq!(catalog.users.len(), 1);
    assert!(catalog.users.contains_key(&canonical_id));
    assert!(!catalog.users.contains_key(&duplicate_id));
    let repo = catalog.repositories.get(TEST_REPO_ID).unwrap();
    assert_eq!(repo.memberships.len(), 1);
    assert_eq!(repo.memberships[0].user_id, canonical_id);
    assert_eq!(repo.memberships[0].role, RepoRole::Owner);
    assert_eq!(repo.graph.commits[0].author_id, canonical_id);
}

#[tokio::test]
async fn shoo_verifier_requires_configured_audience() {
    let verifier = ShooVerifier::new(SHOO_ISSUER, None, "http://127.0.0.1/.well-known/jwks.json");
    let jwt = token("origin:http://localhost:3000", TEST_PAIRWISE_SUB, true);
    let error = verifier.verify(&jwt).await.unwrap_err();

    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}
