use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::import::{run_git, run_git_output},
    persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::store::SourceBlob;
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime},
};

const MAX_RAW_GIT_CACHES: usize = 8;
const RAW_GIT_CACHE_MAX_IDLE: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum GitDerivedCacheNamespace {
    Projection,
    RawSnapshot,
    RequestReadView,
}

impl GitDerivedCacheNamespace {
    fn as_str(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::RawSnapshot => "raw_snapshot",
            Self::RequestReadView => "request_read_view",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitDerivedCacheOutcome {
    Hit,
    Miss,
    Waiter,
}

impl GitDerivedCacheOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Waiter => "waiter",
        }
    }
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

pub(crate) struct RawGitCacheRegistry {
    root: PathBuf,
    users: Mutex<BTreeMap<PathBuf, usize>>,
}

pub(crate) struct GitRepoHandle {
    path: PathBuf,
    _lease: Option<RawGitCacheLease>,
}

impl GitDerivedCacheCoordinator {
    pub(crate) fn materialize(
        &self,
        namespace: GitDerivedCacheNamespace,
        value: String,
        is_ready: impl Fn() -> bool,
        build: impl FnOnce() -> Result<(), ApiError>,
    ) -> Result<GitDerivedCacheOutcome, ApiError> {
        let started_at = Instant::now();
        if is_ready() {
            let outcome = GitDerivedCacheOutcome::Hit;
            log_cache_operation(namespace, outcome, started_at, true);
            return Ok(outcome);
        }

        let key = GitDerivedCacheKey { namespace, value };
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
            let result = wait_for_cache_build(&state);
            let outcome = GitDerivedCacheOutcome::Waiter;
            log_cache_operation(namespace, outcome, started_at, result.is_ok());
            return result.map(|()| outcome);
        }

        // The first lookup and leader election are intentionally separate. Another
        // process or a just-finished local builder may have promoted the cache in between.
        let mut cache_outcome = GitDerivedCacheOutcome::Hit;
        let built = catch_unwind(AssertUnwindSafe(|| {
            if is_ready() {
                Ok(())
            } else {
                cache_outcome = GitDerivedCacheOutcome::Miss;
                build()
            }
        }));
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

        log_cache_operation(namespace, cache_outcome, started_at, outcome.is_ok());

        match built {
            Ok(result) => result.map(|()| cache_outcome),
            Err(payload) => resume_unwind(payload),
        }
    }

    #[cfg(test)]
    fn follower_count(&self, namespace: GitDerivedCacheNamespace, value: &str) -> usize {
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

fn log_cache_operation(
    namespace: GitDerivedCacheNamespace,
    outcome: GitDerivedCacheOutcome,
    started_at: Instant,
    success: bool,
) {
    let duration_ms = started_at.elapsed().as_millis();
    match outcome {
        GitDerivedCacheOutcome::Hit => tracing::info!(
            cache_namespace = namespace.as_str(),
            cache_outcome = outcome.as_str(),
            duration_ms,
            success,
            "Git cache operation completed"
        ),
        GitDerivedCacheOutcome::Miss => tracing::info!(
            cache_namespace = namespace.as_str(),
            cache_outcome = outcome.as_str(),
            duration_ms,
            repo_git_materialize_ms = duration_ms,
            success,
            "Git cache operation completed"
        ),
        GitDerivedCacheOutcome::Waiter => tracing::info!(
            cache_namespace = namespace.as_str(),
            cache_outcome = outcome.as_str(),
            duration_ms,
            repo_git_cache_wait_ms = duration_ms,
            success,
            "Git cache operation completed"
        ),
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

struct RawGitCacheLease {
    registry: Arc<RawGitCacheRegistry>,
    path: PathBuf,
}

impl RawGitCacheRegistry {
    pub(crate) fn new(root: PathBuf) -> Result<Arc<Self>, ApiError> {
        ensure_private_dir(&root)?;
        let registry = Arc::new(Self {
            root,
            users: Mutex::new(BTreeMap::new()),
        });
        registry.prune()?;
        Ok(registry)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_for(&self, manifest: &SourceBlob) -> PathBuf {
        self.root
            .join(format!("raw-{}.git", raw_git_cache_key(manifest)))
    }

    pub(crate) fn lease(
        self: &Arc<Self>,
        manifest: &SourceBlob,
    ) -> Result<GitRepoHandle, ApiError> {
        let path = self.path_for(manifest);
        {
            let mut users = self
                .users
                .lock()
                .map_err(|_| ApiError::internal_message("raw Git cache registry is poisoned"))?;
            touch_if_materialized(&path)?;
            *users.entry(path.clone()).or_default() += 1;
        }
        Ok(GitRepoHandle {
            path: path.clone(),
            _lease: Some(RawGitCacheLease {
                registry: self.clone(),
                path,
            }),
        })
    }

    pub(crate) fn note_materialized(&self, path: &Path) -> Result<(), ApiError> {
        touch_if_materialized(path)?;
        self.prune()
    }

    pub(crate) fn prune(&self) -> Result<(), ApiError> {
        let users = self
            .users
            .lock()
            .map_err(|_| ApiError::internal_message("raw Git cache registry is poisoned"))?;
        let mut caches = raw_cache_directories(&self.root)?;
        let now = SystemTime::now();
        prune_stale_materializations(&self.root, now)?;
        caches.sort_by_key(|(_, last_used)| *last_used);

        let mut retained = caches.len();
        for (path, last_used) in caches {
            if users.get(&path).copied().unwrap_or_default() > 0 {
                continue;
            }
            let expired = now
                .duration_since(last_used)
                .is_ok_and(|idle| idle >= RAW_GIT_CACHE_MAX_IDLE);
            if expired || retained > MAX_RAW_GIT_CACHES {
                remove_dir_if_exists(&path)?;
                retained = retained.saturating_sub(1);
            }
        }
        Ok(())
    }
}

impl AppState {
    pub(crate) fn git_cache_root(&self) -> Result<PathBuf, ApiError> {
        Ok(self.raw_git_cache.root().to_path_buf())
    }

    pub(crate) fn start_raw_git_cache_reaper(&self) {
        let raw_git_cache = self.raw_git_cache.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5 * 60));
            loop {
                interval.tick().await;
                if let Err(error) = raw_git_cache.prune() {
                    tracing::warn!(error = %error.operator_diagnostic(), "failed to prune local raw Git caches");
                }
            }
        });
    }
}

pub(crate) fn sanitize_raw_git_cache_repo(
    repo: &Path,
    expected_head: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(repo),
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
        "reading refs before raw Git cache promotion",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading refs before raw Git cache promotion: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let refs = String::from_utf8(output.stdout).map_err(ApiError::internal)?;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let mut found_main = false;
    for line in refs.lines() {
        let (refname, oid) = line
            .split_once('\0')
            .ok_or_else(|| ApiError::internal_message("invalid raw Git cache ref listing"))?;
        if refname == main_ref {
            if oid != expected_head {
                return Err(ApiError::internal_message(
                    "raw Git cache main ref does not match committed head",
                ));
            }
            found_main = true;
        } else {
            run_git(
                Some(repo),
                &["update-ref", "-d", refname],
                "removing non-main ref before raw Git cache promotion",
            )?;
        }
    }
    if !found_main {
        return Err(ApiError::internal_message(
            "raw Git cache is missing the committed main ref",
        ));
    }
    Ok(())
}

fn prune_stale_materializations(root: &Path, now: SystemTime) -> Result<(), ApiError> {
    for entry in fs::read_dir(root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("raw-") || !name.ends_with(".tmp") || !path.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if now
            .duration_since(modified)
            .is_ok_and(|idle| idle >= RAW_GIT_CACHE_MAX_IDLE)
        {
            remove_dir_if_exists(&path)?;
        }
    }
    Ok(())
}

impl GitRepoHandle {
    pub(crate) fn from_path(path: PathBuf) -> Self {
        Self { path, _lease: None }
    }
}

impl std::fmt::Debug for GitRepoHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitRepoHandle")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Deref for GitRepoHandle {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for GitRepoHandle {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for RawGitCacheLease {
    fn drop(&mut self) {
        if let Ok(mut users) = self.registry.users.lock() {
            match users.get_mut(&self.path) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    users.remove(&self.path);
                }
                None => {}
            }
        }
        if let Err(error) = touch_if_materialized(&self.path).and_then(|()| self.registry.prune()) {
            tracing::warn!(
                path = %self.path.display(),
                error = %error.operator_diagnostic(),
                "failed to prune local raw Git caches"
            );
        }
    }
}

fn raw_git_cache_key(manifest: &SourceBlob) -> &str {
    manifest
        .sha256
        .get(..16)
        .unwrap_or(manifest.sha256.as_str())
}

fn touch_if_materialized(path: &Path) -> Result<(), ApiError> {
    if path.is_dir() {
        fs::write(path.join("scope-cache-last-used"), []).map_err(ApiError::internal)?;
    }
    Ok(())
}

fn raw_cache_directories(root: &Path) -> Result<Vec<(PathBuf, SystemTime)>, ApiError> {
    let mut caches = Vec::new();
    for entry in fs::read_dir(root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("raw-") || !name.ends_with(".git") || !path.is_dir() {
            continue;
        }
        let last_used = fs::metadata(path.join("scope-cache-last-used"))
            .or_else(|_| fs::metadata(&path))
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        caches.push((path, last_used));
    }
    Ok(caches)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), ApiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::store::DEFAULT_GIT_FILE_MODE;
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Instant,
    };

    fn manifest(sha256: &str) -> SourceBlob {
        SourceBlob {
            content_ref: scope_domain::content_ref::ContentRef::git_manifest_sha256(sha256),
            sha256: sha256.to_string(),
            git_oid: String::new(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 0,
        }
    }

    #[test]
    fn active_cache_is_not_evicted_when_the_registry_is_over_capacity() {
        let root = std::env::temp_dir().join(format!(
            "scope-git-cache-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = RawGitCacheRegistry::new(root.clone()).unwrap();
        let active_manifest = manifest("0000000000000000active");
        let active_path = registry.path_for(&active_manifest);
        fs::create_dir_all(&active_path).unwrap();
        let lease = registry.lease(&active_manifest).unwrap();
        for index in 1..=MAX_RAW_GIT_CACHES {
            fs::create_dir_all(root.join(format!("raw-{index:016x}.git"))).unwrap();
        }

        registry.prune().unwrap();

        assert!(active_path.exists());
        drop(lease);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sanitizer_keeps_only_the_committed_main_ref() {
        let repo = std::env::temp_dir().join(format!(
            "scope-raw-cache-sanitize-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        run_git(
            None,
            &["init", repo.to_string_lossy().as_ref()],
            "init test repo",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.name", "Scope Test"],
            "set user",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.email", "scope@example.test"],
            "set email",
        )
        .unwrap();
        fs::write(repo.join("README.md"), "hello\n").unwrap();
        run_git(Some(&repo), &["add", "README.md"], "stage test file").unwrap();
        run_git(
            Some(&repo),
            &["commit", "-m", "initial"],
            "commit test file",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["branch", "-M", DEFAULT_GIT_BRANCH],
            "set default branch",
        )
        .unwrap();
        let head = String::from_utf8(
            run_git_output(Some(&repo), &["rev-parse", "HEAD"], "read test head")
                .unwrap()
                .stdout,
        )
        .unwrap();
        let head = head.trim();
        run_git(
            Some(&repo),
            &["update-ref", "refs/heads/private-request", head],
            "add request ref",
        )
        .unwrap();
        run_git(Some(&repo), &["tag", "private-tag"], "add tag").unwrap();

        sanitize_raw_git_cache_repo(&repo, head).unwrap();

        let output = run_git_output(
            Some(&repo),
            &["for-each-ref", "--format=%(refname)%00%(objectname)"],
            "read sanitized refs",
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("refs/heads/{DEFAULT_GIT_BRANCH}\0{head}\n")
        );
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn concurrent_builds_coalesce_in_every_cache_namespace() {
        for namespace in [
            GitDerivedCacheNamespace::Projection,
            GitDerivedCacheNamespace::RawSnapshot,
            GitDerivedCacheNamespace::RequestReadView,
        ] {
            let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
            let ready = Arc::new(AtomicBool::new(false));
            let builds = Arc::new(AtomicUsize::new(0));
            let (leader_started_tx, leader_started_rx) = mpsc::channel();
            let (release_leader_tx, release_leader_rx) = mpsc::channel();
            let leader = {
                let coordinator = coordinator.clone();
                let ready = ready.clone();
                let builds = builds.clone();
                thread::spawn(move || {
                    coordinator.materialize(
                        namespace,
                        "same-key".to_string(),
                        || ready.load(Ordering::SeqCst),
                        || {
                            builds.fetch_add(1, Ordering::SeqCst);
                            leader_started_tx.send(()).unwrap();
                            release_leader_rx.recv().unwrap();
                            ready.store(true, Ordering::SeqCst);
                            Ok(())
                        },
                    )
                })
            };
            leader_started_rx.recv().unwrap();
            let follower = {
                let coordinator = coordinator.clone();
                let ready = ready.clone();
                thread::spawn(move || {
                    coordinator.materialize(
                        namespace,
                        "same-key".to_string(),
                        || ready.load(Ordering::SeqCst),
                        || panic!("follower must not build"),
                    )
                })
            };
            wait_for_follower(&coordinator, namespace, "same-key");
            release_leader_tx.send(()).unwrap();
            assert_eq!(
                leader.join().unwrap().unwrap(),
                GitDerivedCacheOutcome::Miss
            );
            assert_eq!(
                follower.join().unwrap().unwrap(),
                GitDerivedCacheOutcome::Waiter
            );
            assert_eq!(builds.load(Ordering::SeqCst), 1, "{namespace:?}");
        }
    }

    #[test]
    fn ready_cache_is_a_hit_without_running_the_builder() {
        let outcome = GitDerivedCacheCoordinator::default()
            .materialize(
                GitDerivedCacheNamespace::RawSnapshot,
                "ready-key".to_string(),
                || true,
                || panic!("ready cache must not build"),
            )
            .unwrap();

        assert_eq!(outcome, GitDerivedCacheOutcome::Hit);
    }

    #[test]
    fn externally_completed_cache_is_a_hit_after_leader_election() {
        let readiness_checks = AtomicUsize::new(0);
        let outcome = GitDerivedCacheCoordinator::default()
            .materialize(
                GitDerivedCacheNamespace::RawSnapshot,
                "externally-ready-key".to_string(),
                || readiness_checks.fetch_add(1, Ordering::SeqCst) > 0,
                || panic!("externally completed cache must not build"),
            )
            .unwrap();

        assert_eq!(outcome, GitDerivedCacheOutcome::Hit);
        assert_eq!(readiness_checks.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failed_build_is_shared_with_followers_and_then_can_retry() {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let builds = Arc::new(AtomicUsize::new(0));
        let (leader_started_tx, leader_started_rx) = mpsc::channel();
        let (release_leader_tx, release_leader_rx) = mpsc::channel();
        let leader = {
            let coordinator = coordinator.clone();
            let builds = builds.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    GitDerivedCacheNamespace::Projection,
                    "failed-key".to_string(),
                    || false,
                    || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        leader_started_tx.send(()).unwrap();
                        release_leader_rx.recv().unwrap();
                        Err(ApiError::infrastructure_unavailable("shared build failure"))
                    },
                )
            })
        };
        leader_started_rx.recv().unwrap();
        let follower = {
            let coordinator = coordinator.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    GitDerivedCacheNamespace::Projection,
                    "failed-key".to_string(),
                    || false,
                    || panic!("follower must not build"),
                )
            })
        };
        wait_for_follower(
            &coordinator,
            GitDerivedCacheNamespace::Projection,
            "failed-key",
        );
        release_leader_tx.send(()).unwrap();
        for worker in [leader, follower] {
            let error = worker.join().unwrap().unwrap_err();
            assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
            assert_eq!(error.operator_diagnostic(), "shared build failure");
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        let ready = AtomicBool::new(false);
        coordinator
            .materialize(
                GitDerivedCacheNamespace::Projection,
                "failed-key".to_string(),
                || ready.load(Ordering::SeqCst),
                || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cache_namespaces_do_not_hide_global_capacity_pressure() {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let budgets = Arc::new(crate::runtime_budgets::RuntimeBudgets::from_config(
            crate::runtime_budgets::RuntimeBudgetConfig {
                projection_build_concurrency: 1,
                ..Default::default()
            },
        ));
        let projection_ready = Arc::new(AtomicBool::new(false));
        let (leader_started_tx, leader_started_rx) = mpsc::channel();
        let (release_leader_tx, release_leader_rx) = mpsc::channel();
        let leader = {
            let coordinator = coordinator.clone();
            let budgets = budgets.clone();
            let projection_ready = projection_ready.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    GitDerivedCacheNamespace::Projection,
                    "shared-value".to_string(),
                    || projection_ready.load(Ordering::SeqCst),
                    || {
                        let _permit = budgets.try_projection_build()?;
                        leader_started_tx.send(()).unwrap();
                        release_leader_rx.recv().unwrap();
                        projection_ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        leader_started_rx.recv().unwrap();

        let error = coordinator
            .materialize(
                GitDerivedCacheNamespace::RawSnapshot,
                "shared-value".to_string(),
                || false,
                || {
                    let _permit = budgets.try_projection_build()?;
                    Ok(())
                },
            )
            .unwrap_err();

        assert_eq!(error.kind, crate::error::ErrorKind::TooManyRequests);
        assert_eq!(
            error.operator_diagnostic(),
            "Git projection build capacity is exhausted; retry later"
        );
        release_leader_tx.send(()).unwrap();
        leader.join().unwrap().unwrap();
    }

    fn wait_for_follower(
        coordinator: &GitDerivedCacheCoordinator,
        namespace: GitDerivedCacheNamespace,
        key: &str,
    ) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while coordinator.follower_count(namespace, key) == 0 {
            assert!(
                Instant::now() < deadline,
                "follower did not join the in-flight cache build"
            );
            thread::yield_now();
        }
    }
}
