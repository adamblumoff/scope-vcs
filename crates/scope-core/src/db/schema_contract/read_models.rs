use super::{repositories, *};

const PROJECTION_READ_MODEL_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("repo_version", ColumnType::BigInteger),
    required_column("source", ColumnType::String),
    required_column("audience", ColumnType::String),
    required_column("rebuilt_at_unix", ColumnType::BigInteger),
    required_column("file_count", ColumnType::BigInteger),
];

const PROJECTION_READ_MODEL_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_projection_read_models_repo",
    "repo_id",
    repositories::REPOSITORIES.name,
    repositories::REPOSITORY_ID,
)];

pub const PROJECTION_READ_MODELS: TableSpec = TableSpec {
    name: "scope_projection_read_models",
    columns: PROJECTION_READ_MODEL_COLUMNS,
    primary_key: composite_primary_key(
        "pk_scope_projection_read_models",
        &["repo_id", "source", "audience"],
    ),
    indexes: &[],
    foreign_keys: PROJECTION_READ_MODEL_FOREIGN_KEYS,
    counts_for_catalog_rows: false,
    reset_order: 120,
};

const PROJECTION_FILE_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("repo_version", ColumnType::BigInteger),
    required_column("source", ColumnType::String),
    required_column("audience", ColumnType::String),
    required_column("path_key", ColumnType::String),
    required_column("path", ColumnType::String),
    required_column("oid", ColumnType::String),
    required_column("visibility", ColumnType::String),
];

const PROJECTION_FILE_INDEXES: &[IndexSpec] = &[index(
    "idx_scope_projection_files_lookup",
    &["repo_id", "repo_version", "source", "audience"],
)];

const PROJECTION_FILE_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_projection_files_repo",
    "repo_id",
    repositories::REPOSITORIES.name,
    repositories::REPOSITORY_ID,
)];

pub const PROJECTION_FILES: TableSpec = TableSpec {
    name: "scope_projection_files",
    columns: PROJECTION_FILE_COLUMNS,
    primary_key: composite_primary_key(
        "pk_scope_projection_files",
        &["repo_id", "source", "audience", "path_key"],
    ),
    indexes: PROJECTION_FILE_INDEXES,
    foreign_keys: PROJECTION_FILE_FOREIGN_KEYS,
    counts_for_catalog_rows: false,
    reset_order: 110,
};

pub const TABLES: &[TableSpec] = &[PROJECTION_READ_MODELS, PROJECTION_FILES];
