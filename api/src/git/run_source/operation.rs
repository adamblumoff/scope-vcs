use super::TemporarySourceDirectory;
use crate::{error::ApiError, runtime_budgets::RuntimePermit, state::AppState};
use std::{future::Future, sync::Arc};

pub(crate) struct RunSourceOperation {
    // Drop the repository before returning capacity to the next operation.
    repository: TemporarySourceDirectory,
    _permit: RuntimePermit,
    #[cfg(test)]
    hook: Option<TestHook>,
}

impl RunSourceOperation {
    pub(super) fn new(state: &AppState) -> Result<Arc<Self>, ApiError> {
        let permit = state.runtime_budgets.try_git_materialization()?;
        Ok(Arc::new(Self {
            repository: TemporarySourceDirectory::new(&state.data_dir.join("run-source"))?,
            _permit: permit,
            #[cfg(test)]
            hook: None,
        }))
    }
}

pub(super) fn repository(owner: &RunSourceOperation) -> std::path::PathBuf {
    owner.repository.path().to_path_buf()
}

pub(super) async fn supervise<T: Send + 'static>(
    work: impl Future<Output = Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    // A disconnected request drops only the join handle. The operation finishes
    // under the existing storage and Git limits. Runtime shutdown drops this
    // supervisor, preventing any subsequent phase from starting.
    tokio::spawn(work).await.map_err(|error| {
        ApiError::internal_message(format!("run source materialization task failed: {error}"))
    })?
}

pub(crate) fn spawn_blocking<F, T>(
    owner: Option<&Arc<RunSourceOperation>>,
    work: F,
) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    // The child retains the operation even if shutdown or a panic drops its
    // async supervisor. Other restore callers supply no run-source owner.
    let owner = owner.cloned();
    tokio::task::spawn_blocking(move || {
        let _owner = owner;
        #[cfg(test)]
        if let Some(hook) = _owner.as_ref().and_then(|owner| owner.hook.as_ref()) {
            hook();
        }
        work()
    })
}

#[cfg(test)]
type TestHook = Box<dyn Fn() + Send + Sync>;

#[cfg(test)]
pub(super) fn with_hook(state: &AppState, hook: TestHook) -> Arc<RunSourceOperation> {
    let mut owner = RunSourceOperation::new(state).unwrap();
    Arc::get_mut(&mut owner).unwrap().hook = Some(hook);
    owner
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::mpsc, time::Duration};

    #[tokio::test]
    async fn supervisor_abort_or_panic_keeps_resources_until_its_blocking_child_exits() {
        for panic in [false, true] {
            let state = AppState::test_state();
            let _other_permit = state.runtime_budgets.try_git_materialization().unwrap();
            let owner = RunSourceOperation::new(&state).unwrap();
            let path = repository(&owner);
            fs::create_dir_all(&path).unwrap();
            let child_path = path.clone();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let (panic_tx, panic_rx) = tokio::sync::oneshot::channel::<()>();
            let supervisor = tokio::spawn(async move {
                let child = spawn_blocking(Some(&owner), move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    assert!(child_path.exists());
                });
                if panic {
                    panic_rx.await.unwrap();
                    panic!("injected supervisor panic");
                }
                child.await.unwrap();
            });
            started_rx.await.unwrap();
            if panic {
                panic_tx.send(()).unwrap();
                assert!(supervisor.await.unwrap_err().is_panic());
            } else {
                supervisor.abort();
                assert!(supervisor.await.unwrap_err().is_cancelled());
            }
            assert!(path.exists());
            assert!(state.runtime_budgets.try_git_materialization().is_err());
            release_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(_permit) = state.runtime_budgets.try_git_materialization() {
                        assert!(!path.exists());
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }
}
