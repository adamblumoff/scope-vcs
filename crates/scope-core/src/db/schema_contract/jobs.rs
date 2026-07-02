use super::{repositories, *};

pub const METADATA_LOCKS: TableSpec = TableSpec {
    name: "scope_metadata_locks",
    columns: &[required_column("key", ColumnType::String)],
    primary_key: inline_primary_key("key"),
    indexes: &[],
    foreign_keys: &[],
    counts_for_catalog_rows: false,
    reset_order: 200,
};

const REPO_STORAGE_CLEANUP_JOB_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("generation", ColumnType::String),
    required_column("owner_handle", ColumnType::String),
    required_column("repo_name", ColumnType::String),
    required_column("attempts", ColumnType::Integer),
    required_column("next_run_at_unix", ColumnType::BigInteger),
    nullable_column("last_error", ColumnType::Text),
    nullable_column("completed_at_unix", ColumnType::BigInteger),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
];

const REPO_STORAGE_CLEANUP_JOB_INDEXES: &[IndexSpec] = &[index(
    "idx_scope_repo_storage_cleanup_jobs_pending",
    &["completed_at_unix", "next_run_at_unix"],
)];

pub const REPO_STORAGE_CLEANUP_JOBS: TableSpec = TableSpec {
    name: "scope_repo_storage_cleanup_jobs",
    columns: REPO_STORAGE_CLEANUP_JOB_COLUMNS,
    primary_key: inline_primary_key("repo_id"),
    indexes: REPO_STORAGE_CLEANUP_JOB_INDEXES,
    foreign_keys: &[],
    counts_for_catalog_rows: true,
    reset_order: 30,
};

const SOURCE_BLOB_CLEANUP_JOB_COLUMNS: &[ColumnSpec] = &[
    required_column("object_key", ColumnType::String),
    required_column("generation", ColumnType::String),
    required_column("sha256", ColumnType::String),
    required_column("git_oid", ColumnType::String),
    required_column("size_bytes", ColumnType::BigInteger),
    required_column("line_count", ColumnType::BigInteger),
    required_column("attempts", ColumnType::Integer),
    required_column("next_run_at_unix", ColumnType::BigInteger),
    nullable_column("last_error", ColumnType::Text),
    nullable_column("completed_at_unix", ColumnType::BigInteger),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
];

const SOURCE_BLOB_CLEANUP_JOB_INDEXES: &[IndexSpec] = &[index(
    "idx_scope_source_blob_cleanup_jobs_pending",
    &["completed_at_unix", "next_run_at_unix"],
)];

pub const SOURCE_BLOB_CLEANUP_JOBS: TableSpec = TableSpec {
    name: "scope_source_blob_cleanup_jobs",
    columns: SOURCE_BLOB_CLEANUP_JOB_COLUMNS,
    primary_key: inline_primary_key("object_key"),
    indexes: SOURCE_BLOB_CLEANUP_JOB_INDEXES,
    foreign_keys: &[],
    counts_for_catalog_rows: true,
    reset_order: 20,
};

const OUTBOX_JOB_COLUMNS: &[ColumnSpec] = &[
    required_column("id", ColumnType::String),
    unique_column("idempotency_key", ColumnType::String),
    required_column("kind", ColumnType::String),
    required_column("repo_id", ColumnType::String),
    required_column("repo_version", ColumnType::BigInteger),
    required_column("payload", ColumnType::JsonBinary),
    required_column("state", ColumnType::String),
    required_column("attempts", ColumnType::BigInteger),
    required_column("next_run_at_unix", ColumnType::BigInteger),
    nullable_column("lease_owner", ColumnType::String),
    nullable_column("lease_expires_at_unix", ColumnType::BigInteger),
    nullable_column("last_error", ColumnType::Text),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
    nullable_column("completed_at_unix", ColumnType::BigInteger),
];

const OUTBOX_JOB_INDEXES: &[IndexSpec] = &[
    index(
        "idx_scope_outbox_jobs_ready",
        &["state", "next_run_at_unix", "created_at_unix"],
    ),
    index("idx_scope_outbox_jobs_repo", &["repo_id", "repo_version"]),
];

const OUTBOX_JOB_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_outbox_jobs_repo",
    "repo_id",
    repositories::REPOSITORIES.name,
    repositories::REPOSITORY_ID,
)];

pub const OUTBOX_JOBS: TableSpec = TableSpec {
    name: "scope_outbox_jobs",
    columns: OUTBOX_JOB_COLUMNS,
    primary_key: inline_primary_key("id"),
    indexes: OUTBOX_JOB_INDEXES,
    foreign_keys: OUTBOX_JOB_FOREIGN_KEYS,
    counts_for_catalog_rows: false,
    reset_order: 10,
};

pub const METADATA_RESET_EVENTS: TableSpec = TableSpec {
    name: "scope_metadata_reset_events",
    columns: &[
        required_column("id", ColumnType::String),
        required_column("reset_at_unix", ColumnType::BigInteger),
        required_column("trigger", ColumnType::String),
        required_column("reason", ColumnType::Text),
    ],
    primary_key: inline_primary_key("id"),
    indexes: &[],
    foreign_keys: &[],
    counts_for_catalog_rows: false,
    reset_order: 0,
};

pub const LOCK_AND_CLEANUP_TABLES: &[TableSpec] = &[
    METADATA_LOCKS,
    REPO_STORAGE_CLEANUP_JOBS,
    SOURCE_BLOB_CLEANUP_JOBS,
];

pub const OUTBOX_TABLES: &[TableSpec] = &[OUTBOX_JOBS];
