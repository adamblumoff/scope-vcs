use super::{auth, repositories, *};

pub const REQUEST_ID: &str = "id";

const REQUEST_COLUMNS: &[ColumnSpec] = &[
    required_column(REQUEST_ID, ColumnType::String),
    required_column("repo_id", ColumnType::String),
    required_column("author_user_id", ColumnType::String),
    required_column("author_role", ColumnType::String),
    required_column("base_audience", ColumnType::String),
    required_column("target_branch", ColumnType::String),
    unique_column("request_ref", ColumnType::String),
    required_column("base_main_oid", ColumnType::String),
    required_column("head_oid", ColumnType::String),
    required_column("title", ColumnType::Text),
    required_column("state", ColumnType::String),
    required_column("stake_credits", ColumnType::Integer),
    nullable_column("disposition", ColumnType::String),
    nullable_column("settlement", ColumnType::JsonBinary),
    required_column("created_at_unix", ColumnType::BigInteger),
    required_column("updated_at_unix", ColumnType::BigInteger),
    nullable_column("resolved_at_unix", ColumnType::BigInteger),
];

const REQUEST_INDEXES: &[IndexSpec] = &[
    index("idx_scope_requests_repo_state", &["repo_id", "state"]),
    index("idx_scope_requests_author", &["author_user_id"]),
];

const REQUEST_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_requests_repo",
        "repo_id",
        repositories::REPOSITORIES.name,
        repositories::REPOSITORY_ID,
    ),
    cascade_fk(
        "fk_scope_requests_author",
        "author_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REQUESTS: TableSpec = TableSpec {
    name: "scope_requests",
    columns: REQUEST_COLUMNS,
    primary_key: inline_primary_key(REQUEST_ID),
    indexes: REQUEST_INDEXES,
    foreign_keys: REQUEST_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 139,
};

const REQUEST_EVENT_COLUMNS: &[ColumnSpec] = &[
    required_column("id", ColumnType::String),
    required_column("request_id", ColumnType::String),
    required_column("actor_user_id", ColumnType::String),
    required_column("kind", ColumnType::String),
    nullable_column("body", ColumnType::Text),
    nullable_column("old_head_oid", ColumnType::String),
    nullable_column("new_head_oid", ColumnType::String),
    required_column("created_at_unix", ColumnType::BigInteger),
];

const REQUEST_EVENT_INDEXES: &[IndexSpec] = &[index(
    "idx_scope_request_events_request_time",
    &["request_id", "created_at_unix"],
)];

const REQUEST_EVENT_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_request_events_request",
        "request_id",
        REQUESTS.name,
        REQUEST_ID,
    ),
    cascade_fk(
        "fk_scope_request_events_actor",
        "actor_user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
];

pub const REQUEST_EVENTS: TableSpec = TableSpec {
    name: "scope_request_events",
    columns: REQUEST_EVENT_COLUMNS,
    primary_key: inline_primary_key("id"),
    indexes: REQUEST_EVENT_INDEXES,
    foreign_keys: REQUEST_EVENT_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 138,
};

const USER_CREDIT_ACCOUNT_COLUMNS: &[ColumnSpec] = &[
    required_column("user_id", ColumnType::String),
    required_column("balance_credits", ColumnType::Integer),
];

const USER_CREDIT_ACCOUNT_FOREIGN_KEYS: &[ForeignKeySpec] = &[cascade_fk(
    "fk_scope_user_credit_accounts_user",
    "user_id",
    auth::USERS.name,
    auth::USER_ID,
)];

pub const USER_CREDIT_ACCOUNTS: TableSpec = TableSpec {
    name: "scope_user_credit_accounts",
    columns: USER_CREDIT_ACCOUNT_COLUMNS,
    primary_key: inline_primary_key("user_id"),
    indexes: &[],
    foreign_keys: USER_CREDIT_ACCOUNT_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 137,
};

const CREDIT_LEDGER_ENTRY_COLUMNS: &[ColumnSpec] = &[
    required_column("id", ColumnType::String),
    required_column("user_id", ColumnType::String),
    nullable_column("request_id", ColumnType::String),
    required_column("kind", ColumnType::String),
    required_column("amount_credits", ColumnType::Integer),
    required_column("created_at_unix", ColumnType::BigInteger),
];

const CREDIT_LEDGER_ENTRY_INDEXES: &[IndexSpec] = &[
    index(
        "idx_scope_credit_ledger_entries_user_time",
        &["user_id", "created_at_unix"],
    ),
    index("idx_scope_credit_ledger_entries_request", &["request_id"]),
];

const CREDIT_LEDGER_ENTRY_FOREIGN_KEYS: &[ForeignKeySpec] = &[
    cascade_fk(
        "fk_scope_credit_ledger_entries_user",
        "user_id",
        auth::USERS.name,
        auth::USER_ID,
    ),
    set_null_fk(
        "fk_scope_credit_ledger_entries_request",
        "request_id",
        REQUESTS.name,
        REQUEST_ID,
    ),
];

pub const CREDIT_LEDGER_ENTRIES: TableSpec = TableSpec {
    name: "scope_credit_ledger_entries",
    columns: CREDIT_LEDGER_ENTRY_COLUMNS,
    primary_key: inline_primary_key("id"),
    indexes: CREDIT_LEDGER_ENTRY_INDEXES,
    foreign_keys: CREDIT_LEDGER_ENTRY_FOREIGN_KEYS,
    counts_for_catalog_rows: true,
    reset_order: 136,
};

pub const TABLES: &[TableSpec] = &[
    REQUESTS,
    REQUEST_EVENTS,
    USER_CREDIT_ACCOUNTS,
    CREDIT_LEDGER_ENTRIES,
];
