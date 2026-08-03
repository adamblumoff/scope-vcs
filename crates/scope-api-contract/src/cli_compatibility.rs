use crate::{ErrorCode, ErrorResponse};

pub const CLI_PROTOCOL_VERSION: u32 = 1;
pub const CLI_PROTOCOL_HEADER: &str = "x-scope-cli-protocol";
pub const CLI_VERSION_HEADER: &str = "x-scope-cli-version";
pub const CLI_BUILD_HEADER: &str = "x-scope-cli-build";
pub const CLI_INSTALL_COMMAND: &str =
    "curl -fsSL https://scope-cli-production.up.railway.app/install.sh | sh";

impl ErrorResponse {
    pub fn cli_upgrade_required(installed_protocol: Option<u32>) -> Self {
        let installed_label = installed_protocol
            .map(|protocol| protocol.to_string())
            .unwrap_or_else(|| "missing".to_string());
        let instruction = match installed_protocol {
            Some(protocol) if protocol > CLI_PROTOCOL_VERSION => format!(
                "This Scope CLI requires API protocol {protocol}. Retry after the Scope API is updated."
            ),
            _ => format!("Upgrade with `{CLI_INSTALL_COMMAND}`, then retry."),
        };
        let mut response = Self::new(
            ErrorCode::CliUpgradeRequired,
            format!(
                "installed Scope CLI protocol {installed_label}; this API supports protocol {CLI_PROTOCOL_VERSION}"
            ),
        )
        .with_instruction(instruction);
        response.fields.installed_protocol = installed_protocol;
        response.fields.supported_protocol = Some(CLI_PROTOCOL_VERSION);
        response.retryable =
            installed_protocol.is_some_and(|protocol| protocol > CLI_PROTOCOL_VERSION);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_error_is_structured_and_actionable() {
        let error = ErrorResponse::cli_upgrade_required(Some(0));

        assert_eq!(error.code, ErrorCode::CliUpgradeRequired);
        assert_eq!(error.fields.installed_protocol, Some(0));
        assert_eq!(error.fields.supported_protocol, Some(1));
        assert!(error.message.contains("installed Scope CLI protocol 0"));
        assert!(error.message.contains("supports protocol 1"));
        assert_eq!(
            error.instruction.as_deref(),
            Some(
                "Upgrade with `curl -fsSL https://scope-cli-production.up.railway.app/install.sh | sh`, then retry."
            )
        );
    }

    #[test]
    fn newer_cli_waits_for_the_api_instead_of_reinstalling() {
        let error = ErrorResponse::cli_upgrade_required(Some(CLI_PROTOCOL_VERSION + 1));

        assert_eq!(
            error.instruction.as_deref(),
            Some("This Scope CLI requires API protocol 2. Retry after the Scope API is updated.")
        );
        assert!(!error.instruction.unwrap().contains(CLI_INSTALL_COMMAND));
        assert!(error.retryable);
    }
}
