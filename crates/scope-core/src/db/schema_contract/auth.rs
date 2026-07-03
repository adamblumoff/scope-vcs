use super::*;

pub const USER_ID: &str = "id";
pub const USER_EMAIL: &str = "email";

const USER_COLUMNS: &[ColumnSpec] = &[
    required_column(USER_ID, ColumnType::String),
    unique_column("handle", ColumnType::String),
    required_column(USER_EMAIL, ColumnType::String),
    required_column("email_verified", ColumnType::Boolean),
];

const USER_INDEXES: &[IndexSpec] = &[unique_index("idx_scope_users_email", &[USER_EMAIL])];

pub const USERS: TableSpec = TableSpec {
    name: "scope_users",
    columns: USER_COLUMNS,
    primary_key: inline_primary_key(USER_ID),
    indexes: USER_INDEXES,
    foreign_keys: &[],
    counts_for_catalog_rows: true,
    reset_order: 190,
};

const AUTH_IDENTITY_COLUMNS: &[ColumnSpec] = &[
    required_column("provider", ColumnType::String),
    required_column("subject", ColumnType::String),
    required_column("user_id", ColumnType::String),
];

const AUTH_IDENTITY_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_auth_identities_user",
    "user_id",
    USERS.name,
    USER_ID,
)];

const AUTH_IDENTITY_INDEXES: &[IndexSpec] =
    &[index("idx_scope_auth_identities_user", &["user_id"])];

pub const AUTH_IDENTITIES: TableSpec = TableSpec {
    name: "scope_auth_identities",
    columns: AUTH_IDENTITY_COLUMNS,
    primary_key: composite_primary_key("pk_scope_auth_identities", &["provider", "subject"]),
    indexes: AUTH_IDENTITY_INDEXES,
    foreign_keys: AUTH_IDENTITY_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 80,
};

const CLI_DEVICE_LOGIN_COLUMNS: &[ColumnSpec] = &[
    required_column("device_code_hash", ColumnType::String),
    unique_column("user_code_hash", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("completed_user_id", ColumnType::String),
    nullable_column("completed_at_unix", ColumnType::BigInteger),
    nullable_column("consumed_at_unix", ColumnType::BigInteger),
];

const CLI_DEVICE_LOGIN_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_cli_device_logins_completed_user",
    "completed_user_id",
    USERS.name,
    USER_ID,
)];

pub const CLI_DEVICE_LOGINS: TableSpec = TableSpec {
    name: "scope_cli_device_logins",
    columns: CLI_DEVICE_LOGIN_COLUMNS,
    primary_key: inline_primary_key("device_code_hash"),
    indexes: &[],
    foreign_keys: CLI_DEVICE_LOGIN_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 70,
};

const CLI_BROWSER_LOGIN_COLUMNS: &[ColumnSpec] = &[
    required_column("request_id", ColumnType::String),
    required_column("request_secret_hash", ColumnType::String),
    required_column("callback_url", ColumnType::Text),
    nullable_column("callback_code_hash", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("completed_user_id", ColumnType::String),
    nullable_column("completed_at_unix", ColumnType::BigInteger),
    nullable_column("consumed_at_unix", ColumnType::BigInteger),
];

const CLI_BROWSER_LOGIN_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_cli_browser_logins_completed_user",
    "completed_user_id",
    USERS.name,
    USER_ID,
)];

pub const CLI_BROWSER_LOGINS: TableSpec = TableSpec {
    name: "scope_cli_browser_logins",
    columns: CLI_BROWSER_LOGIN_COLUMNS,
    primary_key: inline_primary_key("request_id"),
    indexes: &[],
    foreign_keys: CLI_BROWSER_LOGIN_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 60,
};

const CLI_EXCHANGE_GRANT_COLUMNS: &[ColumnSpec] = &[
    required_column("grant_hash", ColumnType::String),
    required_column("user_id", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("consumed_at_unix", ColumnType::BigInteger),
];

const CLI_EXCHANGE_GRANT_INDEXES: &[IndexSpec] =
    &[index("idx_scope_cli_exchange_grants_user", &["user_id"])];

const CLI_EXCHANGE_GRANT_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_cli_exchange_grants_user",
    "user_id",
    USERS.name,
    USER_ID,
)];

pub const CLI_EXCHANGE_GRANTS: TableSpec = TableSpec {
    name: "scope_cli_exchange_grants",
    columns: CLI_EXCHANGE_GRANT_COLUMNS,
    primary_key: inline_primary_key("grant_hash"),
    indexes: CLI_EXCHANGE_GRANT_INDEXES,
    foreign_keys: CLI_EXCHANGE_GRANT_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 50,
};

const CLI_SESSION_COLUMNS: &[ColumnSpec] = &[
    required_column("id", ColumnType::String),
    unique_column("token_hash", ColumnType::String),
    required_column("user_id", ColumnType::String),
    required_column("label", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
    nullable_column("last_used_at_unix", ColumnType::BigInteger),
    required_column("expires_at_unix", ColumnType::BigInteger),
    nullable_column("revoked_at_unix", ColumnType::BigInteger),
];

const CLI_SESSION_INDEXES: &[IndexSpec] = &[index("idx_scope_cli_sessions_user", &["user_id"])];

const CLI_SESSION_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_cli_sessions_user",
    "user_id",
    USERS.name,
    USER_ID,
)];

pub const CLI_SESSIONS: TableSpec = TableSpec {
    name: "scope_cli_sessions",
    columns: CLI_SESSION_COLUMNS,
    primary_key: inline_primary_key("id"),
    indexes: CLI_SESSION_INDEXES,
    foreign_keys: CLI_SESSION_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 40,
};

pub const TABLES: &[TableSpec] = &[
    USERS,
    AUTH_IDENTITIES,
    CLI_DEVICE_LOGINS,
    CLI_BROWSER_LOGINS,
    CLI_EXCHANGE_GRANTS,
    CLI_SESSIONS,
];
