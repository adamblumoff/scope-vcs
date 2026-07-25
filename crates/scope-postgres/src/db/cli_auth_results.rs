use scope_domain::account::SessionIdentity;

pub struct StartDeviceLoginCommand {
    pub device_code_hash: String,
    pub user_code_hash: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

pub enum DeviceLoginPoll {
    Pending { expires_at_unix: u64 },
    Complete { identity: SessionIdentity },
}

pub struct StartBrowserLoginCommand {
    pub request_id: String,
    pub request_secret_hash: String,
    pub callback_url: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

pub struct BrowserLoginCompletion {
    pub request_id: String,
    pub callback_url: String,
}

pub struct CreateCliExchangeGrantCommand {
    pub grant_hash: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

pub struct NewCliSession {
    pub id: String,
    pub token_hash: String,
    pub label: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

pub struct CliSessionSummary {
    pub id: String,
    pub label: String,
    pub created_at_unix: u64,
    pub last_used_at_unix: Option<u64>,
    pub expires_at_unix: u64,
}
