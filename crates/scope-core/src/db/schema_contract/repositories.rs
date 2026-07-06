use super::{auth, *};

pub const REPOSITORY_ID: &str = "id";

const REPOSITORY_COLUMNS: &[ColumnSpec] = &[
    required_column(REPOSITORY_ID, ColumnType::String),
    required_column("owner_handle", ColumnType::String),
    required_column("name", ColumnType::String),
    required_column("owner_user_id", ColumnType::String),
    required_column("publication_state", ColumnType::String),
    required_column("default_visibility", ColumnType::String),
    required_column("change_version", ColumnType::BigInteger),
    required_column("repo_config", ColumnType::JsonBinary),
    nullable_column("pending_import", ColumnType::JsonBinary),
    required_column("policy", ColumnType::JsonBinary),
    required_column("graph", ColumnType::JsonBinary),
    required_column("visibility_events", ColumnType::JsonBinary),
    nullable_column("staged_update", ColumnType::JsonBinary),
];

const REPOSITORY_INDEXES: &[IndexSpec] = &[unique_index(
    "idx_scope_repositories_owner_name",
    &["owner_handle", "name"],
)];

const REPOSITORY_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_repositories_owner",
    "owner_user_id",
    auth::USERS.name,
    auth::USER_ID,
)];

pub const REPOSITORIES: TableSpec = TableSpec {
    name: "scope_repositories",
    columns: REPOSITORY_COLUMNS,
    primary_key: inline_primary_key(REPOSITORY_ID),
    indexes: REPOSITORY_INDEXES,
    foreign_keys: REPOSITORY_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 180,
};

const REPOSITORY_SETTING_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("include_ignored_files", ColumnType::Boolean),
    required_column("review_pushes_before_applying", ColumnType::Boolean),
];

const REPOSITORY_SETTING_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_repository_settings_repo",
    "repo_id",
    REPOSITORIES.name,
    REPOSITORY_ID,
)];

pub const REPOSITORY_SETTINGS: TableSpec = TableSpec {
    name: "scope_repository_settings",
    columns: REPOSITORY_SETTING_COLUMNS,
    primary_key: inline_primary_key("repo_id"),
    indexes: &[],
    foreign_keys: REPOSITORY_SETTING_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 170,
};

const REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("token_hash", ColumnType::String),
    required_column("owner_user_id", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("used_at_unix", ColumnType::BigInteger),
];

const REPOSITORY_FIRST_PUSH_TOKEN_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_repository_first_push_tokens_repo",
        "repo_id",
        REPOSITORIES.name,
        REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_repository_first_push_tokens_owner",
        "owner_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REPOSITORY_FIRST_PUSH_TOKENS: TableSpec = TableSpec {
    name: "scope_repository_first_push_tokens",
    columns: REPOSITORY_FIRST_PUSH_TOKEN_COLUMNS,
    primary_key: inline_primary_key("repo_id"),
    indexes: &[],
    foreign_keys: REPOSITORY_FIRST_PUSH_TOKEN_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 160,
};

const REPOSITORY_GIT_PUSH_TOKEN_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("token_hash", ColumnType::String),
    required_column("owner_user_id", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
];

const REPOSITORY_GIT_PUSH_TOKEN_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_repository_git_push_tokens_repo",
        "repo_id",
        REPOSITORIES.name,
        REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_repository_git_push_tokens_owner",
        "owner_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REPOSITORY_GIT_PUSH_TOKENS: TableSpec = TableSpec {
    name: "scope_repository_git_push_tokens",
    columns: REPOSITORY_GIT_PUSH_TOKEN_COLUMNS,
    primary_key: inline_primary_key("repo_id"),
    indexes: &[],
    foreign_keys: REPOSITORY_GIT_PUSH_TOKEN_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 150,
};

const REPOSITORY_GIT_CLONE_TOKEN_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("token_hash", ColumnType::String),
    required_column("user_id", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
];

const REPOSITORY_GIT_CLONE_TOKEN_INDEXES: &[IndexSpec] = &[index(
    "idx_scope_repository_git_clone_tokens_user",
    &["user_id"],
)];

const REPOSITORY_GIT_CLONE_TOKEN_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_repository_git_clone_tokens_repo",
        "repo_id",
        REPOSITORIES.name,
        REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_repository_git_clone_tokens_user",
        "user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REPOSITORY_GIT_CLONE_TOKENS: TableSpec = TableSpec {
    name: "scope_repository_git_clone_tokens",
    columns: REPOSITORY_GIT_CLONE_TOKEN_COLUMNS,
    primary_key: composite_primary_key(
        "pk_scope_repository_git_clone_tokens",
        &["repo_id", "token_hash"],
    ),
    indexes: REPOSITORY_GIT_CLONE_TOKEN_INDEXES,
    foreign_keys: REPOSITORY_GIT_CLONE_TOKEN_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 130,
};

const REPOSITORY_GIT_SNAPSHOT_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("object_key", ColumnType::String),
    required_column("sha256", ColumnType::String),
    required_column("git_oid", ColumnType::String),
    required_column("size_bytes", ColumnType::BigInteger),
];

const REPOSITORY_GIT_SNAPSHOT_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_repository_git_snapshots_repo",
    "repo_id",
    REPOSITORIES.name,
    REPOSITORY_ID,
)];

pub const REPOSITORY_GIT_SNAPSHOTS: TableSpec = TableSpec {
    name: "scope_repository_git_snapshots",
    columns: REPOSITORY_GIT_SNAPSHOT_COLUMNS,
    primary_key: inline_primary_key("repo_id"),
    indexes: &[],
    foreign_keys: REPOSITORY_GIT_SNAPSHOT_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 140,
};

pub const TABLES: &[TableSpec] = &[
    REPOSITORIES,
    REPOSITORY_SETTINGS,
    REPOSITORY_FIRST_PUSH_TOKENS,
    REPOSITORY_GIT_PUSH_TOKENS,
    REPOSITORY_GIT_CLONE_TOKENS,
    REPOSITORY_GIT_SNAPSHOTS,
];
