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
    Unpublished,
    Published,
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

        self.role_for_principal(repo, principal).is_some() && repo.policy.can_read(principal, path)
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
    format!("{}/{}", owner.trim(), name.trim())
}

fn normalize_email(email: impl Into<String>) -> String {
    email.into().trim().to_ascii_lowercase()
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
        repo.record.publication_state = RepoPublicationState::Unpublished;

        assert!(!catalog.can_read_path(&repo, &Principal::public(), &ScopePath::root()));
    }

    #[test]
    fn pending_invite_does_not_grant_private_access() {
        let mut catalog = catalog_with_test_repo();
        let invited = UserAccount {
            id: "user_invited".to_string(),
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
