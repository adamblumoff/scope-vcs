use crate::error::ApiError;

pub(super) struct DbRuntime(Option<tokio::runtime::Runtime>);

impl DbRuntime {
    pub(super) fn new() -> anyhow::Result<Self> {
        Ok(Self(Some(tokio::runtime::Runtime::new()?)))
    }
}

impl std::ops::Deref for DbRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("database runtime has already shut down")
    }
}

impl Drop for DbRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            // This store is process-lifetime state. Dropping a Tokio runtime from
            // inside the server runtime panics, so let the OS reclaim it on exit.
            std::mem::forget(runtime);
        }
    }
}

pub(super) fn run_api_db_on<R>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, ApiError>> + Send + 'static,
) -> Result<R, ApiError>
where
    R: Send + 'static,
{
    run_on_runtime(runtime, future)
        .map_err(|error| ApiError::internal_message(error.to_string()))?
}

pub(super) fn run_db_on<R>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, sea_orm::DbErr>> + Send + 'static,
) -> Result<R, sea_orm::DbErr>
where
    R: Send + 'static,
{
    run_on_runtime(runtime, future).map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?
}

fn run_on_runtime<R, E>(
    runtime: &tokio::runtime::Runtime,
    future: impl std::future::Future<Output = Result<R, E>> + Send + 'static,
) -> Result<Result<R, E>, anyhow::Error>
where
    R: Send + 'static,
    E: Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver
        .recv()
        .map_err(|_| anyhow::anyhow!("database runtime task was cancelled"))
}
