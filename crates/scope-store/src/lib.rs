use scope_policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule};
use scope_projection::{
    AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, SourceGraph,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
            ScopePath::parse("/crates/scope-server/src").unwrap(),
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
                message: "Import Scope VCS public workspace".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    repo_file("/README.md", include_str!("../../../README.md")),
                    repo_file(
                        "/crates/scope-policy/src/lib.rs",
                        include_str!("../../scope-policy/src/lib.rs"),
                    ),
                    repo_file(
                        "/crates/scope-projection/src/lib.rs",
                        include_str!("../../scope-projection/src/lib.rs"),
                    ),
                    repo_file(
                        "/apps/web/src/routes/index.tsx",
                        include_str!("../../../apps/web/src/routes/index.tsx"),
                    ),
                ],
            },
            LogicalCommit {
                id: "rv_bootstrap_002".to_string(),
                parent_ids: vec!["rv_bootstrap_001".to_string()],
                author_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "Import private server implementation".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![repo_file(
                    "/crates/scope-server/src/main.rs",
                    include_str!("../../scope-server/src/main.rs"),
                )],
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

fn repo_file(path: &str, content: &str) -> FileChange {
    FileChange {
        path: ScopePath::parse(path).unwrap(),
        old_content: None,
        new_content: Some(content.to_string()),
    }
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
        assert!(catalog.can_write_path(
            repo,
            &principal,
            &ScopePath::parse("/crates/scope-server/src/main.rs").unwrap(),
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

    #[test]
    fn unpublished_repo_blocks_public_reads() {
        let catalog = app_catalog();
        let mut repo = catalog
            .repository(BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap()
            .clone();
        repo.record.publication_state = RepoPublicationState::Unpublished;

        assert!(!catalog.can_read_path(
            &repo,
            &Principal::public(),
            &ScopePath::parse("/README.md").unwrap(),
        ));
    }

    #[test]
    fn pending_invite_does_not_grant_private_access() {
        let mut catalog = app_catalog();
        let invited = UserAccount {
            id: "user_invited".to_string(),
            email: "invited@example.com".to_string(),
            email_verified: true,
            access: AccountAccess::Member,
        };
        catalog.users.insert(invited.id.clone(), invited);
        let repo = catalog
            .repositories
            .get_mut(BOOTSTRAP_REPO_ID)
            .expect("bootstrap repo exists");
        repo.invitations.push(RepoInvitation {
            id: "invite_pending".to_string(),
            repo_id: BOOTSTRAP_REPO_ID.to_string(),
            invited_email: "invited@example.com".to_string(),
            role: RepoRole::Reader,
            invited_by_user_id: BOOTSTRAP_OWNER_USER_ID.to_string(),
            state: InvitationState::Pending,
        });
        let repo = catalog
            .repository(BOOTSTRAP_REPO_OWNER, BOOTSTRAP_REPO_NAME)
            .unwrap();
        let identity = VerifiedEmail::new("invited@example.com", true);

        let principal = catalog.principal_for_repo(repo, Some(&identity));

        assert_eq!(principal, Principal::public());
        assert!(!catalog.can_read_path(
            repo,
            &principal,
            &ScopePath::parse("/crates/scope-server/src/main.rs").unwrap(),
        ));
    }
}
