use scope_api_contract::CLI_PROTOCOL_VERSION;
use std::sync::OnceLock;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_SHA: &str = match option_env!("SCOPE_BUILD_SHA") {
    Some(sha) => sha,
    None => "development",
};

pub fn version_identity() -> &'static str {
    static IDENTITY: OnceLock<String> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            format!("{PACKAGE_VERSION} (build {BUILD_SHA}; protocol {CLI_PROTOCOL_VERSION})")
        })
        .as_str()
}
