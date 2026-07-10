use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}
macro_rules! message_errors {
    ($($name:ident => $status:ident),+ $(,)?) => {$(
        pub fn $name(message: impl Into<String>) -> Self {
            Self::new(StatusCode::$status, message)
        }
    )+};
}
impl ApiError {
    pub fn bad_request(error: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error.to_string())
    }
    pub fn internal(error: impl std::error::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
    message_errors! {
        forbidden => FORBIDDEN,
        conflict => CONFLICT,
        payload_too_large => PAYLOAD_TOO_LARGE,
        too_many_requests => TOO_MANY_REQUESTS,
        unauthorized => UNAUTHORIZED,
        not_found => NOT_FOUND,
        internal_message => INTERNAL_SERVER_ERROR,
        service_unavailable => SERVICE_UNAVAILABLE,
    }
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
impl From<scope_core::error::ApiError> for ApiError {
    fn from(error: scope_core::error::ApiError) -> Self {
        use scope_core::error::ErrorKind;
        let status = match error.kind {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        };
        Self::new(status, error.message)
    }
}
impl From<ApiError> for scope_core::error::ApiError {
    fn from(error: ApiError) -> Self {
        use scope_core::error::ErrorKind;
        let kind = match error.status {
            StatusCode::BAD_REQUEST => ErrorKind::BadRequest,
            StatusCode::CONFLICT => ErrorKind::Conflict,
            StatusCode::FORBIDDEN => ErrorKind::Forbidden,
            StatusCode::NOT_FOUND => ErrorKind::NotFound,
            StatusCode::PAYLOAD_TOO_LARGE => ErrorKind::PayloadTooLarge,
            StatusCode::SERVICE_UNAVAILABLE => ErrorKind::ServiceUnavailable,
            StatusCode::TOO_MANY_REQUESTS => ErrorKind::TooManyRequests,
            StatusCode::UNAUTHORIZED => ErrorKind::Unauthorized,
            _ => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.message,
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}
