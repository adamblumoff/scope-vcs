use super::{auth, repositories, *};

const REPOSITORY_MEMBER_COLUMNS: &[ColumnSpec] = &[
    required_column("repo_id", ColumnType::String),
    required_column("user_id", ColumnType::String),
    required_column("permissions", ColumnType::JsonBinary),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
];

const REPOSITORY_MEMBER_INDEXES: &[IndexSpec] =
    &[index("idx_scope_repository_members_user", &["user_id"])];

const REPOSITORY_MEMBER_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_repository_members_repo",
        "repo_id",
        repositories::REPOSITORIES.name,
        repositories::REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_repository_members_user",
        "user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REPOSITORY_MEMBERS: TableSpec = TableSpec {
    name: "scope_repository_members",
    columns: REPOSITORY_MEMBER_COLUMNS,
    primary_key: composite_primary_key("pk_scope_repository_members", &["repo_id", "user_id"]),
    indexes: REPOSITORY_MEMBER_INDEXES,
    foreign_keys: REPOSITORY_MEMBER_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 100,
};

const REPOSITORY_INVITE_COLUMNS: &[ColumnSpec] = &[
    required_column("id", ColumnType::String),
    required_column("repo_id", ColumnType::String),
    required_column("invited_email", ColumnType::String),
    required_column("invited_email_normalized", ColumnType::String),
    required_column("permissions", ColumnType::JsonBinary),
    required_column("invited_by_user_id", ColumnType::String),
    required_column("state", ColumnType::String),
    required_column("token_hash", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("accepted_by_user_id", ColumnType::String),
    nullable_column("accepted_at_unix", ColumnType::BigInteger),
    nullable_column("revoked_at_unix", ColumnType::BigInteger),
];

const REPOSITORY_INVITE_INDEXES: &[IndexSpec] = &[
    index(
        "idx_scope_repository_invites_repo_email",
        &["repo_id", "invited_email_normalized"],
    ),
    index("idx_scope_repository_invites_token_hash", &["token_hash"]),
];

const REPOSITORY_INVITE_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_repository_invites_repo",
        "repo_id",
        repositories::REPOSITORIES.name,
        repositories::REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_repository_invites_inviter",
        "invited_by_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
    set_null_fk(
        "fk_scope_repository_invites_accepted_user",
        "accepted_by_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REPOSITORY_INVITES: TableSpec = TableSpec {
    name: "scope_repository_invites",
    columns: REPOSITORY_INVITE_COLUMNS,
    primary_key: inline_primary_key("id"),
    indexes: REPOSITORY_INVITE_INDEXES,
    foreign_keys: REPOSITORY_INVITE_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 90,
};

pub const TABLES: &[TableSpec] = &[REPOSITORY_MEMBERS, REPOSITORY_INVITES];
