use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectStoreErrorKind {
    CapacityExhausted,
    Integrity,
    Internal,
    NotFound,
    PayloadTooLarge,
    ServiceUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectStoreError {
    pub kind: ObjectStoreErrorKind,
    pub message: String,
}

impl ObjectStoreError {
    pub fn capacity_exhausted(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::CapacityExhausted, message)
    }

    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::Integrity, message)
    }

    pub fn internal(error: impl fmt::Display) -> Self {
        Self::new(ObjectStoreErrorKind::Internal, error.to_string())
    }

    pub fn internal_message(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::Internal, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::NotFound, message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::PayloadTooLarge, message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(ObjectStoreErrorKind::ServiceUnavailable, message)
    }

    fn new(kind: ObjectStoreErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ObjectStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ObjectStoreError {}
