use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use scope_cache_domain::CacheDomainError;
use scope_postgres::error::{PostgresError, PostgresErrorKind};
use serde::Serialize;

#[derive(Debug)]
pub(crate) struct ServiceError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

impl ServiceError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(error = %message, "cache service internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "cache service failed")
    }

    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: &self.message,
            }),
        )
            .into_response()
    }
}

impl From<CacheDomainError> for ServiceError {
    fn from(error: CacheDomainError) -> Self {
        match error {
            CacheDomainError::RepositoryBudgetExceeded { .. } => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, error.to_string())
            }
            CacheDomainError::StaleUploadLease | CacheDomainError::UploadLeaseExpired => {
                Self::conflict(error.to_string())
            }
            _ => Self::bad_request(error.to_string()),
        }
    }
}

impl From<PostgresError> for ServiceError {
    fn from(error: PostgresError) -> Self {
        match error.kind {
            PostgresErrorKind::InvalidInput => Self::bad_request(error.message),
            PostgresErrorKind::Conflict => Self::conflict(error.message),
            PostgresErrorKind::PermissionDenied => Self::forbidden(error.message),
            PostgresErrorKind::NotFound => Self::new(StatusCode::NOT_FOUND, error.message),
            PostgresErrorKind::ResourceExhausted => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, error.message)
            }
            PostgresErrorKind::Unauthenticated => Self::unauthorized(error.message),
            PostgresErrorKind::Unavailable => Self::unavailable(error.message),
            PostgresErrorKind::Internal => Self::internal(error.message),
        }
    }
}
