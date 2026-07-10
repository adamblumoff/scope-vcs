use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use scope_core::error::{ApiError as CoreApiError, ErrorKind};

#[derive(Debug)]
pub struct ApiError(CoreApiError);

macro_rules! message_errors {
    ($($name:ident),+ $(,)?) => {$(
        pub fn $name(message: impl Into<String>) -> Self {
            CoreApiError::$name(message).into()
        }
    )+};
}

impl ApiError {
    pub fn bad_request(error: impl std::fmt::Display) -> Self {
        CoreApiError::bad_request(error).into()
    }

    pub fn internal(error: impl std::error::Error) -> Self {
        CoreApiError::internal(error).into()
    }

    message_errors! {
        forbidden,
        conflict,
        payload_too_large,
        too_many_requests,
        unauthorized,
        not_found,
        internal_message,
        service_unavailable,
    }

    pub fn status(&self) -> StatusCode {
        match self.0.kind {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        }
    }

    pub fn message(&self) -> &str {
        &self.0.message
    }

    pub fn into_message(self) -> String {
        self.0.message
    }
}

impl From<CoreApiError> for ApiError {
    fn from(error: CoreApiError) -> Self {
        Self(error)
    }
}

impl From<ApiError> for CoreApiError {
    fn from(error: ApiError) -> Self {
        error.0
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = serde_json::json!({ "error": self.into_message() });
        (status, Json(body)).into_response()
    }
}
