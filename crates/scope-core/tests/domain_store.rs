use scope_core::domain::{
    policy::{
        Principal, PrincipalKind, ScopePath, Visibility,
        Visibility::{Private, Public},
    },
    repo_collaboration::{CreateRepositoryInviteCommand, create_or_refresh_repository_invite},
    store::{
        FirstPushToken, FirstPushTokenStatus, MainPushMode,
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

fn add_member(repo: &mut StoredRepository, user_id: &str, can_push: bool) {
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: user_id.to_string(),
        permissions: RepositoryMemberPermissions {
            can_push,
            ..RepositoryMemberPermissions::default()
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
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
    for (user, mode) in [
        (TEST_OWNER_ID, MainPushMode::FirstPush),
        ("user_other", MainPushMode::Denied),
    ] {
        assert_eq!(repo.push_policy_for_user_id(user).mode, mode);
    }
}

#[test]
fn published_push_policy_uses_repository_permissions() {
    let mut repo = test_repo(Public);
    add_member(&mut repo, "user_member", true);

    for (user, mode) in [
        (TEST_OWNER_ID, MainPushMode::Published),
        ("user_member", MainPushMode::Published),
        ("user_other", MainPushMode::Denied),
    ] {
        assert_eq!(repo.push_policy_for_user_id(user).mode, mode);
    }
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

    for (now, status) in [
        (150, FirstPushTokenStatus::Active),
        (200, FirstPushTokenStatus::Expired),
    ] {
        assert_eq!(token.status_at(now), status);
    }

    token.used_at_unix = Some(175);
    assert_eq!(token.status_at(180), FirstPushTokenStatus::Used);
}

#[test]
fn unpublished_repo_is_owner_only_even_with_reader_membership() {
    let mut repo = test_repo(Public);
    repo.record.publication_state = Unpublished;
    add_member(&mut repo, "user_reader", false);
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
