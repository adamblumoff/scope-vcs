use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::import::{run_git, run_git_output},
    persistence::ensure_private_dir,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime},
};

const REPOSITORY_GIT_CACHE_MAX_IDLE: Duration = Duration::from_secs(30 * 60);
const REPOSITORY_GIT_CACHE_TOUCH_INTERVAL: Duration = Duration::from_secs(60);
const APPLIED_SEQUENCE_FILE: &str = "scope-cache-applied-sequence";
const LAST_USED_FILE: &str = "scope-cache-last-used";

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

pub(crate) struct RepositoryGitCache {
    root: PathBuf,
    max_bytes: usize,
    users: Mutex<BTreeMap<PathBuf, usize>>,
}

pub(crate) struct GitRepoHandle {
    path: PathBuf,
    _lease: RepositoryGitCacheLease,
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

            // Leader election and the build are separate. Another process may have
            // finished the requested materialization between those operations.
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

struct RepositoryGitCacheLease {
    registry: Arc<RepositoryGitCache>,
    path: PathBuf,
}

impl RepositoryGitCache {
    pub(crate) fn new(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        if max_bytes == 0 {
            return Err(ApiError::internal_message(
                "repository Git cache byte budget must be greater than zero",
            ));
        }
        ensure_private_dir(&root)?;
        let registry = Arc::new(Self {
            root,
            max_bytes,
            users: Mutex::new(BTreeMap::new()),
        });
        registry.prune()?;
        Ok(registry)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_for(&self, repository_id: &str) -> PathBuf {
        self.root.join(format!(
            "repo-{}.git",
            repository_git_cache_key(repository_id)
        ))
    }

    pub(crate) fn lease(self: &Arc<Self>, repository_id: &str) -> Result<GitRepoHandle, ApiError> {
        self.lease_path(self.path_for(repository_id))
    }

    pub(crate) fn lease_derived(
        self: &Arc<Self>,
        path: PathBuf,
    ) -> Result<GitRepoHandle, ApiError> {
        let is_direct_child = path.parent() == Some(self.root.as_path());
        let is_git_repository = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".git"));
        if !is_direct_child || !is_git_repository {
            return Err(ApiError::internal_message(
                "derived Git cache path is outside the managed cache root",
            ));
        }
        self.lease_path(path)
    }

    fn lease_path(self: &Arc<Self>, path: PathBuf) -> Result<GitRepoHandle, ApiError> {
        {
            let mut users = self.users.lock().map_err(|_| {
                ApiError::internal_message("repository Git cache registry is poisoned")
            })?;
            touch_if_materialized(&path)?;
            *users.entry(path.clone()).or_default() += 1;
        }
        Ok(GitRepoHandle {
            path: path.clone(),
            _lease: RepositoryGitCacheLease {
                registry: self.clone(),
                path,
            },
        })
    }

    pub(crate) fn note_applied(&self, path: &Path, push_sequence: u64) -> Result<(), ApiError> {
        fs::write(path.join(APPLIED_SEQUENCE_FILE), push_sequence.to_string())
            .map_err(ApiError::internal)?;
        touch_if_materialized(path)
    }

    pub(crate) fn applied_sequence(&self, path: &Path) -> Option<u64> {
        fs::read_to_string(path.join(APPLIED_SEQUENCE_FILE))
            .ok()
            .and_then(|value| value.parse().ok())
    }

    pub(crate) fn prune(&self) -> Result<(), ApiError> {
        let users = self
            .users
            .lock()
            .map_err(|_| ApiError::internal_message("repository Git cache registry is poisoned"))?;
        let mut caches = repository_cache_directories(&self.root)?;
        let now = SystemTime::now();
        prune_stale_materializations(&self.root, now)?;
        caches.sort_by_key(|entry| entry.last_used);

        let mut retained_bytes = caches
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
            .ok_or_else(|| ApiError::internal_message("repository Git cache size overflow"))?;
        let max_bytes = self.max_bytes as u64;
        for entry in caches {
            if users.get(&entry.path).copied().unwrap_or_default() > 0 {
                continue;
            }
            let expired = now
                .duration_since(entry.last_used)
                .is_ok_and(|idle| idle >= REPOSITORY_GIT_CACHE_MAX_IDLE);
            if expired || retained_bytes > max_bytes {
                remove_dir_if_exists(&entry.path)?;
                retained_bytes = retained_bytes.saturating_sub(entry.size_bytes);
            }
        }
        Ok(())
    }
}

pub(crate) fn sanitize_repository_git_cache_repo(
    repo: &Path,
    expected_head: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(repo),
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
        "reading refs before repository Git cache synchronization",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading refs before repository Git cache synchronization: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let refs = String::from_utf8(output.stdout).map_err(ApiError::internal)?;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let mut found_main = false;
    for line in refs.lines() {
        let (refname, oid) = line.split_once('\0').ok_or_else(|| {
            ApiError::internal_message("invalid repository Git cache ref listing")
        })?;
        if refname == main_ref {
            if oid != expected_head {
                return Err(ApiError::internal_message(
                    "repository Git cache main ref does not match committed head",
                ));
            }
            found_main = true;
        } else {
            run_git(
                Some(repo),
                &["update-ref", "-d", refname],
                "removing non-main ref before repository Git cache synchronization",
            )?;
        }
    }
    if !found_main {
        return Err(ApiError::internal_message(
            "repository Git cache is missing the committed main ref",
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
        if !name.ends_with(".tmp") || !path.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if now
            .duration_since(modified)
            .is_ok_and(|idle| idle >= REPOSITORY_GIT_CACHE_MAX_IDLE)
        {
            remove_dir_if_exists(&path)?;
        }
    }
    Ok(())
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

impl Drop for RepositoryGitCacheLease {
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
        if let Err(error) = touch_if_materialized(&self.path) {
            tracing::warn!(
                path = %self.path.display(),
                error = %error.operator_diagnostic(),
                "failed to prune local repository Git caches"
            );
        }
    }
}

fn repository_git_cache_key(repository_id: &str) -> String {
    let digest = Sha256::digest(repository_id.as_bytes());
    hex::encode(&digest[..16])
}

fn touch_if_materialized(path: &Path) -> Result<(), ApiError> {
    if path.is_dir() {
        let marker = path.join(LAST_USED_FILE);
        let touched_recently = fs::metadata(&marker)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|elapsed| elapsed < REPOSITORY_GIT_CACHE_TOUCH_INTERVAL);
        if !touched_recently {
            fs::write(marker, []).map_err(ApiError::internal)?;
        }
    }
    Ok(())
}

struct RepositoryCacheEntry {
    path: PathBuf,
    last_used: SystemTime,
    size_bytes: u64,
}

fn repository_cache_directories(root: &Path) -> Result<Vec<RepositoryCacheEntry>, ApiError> {
    let mut caches = Vec::new();
    for entry in fs::read_dir(root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".git") || !path.is_dir() {
            continue;
        }
        let last_used = fs::metadata(path.join(LAST_USED_FILE))
            .or_else(|_| fs::metadata(&path))
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        caches.push(RepositoryCacheEntry {
            size_bytes: directory_size(&path)?,
            path,
            last_used,
        });
    }
    Ok(caches)
}

fn directory_size(root: &Path) -> Result<u64, ApiError> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ApiError::internal(error)),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ApiError::internal(error)),
            };
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ApiError::internal(error)),
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    ApiError::internal_message("repository Git cache size overflow")
                })?;
            }
        }
    }
    Ok(total)
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
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Instant,
    };

    fn temp_cache_root(test: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "scope-{test}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn active_repository_is_not_evicted_when_the_cache_exceeds_its_byte_budget() {
        let root = temp_cache_root("git-cache-active");
        let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
        let active_path = registry.path_for("owner/active");
        fs::create_dir_all(&active_path).unwrap();
        fs::write(active_path.join("pack"), [0_u8; 40]).unwrap();
        let lease = registry.lease("owner/active").unwrap();
        let inactive_path = registry.path_for("owner/inactive");
        fs::create_dir_all(&inactive_path).unwrap();
        fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

        registry.prune().unwrap();

        assert!(active_path.exists());
        assert!(!inactive_path.exists());
        drop(lease);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn derived_repository_counts_toward_the_budget_and_is_leased_while_in_use() {
        let root = temp_cache_root("git-cache-derived");
        let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
        let derived_path = root.join("read-view-derived.git");
        fs::create_dir_all(&derived_path).unwrap();
        fs::write(derived_path.join("pack"), [0_u8; 40]).unwrap();
        let lease = registry.lease_derived(derived_path.clone()).unwrap();
        let inactive_path = registry.path_for("owner/inactive");
        fs::create_dir_all(&inactive_path).unwrap();
        fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

        registry.prune().unwrap();

        assert!(derived_path.exists());
        assert!(!inactive_path.exists());
        drop(lease);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_frontier_updates_do_not_run_global_eviction() {
        let root = temp_cache_root("git-cache-lru");
        let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
        let old_path = registry.path_for("owner/old");
        fs::create_dir_all(&old_path).unwrap();
        fs::write(old_path.join("pack"), [0_u8; 40]).unwrap();
        registry.note_applied(&old_path, 1).unwrap();
        thread::sleep(Duration::from_millis(10));

        let new_path = registry.path_for("owner/new");
        fs::create_dir_all(&new_path).unwrap();
        fs::write(new_path.join("pack"), [0_u8; 40]).unwrap();
        registry.note_applied(&new_path, 1).unwrap();

        assert!(old_path.exists());
        assert!(new_path.exists());

        registry.prune().unwrap();

        assert!(!old_path.exists());
        assert!(new_path.exists());
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

        sanitize_repository_git_cache_repo(&repo, head).unwrap();

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
            GitDerivedCacheNamespace::Repository,
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
            leader.join().unwrap().unwrap();
            follower.join().unwrap().unwrap();
            assert_eq!(builds.load(Ordering::SeqCst), 1, "{namespace:?}");
        }
    }

    #[test]
    fn ready_cache_does_not_run_the_builder() {
        GitDerivedCacheCoordinator::default()
            .materialize(
                GitDerivedCacheNamespace::Repository,
                "ready-key".to_string(),
                || true,
                || panic!("ready cache must not build"),
            )
            .unwrap();
    }

    #[test]
    fn externally_completed_cache_does_not_build_after_leader_election() {
        let readiness_checks = AtomicUsize::new(0);
        GitDerivedCacheCoordinator::default()
            .materialize(
                GitDerivedCacheNamespace::Repository,
                "externally-ready-key".to_string(),
                || readiness_checks.fetch_add(1, Ordering::SeqCst) > 0,
                || panic!("externally completed cache must not build"),
            )
            .unwrap();

        assert_eq!(readiness_checks.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn follower_with_a_newer_repository_frontier_runs_a_second_build() {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let first_ready = Arc::new(AtomicBool::new(false));
        let newer_ready = Arc::new(AtomicBool::new(false));
        let builds = Arc::new(AtomicUsize::new(0));
        let (leader_started_tx, leader_started_rx) = mpsc::channel();
        let (release_leader_tx, release_leader_rx) = mpsc::channel();
        let leader = {
            let coordinator = coordinator.clone();
            let first_ready = first_ready.clone();
            let builds = builds.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    GitDerivedCacheNamespace::Repository,
                    "same-repository".to_string(),
                    || first_ready.load(Ordering::SeqCst),
                    || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        leader_started_tx.send(()).unwrap();
                        release_leader_rx.recv().unwrap();
                        first_ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        leader_started_rx.recv().unwrap();
        let follower = {
            let coordinator = coordinator.clone();
            let newer_ready = newer_ready.clone();
            let builds = builds.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    GitDerivedCacheNamespace::Repository,
                    "same-repository".to_string(),
                    || newer_ready.load(Ordering::SeqCst),
                    || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        newer_ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        wait_for_follower(
            &coordinator,
            GitDerivedCacheNamespace::Repository,
            "same-repository",
        );
        release_leader_tx.send(()).unwrap();
        leader.join().unwrap().unwrap();
        follower.join().unwrap().unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert!(newer_ready.load(Ordering::SeqCst));
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
                git_materialization_concurrency: 1,
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
                        let _permit = budgets.try_git_materialization()?;
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
                GitDerivedCacheNamespace::Repository,
                "shared-value".to_string(),
                || false,
                || {
                    let _permit = budgets.try_git_materialization()?;
                    Ok(())
                },
            )
            .unwrap_err();

        assert_eq!(error.kind, crate::error::ErrorKind::TooManyRequests);
        assert_eq!(
            error.operator_diagnostic(),
            "Git materialization capacity is exhausted; retry later"
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
