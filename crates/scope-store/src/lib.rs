use scope_policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility};
use scope_projection::SourceGraph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedEmail {
    pub email: String,
    pub verified: bool,
}

impl VerifiedEmail {
    pub fn new(email: impl Into<String>, verified: bool) -> Self {
        Self {
            email: normalize_email(email),
            verified,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountAccess {
    Public,
    Member,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub handle: String,
    pub email: String,
    pub email_verified: bool,
    pub access: AccountAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RepoRole {
    Reader,
    Writer,
    Maintainer,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoPublicationState {
    PendingFirstPush,
    PendingPublish,
    Published,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirstPushTokenStatus {
    Active,
    Expired,
    Used,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstPushToken {
    pub token_hash: String,
    pub owner_user_id: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub used_at_unix: Option<u64>,
}

impl FirstPushToken {
    pub fn status_at(&self, now_unix: u64) -> FirstPushTokenStatus {
        if self.used_at_unix.is_some() {
            FirstPushTokenStatus::Used
        } else if now_unix >= self.expires_at_unix {
            FirstPushTokenStatus::Expired
        } else {
            FirstPushTokenStatus::Active
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingImportFile {
    pub path: String,
    pub mode: String,
    pub oid: String,
    pub content_base64: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingImport {
    pub default_branch: String,
    pub head_oid: String,
    pub tree_oid: String,
    pub imported_at_unix: u64,
    pub files: Vec<PendingImportFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRecord {
    pub id: String,
    pub owner_handle: String,
    pub name: String,
    pub owner_user_id: String,
    pub publication_state: RepoPublicationState,
    pub default_visibility: Visibility,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSettings {
    pub include_ignored_files: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMembership {
    pub repo_id: String,
    pub user_id: String,
    pub role: RepoRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationState {
    Pending,
    Accepted,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInvitation {
    pub id: String,
    pub repo_id: String,
    pub invited_email: String,
    pub role: RepoRole,
    pub invited_by_user_id: String,
    pub state: InvitationState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRepository {
    pub record: RepoRecord,
    pub settings: RepoSettings,
    pub first_push_token: Option<FirstPushToken>,
    pub pending_import: Option<PendingImport>,
    pub policy: Policy,
    pub graph: SourceGraph,
    pub memberships: Vec<RepoMembership>,
    pub invitations: Vec<RepoInvitation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppCatalog {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, StoredRepository>,
}

impl AppCatalog {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&StoredRepository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn repositories_for_user(&self, user_id: &str) -> Vec<&StoredRepository> {
        self.repositories
            .values()
            .filter(|repo| {
                repo.memberships
                    .iter()
                    .any(|membership| membership.user_id == user_id)
            })
            .collect()
    }

    pub fn create_repository(
        &mut self,
        owner: &UserAccount,
        name: &str,
        default_visibility: Visibility,
    ) -> Result<&StoredRepository, CatalogError> {
        let name = validate_repo_name(name)?;
        let id = repo_id(&owner.handle, &name);
        if self.repositories.contains_key(&id) {
            return Err(CatalogError::RepositoryExists(id));
        }

        let repository = StoredRepository {
            record: RepoRecord {
                id: id.clone(),
                owner_handle: owner.handle.clone(),
                name,
                owner_user_id: owner.id.clone(),
                publication_state: RepoPublicationState::PendingFirstPush,
                default_visibility,
            },
            settings: RepoSettings::default(),
            first_push_token: None,
            pending_import: None,
            policy: Policy::new(default_visibility, owner.id.clone()),
            graph: SourceGraph {
                repo_id: id.clone(),
                commits: Vec::new(),
            },
            memberships: vec![RepoMembership {
                repo_id: id.clone(),
                user_id: owner.id.clone(),
                role: RepoRole::Owner,
            }],
            invitations: Vec::new(),
        };
        self.repositories.insert(id.clone(), repository);
        Ok(self.repositories.get(&id).expect("repository was inserted"))
    }

    pub fn verified_user_for_email(&self, email: &VerifiedEmail) -> Option<&UserAccount> {
        if !email.verified {
            return None;
        }

        self.users.values().find(|user| user.email == email.email)
    }

    pub fn principal_for_repo(
        &self,
        repo: &StoredRepository,
        identity: Option<&VerifiedEmail>,
    ) -> Principal {
        let Some(identity) = identity else {
            return Principal::public();
        };

        let Some(user) = self.verified_user_for_email(identity) else {
            return Principal::public();
        };

        if repo
            .memberships
            .iter()
            .any(|membership| membership.user_id == user.id)
        {
            Principal {
                id: user.id.clone(),
                kind: PrincipalKind::User,
            }
        } else {
            Principal::public()
        }
    }

    pub fn role_for_principal(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
    ) -> Option<RepoRole> {
        repo.memberships
            .iter()
            .find(|membership| membership.user_id == principal.id)
            .map(|membership| membership.role)
    }

    pub fn can_read_path(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
        path: &ScopePath,
    ) -> bool {
        if principal.kind == PrincipalKind::Public {
            return repo.record.publication_state == RepoPublicationState::Published
                && repo.policy.can_read(principal, path);
        }

        let Some(role) = self.role_for_principal(repo, principal) else {
            return false;
        };

        let lifecycle_allows_read = repo.record.publication_state
            == RepoPublicationState::Published
            || role == RepoRole::Owner;

        lifecycle_allows_read && repo.policy.can_read(principal, path)
    }

    pub fn can_write_path(
        &self,
        repo: &StoredRepository,
        principal: &Principal,
        path: &ScopePath,
    ) -> bool {
        self.role_for_principal(repo, principal)
            .is_some_and(|role| role >= RepoRole::Writer)
            && self.can_read_path(repo, principal, path)
    }
}

pub fn app_catalog() -> AppCatalog {
    AppCatalog::default()
}

pub fn repo_id(owner: &str, name: &str) -> String {
    format!(
        "{}/{}",
        owner.trim().to_ascii_lowercase(),
        name.trim().to_ascii_lowercase()
    )
}

fn normalize_email(email: impl Into<String>) -> String {
    email.into().trim().to_ascii_lowercase()
}

fn validate_repo_name(name: &str) -> Result<String, CatalogError> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name is required".to_string(),
        ));
    }
    if name == "." || name == ".." {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name cannot be . or ..".to_string(),
        ));
    }
    if name.len() > 80 {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name must be 80 characters or fewer".to_string(),
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(CatalogError::InvalidRepositoryName(
            "repository name can only use letters, numbers, dots, dashes, or underscores"
                .to_string(),
        ));
    }

    Ok(name)
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("{0}")]
    InvalidRepositoryName(String),
    #[error("repo {0} already exists")]
    RepositoryExists(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_policy::VisibilityRule;

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
            },
            settings: RepoSettings::default(),
            first_push_token: None,
            pending_import: None,
            policy: Policy::new(Visibility::Public, TEST_OWNER_ID),
            graph: SourceGraph {
                repo_id: TEST_REPO_ID.to_string(),
                commits: Vec::new(),
            },
            memberships: vec![RepoMembership {
                repo_id: TEST_REPO_ID.to_string(),
                user_id: TEST_OWNER_ID.to_string(),
                role: RepoRole::Owner,
            }],
            invitations: Vec::new(),
        }
    }

    #[test]
    fn app_catalog_starts_empty() {
        let catalog = app_catalog();

        assert!(catalog.users.is_empty());
        assert!(catalog.repositories.is_empty());
        assert!(
            catalog
                .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
                .is_none()
        );
    }

    #[test]
    fn verified_member_email_becomes_repo_principal() {
        let catalog = catalog_with_test_repo();
        let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = VerifiedEmail::new("Owner@Example.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal.id, TEST_OWNER_ID);
        assert_eq!(principal.kind, PrincipalKind::User);
        assert!(catalog.can_write_path(repo, &principal, &ScopePath::root()));
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
            RepoPublicationState::PendingFirstPush
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
        assert!(catalog.can_write_path(&repo, &principal, &ScopePath::root()));
        assert!(!catalog.can_read_path(&repo, &Principal::public(), &ScopePath::root()));
    }

    #[test]
    fn first_push_token_reports_active_expired_and_used_shape() {
        let mut token = FirstPushToken {
            token_hash: "sha256:test".to_string(),
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
    fn unverified_email_stays_public() {
        let catalog = catalog_with_test_repo();
        let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = VerifiedEmail::new(TEST_OWNER_EMAIL, false);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn unknown_verified_user_defaults_to_public() {
        let catalog = catalog_with_test_repo();
        let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = VerifiedEmail::new("someone@example.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn unpublished_repo_blocks_public_reads() {
        let catalog = catalog_with_test_repo();
        let mut repo = catalog
            .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
            .unwrap()
            .clone();
        repo.record.publication_state = RepoPublicationState::PendingPublish;

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
        repo.record.publication_state = RepoPublicationState::PendingPublish;
        repo.memberships.push(RepoMembership {
            repo_id: TEST_REPO_ID.to_string(),
            user_id: reader.id.clone(),
            role: RepoRole::Reader,
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
                [TEST_OWNER_ID.to_string()],
            ))
            .unwrap();
        repo.invitations.push(RepoInvitation {
            id: "invite_pending".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            invited_email: "invited@example.com".to_string(),
            role: RepoRole::Reader,
            invited_by_user_id: TEST_OWNER_ID.to_string(),
            state: InvitationState::Pending,
        });
        let repo = catalog.repository(TEST_REPO_OWNER, TEST_REPO_NAME).unwrap();
        let identity = VerifiedEmail::new("invited@example.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
        assert!(!catalog.can_read_path(
            repo,
            &principal,
            &ScopePath::parse("/private.txt").unwrap(),
        ));
    }
}
