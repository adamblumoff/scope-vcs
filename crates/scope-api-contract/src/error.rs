use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    CliUpgradeRequired,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    NotImplemented,
    PayloadTooLarge,
    ProtectedPath,
    ServiceUnavailable,
    TooManyRequests,
    Unauthorized,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::CliUpgradeRequired => "cli_upgrade_required",
            Self::Conflict => "conflict",
            Self::Forbidden => "forbidden",
            Self::Internal => "internal",
            Self::NotFound => "not_found",
            Self::NotImplemented => "not_implemented",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ProtectedPath => "protected_path",
            Self::ServiceUnavailable => "service_unavailable",
            Self::TooManyRequests => "too_many_requests",
            Self::Unauthorized => "unauthorized",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ErrorFields {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_protocol: Option<u32>,
}

impl ErrorFields {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
            && self.installed_protocol.is_none()
            && self.supported_protocol.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ErrorResponse {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "ErrorFields::is_empty")]
    pub fields: ErrorFields,
    #[serde(default)]
    pub retryable: bool,
}

impl ErrorResponse {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            error_reference: None,
            instruction: None,
            fields: ErrorFields::default(),
            retryable: false,
        }
    }

    pub fn with_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.instruction = Some(instruction.into());
        self
    }

    pub fn with_paths(mut self, paths: Vec<String>) -> Self {
        self.fields.paths = paths;
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_reference_is_optional_in_the_wire_contract() {
        let public = ErrorResponse::new(ErrorCode::BadRequest, "invalid request");
        let public_json = serde_json::to_value(&public).unwrap();
        assert_eq!(public_json.get("error_reference"), None);

        let mut internal = ErrorResponse::new(ErrorCode::Internal, "internal error");
        internal.error_reference = Some("err_0123456789abcdef0123456789abcdef".to_string());
        let internal_json = serde_json::to_value(&internal).unwrap();
        assert_eq!(
            internal_json
                .get("error_reference")
                .and_then(|value| value.as_str()),
            internal.error_reference.as_deref()
        );
    }
}
