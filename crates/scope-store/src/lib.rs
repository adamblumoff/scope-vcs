use scope_policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule};
use scope_projection::{
    AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, SourceGraph,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const POSTGRES_SCHEMA: &str = include_str!("schema.sql");

pub const BOOTSTRAP_OWNER_EMAIL: &str = "adamblumoff@gmail.com";
pub const BOOTSTRAP_OWNER_USER_ID: &str = "user_adamblumoff";
pub const BOOTSTRAP_REPO_OWNER: &str = "adamblumoff";
pub const BOOTSTRAP_REPO_NAME: &str = "scope-vcs";
pub const BOOTSTRAP_REPO_ID: &str = "adamblumoff/scope-vcs";

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
    pub policy: Policy,
    pub graph: SourceGraph,
    pub memberships: Vec<RepoMembership>,
    pub invitations: Vec<RepoInvitation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppCatalog {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, StoredRepository>,
}

impl AppCatalog {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&StoredRepository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn user_for_email(&self, email: &VerifiedEmail) -> UserAccount {
        if email.verified && email.email == BOOTSTRAP_OWNER_EMAIL {
            return self
                .users
                .get(BOOTSTRAP_OWNER_USER_ID)
                .cloned()
                .expect("bootstrap catalog must contain bootstrap owner");
        }

        UserAccount {
            id: format!("public:{}", email.email),
            email: email.email.clone(),
            email_verified: email.verified,
            access: AccountAccess::Public,
        }
    }

    pub fn principal_for_repo(
        &self,
        repo: &StoredRepository,
        identity: Option<&VerifiedEmail>,
    ) -> Principal {
        let Some(identity) = identity else {
            return Principal::public();
        };

        let user = self.user_for_email(identity);
        let has_membership = repo
            .memberships
            .iter()
            .any(|membership| membership.user_id == user.id);

        if has_membership {
            Principal {
                id: user.id,
                kind: PrincipalKind::User,
            }
        } else {
            Principal::public()
        }
    }
}

pub fn app_catalog() -> AppCatalog {
    let owner = UserAccount {
        id: BOOTSTRAP_OWNER_USER_ID.to_string(),
        email: BOOTSTRAP_OWNER_EMAIL.to_string(),
        email_verified: true,
        access: AccountAccess::Member,
    };

    let repo = canonical_repository();
    AppCatalog {
        users: BTreeMap::from([(owner.id.clone(), owner)]),
        repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
    }
}

pub fn repo_id(owner: &str, name: &str) -> String {
    format!("{}/{}", owner.trim(), name.trim())
}

fn canonical_repository() -> StoredRepository {
    let mut policy = Policy::new(Visibility::Public, BOOTSTRAP_OWNER_USER_ID);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/crates/scope-server/src/auth").unwrap(),
            [BOOTSTRAP_OWNER_USER_ID.to_string()],
        ))
        .unwrap();

    let graph = SourceGraph {
        repo_id: BOOTSTRAP_REPO_ID.to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv_bootstrap_001".to_string(),
                parent_ids: vec![],
                author_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "Publish Scope VCS public surface".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(
                        "# Scope VCS\n\nACL-aware source-control projections.\n".to_string(),
                    ),
                }],
            },
            LogicalCommit {
                id: "rv_bootstrap_002".to_string(),
                parent_ids: vec!["rv_bootstrap_001".to_string()],
                author_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
                author_visibility: AuthorVisibility::Hidden,
                message: "Add server auth groundwork".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/docs/api.md").unwrap(),
                        old_content: None,
                        new_content: Some("Repository-scoped projection API.\n".to_string()),
                    },
                    FileChange {
                        path: ScopePath::parse("/crates/scope-server/src/auth/bootstrap.rs")
                            .unwrap(),
                        old_content: None,
                        new_content: Some("bootstrap owner claims live here\n".to_string()),
                    },
                ],
            },
        ],
    };

    StoredRepository {
        record: RepoRecord {
            id: BOOTSTRAP_REPO_ID.to_string(),
            owner_handle: BOOTSTRAP_REPO_OWNER.to_string(),
            name: BOOTSTRAP_REPO_NAME.to_string(),
            owner_user_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
            publication_state: RepoPublicationState::Published,
            default_visibility: Visibility::Public,
        },
        policy,
        graph,
        memberships: vec![RepoMembership {
            repo_id: BOOTSTRAP_REPO_ID.to_string(),
            user_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
            role: RepoRole::Owner,
        }],
        invitations: Vec::new(),
    }
}

fn normalize_email(email: impl Into<String>) -> String {
    email.into().trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_bootstrap_email_becomes_repo_owner() {
        let catalog = app_catalog();
        let repo = catalog
            .repository(BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap();
        let identity = VerifiedEmail::new("AdamBlumoff@gmail.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal.id, BOOTSTRAP_OWNER_USER_ID);
        assert_eq!(principal.kind, PrincipalKind::User);
        assert!(repo.policy.can_write(
            &principal,
            &ScopePath::parse("/crates/scope-server/src/auth/bootstrap.rs").unwrap(),
        ));
    }

    #[test]
    fn unverified_bootstrap_email_stays_public() {
        let catalog = app_catalog();
        let repo = catalog
            .repository(BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap();
        let identity = VerifiedEmail::new(BOOTSTRAP_OWNER_EMAIL, false);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
    }

    #[test]
    fn unknown_verified_user_defaults_to_public() {
        let catalog = app_catalog();
        let repo = catalog
            .repository(BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap();
        let identity = VerifiedEmail::new("someone@example.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
    }
}
