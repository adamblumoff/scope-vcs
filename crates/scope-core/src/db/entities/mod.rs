use crate::db::projection_encoding::{ProjectionAudience, ProjectionSource};
use crate::db::{decode_json, encode_json};
use crate::domain::policy::{Policy, ScopePath, Visibility};
use crate::domain::projection::SourceGraph;
use crate::domain::projection_views::ProjectionViewFile;
use crate::domain::store::{
    DEFAULT_GIT_FILE_MODE, FirstPushToken, GitPushToken, RepoPublicationState, RepoRecord,
    RepoStorageCleanup, RepositoryInvite, RepositoryInviteState, RepositoryMember,
    RepositoryMemberPermissions, SourceBlob, StoredRepository, UserAccount,
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

fn u64_to_i64(value: u64, field: &str) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| ApiError::internal_message(format!("{field} exceeds PostgreSQL bigint range")))
}

fn i64_to_u64(value: i64, field: &str) -> Result<u64, ApiError> {
    u64::try_from(value)
        .map_err(|_| ApiError::internal_message(format!("{field} cannot be negative")))
}

fn u32_to_i32(value: u32, field: &str) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| {
        ApiError::internal_message(format!("{field} exceeds PostgreSQL integer range"))
    })
}

fn i32_to_u32(value: i32, field: &str) -> Result<u32, ApiError> {
    u32::try_from(value)
        .map_err(|_| ApiError::internal_message(format!("{field} cannot be negative")))
}

fn usize_to_i64(value: usize, field: &str) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| ApiError::internal_message(format!("{field} exceeds PostgreSQL bigint range")))
}

pub struct RepositoryFacts {
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_snapshot: Option<SourceBlob>,
}

mod auth;
mod collaboration;
mod jobs;
mod read_models;
mod repositories;
mod requests;

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
};
pub use requests::{credit_ledger_entry, request, request_event, user_credit_account};

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

        let persisted = repository_first_push_token::Model::from_domain("repo-1", &token).unwrap();
        let json = serde_json::to_value(&persisted).expect("token serializes");
        assert!(json.get("secret").is_none());

        let rehydrated = serde_json::from_value::<repository_first_push_token::Model>(json)
            .expect("token deserializes")
            .try_into_domain()
            .unwrap();
        assert_eq!(rehydrated.secret, None);
    }

    #[test]
    fn projection_file_uses_bounded_path_key_without_truncating_path() {
        let path = format!("/{}", "deep/".repeat(900)) + "file.txt";
        let model = projection_file::Model::live(
            "owner/repo",
            1,
            ProjectionAudience::Public,
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

    #[test]
    fn oversized_domain_values_are_rejected_instead_of_truncated() {
        let token = FirstPushToken {
            token_hash: "hash".to_string(),
            secret: None,
            owner_user_id: "user-1".to_string(),
            created_at_unix: u64::MAX,
            expires_at_unix: u64::MAX,
            used_at_unix: None,
        };

        assert!(repository_first_push_token::Model::from_domain("repo-1", &token).is_err());
    }

    #[test]
    fn negative_persisted_values_are_rejected_instead_of_floored() {
        let row = repository_git_snapshot::Model {
            repo_id: "repo-1".to_string(),
            object_key: "objects/snapshot".to_string(),
            sha256: "sha".to_string(),
            git_oid: "oid".to_string(),
            size_bytes: -1,
        };

        assert!(row.try_into_domain().is_err());
    }
}
