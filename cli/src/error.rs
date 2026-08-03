use scope_api_contract::{ErrorCode, ErrorResponse};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExitCategory {
    Unexpected = 1,
    Usage = 2,
    Authentication = 3,
    Policy = 4,
    StateConflict = 5,
    Temporary = 6,
}

#[derive(Debug)]
pub struct CliError {
    response: ErrorResponse,
}

impl CliError {
    pub fn new(response: ErrorResponse) -> Self {
        Self { response }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorResponse::new(ErrorCode::BadRequest, message))
    }

    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorResponse::new(ErrorCode::Unauthorized, message))
    }

    pub fn response(&self) -> &ErrorResponse {
        &self.response
    }

    pub fn exit_category(&self) -> ExitCategory {
        if self.response.retryable {
            return ExitCategory::Temporary;
        }
        match self.response.code {
            ErrorCode::BadRequest | ErrorCode::PayloadTooLarge => ExitCategory::Usage,
            ErrorCode::Unauthorized => ExitCategory::Authentication,
            ErrorCode::CliUpgradeRequired | ErrorCode::Forbidden | ErrorCode::ProtectedPath => {
                ExitCategory::Policy
            }
            ErrorCode::Conflict | ErrorCode::NotFound => ExitCategory::StateConflict,
            ErrorCode::ServiceUnavailable | ErrorCode::TooManyRequests => ExitCategory::Temporary,
            ErrorCode::Internal | ErrorCode::NotImplemented => ExitCategory::Unexpected,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.response.message)?;
        if let Some(instruction) = self.response.instruction.as_deref() {
            write!(formatter, "\n{instruction}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {}

pub fn exit_code(error: &anyhow::Error) -> u8 {
    if let Some(error) = error.downcast_ref::<CliError>() {
        return error.exit_category() as u8;
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_connect() || error.is_timeout())
    {
        return ExitCategory::Temporary as u8;
    }
    ExitCategory::Unexpected as u8
}

pub fn response(error: &anyhow::Error) -> ErrorResponse {
    if let Some(error) = error.downcast_ref::<CliError>() {
        return error.response().clone();
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_connect() || error.is_timeout())
    {
        return ErrorResponse::new(
            ErrorCode::ServiceUnavailable,
            "Scope is temporarily unavailable; retry with bounded backoff",
        )
        .retryable();
    }
    ErrorResponse::new(ErrorCode::Internal, format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use std::{net::TcpListener, time::Duration};

    #[test]
    fn stable_error_codes_map_to_small_process_categories() {
        for (code, expected) in [
            (ErrorCode::BadRequest, ExitCategory::Usage),
            (ErrorCode::Unauthorized, ExitCategory::Authentication),
            (ErrorCode::ProtectedPath, ExitCategory::Policy),
            (ErrorCode::Conflict, ExitCategory::StateConflict),
            (ErrorCode::TooManyRequests, ExitCategory::Temporary),
            (ErrorCode::Internal, ExitCategory::Unexpected),
        ] {
            let error = CliError::new(ErrorResponse::new(code, "fixture"));
            assert_eq!(error.exit_category(), expected);
        }
    }

    #[test]
    fn retryable_protocol_skew_is_temporary() {
        let error = CliError::new(ErrorResponse::cli_upgrade_required(Some(
            scope_api_contract::CLI_PROTOCOL_VERSION + 1,
        )));

        assert_eq!(error.exit_category(), ExitCategory::Temporary);
    }

    #[test]
    fn unavailable_api_connections_are_temporary_even_with_context() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let error = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .unwrap()
            .get(format!("http://{address}/unavailable"))
            .send()
            .context("load fixture")
            .unwrap_err();

        assert_eq!(exit_code(&error), ExitCategory::Temporary as u8);
    }
}
