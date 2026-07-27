use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    PayloadTooLarge,
    ServiceUnavailable,
    TooManyRequests,
    Unauthorized,
}

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) kind: ErrorKind,
    message: String,
}

macro_rules! message_errors {
    ($($name:ident => $kind:ident),+ $(,)?) => {$(
        pub(crate) fn $name(message: impl Into<String>) -> Self {
            Self::new(ErrorKind::$kind, message)
        }
    )+};
}

impl ApiError {
    pub(crate) fn bad_request(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorKind::BadRequest, error.to_string())
    }

    pub(crate) fn internal(error: impl std::error::Error) -> Self {
        Self::new(ErrorKind::Internal, error.to_string())
    }

    message_errors! {
        forbidden => Forbidden,
        conflict => Conflict,
        payload_too_large => PayloadTooLarge,
        too_many_requests => TooManyRequests,
        unauthorized => Unauthorized,
        not_found => NotFound,
        internal_message => Internal,
        service_unavailable => ServiceUnavailable,
    }

    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self.kind {
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

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn into_message(self) -> String {
        self.message
    }
}

impl From<scope_postgres::error::PostgresError> for ApiError {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        use scope_postgres::error::PostgresErrorKind;

        let kind = match error.kind {
            PostgresErrorKind::InvalidInput => ErrorKind::BadRequest,
            PostgresErrorKind::Conflict => ErrorKind::Conflict,
            PostgresErrorKind::PermissionDenied => ErrorKind::Forbidden,
            PostgresErrorKind::Internal => ErrorKind::Internal,
            PostgresErrorKind::NotFound => ErrorKind::NotFound,
            PostgresErrorKind::Unavailable => ErrorKind::ServiceUnavailable,
            PostgresErrorKind::ResourceExhausted => ErrorKind::TooManyRequests,
            PostgresErrorKind::Unauthenticated => ErrorKind::Unauthorized,
        };
        Self::new(kind, error.message)
    }
}

impl From<scope_postgres::db::RepositoryMutationError> for ApiError {
    fn from(error: scope_postgres::db::RepositoryMutationError) -> Self {
        match error {
            scope_postgres::db::RepositoryMutationError::Behavior(error) => error.into(),
            scope_postgres::db::RepositoryMutationError::Persistence(error) => error.into(),
        }
    }
}

impl From<scope_postgres::db::RepositoryCreationError<ApiError>> for ApiError {
    fn from(error: scope_postgres::db::RepositoryCreationError<ApiError>) -> Self {
        match error {
            scope_postgres::db::RepositoryCreationError::Cleanup(error) => error,
            scope_postgres::db::RepositoryCreationError::Persistence(error) => error.into(),
        }
    }
}

impl From<scope_domain::error::DomainError> for ApiError {
    fn from(error: scope_domain::error::DomainError) -> Self {
        use scope_domain::error::DomainErrorKind;

        let kind = match error.kind {
            DomainErrorKind::InvalidInput => ErrorKind::BadRequest,
            DomainErrorKind::Conflict => ErrorKind::Conflict,
            DomainErrorKind::Forbidden => ErrorKind::Forbidden,
            DomainErrorKind::AuthenticationFailed => ErrorKind::Unauthorized,
            DomainErrorKind::RateLimited => ErrorKind::TooManyRequests,
            DomainErrorKind::NotFound => ErrorKind::NotFound,
            DomainErrorKind::InvariantViolation => ErrorKind::Internal,
        };
        Self::new(kind, error.message)
    }
}

impl From<scope_git::GitStorageError> for ApiError {
    fn from(error: scope_git::GitStorageError) -> Self {
        match error {
            scope_git::GitStorageError::StorageLimit(error) => {
                Self::service_unavailable(format!("{error}; retry after compaction"))
            }
            scope_git::GitStorageError::ObjectStore(error) => error.into(),
            error => Self::internal(error),
        }
    }
}

impl From<scope_object_store::ObjectStoreError> for ApiError {
    fn from(error: scope_object_store::ObjectStoreError) -> Self {
        use scope_object_store::ObjectStoreErrorKind;

        match error.kind {
            ObjectStoreErrorKind::CapacityExhausted => Self::too_many_requests(error.message),
            ObjectStoreErrorKind::Integrity | ObjectStoreErrorKind::Internal => {
                Self::internal_message(error.message)
            }
            ObjectStoreErrorKind::NotFound => Self::not_found(error.message),
            ObjectStoreErrorKind::PayloadTooLarge => Self::payload_too_large(error.message),
            ObjectStoreErrorKind::ServiceUnavailable => Self::service_unavailable(error.message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = serde_json::json!({ "error": self.into_message() });
        (status, Json(body)).into_response()
    }
}
