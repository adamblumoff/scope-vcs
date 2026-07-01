use crate::db::{decode_json, encode_json};
use crate::domain::policy::{Policy, Visibility};
use crate::domain::projection::SourceGraph;
use crate::domain::store::{
    AccountAccess, FirstPushToken, GitCloneToken, GitPushToken, PendingImport,
    RepoPublicationState, RepoRecord, RepoSettings, RepoStorageCleanup, RepositoryInvite,
    RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions, SourceBlob,
    StagedRepoUpdate, StoredRepository, UserAccount,
};
use crate::error::ApiError;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

fn encode_enum<T: serde::Serialize>(value: T) -> Result<String, ApiError> {
    match serde_json::to_value(value).map_err(ApiError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(ApiError::internal_message(
            "enum did not serialize to string",
        )),
    }
}

fn decode_enum<T: serde::de::DeserializeOwned>(value: String) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(ApiError::internal)
}

fn u64_to_i64_saturating(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn i64_to_u64_floor(value: i64) -> u64 {
    value.max(0) as u64
}

fn usize_to_i64_saturating(value: usize) -> i64 {
    value.min(i64::MAX as usize) as i64
}

pub(crate) struct RepositoryFacts {
    pub(crate) settings: RepoSettings,
    pub(crate) first_push_token: Option<FirstPushToken>,
    pub(crate) git_push_token: Option<GitPushToken>,
    pub(crate) git_clone_tokens: Vec<GitCloneToken>,
    pub(crate) git_snapshot: Option<SourceBlob>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_first_push_token_never_stores_plaintext_secret() {
        let token = FirstPushToken {
            token_hash: "hash".to_string(),
            secret: Some("scope-first-push-secret".to_string()),
            owner_user_id: "user-1".to_string(),
            created_at_unix: 10,
            expires_at_unix: 20,
            used_at_unix: None,
        };

        let persisted = repository_first_push_token::Model::from_domain("repo-1", &token);
        let json = serde_json::to_value(&persisted).expect("token serializes");
        assert!(json.get("secret").is_none());

        let rehydrated = serde_json::from_value::<repository_first_push_token::Model>(json)
            .expect("token deserializes")
            .into_domain();
        assert_eq!(rehydrated.secret, None);
    }
}

pub(crate) mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub handle: String,
        #[sea_orm(unique)]
        pub email: String,
        pub email_verified: bool,
        pub access: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(user: &UserAccount) -> Self {
            Self {
                id: user.id.clone(),
                handle: user.handle.clone(),
                email: user.email.clone(),
                email_verified: user.email_verified,
                access: encode_enum(user.access).expect("AccountAccess serializes to a string"),
            }
        }

        pub(crate) fn try_into_domain(self) -> Result<UserAccount, ApiError> {
            Ok(UserAccount {
                id: self.id,
                handle: self.handle,
                email: self.email,
                email_verified: self.email_verified,
                access: decode_enum::<AccountAccess>(self.access)?,
            })
        }
    }
}

pub(crate) mod auth_identity {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_auth_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub provider: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub subject: String,
        pub user_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod repository {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repositories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub owner_handle: String,
        pub name: String,
        pub owner_user_id: String,
        pub publication_state: String,
        pub default_visibility: String,
        pub change_version: i64,
        pub pending_import: Option<Json>,
        pub policy: Json,
        pub graph: Json,
        pub visibility_events: Json,
        pub staged_update: Option<Json>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo: &StoredRepository) -> Result<Self, ApiError> {
            Ok(Self {
                id: repo.record.id.clone(),
                owner_handle: repo.record.owner_handle.clone(),
                name: repo.record.name.clone(),
                owner_user_id: repo.record.owner_user_id.clone(),
                publication_state: encode_enum(repo.record.publication_state)?,
                default_visibility: encode_enum(repo.record.default_visibility)?,
                change_version: repo.record.change_version.min(i64::MAX as u64) as i64,
                pending_import: repo.pending_import.as_ref().map(encode_json).transpose()?,
                policy: encode_json(&repo.policy)?,
                graph: encode_json(&repo.graph)?,
                visibility_events: encode_json(&repo.visibility_events)?,
                staged_update: repo.staged_update.as_ref().map(encode_json).transpose()?,
            })
        }

        pub(crate) fn try_into_domain(
            self,
            facts: RepositoryFacts,
            members: Vec<RepositoryMember>,
            invitations: Vec<RepositoryInvite>,
        ) -> Result<StoredRepository, ApiError> {
            let publication_state = decode_enum::<RepoPublicationState>(self.publication_state)?;
            let default_visibility = decode_enum::<Visibility>(self.default_visibility)?;
            Ok(StoredRepository {
                record: RepoRecord {
                    id: self.id.clone(),
                    owner_handle: self.owner_handle,
                    name: self.name,
                    owner_user_id: self.owner_user_id,
                    publication_state,
                    default_visibility,
                    change_version: self.change_version.max(0) as u64,
                },
                settings: facts.settings,
                first_push_token: facts.first_push_token,
                git_push_token: facts.git_push_token,
                git_clone_tokens: facts.git_clone_tokens,
                pending_import: self
                    .pending_import
                    .map(decode_json::<PendingImport>)
                    .transpose()?,
                policy: decode_json::<Policy>(self.policy)?,
                graph: decode_json::<SourceGraph>(self.graph)?,
                visibility_events: decode_json(self.visibility_events)?,
                git_snapshot: facts.git_snapshot,
                staged_update: self
                    .staged_update
                    .map(decode_json::<StagedRepoUpdate>)
                    .transpose()?,
                members,
                invitations,
            })
        }
    }
}

pub(crate) mod repository_setting {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub include_ignored_files: bool,
        pub review_pushes_before_applying: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo_id: &str, settings: RepoSettings) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                include_ignored_files: settings.include_ignored_files,
                review_pushes_before_applying: settings.review_pushes_before_applying,
            }
        }

        pub(crate) fn into_domain(self) -> RepoSettings {
            RepoSettings {
                include_ignored_files: self.include_ignored_files,
                review_pushes_before_applying: self.review_pushes_before_applying,
            }
        }
    }
}

pub(crate) mod repository_first_push_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "scope_repository_first_push_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub token_hash: String,
        pub owner_user_id: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub used_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo_id: &str, token: &FirstPushToken) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                owner_user_id: token.owner_user_id.clone(),
                created_at_unix: u64_to_i64_saturating(token.created_at_unix),
                expires_at_unix: u64_to_i64_saturating(token.expires_at_unix),
                used_at_unix: token.used_at_unix.map(u64_to_i64_saturating),
            }
        }

        pub(crate) fn into_domain(self) -> FirstPushToken {
            FirstPushToken {
                token_hash: self.token_hash,
                secret: None,
                owner_user_id: self.owner_user_id,
                created_at_unix: i64_to_u64_floor(self.created_at_unix),
                expires_at_unix: i64_to_u64_floor(self.expires_at_unix),
                used_at_unix: self.used_at_unix.map(i64_to_u64_floor),
            }
        }
    }
}

pub(crate) mod repository_git_push_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_git_push_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub token_hash: String,
        pub owner_user_id: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo_id: &str, token: &GitPushToken) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                owner_user_id: token.owner_user_id.clone(),
                created_at_unix: u64_to_i64_saturating(token.created_at_unix),
            }
        }

        pub(crate) fn into_domain(self) -> GitPushToken {
            GitPushToken {
                token_hash: self.token_hash,
                owner_user_id: self.owner_user_id,
                created_at_unix: i64_to_u64_floor(self.created_at_unix),
            }
        }
    }
}

pub(crate) mod repository_git_clone_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_git_clone_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub token_hash: String,
        pub user_id: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo_id: &str, token: &GitCloneToken) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                token_hash: token.token_hash.clone(),
                user_id: token.user_id.clone(),
                created_at_unix: u64_to_i64_saturating(token.created_at_unix),
            }
        }

        pub(crate) fn into_domain(self) -> GitCloneToken {
            GitCloneToken {
                token_hash: self.token_hash,
                user_id: self.user_id,
                created_at_unix: i64_to_u64_floor(self.created_at_unix),
            }
        }
    }
}

pub(crate) mod repository_git_snapshot {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_git_snapshots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub object_key: String,
        pub sha256: String,
        pub git_oid: String,
        pub size_bytes: i64,
        pub line_count: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(repo_id: &str, blob: &SourceBlob) -> Self {
            Self {
                repo_id: repo_id.to_string(),
                object_key: blob.object_key.clone(),
                sha256: blob.sha256.clone(),
                git_oid: blob.git_oid.clone(),
                size_bytes: u64_to_i64_saturating(blob.size_bytes),
                line_count: usize_to_i64_saturating(blob.line_count),
            }
        }

        pub(crate) fn into_domain(self) -> SourceBlob {
            SourceBlob {
                object_key: self.object_key,
                sha256: self.sha256,
                git_oid: self.git_oid,
                size_bytes: i64_to_u64_floor(self.size_bytes),
                line_count: self.line_count.max(0) as usize,
            }
        }
    }
}

pub(crate) mod cli_device_login {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_device_logins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub device_code_hash: String,
        #[sea_orm(unique)]
        pub user_code_hash: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub completed_user_id: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod cli_browser_login {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_browser_logins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub request_id: String,
        pub request_secret_hash: String,
        pub callback_url: String,
        pub callback_code_hash: Option<String>,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub completed_user_id: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod cli_exchange_grant {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_exchange_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub grant_hash: String,
        pub user_id: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod cli_session {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub user_id: String,
        pub label: String,
        pub created_at_unix: i64,
        pub last_used_at_unix: Option<i64>,
        pub expires_at_unix: i64,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod repository_member {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub permissions: Json,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(member: &RepositoryMember) -> Result<Self, ApiError> {
            Ok(Self {
                repo_id: member.repo_id.clone(),
                user_id: member.user_id.clone(),
                permissions: encode_json(&member.permissions)?,
                created_at_unix: member.created_at_unix.min(i64::MAX as u64) as i64,
                updated_at_unix: member.updated_at_unix.min(i64::MAX as u64) as i64,
            })
        }

        pub(crate) fn try_into_domain(self) -> Result<RepositoryMember, ApiError> {
            Ok(RepositoryMember {
                repo_id: self.repo_id,
                user_id: self.user_id,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                created_at_unix: self.created_at_unix.max(0) as u64,
                updated_at_unix: self.updated_at_unix.max(0) as u64,
            })
        }
    }
}

pub(crate) mod repository_invite {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_invites")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub repo_id: String,
        pub invited_email: String,
        pub invited_email_normalized: String,
        pub permissions: Json,
        pub invited_by_user_id: String,
        pub state: String,
        pub token_hash: String,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub expires_at_unix: i64,
        pub accepted_by_user_id: Option<String>,
        pub accepted_at_unix: Option<i64>,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(invite: &RepositoryInvite) -> Result<Self, ApiError> {
            Ok(Self {
                id: invite.id.clone(),
                repo_id: invite.repo_id.clone(),
                invited_email: invite.invited_email.clone(),
                invited_email_normalized: invite.invited_email_normalized.clone(),
                permissions: encode_json(&invite.permissions)?,
                invited_by_user_id: invite.invited_by_user_id.clone(),
                state: encode_enum(invite.state)?,
                token_hash: invite.token_hash.clone(),
                created_at_unix: invite.created_at_unix.min(i64::MAX as u64) as i64,
                updated_at_unix: invite.updated_at_unix.min(i64::MAX as u64) as i64,
                expires_at_unix: invite.expires_at_unix.min(i64::MAX as u64) as i64,
                accepted_by_user_id: invite.accepted_by_user_id.clone(),
                accepted_at_unix: invite
                    .accepted_at_unix
                    .map(|value| value.min(i64::MAX as u64) as i64),
                revoked_at_unix: invite
                    .revoked_at_unix
                    .map(|value| value.min(i64::MAX as u64) as i64),
            })
        }

        pub(crate) fn try_into_domain(self) -> Result<RepositoryInvite, ApiError> {
            Ok(RepositoryInvite {
                id: self.id,
                repo_id: self.repo_id,
                invited_email: self.invited_email,
                invited_email_normalized: self.invited_email_normalized,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                invited_by_user_id: self.invited_by_user_id,
                state: decode_enum::<RepositoryInviteState>(self.state)?,
                token_hash: self.token_hash,
                created_at_unix: self.created_at_unix.max(0) as u64,
                updated_at_unix: self.updated_at_unix.max(0) as u64,
                expires_at_unix: self.expires_at_unix.max(0) as u64,
                accepted_by_user_id: self.accepted_by_user_id,
                accepted_at_unix: self.accepted_at_unix.map(|value| value.max(0) as u64),
                revoked_at_unix: self.revoked_at_unix.map(|value| value.max(0) as u64),
            })
        }
    }
}

pub(crate) mod metadata_lock {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_metadata_locks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod repo_storage_cleanup_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repo_storage_cleanup_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub generation: String,
        pub owner_handle: String,
        pub repo_name: String,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(
            cleanup: &RepoStorageCleanup,
            generation: String,
            now_unix: u64,
        ) -> Self {
            let repo_id = crate::domain::store::repo_id(&cleanup.owner_handle, &cleanup.repo_name);
            let now_unix = now_unix.min(i64::MAX as u64) as i64;
            Self {
                repo_id,
                generation,
                owner_handle: cleanup.owner_handle.clone(),
                repo_name: cleanup.repo_name.clone(),
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
            }
        }

        pub(crate) fn into_domain(self) -> RepoStorageCleanup {
            RepoStorageCleanup {
                owner_handle: self.owner_handle,
                repo_name: self.repo_name,
            }
        }
    }
}

pub(crate) mod source_blob_cleanup_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_source_blob_cleanup_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub object_key: String,
        pub generation: String,
        pub sha256: String,
        pub git_oid: String,
        pub size_bytes: i64,
        pub line_count: i64,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(blob: &SourceBlob, generation: String, now_unix: u64) -> Self {
            let now_unix = now_unix.min(i64::MAX as u64) as i64;
            Self {
                object_key: blob.object_key.clone(),
                generation,
                sha256: blob.sha256.clone(),
                git_oid: blob.git_oid.clone(),
                size_bytes: blob.size_bytes.min(i64::MAX as u64) as i64,
                line_count: (blob.line_count.min(i64::MAX as usize)) as i64,
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
            }
        }

        pub(crate) fn into_domain(self) -> SourceBlob {
            SourceBlob {
                object_key: self.object_key,
                sha256: self.sha256,
                git_oid: self.git_oid,
                size_bytes: self.size_bytes.max(0) as u64,
                line_count: self.line_count.max(0) as usize,
            }
        }
    }
}

pub(crate) mod metadata_reset_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_metadata_reset_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub reset_at_unix: i64,
        pub trigger: String,
        pub reason: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
