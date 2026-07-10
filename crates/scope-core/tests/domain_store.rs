use scope_core::domain::{
    policy::{
        Principal, PrincipalKind, ScopePath, Visibility,
        Visibility::{Private, Public},
    },
    repo_collaboration::{CreateRepositoryInviteCommand, create_or_refresh_repository_invite},
    store::{
        CatalogError, FirstPushToken, FirstPushTokenStatus, MainPushMode,
        RepoPublicationState::{Published, Unpublished},
        RepositoryMember, RepositoryMemberPermissions, StoredRepository, UserAccount, app_catalog,
    },
};

const TEST_OWNER_ID: &str = "user_owner";

fn test_owner() -> UserAccount {
    UserAccount {
        id: TEST_OWNER_ID.to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    }
}

fn test_repo(visibility: Visibility) -> StoredRepository {
    let mut repo = StoredRepository::new(&test_owner(), "repo", visibility).unwrap();
    repo.record.publication_state = Published;
    repo
}

fn user_principal(id: &str) -> Principal {
    Principal {
        id: id.to_string(),
        kind: PrincipalKind::User,
    }
}

#[test]
fn app_catalog_starts_empty() {
    let catalog = app_catalog();

    assert!(catalog.users.is_empty());
    assert!(catalog.repositories.is_empty());
    assert!(catalog.pending_source_blob_deletions.is_empty());
    assert!(catalog.repository("owner", "repo").is_none());
}

#[test]
fn member_scope_user_principal_can_write_repo() {
    let repo = test_repo(Public);
    let principal = user_principal(TEST_OWNER_ID);

    assert_eq!(principal.id, TEST_OWNER_ID);
    assert_eq!(principal.kind, PrincipalKind::User);
    assert!(repo.can_push(&principal));
}

#[test]
fn create_repository_makes_private_owner_repo_pending_first_push() {
    let mut catalog = app_catalog();
    let owner = test_owner();

    let repo = catalog
        .create_repository(&owner, "Draft.Repo", Private)
        .unwrap()
        .clone();

    assert_eq!(repo.record.id, "owner/draft.repo");
    assert_eq!(repo.record.publication_state, Unpublished);
    assert_eq!(repo.record.default_visibility, Private);
    assert!(repo.graph.commits.is_empty());
    let root = ScopePath::root();
    assert_eq!(repo.policy.effective_visibility(&root), Private);

    let principal = user_principal(TEST_OWNER_ID);
    assert!(repo.can_read_path(&principal, &root));
    assert!(!repo.can_push(&principal));
    assert!(!repo.can_read_path(&Principal::public(), &root));
    assert_eq!(
        repo.push_policy_for_user_id(TEST_OWNER_ID).mode,
        MainPushMode::FirstPush
    );
    assert_eq!(
        repo.push_policy_for_user_id("user_other").mode,
        MainPushMode::Denied
    );
}

#[test]
fn published_push_policy_uses_repository_permissions() {
    let mut repo = test_repo(Public);
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: "user_member".to_string(),
        permissions: RepositoryMemberPermissions {
            can_push: true,
            ..RepositoryMemberPermissions::default()
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });

    assert_eq!(
        repo.push_policy_for_user_id(TEST_OWNER_ID).mode,
        MainPushMode::Published
    );
    assert_eq!(
        repo.push_policy_for_user_id("user_member").mode,
        MainPushMode::Published
    );
    assert_eq!(
        repo.push_policy_for_user_id("user_other").mode,
        MainPushMode::Denied
    );
}

#[test]
fn first_push_token_reports_active_expired_and_used_shape() {
    let mut token = FirstPushToken {
        token_hash: "sha256:test".to_string(),
        secret: Some("scope_fp_test".to_string()),
        owner_user_id: TEST_OWNER_ID.to_string(),
        created_at_unix: 100,
        expires_at_unix: 200,
        used_at_unix: None,
    };

    assert_eq!(token.status_at(150), FirstPushTokenStatus::Active);
    assert_eq!(token.status_at(200), FirstPushTokenStatus::Expired);

    token.used_at_unix = Some(175);
    assert_eq!(token.status_at(180), FirstPushTokenStatus::Used);
}

#[test]
fn duplicate_owner_repo_name_is_rejected() {
    let mut catalog = app_catalog();
    let owner = test_owner();

    catalog.create_repository(&owner, "scope", Private).unwrap();
    let error = catalog
        .create_repository(&owner, "SCOPE", Private)
        .unwrap_err();

    assert!(matches!(error, CatalogError::RepositoryExists(id) if id == "owner/scope"));
}

#[test]
fn unpublished_repo_blocks_public_reads() {
    let mut repo = test_repo(Public);
    repo.record.publication_state = Unpublished;

    assert!(!repo.can_read_path(&Principal::public(), &ScopePath::root()));
}

#[test]
fn unpublished_repo_is_owner_only_even_with_reader_membership() {
    let mut repo = test_repo(Public);
    repo.record.publication_state = Unpublished;
    repo.members.push(RepositoryMember {
        repo_id: "owner/repo".to_string(),
        user_id: "user_reader".to_string(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let owner_principal = user_principal(TEST_OWNER_ID);
    let reader_principal = user_principal("user_reader");

    assert!(repo.can_read_path(&owner_principal, &ScopePath::root()));
    assert!(!repo.can_read_path(&reader_principal, &ScopePath::root()));
    assert!(!repo.can_read_path(&Principal::public(), &ScopePath::root()));
}

#[test]
fn pending_invite_does_not_grant_private_access() {
    let mut repo = test_repo(Private);
    let private_path = ScopePath::parse("/private.txt").unwrap();
    create_or_refresh_repository_invite(
        &mut repo,
        CreateRepositoryInviteCommand {
            id: "invite_pending".to_string(),
            invited_email: "invited@example.com".to_string(),
            invitee: None,
            owner: &test_owner(),
            permissions: RepositoryMemberPermissions::default(),
            token_hash: "sha256:invite".to_string(),
            now_unix: 1,
        },
    )
    .unwrap();
    let principal = user_principal("user_invited");

    assert!(!repo.can_read_path(&principal, &private_path));
}
