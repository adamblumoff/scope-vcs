#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostgresErrorKind {
    InvalidInput,
    Conflict,
    PermissionDenied,
    Internal,
    NotFound,
    Unavailable,
    ResourceExhausted,
    Unauthenticated,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PostgresError {
    pub kind: PostgresErrorKind,
    pub message: String,
}

macro_rules! message_errors {
    ($($name:ident => $kind:ident),+ $(,)?) => {$(
        pub fn $name(message: impl Into<String>) -> Self {
            Self::new(PostgresErrorKind::$kind, message)
        }
    )+};
}

impl PostgresError {
    pub fn invalid_input(error: impl std::fmt::Display) -> Self {
        Self::new(PostgresErrorKind::InvalidInput, error.to_string())
    }

    pub fn internal(error: impl std::error::Error) -> Self {
        Self::new(PostgresErrorKind::Internal, error.to_string())
    }

    message_errors! {
        permission_denied => PermissionDenied,
        conflict => Conflict,
        resource_exhausted => ResourceExhausted,
        unauthenticated => Unauthenticated,
        not_found => NotFound,
        internal_message => Internal,
        unavailable => Unavailable,
    }

    fn new(kind: PostgresErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<scope_domain::error::DomainError> for PostgresError {
    fn from(error: scope_domain::error::DomainError) -> Self {
        use scope_domain::error::DomainErrorKind;

        let kind = match error.kind {
            DomainErrorKind::InvalidInput => PostgresErrorKind::InvalidInput,
            DomainErrorKind::Conflict => PostgresErrorKind::Conflict,
            DomainErrorKind::Forbidden => PostgresErrorKind::PermissionDenied,
            DomainErrorKind::AuthenticationFailed => PostgresErrorKind::Unauthenticated,
            DomainErrorKind::RateLimited => PostgresErrorKind::ResourceExhausted,
            DomainErrorKind::NotFound => PostgresErrorKind::NotFound,
            DomainErrorKind::InvariantViolation => PostgresErrorKind::Internal,
        };
        Self::new(kind, error.message)
    }
}

impl From<scope_cache_domain::CacheDomainError> for PostgresError {
    fn from(error: scope_cache_domain::CacheDomainError) -> Self {
        use scope_cache_domain::CacheDomainError;

        let kind = match error {
            CacheDomainError::RepositoryBudgetExceeded { .. } => {
                PostgresErrorKind::ResourceExhausted
            }
            CacheDomainError::UploadLeaseExpired
            | CacheDomainError::StaleUploadLease
            | CacheDomainError::ReferenceVersionOverflow => PostgresErrorKind::Conflict,
            CacheDomainError::InvalidReferenceExpiry
            | CacheDomainError::InvalidUploadLeaseExpiry => PostgresErrorKind::Internal,
            _ => PostgresErrorKind::InvalidInput,
        };
        Self::new(kind, error.to_string())
    }
}

impl From<scope_git::GitStorageError> for PostgresError {
    fn from(error: scope_git::GitStorageError) -> Self {
        Self::internal(error)
    }
}
