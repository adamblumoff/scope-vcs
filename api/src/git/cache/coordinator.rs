use crate::error::ApiError;
use futures_util::FutureExt as _;
use std::{
    collections::BTreeMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{Arc, Condvar, Mutex},
};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum GitDerivedCacheNamespace {
    Projection,
    Repository,
    RequestReadView,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GitDerivedCacheKey {
    namespace: GitDerivedCacheNamespace,
    value: String,
}

type CacheBuildOutcome = Result<(), ApiError>;

#[derive(Default)]
struct CacheBuildState {
    outcome: Mutex<Option<CacheBuildOutcome>>,
    completed: Condvar,
    completed_async: Notify,
    #[cfg(test)]
    followers: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
pub(crate) struct GitDerivedCacheCoordinator {
    builds: Mutex<BTreeMap<GitDerivedCacheKey, Arc<CacheBuildState>>>,
}

struct AsyncBuildLeader<'a> {
    coordinator: &'a GitDerivedCacheCoordinator,
    key: GitDerivedCacheKey,
    state: Arc<CacheBuildState>,
    completed: bool,
}

impl GitDerivedCacheCoordinator {
    pub(crate) async fn materialize_async<Build, BuildFuture>(
        &self,
        namespace: GitDerivedCacheNamespace,
        value: String,
        is_ready: impl Fn() -> bool,
        build: Build,
    ) -> Result<(), ApiError>
    where
        Build: FnOnce() -> BuildFuture,
        BuildFuture: Future<Output = Result<(), ApiError>>,
    {
        let key = GitDerivedCacheKey { namespace, value };
        let mut build = Some(build);
        loop {
            if is_ready() {
                return Ok(());
            }
            let (state, is_leader) = self.build_state(&key)?;
            if !is_leader {
                #[cfg(test)]
                state
                    .followers
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                wait_for_cache_build_async(&state).await?;
                continue;
            }
            let build = build
                .take()
                .expect("a cache request can become leader only once");
            let leader = AsyncBuildLeader {
                coordinator: self,
                key: key.clone(),
                state: state.clone(),
                completed: false,
            };
            let built = if is_ready() {
                Ok(Ok(()))
            } else {
                AssertUnwindSafe(build()).catch_unwind().await
            };
            leader.complete(cache_build_outcome(&built));
            return match built {
                Ok(result) => result,
                Err(payload) => resume_unwind(payload),
            };
        }
    }

    pub(crate) fn materialize(
        &self,
        namespace: GitDerivedCacheNamespace,
        value: String,
        is_ready: impl Fn() -> bool,
        build: impl FnOnce() -> Result<(), ApiError>,
    ) -> Result<(), ApiError> {
        let key = GitDerivedCacheKey { namespace, value };
        let mut build = Some(build);
        loop {
            if is_ready() {
                return Ok(());
            }
            let (state, is_leader) = self.build_state(&key)?;
            if !is_leader {
                #[cfg(test)]
                state
                    .followers
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                wait_for_cache_build(&state)?;
                continue;
            }
            let build = build
                .take()
                .expect("a cache request can become leader only once");
            let built = catch_unwind(AssertUnwindSafe(
                || {
                    if is_ready() { Ok(()) } else { build() }
                },
            ));
            self.complete_build(&key, &state, cache_build_outcome(&built));
            return match built {
                Ok(result) => result,
                Err(payload) => resume_unwind(payload),
            };
        }
    }

    fn build_state(
        &self,
        key: &GitDerivedCacheKey,
    ) -> Result<(Arc<CacheBuildState>, bool), ApiError> {
        let mut builds = self
            .builds
            .lock()
            .map_err(|_| ApiError::internal_message("Git cache build coordinator is poisoned"))?;
        Ok(match builds.get(key) {
            Some(state) => (state.clone(), false),
            None => {
                let state = Arc::new(CacheBuildState::default());
                builds.insert(key.clone(), state.clone());
                (state, true)
            }
        })
    }

    fn complete_build(
        &self,
        key: &GitDerivedCacheKey,
        state: &CacheBuildState,
        outcome: CacheBuildOutcome,
    ) {
        {
            let mut completed = state
                .outcome
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *completed = Some(outcome);
            state.completed.notify_all();
            state.completed_async.notify_waiters();
        }
        if let Ok(mut builds) = self.builds.lock()
            && builds
                .get(key)
                .is_some_and(|current| std::ptr::eq(current.as_ref(), state))
        {
            builds.remove(key);
        }
    }

    #[cfg(test)]
    pub(super) fn follower_count(&self, namespace: GitDerivedCacheNamespace, value: &str) -> usize {
        let key = GitDerivedCacheKey {
            namespace,
            value: value.to_string(),
        };
        self.builds
            .lock()
            .unwrap()
            .get(&key)
            .map(|state| state.followers.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or_default()
    }
}

impl AsyncBuildLeader<'_> {
    fn complete(mut self, outcome: CacheBuildOutcome) {
        self.coordinator
            .complete_build(&self.key, &self.state, outcome);
        self.completed = true;
    }
}

impl Drop for AsyncBuildLeader<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.coordinator.complete_build(
                &self.key,
                &self.state,
                Err(ApiError::infrastructure_unavailable(
                    "Git cache build was cancelled",
                )),
            );
        }
    }
}

fn cache_build_outcome(
    built: &Result<Result<(), ApiError>, Box<dyn std::any::Any + Send>>,
) -> CacheBuildOutcome {
    match built {
        Ok(result) => result.clone(),
        Err(_) => Err(ApiError::internal_message("Git cache build panicked")),
    }
}

fn wait_for_cache_build(state: &CacheBuildState) -> Result<(), ApiError> {
    let mut outcome = state
        .outcome
        .lock()
        .map_err(|_| ApiError::internal_message("Git cache build state is poisoned"))?;
    while outcome.is_none() {
        outcome = state
            .completed
            .wait(outcome)
            .map_err(|_| ApiError::internal_message("Git cache build state is poisoned"))?;
    }
    outcome
        .as_ref()
        .expect("cache build outcome checked")
        .clone()
}

async fn wait_for_cache_build_async(state: &CacheBuildState) -> Result<(), ApiError> {
    loop {
        let completed = state.completed_async.notified();
        if let Some(outcome) = state
            .outcome
            .lock()
            .map_err(|_| ApiError::internal_message("Git cache build state is poisoned"))?
            .clone()
        {
            return outcome;
        }
        completed.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn async_followers_share_one_build() {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let ready = Arc::new(AtomicBool::new(false));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_wait = started.notified();
        let leader = {
            let coordinator = coordinator.clone();
            let ready = ready.clone();
            let started = started.clone();
            let release = release.clone();
            tokio::spawn(async move {
                coordinator
                    .materialize_async(
                        GitDerivedCacheNamespace::Repository,
                        "repo".to_string(),
                        || ready.load(Ordering::SeqCst),
                        || async {
                            started.notify_one();
                            release.notified().await;
                            ready.store(true, Ordering::SeqCst);
                            Ok(())
                        },
                    )
                    .await
            })
        };
        started_wait.await;
        let follower = {
            let coordinator = coordinator.clone();
            let ready = ready.clone();
            tokio::spawn(async move {
                coordinator
                    .materialize_async(
                        GitDerivedCacheNamespace::Repository,
                        "repo".to_string(),
                        || ready.load(Ordering::SeqCst),
                        || async { panic!("follower must not build") },
                    )
                    .await
            })
        };
        while coordinator.follower_count(GitDerivedCacheNamespace::Repository, "repo") == 0 {
            tokio::task::yield_now().await;
        }
        release.notify_one();

        leader.await.unwrap().unwrap();
        follower.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancelling_async_leader_releases_followers() {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let started = Arc::new(Notify::new());
        let started_wait = started.notified();
        let leader = {
            let coordinator = coordinator.clone();
            let started = started.clone();
            tokio::spawn(async move {
                coordinator
                    .materialize_async(
                        GitDerivedCacheNamespace::Repository,
                        "repo".to_string(),
                        || false,
                        || async {
                            started.notify_one();
                            std::future::pending().await
                        },
                    )
                    .await
            })
        };
        started_wait.await;
        let follower = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .materialize_async(
                        GitDerivedCacheNamespace::Repository,
                        "repo".to_string(),
                        || false,
                        || async { panic!("follower must not build") },
                    )
                    .await
            })
        };
        while coordinator.follower_count(GitDerivedCacheNamespace::Repository, "repo") == 0 {
            tokio::task::yield_now().await;
        }
        leader.abort();

        let error = follower.await.unwrap().unwrap_err();
        assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
        assert_eq!(error.operator_diagnostic(), "Git cache build was cancelled");
    }
}
