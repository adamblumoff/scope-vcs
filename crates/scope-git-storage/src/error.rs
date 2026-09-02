use std::{error::Error, fmt, io};

#[derive(Debug, thiserror::Error)]
pub enum GitStorageError {
    #[error("invalid Git segment storage configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Git segment input failed: {0}")]
    Input(#[source] io::Error),
    #[error("Git segment plaintext exceeds {max_bytes} bytes")]
    PlaintextLimitExceeded { max_bytes: u64 },
    #[error("Git segment ingest was cancelled")]
    Cancelled,
    #[error("Git segment remote cleanup exceeded {timeout_ms} ms")]
    RemoteCleanupTimedOut { timeout_ms: u128 },
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

#[derive(Debug)]
pub struct MultipartError {
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl MultipartError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source(
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for MultipartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        let mut source = self.source();
        while let Some(error) = source {
            write!(formatter, ": {error}")?;
            source = error.source();
        }
        Ok(())
    }
}

impl Error for MultipartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<io::Error> for MultipartError {
    fn from(error: io::Error) -> Self {
        Self::with_source("multipart I/O failed", error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_error_keeps_its_source() {
        let error = MultipartError::with_source("S3 get object failed", io::Error::other("closed"));

        assert_eq!(error.to_string(), "S3 get object failed: closed");
        assert_eq!(error.source().unwrap().to_string(), "closed");
    }

    #[test]
    fn multipart_error_diagnostic_includes_the_complete_source_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("dispatch failure")]
        struct DispatchError {
            #[source]
            source: io::Error,
        }

        let error = MultipartError::with_source(
            "S3 get object failed",
            DispatchError {
                source: io::Error::other("runtime dropped the dispatch task"),
            },
        );

        assert_eq!(
            error.to_string(),
            "S3 get object failed: dispatch failure: runtime dropped the dispatch task"
        );
    }
}
