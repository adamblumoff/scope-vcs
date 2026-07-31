use crate::db::projection_encoding::{LIVE_PROJECTION_SOURCE, ProjectionAudience};
use crate::db::{decode_json, encode_json};
use crate::error::PostgresError;
use scope_domain::policy::{Policy, ScopePath, Visibility};
use scope_domain::projection_views::{ProjectionViewFile, ProjectionViewFileContent};
use scope_domain::runs::{
    run::{
        AttemptState, AttemptTerminalReason, PinnedContainerImage, Run, RunAttempt, RunAttemptStep,
        RunLogChunk, RunSource, RunState, RunTrigger, StepState,
    },
    runner::{Runner, RunnerCapabilities, RunnerGrant, RunnerName},
    trigger::PushTriggerEvaluation,
    workflow::{
        CompiledWorkflow, RunnerSelector, WorkflowIdentity, WorkflowPath, WorkflowRevision,
    },
};
use scope_domain::store::{
    DEFAULT_GIT_FILE_MODE, FirstPushToken, GitHead, GitPushToken, GitSegment, RepoPublicationState,
    RepoRecord, RepoStorageCleanup, RepositoryInvite, RepositoryInviteState, RepositoryMember,
    RepositoryMemberPermissions, SourceBlob, StoredRepository, UserAccount,
    is_supported_git_file_mode,
};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

pub(super) fn encode_enum<T: serde::Serialize>(value: T) -> Result<String, PostgresError> {
    match serde_json::to_value(value).map_err(PostgresError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(PostgresError::internal_message(
            "enum did not serialize to string",
        )),
    }
}

pub(super) fn decode_enum<T: serde::de::DeserializeOwned>(
    value: String,
) -> Result<T, PostgresError> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(PostgresError::internal)
}

pub(super) fn u64_to_i64(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL bigint range"))
    })
}

pub(super) fn i64_to_u64(value: i64, field: &str) -> Result<u64, PostgresError> {
    u64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

fn u32_to_i32(value: u32, field: &str) -> Result<i32, PostgresError> {
    i32::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL integer range"))
    })
}

pub(super) fn i32_to_u32(value: i32, field: &str) -> Result<u32, PostgresError> {
    u32::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

fn usize_to_i64(value: usize, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL bigint range"))
    })
}

pub struct RepositoryFacts {
    pub first_push_token: Option<FirstPushToken>,
    pub git_push_token: Option<GitPushToken>,
    pub git_head: Option<GitHead>,
    pub git_segments: Vec<GitSegment>,
}

mod auth;
mod collaboration;
mod history;
mod jobs;
mod read_models;
mod repositories;
mod requests;
mod runs;

pub use auth::{
    auth_identity, cli_browser_login, cli_device_login, cli_exchange_grant, cli_session, user,
};
pub use collaboration::{repository_invite, repository_member};
pub use history::{file_change, live_file, logical_commit, object_reference, visibility_event};
pub use jobs::{metadata_lock, outbox_job, repo_storage_cleanup_job, source_blob_cleanup_job};
pub use read_models::{projection_file, projection_read_model};
pub use repositories::{
    git_head, git_segment, repository, repository_first_push_token, repository_git_push_token,
};
pub use requests::{
    credit_ledger_entry, request, request_change_block, request_discussion,
    request_discussion_read_state, request_discussion_reply, request_event, request_invitee,
    user_credit_account,
};
pub use runs::{
    push_trigger_evaluation, run, run_attempt, run_attempt_step, run_log, runner, runner_grant,
    workflow_revision,
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
            ProjectionViewFileContent {
                file: ProjectionViewFile {
                    path: ScopePath::parse(&path).unwrap(),
                    oid: "1111111111111111111111111111111111111111".to_string(),
                    tracked: true,
                    visibility: Visibility::Public,
                },
                blob: SourceBlob {
                    content_ref: scope_domain::content_ref::ContentRef::blob_sha256("sha256"),
                    sha256: "sha256".to_string(),
                    git_oid: "1111111111111111111111111111111111111111".to_string(),
                    git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                    size_bytes: 10,
                },
            },
        )
        .unwrap();

        assert_eq!(model.path, path);
        assert_eq!(model.path_key.len(), "sha256:".len() + 64);
        assert!(model.path_key.starts_with("sha256:"));

        let content = model.try_into_content().unwrap();
        assert_eq!(
            content.blob.content_ref,
            scope_domain::content_ref::ContentRef::blob_sha256("sha256")
        );
        assert_eq!(content.blob.git_oid, content.file.oid);
        assert_eq!(content.blob.size_bytes, 10);
    }

    #[test]
    fn projection_read_model_persists_canonical_identity_contract() {
        let model = projection_read_model::Model::live(
            "owner/repo",
            7,
            ProjectionAudience::Public,
            Some("1111111111111111111111111111111111111111".to_string()),
            10,
            2,
        )
        .unwrap();

        assert_eq!(
            model.head_oid.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(
            model.identity_version,
            scope_git::PROJECTION_IDENTITY_VERSION
        );
    }

    #[test]
    fn projection_file_rejects_inconsistent_domain_content() {
        let content = ProjectionViewFileContent {
            file: ProjectionViewFile {
                path: ScopePath::parse("/README.md").unwrap(),
                oid: "1111111111111111111111111111111111111111".to_string(),
                tracked: true,
                visibility: Visibility::Public,
            },
            blob: SourceBlob {
                content_ref: scope_domain::content_ref::ContentRef::blob_sha256("sha256"),
                sha256: "sha256".to_string(),
                git_oid: "1111111111111111111111111111111111111111".to_string(),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 10,
            },
        };

        let mut untracked = content.clone();
        untracked.file.tracked = false;
        assert!(
            projection_file::Model::live("owner/repo", 1, ProjectionAudience::Public, untracked,)
                .is_err()
        );

        let mut mismatched_oid = content.clone();
        mismatched_oid.blob.git_oid = "2222222222222222222222222222222222222222".to_string();
        assert!(
            projection_file::Model::live(
                "owner/repo",
                1,
                ProjectionAudience::Public,
                mismatched_oid,
            )
            .is_err()
        );

        let mut unsupported_mode = content;
        unsupported_mode.blob.git_file_mode = "120000".to_string();
        assert!(
            projection_file::Model::live(
                "owner/repo",
                1,
                ProjectionAudience::Public,
                unsupported_mode,
            )
            .is_err()
        );
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
        let row = git_segment::Model {
            repo_id: "repo-1".to_string(),
            sequence: 1,
            base_oid: None,
            head_oid: "oid".to_string(),
            object_key: serde_json::to_string(
                &scope_domain::content_ref::ContentRef::git_segment_sha256("sha"),
            )
            .unwrap(),
            sha256: "sha".to_string(),
            size_bytes: -1,
            manifest_object_key: serde_json::to_string(
                &scope_domain::content_ref::ContentRef::git_manifest_sha256("manifest-sha"),
            )
            .unwrap(),
            manifest_sha256: "manifest-sha".to_string(),
            manifest_size_bytes: 1,
        };

        assert!(row.try_into_domain().is_err());
    }
}
