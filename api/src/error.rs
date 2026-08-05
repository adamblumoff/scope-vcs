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
    public_message: String,
    operator_diagnostic: Option<String>,
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
        Self::internal_message(error.to_string())
    }

    message_errors! {
        forbidden => Forbidden,
        conflict => Conflict,
        payload_too_large => PayloadTooLarge,
        too_many_requests => TooManyRequests,
        unauthorized => Unauthorized,
        not_found => NotFound,
        not_implemented => NotImplemented,
    }

    pub(crate) fn internal_message(diagnostic: impl Into<String>) -> Self {
        Self::from_diagnostic(ErrorKind::Internal, INTERNAL_PUBLIC_MESSAGE, diagnostic)
    }

    pub(crate) fn infrastructure_unavailable(diagnostic: impl Into<String>) -> Self {
        Self::from_diagnostic(
            ErrorKind::ServiceUnavailable,
            SERVICE_UNAVAILABLE_PUBLIC_MESSAGE,
            diagnostic,
        )
    }

    fn temporarily_unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ServiceUnavailable, message)
    }

    fn capacity_exhausted(diagnostic: impl Into<String>) -> Self {
        Self::from_diagnostic(
            ErrorKind::TooManyRequests,
            CAPACITY_EXHAUSTED_PUBLIC_MESSAGE,
            diagnostic,
        )
    }

    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            public_message: message.into(),
            operator_diagnostic: None,
            code: error_code(kind),
            paths: Vec::new(),
            instruction: None,
        }
    }

    fn from_diagnostic(
        kind: ErrorKind,
        public_message: impl Into<String>,
        operator_diagnostic: impl Into<String>,
    ) -> Self {
        let mut error = Self::new(kind, public_message);
        error.operator_diagnostic = Some(operator_diagnostic.into());
        error
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

    #[cfg(test)]
    pub(crate) fn public_message(&self) -> &str {
        &self.public_message
    }

    pub(crate) fn into_public_message(self) -> String {
        let Some(operator_diagnostic) = self.operator_diagnostic.as_deref() else {
            return self.public_message;
        };
        match report_operator_diagnostic(self.kind, self.code, operator_diagnostic) {
            Some(error_reference) => {
                format!("{} (reference: {error_reference})", self.public_message)
            }
            None => self.public_message,
        }
    }

    pub(crate) fn operator_diagnostic(&self) -> &str {
        self.operator_diagnostic
            .as_deref()
            .unwrap_or(&self.public_message)
    }

    pub(crate) fn into_operator_diagnostic(self) -> String {
        self.operator_diagnostic.unwrap_or(self.public_message)
    }
}

impl From<scope_postgres::error::PostgresError> for ApiError {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        use scope_postgres::error::PostgresErrorKind;

        match error.kind {
            PostgresErrorKind::InvalidInput => Self::new(ErrorKind::BadRequest, error.message),
            PostgresErrorKind::Conflict => Self::new(ErrorKind::Conflict, error.message),
            PostgresErrorKind::PermissionDenied => Self::new(ErrorKind::Forbidden, error.message),
            PostgresErrorKind::Internal => Self::internal_message(error.message),
            PostgresErrorKind::NotFound => Self::new(ErrorKind::NotFound, error.message),
            PostgresErrorKind::Unavailable => Self::temporarily_unavailable(error.message),
            PostgresErrorKind::ResourceExhausted => {
                Self::new(ErrorKind::TooManyRequests, error.message)
            }
            PostgresErrorKind::Unauthenticated => Self::new(ErrorKind::Unauthorized, error.message),
        }
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

        match error.kind {
            DomainErrorKind::InvalidInput => Self::new(ErrorKind::BadRequest, error.message),
            DomainErrorKind::Conflict => Self::new(ErrorKind::Conflict, error.message),
            DomainErrorKind::Forbidden => Self::new(ErrorKind::Forbidden, error.message),
            DomainErrorKind::AuthenticationFailed => {
                Self::new(ErrorKind::Unauthorized, error.message)
            }
            DomainErrorKind::RateLimited => Self::new(ErrorKind::TooManyRequests, error.message),
            DomainErrorKind::NotFound => Self::new(ErrorKind::NotFound, error.message),
            DomainErrorKind::InvariantViolation => Self::internal_message(error.message),
        }
    }
}

impl From<scope_git::GitStorageError> for ApiError {
    fn from(error: scope_git::GitStorageError) -> Self {
        match error {
            scope_git::GitStorageError::StorageLimit(error) => {
                Self::infrastructure_unavailable(format!("{error}; retry after compaction"))
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
            ObjectStoreErrorKind::CapacityExhausted => Self::capacity_exhausted(error.message),
            ObjectStoreErrorKind::Integrity | ObjectStoreErrorKind::Internal => {
                Self::internal_message(error.message)
            }
            ObjectStoreErrorKind::NotFound => Self::from_diagnostic(
                ErrorKind::NotFound,
                "stored content was not found",
                error.message,
            ),
            ObjectStoreErrorKind::PayloadTooLarge => Self::from_diagnostic(
                ErrorKind::PayloadTooLarge,
                "stored content exceeds the supported size limit",
                error.message,
            ),
            ObjectStoreErrorKind::ServiceUnavailable => {
                Self::infrastructure_unavailable(error.message)
            }
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
        let error_reference = self
            .operator_diagnostic
            .as_deref()
            .and_then(|diagnostic| report_operator_diagnostic(self.kind, self.code, diagnostic));
        let mut body = ErrorResponse::new(self.code, self.public_message);
        body.error_reference = error_reference;
        body.fields.paths = self.paths;
        body.instruction = self.instruction;
        body.retryable = retryable;
        (status, Json(body)).into_response()
    }
}

const INTERNAL_PUBLIC_MESSAGE: &str = "Scope hit an internal error.";
const SERVICE_UNAVAILABLE_PUBLIC_MESSAGE: &str =
    "Scope is temporarily unavailable; retry with bounded backoff.";
const CAPACITY_EXHAUSTED_PUBLIC_MESSAGE: &str =
    "Scope is temporarily at capacity; retry with bounded backoff.";

fn new_error_reference() -> Option<String> {
    let mut bytes = [0_u8; 16];
    if let Err(error) = getrandom::fill(&mut bytes) {
        tracing::error!(%error, "failed to generate API error reference");
        return None;
    }
    Some(format!("err_{}", hex::encode(bytes)))
}

fn report_operator_diagnostic(
    kind: ErrorKind,
    code: ErrorCode,
    diagnostic: &str,
) -> Option<String> {
    let error_reference = new_error_reference();
    tracing::error!(
        error_reference = error_reference.as_deref().unwrap_or("unavailable"),
        error_code = code.as_str(),
        error_kind = ?kind,
        diagnostic,
        "API request failed"
    );
    error_reference
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

    async fn response_error(error: ApiError) -> ErrorResponse {
        let response = error.into_response();
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn assert_opaque_reference(error: &ErrorResponse) {
        let reference = error.error_reference.as_deref().unwrap();
        assert!(reference.starts_with("err_"));
        assert_eq!(reference.len(), 36);
        assert!(reference[4..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn internal_diagnostics_are_not_serialized() {
        let diagnostic = "failed to read /srv/scope/private/repository.git";
        let api_error = ApiError::internal_message(diagnostic);

        assert_eq!(api_error.operator_diagnostic(), diagnostic);
        let response = api_error.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.message, INTERNAL_PUBLIC_MESSAGE);
        assert!(!error.message.contains("/srv/scope/private"));
        assert_opaque_reference(&error);
    }

    #[tokio::test]
    async fn infrastructure_diagnostic_sources_are_redacted_and_correlated() {
        let cases = [
            ApiError::from(scope_postgres::error::PostgresError::internal_message(
                "database host db.internal.example refused the query",
            )),
            ApiError::internal(std::io::Error::other(
                "permission denied at /srv/scope/repos/private.git",
            )),
            ApiError::infrastructure_unavailable(
                "git upload-pack exited 128: fatal: repository secret.git not found",
            ),
            ApiError::from(scope_object_store::ObjectStoreError::service_unavailable(
                "S3 endpoint https://objects.internal.example timed out",
            )),
        ];

        for api_error in cases {
            let diagnostic = api_error.operator_diagnostic().to_string();
            let error = response_error(api_error).await;
            assert!(!error.message.contains(&diagnostic));
            assert!(
                error.message == INTERNAL_PUBLIC_MESSAGE
                    || error.message == SERVICE_UNAVAILABLE_PUBLIC_MESSAGE
            );
            assert_opaque_reference(&error);
        }
    }

    #[tokio::test]
    async fn user_actionable_messages_stay_exact_and_uncorrelated() {
        for api_error in [
            ApiError::bad_request("branch name is required"),
            ApiError::conflict("request changed; fetch and retry"),
            ApiError::forbidden("maintainer role required"),
            ApiError::unauthorized("sign in before retrying"),
            ApiError::not_found("repository not found"),
            ApiError::too_many_requests("push limit reached; retry in one minute"),
            ApiError::payload_too_large("bundle exceeds the 10 MiB limit"),
            ApiError::from(scope_postgres::error::PostgresError::unavailable(
                "repository projection is rebuilding; retry shortly",
            )),
            ApiError::from(scope_postgres::error::PostgresError::resource_exhausted(
                "run attempt log limit reached",
            )),
        ] {
            let expected = api_error.public_message().to_string();
            let error = response_error(api_error).await;
            assert_eq!(error.message, expected);
            assert_eq!(error.error_reference, None);
        }
    }

    #[test]
    fn non_json_error_surfaces_redact_and_correlate_diagnostics() {
        let message = ApiError::internal_message("git stderr contained /srv/private.git")
            .into_public_message();

        assert!(message.starts_with(INTERNAL_PUBLIC_MESSAGE));
        assert!(message.contains("(reference: err_"));
        assert!(!message.contains("/srv/private.git"));
    }

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
        let response = ApiError::infrastructure_unavailable("upstream timed out").into_response();
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let error: ErrorResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(error.code, ErrorCode::ServiceUnavailable);
        assert_eq!(error.message, SERVICE_UNAVAILABLE_PUBLIC_MESSAGE);
        assert_opaque_reference(&error);
        assert!(error.retryable);
    }
}
