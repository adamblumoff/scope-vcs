use super::*;

#[derive(Copy, Clone)]
pub enum Repositories {
    Table,
    Id,
    OwnerHandle,
    Name,
    OwnerUserId,
    PublicationState,
    DefaultVisibility,
    ChangeVersion,
    PendingImport,
    Policy,
    Graph,
    VisibilityEvents,
    StagedUpdate,
}

impl_iden!(Repositories {
    Table => "scope_repositories",
    Id => "id",
    OwnerHandle => "owner_handle",
    Name => "name",
    OwnerUserId => "owner_user_id",
    PublicationState => "publication_state",
    DefaultVisibility => "default_visibility",
    ChangeVersion => "change_version",
    PendingImport => "pending_import",
    Policy => "policy",
    Graph => "graph",
    VisibilityEvents => "visibility_events",
    StagedUpdate => "staged_update",
});

#[derive(Copy, Clone)]
pub enum RepositorySettings {
    Table,
    RepoId,
    IncludeIgnoredFiles,
    ReviewPushesBeforeApplying,
}

impl_iden!(RepositorySettings {
    Table => "scope_repository_settings",
    RepoId => "repo_id",
    IncludeIgnoredFiles => "include_ignored_files",
    ReviewPushesBeforeApplying => "review_pushes_before_applying",
});

#[derive(Copy, Clone)]
pub enum RepositoryFirstPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAtUnix,
    ExpiresAtUnix,
    UsedAtUnix,
}

impl_iden!(RepositoryFirstPushTokens {
    Table => "scope_repository_first_push_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    OwnerUserId => "owner_user_id",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    UsedAtUnix => "used_at_unix",
});

#[derive(Copy, Clone)]
pub enum RepositoryGitPushTokens {
    Table,
    RepoId,
    TokenHash,
    OwnerUserId,
    CreatedAtUnix,
}

impl_iden!(RepositoryGitPushTokens {
    Table => "scope_repository_git_push_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    OwnerUserId => "owner_user_id",
    CreatedAtUnix => "created_at_unix",
});

#[derive(Copy, Clone)]
pub enum RepositoryGitCloneTokens {
    Table,
    RepoId,
    TokenHash,
    UserId,
    CreatedAtUnix,
}

impl_iden!(RepositoryGitCloneTokens {
    Table => "scope_repository_git_clone_tokens",
    RepoId => "repo_id",
    TokenHash => "token_hash",
    UserId => "user_id",
    CreatedAtUnix => "created_at_unix",
});

#[derive(Copy, Clone)]
pub enum RepositoryGitSnapshots {
    Table,
    RepoId,
    ObjectKey,
    Sha256,
    GitOid,
    SizeBytes,
    LineCount,
}

impl_iden!(RepositoryGitSnapshots {
    Table => "scope_repository_git_snapshots",
    RepoId => "repo_id",
    ObjectKey => "object_key",
    Sha256 => "sha256",
    GitOid => "git_oid",
    SizeBytes => "size_bytes",
    LineCount => "line_count",
});

const REPOSITORY_COLUMNS: &[&str] = &[
    Repositories::Id.as_str(),
    Repositories::OwnerHandle.as_str(),
    Repositories::Name.as_str(),
    Repositories::OwnerUserId.as_str(),
    Repositories::PublicationState.as_str(),
    Repositories::DefaultVisibility.as_str(),
    Repositories::ChangeVersion.as_str(),
    Repositories::PendingImport.as_str(),
    Repositories::Policy.as_str(),
    Repositories::Graph.as_str(),
    Repositories::VisibilityEvents.as_str(),
    Repositories::StagedUpdate.as_str(),
];

const REPOSITORY_SETTING_COLUMNS: &[&str] = &[
    RepositorySettings::RepoId.as_str(),
    RepositorySettings::IncludeIgnoredFiles.as_str(),
    RepositorySettings::ReviewPushesBeforeApplying.as_str(),
];

const REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS: &[&str] = &[
    RepositoryFirstPushTokens::RepoId.as_str(),
    RepositoryFirstPushTokens::TokenHash.as_str(),
    RepositoryFirstPushTokens::OwnerUserId.as_str(),
    RepositoryFirstPushTokens::CreatedAtUnix.as_str(),
    RepositoryFirstPushTokens::ExpiresAtUnix.as_str(),
    RepositoryFirstPushTokens::UsedAtUnix.as_str(),
];

const REPOSITORY_GIT_PUSH_TOKEN_COLUMNS: &[&str] = &[
    RepositoryGitPushTokens::RepoId.as_str(),
    RepositoryGitPushTokens::TokenHash.as_str(),
    RepositoryGitPushTokens::OwnerUserId.as_str(),
    RepositoryGitPushTokens::CreatedAtUnix.as_str(),
];

const REPOSITORY_GIT_CLONE_TOKEN_COLUMNS: &[&str] = &[
    RepositoryGitCloneTokens::RepoId.as_str(),
    RepositoryGitCloneTokens::TokenHash.as_str(),
    RepositoryGitCloneTokens::UserId.as_str(),
    RepositoryGitCloneTokens::CreatedAtUnix.as_str(),
];

const REPOSITORY_GIT_SNAPSHOT_COLUMNS: &[&str] = &[
    RepositoryGitSnapshots::RepoId.as_str(),
    RepositoryGitSnapshots::ObjectKey.as_str(),
    RepositoryGitSnapshots::Sha256.as_str(),
    RepositoryGitSnapshots::GitOid.as_str(),
    RepositoryGitSnapshots::SizeBytes.as_str(),
    RepositoryGitSnapshots::LineCount.as_str(),
];

pub const TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: Repositories::Table.as_str(),
        columns: REPOSITORY_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositorySettings::Table.as_str(),
        columns: REPOSITORY_SETTING_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryFirstPushTokens::Table.as_str(),
        columns: REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitPushTokens::Table.as_str(),
        columns: REPOSITORY_GIT_PUSH_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitCloneTokens::Table.as_str(),
        columns: REPOSITORY_GIT_CLONE_TOKEN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: RepositoryGitSnapshots::Table.as_str(),
        columns: REPOSITORY_GIT_SNAPSHOT_COLUMNS,
        counts_for_catalog_rows: true,
    },
];
