use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use scope_api_contract::{ErrorCode, ErrorResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    NotImplemented,
    PayloadTooLarge,
    ServiceUnavailable,
    TooManyRequests,
    Unauthorized,
}

#[derive(Clone, Debug)]
pub(crate) struct ApiError {
    pub(crate) kind: ErrorKind,
    message: String,
    code: ErrorCode,
    paths: Vec<String>,
    instruction: Option<String>,
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
        not_implemented => NotImplemented,
        internal_message => Internal,
        service_unavailable => ServiceUnavailable,
    }

    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: error_code(kind),
            paths: Vec::new(),
            instruction: None,
        }
    }

    pub(crate) fn protected_paths(paths: Vec<String>) -> Self {
        let message = format!(
            "public request cannot change maintainer-controlled paths: {}",
            paths.join(", ")
        );
        let mut error = Self::new(ErrorKind::Conflict, message);
        error.code = ErrorCode::ProtectedPath;
        error.paths = paths;
        error.instruction = Some(
            "Move maintainer-controlled changes to a maintainer-authored change, then retry."
                .to_string(),
        );
        error
    }

    pub(crate) fn with_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.instruction = Some(instruction.into());
        self
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self.kind {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::NotImplemented => StatusCode::NOT_IMPLEMENTED,
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
            scope_postgres::db::RepositoryCreationError::Persistence(error) => {
                let error = Self::from(error);
                if error.kind == ErrorKind::Conflict {
                    error.with_instruction(
                        "Use `scope init --name <new-name>` to create a different repository, or run `scope push` if this checkout is already linked to Scope.",
                    )
                } else {
                    error
                }
            }
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
        let retryable = matches!(
            self.kind,
            ErrorKind::ServiceUnavailable | ErrorKind::TooManyRequests
        );
        let mut body = ErrorResponse::new(self.code, self.message);
        body.fields.paths = self.paths;
        body.instruction = self.instruction;
        body.retryable = retryable;
        (status, Json(body)).into_response()
    }
}

const fn error_code(kind: ErrorKind) -> ErrorCode {
    match kind {
        ErrorKind::BadRequest => ErrorCode::BadRequest,
        ErrorKind::Conflict => ErrorCode::Conflict,
        ErrorKind::Forbidden => ErrorCode::Forbidden,
        ErrorKind::Internal => ErrorCode::Internal,
        ErrorKind::NotFound => ErrorCode::NotFound,
        ErrorKind::NotImplemented => ErrorCode::NotImplemented,
        ErrorKind::PayloadTooLarge => ErrorCode::PayloadTooLarge,
        ErrorKind::ServiceUnavailable => ErrorCode::ServiceUnavailable,
        ErrorKind::TooManyRequests => ErrorCode::TooManyRequests,
        ErrorKind::Unauthorized => ErrorCode::Unauthorized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn protected_path_errors_preserve_paths_and_remediation() {
        let response =
            ApiError::protected_paths(vec![".scope/RULES.md".to_string()]).into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code, ErrorCode::ProtectedPath);
        assert_eq!(error.fields.paths, [".scope/RULES.md"]);
        assert!(error.instruction.is_some());
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn temporary_errors_are_marked_retryable() {
        let response = ApiError::service_unavailable("try later").into_response();
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(error.code, ErrorCode::ServiceUnavailable);
        assert!(error.retryable);
    }
}
