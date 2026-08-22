use anyhow::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppendLogOutcome {
    Accepted,
    Truncated,
}

#[derive(Debug)]
pub(crate) enum AppendLogError {
    Retryable(Error),
    Fatal(Error),
}

impl AppendLogError {
    pub(crate) fn retryable(error: impl Into<Error>) -> Self {
        Self::Retryable(error.into())
    }

    pub(crate) fn fatal(error: impl Into<Error>) -> Self {
        Self::Fatal(error.into())
    }

    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Retryable(error) | Self::Fatal(error) => error,
        }
    }
}

pub(crate) trait ExecutionSink: Send + Sync + 'static {
    fn start_step(&self, step: u32) -> anyhow::Result<bool>;

    fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError>;

    fn heartbeat(&self) -> anyhow::Result<bool>;

    fn complete_step(&self, step: u32, exit_code: i32, logs_truncated: bool) -> anyhow::Result<()>;

    fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()>;

    fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()>;

    fn abandon(&self) -> anyhow::Result<()>;
}
