use std::collections::BTreeMap;

use api::domain::{
    policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule},
    projection::SourceGraph,
    store::{
        AccountAccess, AppCatalog, CatalogError, FirstPushToken, FirstPushTokenStatus,
        RepoPublicationState, RepoRecord, RepoSettings, RepositoryInvite, RepositoryInviteState,
        RepositoryMember, RepositoryMemberPermissions, StoredRepository, UserAccount, app_catalog,
    },
};

const TEST_OWNER_ID: &str = "user_owner";
const TEST_OWNER_EMAIL: &str = "owner@example.com";
const TEST_REPO_OWNER: &str = "owner";
const TEST_REPO_NAME: &str = "repo";
const TEST_REPO_ID: &str = "owner/repo";

fn catalog_with_test_repo() -> AppCatalog {
    let owner = UserAccount {
        id: TEST_OWNER_ID.to_string(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    let repo = test_repo();

    AppCatalog {
        users: BTreeMap::from([(owner.id.clone(), owner)]),
        repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
        pending_repo_storage_deletions: Vec::new(),
        pending_source_blob_deletions: Vec::new(),
    }
}

fn test_repo() -> StoredRepository {
    StoredRepository {
        record: RepoRecord {
            id: TEST_REPO_ID.to_string(),
            owner_handle: TEST_REPO_OWNER.to_string(),
            name: TEST_REPO_NAME.to_string(),
            owner_user_id: TEST_OWNER_ID.to_string(),
            publication_state: RepoPublicationState::Published,
            default_visibility: Visibility::Public,
            change_version: 1,
        },
        settings: RepoSettings::default(),
        first_push_token: None,
        git_push_token: None,
        git_clone_tokens: Vec::new(),
        pending_import: None,
        policy: Policy::new(Visibility::Public),
        graph: SourceGraph {
            repo_id: TEST_REPO_ID.to_string(),
            commits: Vec::new(),
        },
        visibility_events: Vec::new(),
        git_snapshot: None,
        staged_update: None,
        members: Vec::new(),
        invitations: Vec::new(),
    }
}

#[test]
fn app_catalog_starts_empty() {
    let catalog = app_catalog();

    assert!(catalog.users.is_empty());
    assert!(catalog.repositories.is_empty());
    assert!(catalog.pending_source_blob_deletions.is_empty());
    assert!(
        catalog
            .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
            .is_none()
    );
}

#[test]
fn member_scope_user_principal_can_write_repo() {
    let catalog = catalog_with_test_repo();
    let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let principal = Principal {
        id: TEST_OWNER_ID.to_string(),
        kind: PrincipalKind::User,
    };

    assert_eq!(principal.id, TEST_OWNER_ID);
    assert_eq!(principal.kind, PrincipalKind::User);
    assert!(catalog.can_push(repo, &principal));
}

#[test]
fn create_repository_makes_private_owner_repo_pending_first_push() {
    let mut catalog = app_catalog();
    let owner = UserAccount {
        id: TEST_OWNER_ID.to_string(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    catalog.users.insert(owner.id.clone(), owner.clone());

    let repo = catalog
        .create_repository(&owner, "Draft.Repo", Visibility::Private)
        .unwrap()
        .clone();

    assert_eq!(repo.record.id, "owner/draft.repo");
    assert_eq!(
        repo.record.publication_state,
        RepoPublicationState::Unpublished
    );
    assert_eq!(repo.record.default_visibility, Visibility::Private);
    assert!(repo.graph.commits.is_empty());
    assert_eq!(
        repo.policy.effective_visibility(&ScopePath::root()),
        Visibility::Private
    );

    let principal = Principal {
        id: TEST_OWNER_ID.to_string(),
        kind: PrincipalKind::User,
    };
    assert!(catalog.can_read_path(&repo, &principal, &ScopePath::root()));
    assert!(!catalog.can_push(&repo, &principal));
    assert!(!catalog.can_read_path(&repo, &Principal::public(), &ScopePath::root()));
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
    let owner = UserAccount {
        id: TEST_OWNER_ID.to_string(),
        handle: TEST_REPO_OWNER.to_string(),
        email: TEST_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };

    catalog
        .create_repository(&owner, "scope", Visibility::Private)
        .unwrap();
    let error = catalog
        .create_repository(&owner, "SCOPE", Visibility::Private)
        .unwrap_err();

    assert!(matches!(error, CatalogError::RepositoryExists(id) if id == "owner/scope"));
}

#[test]
fn unpublished_repo_blocks_public_reads() {
    let catalog = catalog_with_test_repo();
    let mut repo = catalog
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .unwrap()
        .clone();
    repo.record.publication_state = RepoPublicationState::Unpublished;

    assert!(!catalog.can_read_path(&repo, &Principal::public(), &ScopePath::root()));
}

#[test]
fn pending_publish_repo_is_owner_only_even_with_reader_membership() {
    let mut catalog = catalog_with_test_repo();
    let reader = UserAccount {
        id: "user_reader".to_string(),
        handle: "reader".to_string(),
        email: "reader@example.com".to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    catalog.users.insert(reader.id.clone(), reader.clone());
    let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
    repo.record.publication_state = RepoPublicationState::Unpublished;
    repo.pending_import = Some(api::domain::store::PendingImport {
        default_branch: "main".to_string(),
        head_oid: "head".to_string(),
        tree_oid: "tree".to_string(),
        imported_at_unix: 1,
        git_snapshot: api::domain::store::SourceBlob {
            object_key: "snapshot".to_string(),
            sha256: "sha".to_string(),
            git_oid: "oid".to_string(),
            git_file_mode: api::domain::store::DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
        files: Vec::new(),
    });
    repo.members.push(RepositoryMember {
        repo_id: TEST_REPO_ID.to_string(),
        user_id: reader.id.clone(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let repo = repo.clone();

    let owner_principal = Principal {
        id: TEST_OWNER_ID.to_string(),
        kind: PrincipalKind::User,
    };
    let reader_principal = Principal {
        id: reader.id,
        kind: PrincipalKind::User,
    };

    assert!(catalog.can_read_path(&repo, &owner_principal, &ScopePath::root()));
    assert!(!catalog.can_read_path(&repo, &reader_principal, &ScopePath::root()));
    assert!(!catalog.can_read_path(&repo, &Principal::public(), &ScopePath::root()));
}

#[test]
fn pending_invite_does_not_grant_private_access() {
    let mut catalog = catalog_with_test_repo();
    let invited = UserAccount {
        id: "user_invited".to_string(),
        handle: "invited".to_string(),
        email: "invited@example.com".to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };
    catalog.users.insert(invited.id.clone(), invited);
    let repo = catalog.repositories.get_mut(TEST_REPO_ID).unwrap();
    repo.policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/private.txt").unwrap(),
        ))
        .unwrap();
    repo.invitations.push(RepositoryInvite {
        id: "invite_pending".to_string(),
        repo_id: TEST_REPO_ID.to_string(),
        invited_email: "invited@example.com".to_string(),
        invited_email_normalized: "invited@example.com".to_string(),
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: TEST_OWNER_ID.to_string(),
        state: RepositoryInviteState::Pending,
        token_hash: "sha256:invite".to_string(),
        created_at_unix: 1,
        updated_at_unix: 1,
        expires_at_unix: 100,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    });
    let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
    let principal = Principal {
        id: "user_invited".to_string(),
        kind: PrincipalKind::User,
    };

    assert!(!catalog.can_read_path(repo, &principal, &ScopePath::parse("/private.txt").unwrap(),));
}
