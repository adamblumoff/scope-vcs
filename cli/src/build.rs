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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_identity_includes_package_build_and_protocol() {
        let identity = version_identity();

        assert!(identity.starts_with(PACKAGE_VERSION));
        assert!(identity.contains(&format!("build {BUILD_SHA}")));
        assert!(identity.contains(&format!("protocol {CLI_PROTOCOL_VERSION}")));
    }
}
