use crate::db::{decode_json, encode_json};
use crate::domain::policy::{Policy, ScopePath, Visibility};
use crate::domain::projection::SourceGraph;
use crate::domain::projection_views::ProjectionViewFile;
use crate::domain::store::{
    DEFAULT_GIT_FILE_MODE, FirstPushToken, GitPushToken, PendingImport, RepoPublicationState,
    RepoRecord, RepoSettings, RepoStorageCleanup, RepositoryInvite, RepositoryInviteState,
    RepositoryMember, RepositoryMemberPermissions, SourceBlob, StagedRepoUpdate, StoredRepository,
    UserAccount,
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

pub struct RepositoryFacts {
    pub settings: RepoSettings,
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_snapshot: Option<SourceBlob>,
}

mod auth;
mod collaboration;
mod jobs;
mod read_models;
mod repositories;

pub use auth::{
    auth_identity, cli_browser_login, cli_device_login, cli_exchange_grant, cli_session, user,
};
pub use collaboration::{repository_invite, repository_member};
pub use jobs::{
    metadata_lock, metadata_reset_event, outbox_job, repo_storage_cleanup_job,
    source_blob_cleanup_job,
};
pub use read_models::{projection_file, projection_read_model};
pub use repositories::{
    repository, repository_first_push_token, repository_git_push_token, repository_git_snapshot,
    repository_setting,
};

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

    #[test]
    fn projection_file_uses_bounded_path_key_without_truncating_path() {
        let path = format!("/{}", "deep/".repeat(900)) + "file.txt";
        let model = projection_file::Model::live(
            "owner/repo",
            1,
            "public",
            ProjectionViewFile {
                path: ScopePath::parse(&path).unwrap(),
                oid: "1111111111111111111111111111111111111111".to_string(),
                tracked: true,
                visibility: Visibility::Public,
            },
        )
        .unwrap();

        assert_eq!(model.path, path);
        assert_eq!(model.path_key.len(), "sha256:".len() + 64);
        assert!(model.path_key.starts_with("sha256:"));
    }
}
