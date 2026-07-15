use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::import::{run_git, run_git_output},
    persistence::ensure_private_dir,
};
use scope_core::domain::store::SourceBlob;
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

const MAX_RAW_GIT_CACHES: usize = 8;
const RAW_GIT_CACHE_MAX_IDLE: Duration = Duration::from_secs(30 * 60);

pub(crate) struct RawGitCacheRegistry {
    root: PathBuf,
    users: Mutex<BTreeMap<PathBuf, usize>>,
}

pub(crate) struct GitRepoHandle {
    path: PathBuf,
    _lease: Option<RawGitCacheLease>,
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
        return Err(ApiError::service_unavailable(format!(
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
                error = %error.message(),
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
    use scope_core::domain::store::DEFAULT_GIT_FILE_MODE;

    fn manifest(sha256: &str) -> SourceBlob {
        SourceBlob {
            object_key: "objects/git-manifests/test".to_string(),
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
}
