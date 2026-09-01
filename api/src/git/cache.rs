use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::import::{run_git, run_git_output},
    persistence::ensure_private_dir,
};
use scope_domain::repository::RepositoryIncarnation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};

mod coordinator;
pub(crate) use coordinator::{GitDerivedCacheCoordinator, GitDerivedCacheNamespace};

const STALE_MATERIALIZATION_MAX_IDLE: Duration = Duration::from_secs(30 * 60);
const REPOSITORY_GIT_CACHE_TOUCH_INTERVAL: Duration = Duration::from_secs(60);
const APPLIED_FRONTIER_FILE: &str = "scope-cache-applied-frontier";
const LAST_USED_FILE: &str = "scope-cache-last-used";

pub(crate) struct RepositoryGitCache {
    root: PathBuf,
    max_bytes: usize,
}

pub(crate) struct GitRepoHandle {
    path: PathBuf,
    _lease: RepositoryGitCacheLease,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AppliedRepositoryFrontier {
    version: u8,
    incarnation: RepositoryIncarnation,
    push_sequence: u64,
}

struct RepositoryGitCacheLease {
    path: PathBuf,
}

fn cache_users() -> &'static Mutex<BTreeMap<PathBuf, usize>> {
    static USERS: OnceLock<Mutex<BTreeMap<PathBuf, usize>>> = OnceLock::new();
    USERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

impl RepositoryGitCache {
    pub(crate) fn new(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        if max_bytes == 0 {
            return Err(ApiError::internal_message(
                "repository Git cache byte budget must be greater than zero",
            ));
        }
        ensure_private_dir(&root)?;
        let registry = Arc::new(Self { root, max_bytes });
        registry.prune()?;
        Ok(registry)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_for(&self, incarnation: &RepositoryIncarnation) -> PathBuf {
        self.root.join(format!(
            "repo-{}.git",
            repository_git_cache_key(incarnation)
        ))
    }

    pub(crate) fn lease(
        self: &Arc<Self>,
        incarnation: &RepositoryIncarnation,
    ) -> Result<GitRepoHandle, ApiError> {
        self.lease_path(self.path_for(incarnation))
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
            let mut users = cache_users().lock().map_err(|_| {
                ApiError::internal_message("repository Git cache registry is poisoned")
            })?;
            touch_if_materialized(&path)?;
            *users.entry(path.clone()).or_default() += 1;
        }
        Ok(GitRepoHandle {
            path: path.clone(),
            _lease: RepositoryGitCacheLease { path },
        })
    }

    pub(crate) fn note_applied(
        &self,
        incarnation: &RepositoryIncarnation,
        path: &Path,
        push_sequence: u64,
    ) -> Result<(), ApiError> {
        let frontier = AppliedRepositoryFrontier {
            version: 1,
            incarnation: incarnation.clone(),
            push_sequence,
        };
        let bytes = serde_json::to_vec(&frontier).map_err(ApiError::internal)?;
        fs::write(path.join(APPLIED_FRONTIER_FILE), bytes).map_err(ApiError::internal)?;
        touch_if_materialized(path)
    }

    pub(crate) fn applied_sequence(
        &self,
        incarnation: &RepositoryIncarnation,
        path: &Path,
    ) -> Option<u64> {
        fs::read(path.join(APPLIED_FRONTIER_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AppliedRepositoryFrontier>(&bytes).ok())
            .filter(|frontier| frontier.version == 1 && &frontier.incarnation == incarnation)
            .map(|frontier| frontier.push_sequence)
    }

    pub(crate) fn remove(&self, incarnation: &RepositoryIncarnation) -> Result<bool, ApiError> {
        let path = self.path_for(incarnation);
        let users = cache_users()
            .lock()
            .map_err(|_| ApiError::internal_message("repository Git cache registry is poisoned"))?;
        if users.get(&path).copied().unwrap_or_default() > 0 {
            return Ok(false);
        }
        remove_dir_if_exists(&path)?;
        Ok(true)
    }

    pub(crate) fn prune(&self) -> Result<(), ApiError> {
        let users = cache_users()
            .lock()
            .map_err(|_| ApiError::internal_message("repository Git cache registry is poisoned"))?;
        let mut caches = repository_cache_directories(&self.root)?;
        let now = SystemTime::now();
        prune_stale_materializations(&self.root, now, STALE_MATERIALIZATION_MAX_IDLE)?;
        caches.sort_by_key(|entry| entry.last_used);

        let mut retained_bytes = caches
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
            .ok_or_else(|| ApiError::internal_message("repository Git cache size overflow"))?;
        let max_bytes = self.max_bytes as u64;
        let mut evicted_bytes = 0_u64;
        let mut evicted_repositories = 0_u64;
        for entry in caches {
            if users.get(&entry.path).copied().unwrap_or_default() > 0 {
                continue;
            }
            if retained_bytes > max_bytes {
                remove_dir_if_exists(&entry.path)?;
                retained_bytes = retained_bytes.saturating_sub(entry.size_bytes);
                evicted_bytes += entry.size_bytes;
                evicted_repositories += 1;
            }
        }
        if evicted_repositories > 0 {
            tracing::info!(
                reason = "size_pressure",
                evicted_repositories,
                evicted_bytes,
                retained_bytes,
                max_bytes,
                "repository Git caches evicted"
            );
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

fn prune_stale_materializations(
    root: &Path,
    now: SystemTime,
    max_idle: Duration,
) -> Result<(), ApiError> {
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
            .is_ok_and(|idle| idle >= max_idle)
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
        if let Ok(mut users) = cache_users().lock() {
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

fn repository_git_cache_key(incarnation: &RepositoryIncarnation) -> String {
    let mut hasher = Sha256::new();
    for value in [
        incarnation.repository_id().as_bytes(),
        incarnation.incarnation_id().as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let digest = hasher.finalize();
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

    fn incarnation(repository_id: &str) -> RepositoryIncarnation {
        RepositoryIncarnation::new(repository_id, format!("repoi_{repository_id}"))
            .expect("test repository identity is valid")
    }

    #[test]
    fn active_repository_is_not_evicted_when_the_cache_exceeds_its_byte_budget() {
        let root = temp_cache_root("git-cache-active");
        let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
        let active = incarnation("owner/active");
        let active_path = registry.path_for(&active);
        fs::create_dir_all(&active_path).unwrap();
        fs::write(active_path.join("pack"), [0_u8; 40]).unwrap();
        let lease = registry.lease(&active).unwrap();
        let inactive = incarnation("owner/inactive");
        let inactive_path = registry.path_for(&inactive);
        fs::create_dir_all(&inactive_path).unwrap();
        fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

        registry.prune().unwrap();

        assert!(active_path.exists());
        assert!(!inactive_path.exists());
        drop(lease);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn idle_repository_is_retained_while_the_cache_fits_its_byte_budget() {
        let root = temp_cache_root("git-cache-idle-retention");
        let registry = RepositoryGitCache::new(root.clone(), 1_000).unwrap();
        let repo = incarnation("owner/idle");
        let repo_path = registry.path_for(&repo);
        fs::create_dir_all(&repo_path).unwrap();
        fs::write(repo_path.join("pack"), [0_u8; 40]).unwrap();
        registry.note_applied(&repo, &repo_path, 1).unwrap();
        fs::File::options()
            .write(true)
            .open(repo_path.join(LAST_USED_FILE))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();

        registry.prune().unwrap();

        assert!(repo_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn exact_repository_removal_waits_for_active_lease() {
        let root = temp_cache_root("git-cache-lease-safe-delete");
        let registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
        let repo = incarnation("owner/recreated");
        let path = registry.path_for(&repo);
        fs::create_dir_all(&path).unwrap();
        let lease = registry.lease(&repo).unwrap();

        assert!(!registry.remove(&repo).unwrap());
        assert!(path.exists());
        drop(lease);
        assert!(registry.remove(&repo).unwrap());
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn exact_repository_removal_waits_for_lease_from_another_registry() {
        let root = temp_cache_root("git-cache-cross-registry-lease-safe-delete");
        let leasing_registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
        let deleting_registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
        let repo = incarnation("owner/shared-root");
        let path = leasing_registry.path_for(&repo);
        fs::create_dir_all(&path).unwrap();
        let lease = leasing_registry.lease(&repo).unwrap();

        assert!(!deleting_registry.remove(&repo).unwrap());
        assert!(path.exists());
        drop(lease);
        assert!(deleting_registry.remove(&repo).unwrap());
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn applied_frontier_requires_the_exact_repository_incarnation() {
        let root = temp_cache_root("git-cache-incarnation-marker");
        let registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
        let predecessor = RepositoryIncarnation::new("owner/repo", "repoi_predecessor").unwrap();
        let recreated = RepositoryIncarnation::new("owner/repo", "repoi_recreated").unwrap();
        let recreated_path = registry.path_for(&recreated);
        fs::create_dir_all(&recreated_path).unwrap();
        registry
            .note_applied(&predecessor, &recreated_path, 41)
            .unwrap();

        assert_eq!(registry.applied_sequence(&recreated, &recreated_path), None);
        assert_ne!(registry.path_for(&predecessor), recreated_path);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_temporary_materialization_is_removed() {
        let root = temp_cache_root("git-cache-stale-materialization");
        fs::create_dir_all(&root).unwrap();
        let materialization = root.join("repo-materializing.tmp");
        fs::create_dir_all(&materialization).unwrap();

        prune_stale_materializations(&root, SystemTime::now(), Duration::ZERO).unwrap();

        assert!(!materialization.exists());
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
        let inactive = incarnation("owner/inactive");
        let inactive_path = registry.path_for(&inactive);
        fs::create_dir_all(&inactive_path).unwrap();
        fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

        registry.prune().unwrap();

        assert!(derived_path.exists());
        assert!(!inactive_path.exists());
        drop(lease);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn least_recently_used_repository_is_evicted_when_cache_exceeds_byte_budget() {
        let root = temp_cache_root("git-cache-lru");
        let registry = RepositoryGitCache::new(root.clone(), 250).unwrap();
        let old = incarnation("owner/old");
        let old_path = registry.path_for(&old);
        fs::create_dir_all(&old_path).unwrap();
        fs::write(old_path.join("pack"), [0_u8; 40]).unwrap();
        registry.note_applied(&old, &old_path, 1).unwrap();
        thread::sleep(Duration::from_millis(10));

        let new = incarnation("owner/new");
        let new_path = registry.path_for(&new);
        fs::create_dir_all(&new_path).unwrap();
        fs::write(new_path.join("pack"), [0_u8; 40]).unwrap();
        registry.note_applied(&new, &new_path, 1).unwrap();

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
