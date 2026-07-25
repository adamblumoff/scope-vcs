use std::fmt;

/// A failure produced by domain behavior, independent of any delivery mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainErrorKind {
    InvalidInput,
    Conflict,
    Forbidden,
    AuthenticationFailed,
    RateLimited,
    NotFound,
    InvariantViolation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainError {
    pub kind: DomainErrorKind,
    pub message: String,
}

impl DomainError {
    pub fn invalid_input(error: impl fmt::Display) -> Self {
        Self::new(DomainErrorKind::InvalidInput, error.to_string())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::Conflict, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::Forbidden, message)
    }

    pub fn authentication_failed(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::AuthenticationFailed, message)
    }

    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::RateLimited, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(DomainErrorKind::NotFound, message)
    }

    pub fn invariant_violation(error: impl fmt::Display) -> Self {
        Self::new(DomainErrorKind::InvariantViolation, error.to_string())
    }

    fn new(kind: DomainErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DomainError {}
