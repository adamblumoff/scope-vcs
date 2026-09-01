use crate::error::ApiError;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{Arc, Condvar, Mutex},
};

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
    #[cfg(test)]
    followers: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
pub(crate) struct GitDerivedCacheCoordinator {
    builds: Mutex<BTreeMap<GitDerivedCacheKey, Arc<CacheBuildState>>>,
}

impl GitDerivedCacheCoordinator {
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
            let (state, is_leader) = {
                let mut builds = self.builds.lock().map_err(|_| {
                    ApiError::internal_message("Git cache build coordinator is poisoned")
                })?;
                match builds.get(&key) {
                    Some(state) => (state.clone(), false),
                    None => {
                        let state = Arc::new(CacheBuildState::default());
                        builds.insert(key.clone(), state.clone());
                        (state, true)
                    }
                }
            };
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
            let outcome = match &built {
                Ok(result) => result.clone(),
                Err(_) => Err(ApiError::internal_message("Git cache build panicked")),
            };
            {
                let mut completed = state
                    .outcome
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                *completed = Some(outcome.clone());
                state.completed.notify_all();
            }
            if let Ok(mut builds) = self.builds.lock()
                && builds
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &state))
            {
                builds.remove(&key);
            }
            return match built {
                Ok(result) => result,
                Err(payload) => resume_unwind(payload),
            };
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
