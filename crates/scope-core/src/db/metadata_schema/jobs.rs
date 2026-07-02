use super::*;

#[derive(Copy, Clone)]
pub enum OutboxJobs {
    Table,
    Id,
    IdempotencyKey,
    Kind,
    RepoId,
    RepoVersion,
    Payload,
    State,
    Attempts,
    NextRunAtUnix,
    LeaseOwner,
    LeaseExpiresAtUnix,
    LastError,
    CreatedAtUnix,
    UpdatedAtUnix,
    CompletedAtUnix,
}

impl_iden!(OutboxJobs {
    Table => "scope_outbox_jobs",
    Id => "id",
    IdempotencyKey => "idempotency_key",
    Kind => "kind",
    RepoId => "repo_id",
    RepoVersion => "repo_version",
    Payload => "payload",
    State => "state",
    Attempts => "attempts",
    NextRunAtUnix => "next_run_at_unix",
    LeaseOwner => "lease_owner",
    LeaseExpiresAtUnix => "lease_expires_at_unix",
    LastError => "last_error",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
    CompletedAtUnix => "completed_at_unix",
});

#[derive(Copy, Clone)]
pub enum MetadataLocks {
    Table,
    Key,
}

impl_iden!(MetadataLocks {
    Table => "scope_metadata_locks",
    Key => "key",
});

#[derive(Copy, Clone)]
pub enum RepoStorageCleanupJobs {
    Table,
    RepoId,
    Generation,
    OwnerHandle,
    RepoName,
    Attempts,
    NextRunAtUnix,
    LastError,
    CompletedAtUnix,
    CreatedAtUnix,
    UpdatedAtUnix,
}

impl_iden!(RepoStorageCleanupJobs {
    Table => "scope_repo_storage_cleanup_jobs",
    RepoId => "repo_id",
    Generation => "generation",
    OwnerHandle => "owner_handle",
    RepoName => "repo_name",
    Attempts => "attempts",
    NextRunAtUnix => "next_run_at_unix",
    LastError => "last_error",
    CompletedAtUnix => "completed_at_unix",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
});

#[derive(Copy, Clone)]
pub enum SourceBlobCleanupJobs {
    Table,
    ObjectKey,
    Generation,
    Sha256,
    GitOid,
    SizeBytes,
    LineCount,
    Attempts,
    NextRunAtUnix,
    LastError,
    CompletedAtUnix,
    CreatedAtUnix,
    UpdatedAtUnix,
}

impl_iden!(SourceBlobCleanupJobs {
    Table => "scope_source_blob_cleanup_jobs",
    ObjectKey => "object_key",
    Generation => "generation",
    Sha256 => "sha256",
    GitOid => "git_oid",
    SizeBytes => "size_bytes",
    LineCount => "line_count",
    Attempts => "attempts",
    NextRunAtUnix => "next_run_at_unix",
    LastError => "last_error",
    CompletedAtUnix => "completed_at_unix",
    CreatedAtUnix => "created_at_unix",
    UpdatedAtUnix => "updated_at_unix",
});

#[derive(Copy, Clone)]
pub enum MetadataResetEvents {
    Table,
    Id,
    ResetAtUnix,
    Trigger,
    Reason,
}

impl_iden!(MetadataResetEvents {
    Table => "scope_metadata_reset_events",
    Id => "id",
    ResetAtUnix => "reset_at_unix",
    Trigger => "trigger",
    Reason => "reason",
});

const METADATA_LOCK_COLUMNS: &[&str] = &[MetadataLocks::Key.as_str()];
const REPO_STORAGE_CLEANUP_JOB_COLUMNS: &[&str] = &[
    RepoStorageCleanupJobs::RepoId.as_str(),
    RepoStorageCleanupJobs::Generation.as_str(),
    RepoStorageCleanupJobs::OwnerHandle.as_str(),
    RepoStorageCleanupJobs::RepoName.as_str(),
    RepoStorageCleanupJobs::Attempts.as_str(),
    RepoStorageCleanupJobs::NextRunAtUnix.as_str(),
    RepoStorageCleanupJobs::LastError.as_str(),
    RepoStorageCleanupJobs::CompletedAtUnix.as_str(),
    RepoStorageCleanupJobs::CreatedAtUnix.as_str(),
    RepoStorageCleanupJobs::UpdatedAtUnix.as_str(),
];

const SOURCE_BLOB_CLEANUP_JOB_COLUMNS: &[&str] = &[
    SourceBlobCleanupJobs::ObjectKey.as_str(),
    SourceBlobCleanupJobs::Generation.as_str(),
    SourceBlobCleanupJobs::Sha256.as_str(),
    SourceBlobCleanupJobs::GitOid.as_str(),
    SourceBlobCleanupJobs::SizeBytes.as_str(),
    SourceBlobCleanupJobs::LineCount.as_str(),
    SourceBlobCleanupJobs::Attempts.as_str(),
    SourceBlobCleanupJobs::NextRunAtUnix.as_str(),
    SourceBlobCleanupJobs::LastError.as_str(),
    SourceBlobCleanupJobs::CompletedAtUnix.as_str(),
    SourceBlobCleanupJobs::CreatedAtUnix.as_str(),
    SourceBlobCleanupJobs::UpdatedAtUnix.as_str(),
];

const OUTBOX_JOB_COLUMNS: &[&str] = &[
    OutboxJobs::Id.as_str(),
    OutboxJobs::IdempotencyKey.as_str(),
    OutboxJobs::Kind.as_str(),
    OutboxJobs::RepoId.as_str(),
    OutboxJobs::RepoVersion.as_str(),
    OutboxJobs::Payload.as_str(),
    OutboxJobs::State.as_str(),
    OutboxJobs::Attempts.as_str(),
    OutboxJobs::NextRunAtUnix.as_str(),
    OutboxJobs::LeaseOwner.as_str(),
    OutboxJobs::LeaseExpiresAtUnix.as_str(),
    OutboxJobs::LastError.as_str(),
    OutboxJobs::CreatedAtUnix.as_str(),
    OutboxJobs::UpdatedAtUnix.as_str(),
    OutboxJobs::CompletedAtUnix.as_str(),
];

pub const LOCK_AND_CLEANUP_TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: MetadataLocks::Table.as_str(),
        columns: METADATA_LOCK_COLUMNS,
        counts_for_catalog_rows: false,
    },
    MetadataTableSpec {
        table: RepoStorageCleanupJobs::Table.as_str(),
        columns: REPO_STORAGE_CLEANUP_JOB_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: SourceBlobCleanupJobs::Table.as_str(),
        columns: SOURCE_BLOB_CLEANUP_JOB_COLUMNS,
        counts_for_catalog_rows: true,
    },
];

pub const OUTBOX_TABLES: &[MetadataTableSpec] = &[MetadataTableSpec {
    table: OutboxJobs::Table.as_str(),
    columns: OUTBOX_JOB_COLUMNS,
    counts_for_catalog_rows: false,
}];
