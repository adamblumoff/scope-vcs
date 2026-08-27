use std::io;

#[derive(Debug, thiserror::Error)]
pub enum GitStorageError {
    #[error("invalid Git segment storage configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Git segment input failed: {0}")]
    Input(#[source] io::Error),
    #[error("Git segment local storage failed: {0}")]
    Local(#[source] io::Error),
    #[error("Git segment encryption failed")]
    Encryption,
    #[error("Git segment envelope is invalid: {0}")]
    InvalidEnvelope(String),
    #[error("Git segment multipart storage failed: {0}")]
    Multipart(#[from] MultipartError),
    #[error("Git segment output failed: {0}")]
    Output(#[source] io::Error),
    #[error("Git segment checksum does not match: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Git segment size does not match: expected {expected}, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("Git segment stream ended before both storage destinations finished")]
    IncompleteIngest,
    #[error("Git segment task failed: {0}")]
    Task(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct MultipartError {
    message: String,
}

impl MultipartError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<io::Error> for MultipartError {
    fn from(error: io::Error) -> Self {
        Self::new(error.to_string())
    }
}
