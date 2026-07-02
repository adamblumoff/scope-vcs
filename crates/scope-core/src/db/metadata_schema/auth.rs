use super::*;

#[derive(Copy, Clone)]
pub enum Users {
    Table,
    Id,
    Handle,
    Email,
    EmailVerified,
    Access,
}

impl_iden!(Users {
    Table => "scope_users",
    Id => "id",
    Handle => "handle",
    Email => "email",
    EmailVerified => "email_verified",
    Access => "access",
});

#[derive(Copy, Clone)]
pub enum AuthIdentities {
    Table,
    Provider,
    Subject,
    UserId,
}

impl_iden!(AuthIdentities {
    Table => "scope_auth_identities",
    Provider => "provider",
    Subject => "subject",
    UserId => "user_id",
});

#[derive(Copy, Clone)]
pub enum CliDeviceLogins {
    Table,
    DeviceCodeHash,
    UserCodeHash,
    CreatedAtUnix,
    ExpiresAtUnix,
    CompletedUserId,
    CompletedAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliDeviceLogins {
    Table => "scope_cli_device_logins",
    DeviceCodeHash => "device_code_hash",
    UserCodeHash => "user_code_hash",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    CompletedUserId => "completed_user_id",
    CompletedAtUnix => "completed_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub enum CliBrowserLogins {
    Table,
    RequestId,
    RequestSecretHash,
    CallbackUrl,
    CallbackCodeHash,
    CreatedAtUnix,
    ExpiresAtUnix,
    CompletedUserId,
    CompletedAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliBrowserLogins {
    Table => "scope_cli_browser_logins",
    RequestId => "request_id",
    RequestSecretHash => "request_secret_hash",
    CallbackUrl => "callback_url",
    CallbackCodeHash => "callback_code_hash",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    CompletedUserId => "completed_user_id",
    CompletedAtUnix => "completed_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub enum CliExchangeGrants {
    Table,
    GrantHash,
    UserId,
    CreatedAtUnix,
    ExpiresAtUnix,
    ConsumedAtUnix,
}

impl_iden!(CliExchangeGrants {
    Table => "scope_cli_exchange_grants",
    GrantHash => "grant_hash",
    UserId => "user_id",
    CreatedAtUnix => "created_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    ConsumedAtUnix => "consumed_at_unix",
});

#[derive(Copy, Clone)]
pub enum CliSessions {
    Table,
    Id,
    TokenHash,
    UserId,
    Label,
    CreatedAtUnix,
    LastUsedAtUnix,
    ExpiresAtUnix,
    RevokedAtUnix,
}

impl_iden!(CliSessions {
    Table => "scope_cli_sessions",
    Id => "id",
    TokenHash => "token_hash",
    UserId => "user_id",
    Label => "label",
    CreatedAtUnix => "created_at_unix",
    LastUsedAtUnix => "last_used_at_unix",
    ExpiresAtUnix => "expires_at_unix",
    RevokedAtUnix => "revoked_at_unix",
});

const USER_COLUMNS: &[&str] = &[
    Users::Id.as_str(),
    Users::Handle.as_str(),
    Users::Email.as_str(),
    Users::EmailVerified.as_str(),
    Users::Access.as_str(),
];

const AUTH_IDENTITY_COLUMNS: &[&str] = &[
    AuthIdentities::Provider.as_str(),
    AuthIdentities::Subject.as_str(),
    AuthIdentities::UserId.as_str(),
];

const CLI_DEVICE_LOGIN_COLUMNS: &[&str] = &[
    CliDeviceLogins::DeviceCodeHash.as_str(),
    CliDeviceLogins::UserCodeHash.as_str(),
    CliDeviceLogins::CreatedAtUnix.as_str(),
    CliDeviceLogins::ExpiresAtUnix.as_str(),
    CliDeviceLogins::CompletedUserId.as_str(),
    CliDeviceLogins::CompletedAtUnix.as_str(),
    CliDeviceLogins::ConsumedAtUnix.as_str(),
];

const CLI_BROWSER_LOGIN_COLUMNS: &[&str] = &[
    CliBrowserLogins::RequestId.as_str(),
    CliBrowserLogins::RequestSecretHash.as_str(),
    CliBrowserLogins::CallbackUrl.as_str(),
    CliBrowserLogins::CallbackCodeHash.as_str(),
    CliBrowserLogins::CreatedAtUnix.as_str(),
    CliBrowserLogins::ExpiresAtUnix.as_str(),
    CliBrowserLogins::CompletedUserId.as_str(),
    CliBrowserLogins::CompletedAtUnix.as_str(),
    CliBrowserLogins::ConsumedAtUnix.as_str(),
];

const CLI_EXCHANGE_GRANT_COLUMNS: &[&str] = &[
    CliExchangeGrants::GrantHash.as_str(),
    CliExchangeGrants::UserId.as_str(),
    CliExchangeGrants::CreatedAtUnix.as_str(),
    CliExchangeGrants::ExpiresAtUnix.as_str(),
    CliExchangeGrants::ConsumedAtUnix.as_str(),
];

const CLI_SESSION_COLUMNS: &[&str] = &[
    CliSessions::Id.as_str(),
    CliSessions::TokenHash.as_str(),
    CliSessions::UserId.as_str(),
    CliSessions::Label.as_str(),
    CliSessions::CreatedAtUnix.as_str(),
    CliSessions::LastUsedAtUnix.as_str(),
    CliSessions::ExpiresAtUnix.as_str(),
    CliSessions::RevokedAtUnix.as_str(),
];

pub const USER_IDENTITY_TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: Users::Table.as_str(),
        columns: USER_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: AuthIdentities::Table.as_str(),
        columns: AUTH_IDENTITY_COLUMNS,
        counts_for_catalog_rows: true,
    },
];

pub const CLI_TABLES: &[MetadataTableSpec] = &[
    MetadataTableSpec {
        table: CliDeviceLogins::Table.as_str(),
        columns: CLI_DEVICE_LOGIN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliBrowserLogins::Table.as_str(),
        columns: CLI_BROWSER_LOGIN_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliExchangeGrants::Table.as_str(),
        columns: CLI_EXCHANGE_GRANT_COLUMNS,
        counts_for_catalog_rows: true,
    },
    MetadataTableSpec {
        table: CliSessions::Table.as_str(),
        columns: CLI_SESSION_COLUMNS,
        counts_for_catalog_rows: true,
    },
];
