#[cfg(test)]
use crate::git::import::run_git_output;
use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        cache::{
            GitDerivedCacheCoordinator, GitDerivedCacheNamespace, GitRepoHandle,
            RepositoryGitCache, sanitize_repository_git_cache_repo,
        },
        import::run_git,
        restore::{index_git_pack, restore_git_pack_spans, run_timed_git_restore_phase},
    },
    state::AppState,
};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitHead, GitPackSpan, validate_git_pack_layout},
};
use scope_git_process::{ProcessLimits, run_with_stdin_reader};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static REPOSITORY_MATERIALIZATION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const MATERIALIZATION_PATH_HIT: u8 = 0;
const MATERIALIZATION_PATH_CATCH_UP: u8 = 1;
const MATERIALIZATION_PATH_RESTORE: u8 = 2;

/// Owns this API process's disposable Git replicas and coordinates mutations of
/// each replica through one repository-scoped stream. Durable publication is
/// still ordered by the Postgres repository aggregate, and compaction remains a
/// worker concern; local promotion receives only an already-committed frontier.
pub(crate) struct RepositoryEngine {
    cache: Arc<RepositoryGitCache>,
    materializations: GitDerivedCacheCoordinator,
}

impl RepositoryEngine {
    pub(crate) fn new(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        Ok(Arc::new(Self {
            cache: RepositoryGitCache::new(root, max_bytes)?,
            materializations: GitDerivedCacheCoordinator::default(),
        }))
    }

    pub(crate) fn cache_root(&self) -> &Path {
        self.cache.root()
    }

    #[cfg(test)]
    pub(crate) fn repository_path(&self, incarnation: &RepositoryIncarnation) -> PathBuf {
        self.cache.path_for(incarnation)
    }

    pub(crate) fn delete_repository_cache(
        &self,
        incarnation: &RepositoryIncarnation,
    ) -> Result<bool, ApiError> {
        self.cache.remove(incarnation)
    }

    /// Coalesces immutable derived views by their content-derived key. These
    /// views do not participate in the repository replica's mutation stream.
    pub(crate) fn materialize_derived(
        &self,
        incarnation: &RepositoryIncarnation,
        namespace: GitDerivedCacheNamespace,
        key: String,
        path: &Path,
        is_ready: impl Fn() -> bool,
        build: impl FnOnce() -> Result<(), ApiError>,
    ) -> Result<GitRepoHandle, ApiError> {
        let started_at = Instant::now();
        let cache_hit = is_ready();
        let built = AtomicBool::new(false);
        let result = self
            .materializations
            .materialize(namespace, key, is_ready, || {
                built.store(true, Ordering::Relaxed);
                build()
            })
            .and_then(|()| self.cache.lease_derived(path.to_path_buf()));
        tracing::info!(
            repository_id = incarnation.repository_id(),
            repository_incarnation_id = incarnation.incarnation_id(),
            namespace = ?namespace,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            success = result.is_ok(),
            "repository derived Git materialization completed"
        );
        result
    }

    /// Opens the local replica at or beyond the requested durable frontier,
    /// serializing any repair or catch-up with post-push replica updates.
    pub(crate) fn materialize_repository(
        &self,
        state: &AppState,
        incarnation: &RepositoryIncarnation,
        head: &GitHead,
        pack_spans: &[GitPackSpan],
    ) -> Result<GitRepoHandle, ApiError> {
        let repository_id = incarnation.repository_id();
        let repo = self.cache.lease(incarnation)?;
        let repo_path = repo.as_ref().to_path_buf();
        let cache_root = self.cache_root();
        let is_ready = || {
            repository_cache_is_ready(&repo_path)
                && self
                    .cache
                    .applied_sequence(incarnation, &repo_path)
                    .is_some_and(|applied| applied >= head.push_sequence)
        };
        let started_at = Instant::now();
        let applied_before = self.cache.applied_sequence(incarnation, &repo_path);
        let cache_hit = is_ready();
        let built = AtomicBool::new(false);
        let materialization_path = AtomicU8::new(MATERIALIZATION_PATH_HIT);
        let result = self.coordinate_repository(incarnation, is_ready, || {
            built.store(true, Ordering::Relaxed);
            let _permit = state.runtime_budgets.try_git_materialization()?;
            match self.cache.applied_sequence(incarnation, &repo_path) {
                Some(applied) if applied < head.push_sequence && repo_path.is_dir() => {
                    materialization_path.store(MATERIALIZATION_PATH_CATCH_UP, Ordering::Relaxed);
                    self.catch_up(
                        state,
                        repository_id,
                        head,
                        pack_spans,
                        applied,
                        &repo_path,
                    )?;
                    self.cache
                        .note_applied(incarnation, &repo_path, head.push_sequence)
                }
                Some(applied)
                    if applied == head.push_sequence && repository_cache_is_ready(&repo_path) =>
                {
                    Ok(())
                }
                // Replicas are monotonic. A reader with an older database
                // frontier may safely use the newer local object set.
                Some(applied)
                    if applied > head.push_sequence && repository_cache_is_ready(&repo_path) =>
                {
                    Ok(())
                }
                _ => {
                    materialization_path.store(MATERIALIZATION_PATH_RESTORE, Ordering::Relaxed);
                    let attempt =
                        REPOSITORY_MATERIALIZATION_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                    let temp_path = cache_root.join(format!(
                        "repo-materializing.{}.{}.tmp",
                        std::process::id(),
                        attempt
                    ));
                    if let Err(error) = restore_git_pack_spans(
                        state,
                        repository_id,
                        head,
                        pack_spans,
                        &temp_path,
                    ) {
                        let _ = fs::remove_dir_all(&temp_path);
                        return Err(error);
                    }
                    self.cache
                        .note_applied(incarnation, &temp_path, head.push_sequence)?;
                    if repo_path.exists()
                        && let Err(error) = fs::remove_dir_all(&repo_path)
                    {
                        let _ = fs::remove_dir_all(&temp_path);
                        return Err(ApiError::internal(error));
                    }
                    match fs::rename(&temp_path, &repo_path) {
                        Ok(()) => Ok(()),
                        Err(error) if is_ready() => {
                            let _ = fs::remove_dir_all(&temp_path);
                            tracing::debug!(%error, path = %repo_path.display(), "using externally-created repository Git cache");
                            Ok(())
                        }
                        Err(error) => {
                            let _ = fs::remove_dir_all(&temp_path);
                            Err(ApiError::internal(error))
                        }
                    }
                }
            }
        });
        tracing::info!(
            repository_id,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            materialization_path = materialization_path_name(
                materialization_path.load(Ordering::Relaxed),
                cache_hit,
                built.load(Ordering::Relaxed),
            ),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = head.push_sequence,
            applied_sequence_before = applied_before,
            applied_sequence_after = self.cache.applied_sequence(incarnation, &repo_path),
            pack_span_count = pack_spans.len(),
            total_pack_bytes = pack_spans
                .iter()
                .map(|span| span.segment.plaintext_bytes)
                .sum::<u64>(),
            success = result.is_ok(),
            "repository Git replica materialization completed"
        );
        result?;
        Ok(repo)
    }

    pub(crate) fn sync_after_push(
        &self,
        incarnation: &RepositoryIncarnation,
        local_pack: &Path,
        expected_head: &str,
        push_sequence: u64,
    ) -> Result<(), ApiError> {
        // Post-commit synchronization mutates the same disposable replica as
        // readers. Keep it leased so the periodic cache reaper cannot remove it
        // while Git is replacing refs or pack files.
        let repository_id = incarnation.repository_id();
        let repo = self.cache.lease(incarnation)?;
        let target = repo.as_ref().to_path_buf();
        let is_ready = || {
            self.cache
                .applied_sequence(incarnation, &target)
                .is_some_and(|applied| applied >= push_sequence)
                && target.is_dir()
        };
        let started_at = Instant::now();
        let cache_hit = is_ready();
        let built = AtomicBool::new(false);
        let result = self.coordinate_repository(incarnation, is_ready, || {
            built.store(true, Ordering::Relaxed);
            if target.is_dir() {
                index_local_pack(&target, local_pack)?;
                run_timed_git_restore_phase(
                    repository_id,
                    "promote_update_ref",
                    Some(&target),
                    &[
                        "update-ref",
                        &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                        expected_head,
                    ],
                    "advancing repository Git cache from accepted segment",
                )?;
            } else if push_sequence > 1 {
                // Later segments exclude objects reachable from the previous head.
                // A reader can rebuild the absent cache from the durable pack layout.
                return Err(ApiError::internal_message(
                    "incremental Git segment cannot seed a missing repository cache",
                ));
            } else {
                let attempt = REPOSITORY_MATERIALIZATION_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                let temp = self.cache_root().join(format!(
                    "repo-promoting.{}.{}.tmp",
                    std::process::id(),
                    attempt
                ));
                run_git(
                    None,
                    &["--bare", "init", temp.to_string_lossy().as_ref()],
                    "seeding repository Git cache from accepted push",
                )?;
                let build = (|| {
                    index_local_pack(&temp, local_pack)?;
                    run_timed_git_restore_phase(
                        repository_id,
                        "promote_update_ref",
                        Some(&temp),
                        &[
                            "update-ref",
                            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                            expected_head,
                        ],
                        "seeding repository Git cache head",
                    )?;
                    sanitize_repository_git_cache_repo(&temp, expected_head)?;
                    self.cache.note_applied(incarnation, &temp, push_sequence)?;
                    fs::rename(&temp, &target).map_err(ApiError::internal)
                })();
                if build.is_err() {
                    let _ = fs::remove_dir_all(&temp);
                }
                return build;
            }
            sanitize_repository_git_cache_repo(&target, expected_head)?;
            self.cache.note_applied(incarnation, &target, push_sequence)
        });
        tracing::info!(
            repository_id,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = push_sequence,
            applied_sequence = self.cache.applied_sequence(incarnation, &target),
            success = result.is_ok(),
            "repository Git replica post-push synchronization completed"
        );
        result
    }

    pub(crate) fn start_reaper(self: &Arc<Self>) {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5 * 60));
            loop {
                interval.tick().await;
                if let Err(error) = engine.cache.prune() {
                    tracing::warn!(error = %error.operator_diagnostic(), "failed to prune local repository Git caches");
                }
            }
        });
    }

    fn coordinate_repository(
        &self,
        incarnation: &RepositoryIncarnation,
        is_ready: impl Fn() -> bool,
        operation: impl FnOnce() -> Result<(), ApiError>,
    ) -> Result<(), ApiError> {
        self.materializations.materialize(
            GitDerivedCacheNamespace::Repository,
            format!(
                "{}:{}{}",
                incarnation.repository_id().len(),
                incarnation.repository_id(),
                incarnation.incarnation_id()
            ),
            is_ready,
            operation,
        )
    }

    fn catch_up(
        &self,
        state: &AppState,
        repository_id: &str,
        head: &GitHead,
        pack_spans: &[GitPackSpan],
        applied_sequence: u64,
        repo_root: &Path,
    ) -> Result<(), ApiError> {
        validate_git_pack_layout(pack_spans)
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        if applied_sequence >= head.push_sequence {
            return Err(ApiError::internal_message(
                "repository Git cache sequence cannot catch up to an older head",
            ));
        }
        let next_sequence = applied_sequence.saturating_add(1);
        let missing = pack_spans
            .iter()
            .skip_while(|span| span.last_sequence < next_sequence)
            .collect::<Vec<_>>();
        let first = missing.first().ok_or_else(|| {
            ApiError::internal_message("repository Git cache has no pack span for its missing tail")
        })?;
        if first.first_sequence > next_sequence {
            return Err(ApiError::internal_message(
                "repository Git cache missing tail starts after the required sequence",
            ));
        }
        let missing_count = missing.len();
        for (index, span) in missing.into_iter().enumerate() {
            index_git_pack(
                state,
                repo_root,
                repository_id,
                span,
                index + 1,
                missing_count,
            )?;
        }
        run_timed_git_restore_phase(
            repository_id,
            "update_ref",
            Some(repo_root),
            &[
                "update-ref",
                &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                &head.head_oid,
            ],
            "advancing repository Git cache head",
        )?;
        run_timed_git_restore_phase(
            repository_id,
            "fsck",
            Some(repo_root),
            &["fsck", "--connectivity-only", &head.head_oid],
            "verifying caught-up repository Git cache",
        )
    }
}

fn index_local_pack(repo_root: &Path, local_pack: &Path) -> Result<(), ApiError> {
    let pack = fs::File::open(local_pack).map_err(ApiError::internal)?;
    let output = run_with_stdin_reader(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        pack,
        ProcessLimits::new(crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout()),
        "indexing accepted local Git segment",
    )
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::infrastructure_unavailable(format!(
            "indexing accepted local Git segment: {}",
            crate::git::upload::truncated_git_stderr(&output.stderr).trim()
        )))
    }
}

fn repository_cache_is_ready(repo_path: &Path) -> bool {
    repo_path.is_dir() && repo_path.join("objects").is_dir()
}

fn materialization_outcome(cache_hit: bool, built: bool) -> &'static str {
    if cache_hit {
        "hit"
    } else if built {
        "build"
    } else {
        "wait"
    }
}

fn materialization_path_name(path: u8, cache_hit: bool, built: bool) -> &'static str {
    if cache_hit {
        return "hit";
    }
    if !built {
        return "wait";
    }
    match path {
        MATERIALIZATION_PATH_CATCH_UP => "catch_up",
        MATERIALIZATION_PATH_RESTORE => "restore",
        _ => "hit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, SystemTime},
    };

    fn test_engine(test: &str) -> Arc<RepositoryEngine> {
        let root = std::env::temp_dir().join(format!(
            "scope-{test}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        RepositoryEngine::new(root, 1024 * 1024 * 1024).unwrap()
    }

    fn incarnation(repository_id: &str) -> RepositoryIncarnation {
        RepositoryIncarnation::new(repository_id, format!("repoi_{repository_id}"))
            .expect("test repository identity is valid")
    }

    #[test]
    fn materialization_path_distinguishes_hits_waits_and_builds() {
        assert_eq!(
            materialization_path_name(MATERIALIZATION_PATH_HIT, true, false),
            "hit"
        );
        assert_eq!(
            materialization_path_name(MATERIALIZATION_PATH_HIT, false, false),
            "wait"
        );
        assert_eq!(
            materialization_path_name(MATERIALIZATION_PATH_CATCH_UP, false, true),
            "catch_up"
        );
        assert_eq!(
            materialization_path_name(MATERIALIZATION_PATH_RESTORE, false, true),
            "restore"
        );
    }

    #[test]
    fn same_repository_operations_are_serialized() {
        let engine = test_engine("repository-engine-serial");
        let root = engine.cache_root().to_path_buf();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let first_ready = Arc::new(AtomicBool::new(false));
        let second_ready = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let repo = incarnation("owner/repo");

        let first = {
            let engine = engine.clone();
            let active = active.clone();
            let max_active = max_active.clone();
            let first_ready = first_ready.clone();
            let repo = repo.clone();
            thread::spawn(move || {
                engine.coordinate_repository(
                    &repo,
                    || first_ready.load(Ordering::SeqCst),
                    || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(now, Ordering::SeqCst);
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        active.fetch_sub(1, Ordering::SeqCst);
                        first_ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        started_rx.recv().unwrap();
        let second = {
            let engine = engine.clone();
            let active = active.clone();
            let max_active = max_active.clone();
            let second_ready = second_ready.clone();
            let repo = repo.clone();
            thread::spawn(move || {
                engine.coordinate_repository(
                    &repo,
                    || second_ready.load(Ordering::SeqCst),
                    || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(now, Ordering::SeqCst);
                        active.fetch_sub(1, Ordering::SeqCst);
                        second_ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        thread::sleep(Duration::from_millis(20));
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn independent_repositories_run_in_parallel() {
        let engine = test_engine("repository-engine-parallel");
        let root = engine.cache_root().to_path_buf();
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first_repo = incarnation("owner/one");
        let first = {
            let engine = engine.clone();
            let first_repo = first_repo.clone();
            thread::spawn(move || {
                engine.coordinate_repository(
                    &first_repo,
                    || false,
                    || {
                        first_started_tx.send(()).unwrap();
                        release_first_rx.recv().unwrap();
                        Ok(())
                    },
                )
            })
        };
        first_started_rx.recv().unwrap();

        let (second_done_tx, second_done_rx) = mpsc::channel();
        let second = {
            let engine = engine.clone();
            let second_repo = incarnation("owner/two");
            thread::spawn(move || {
                engine.coordinate_repository(
                    &second_repo,
                    || false,
                    || {
                        second_done_tx.send(()).unwrap();
                        Ok(())
                    },
                )
            })
        };
        second_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("independent repository was blocked");
        release_first_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    struct IncrementalPushFixture {
        first_head: String,
        second_head: String,
        first_pack: PathBuf,
        second_pack: PathBuf,
    }

    fn incremental_push_fixture(engine: &RepositoryEngine) -> IncrementalPushFixture {
        let root = engine.cache_root();
        let work = root.join("work");
        let first_pack = root.join("first.pack");
        let second_pack = root.join("second.pack");
        run_git(
            None,
            &["init", work.to_string_lossy().as_ref()],
            "init test repo",
        )
        .unwrap();
        run_git(
            Some(&work),
            &["config", "user.name", "Scope Test"],
            "set user",
        )
        .unwrap();
        run_git(
            Some(&work),
            &["config", "user.email", "scope@example.test"],
            "set email",
        )
        .unwrap();
        fs::write(work.join("README.md"), "first\n").unwrap();
        run_git(Some(&work), &["add", "README.md"], "stage first commit").unwrap();
        run_git(
            Some(&work),
            &["commit", "-m", "first"],
            "create first commit",
        )
        .unwrap();
        run_git(
            Some(&work),
            &["branch", "-M", DEFAULT_GIT_BRANCH],
            "set main branch",
        )
        .unwrap();
        let first_head = git_head(&work);
        write_revision_pack(&work, &first_pack, format!("{first_head}\n"));

        fs::write(work.join("README.md"), "second\n").unwrap();
        run_git(
            Some(&work),
            &["commit", "-am", "second"],
            "create second commit",
        )
        .unwrap();
        let second_head = git_head(&work);
        write_revision_pack(
            &work,
            &second_pack,
            format!("{second_head}\n^{first_head}\n"),
        );
        IncrementalPushFixture {
            first_head,
            second_head,
            first_pack,
            second_pack,
        }
    }

    fn write_revision_pack(repo: &Path, destination: &Path, revisions: String) {
        let output = run_with_stdin_reader(
            Command::new("git")
                .current_dir(repo)
                .args(["pack-objects", "--revs", "--stdout"]),
            Cursor::new(revisions.into_bytes()),
            ProcessLimits::new(
                crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout(),
            ),
            "creating test Git pack",
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(destination, output.stdout).unwrap();
    }

    #[test]
    fn missing_cache_is_not_seeded_from_an_incremental_push() {
        let engine = test_engine("repository-engine-missing-incremental-base");
        let root = engine.cache_root().to_path_buf();
        let fixture = incremental_push_fixture(&engine);
        let repo = incarnation("owner/repo");
        let error = engine
            .sync_after_push(&repo, &fixture.second_pack, &fixture.second_head, 2)
            .unwrap_err();

        let replica = engine.repository_path(&repo);
        assert!(!replica.exists());
        assert_eq!(engine.cache.applied_sequence(&repo, &replica), None);
        assert!(
            error
                .operator_diagnostic()
                .contains("incremental Git segment cannot seed a missing repository cache")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delayed_post_push_sync_cannot_regress_a_newer_replica() {
        let engine = test_engine("repository-engine-monotonic-sync");
        let root = engine.cache_root().to_path_buf();
        let fixture = incremental_push_fixture(&engine);
        let repo = incarnation("owner/repo");

        engine
            .sync_after_push(&repo, &fixture.first_pack, &fixture.first_head, 1)
            .unwrap();
        engine
            .sync_after_push(&repo, &fixture.second_pack, &fixture.second_head, 2)
            .unwrap();
        engine
            .sync_after_push(&repo, &fixture.first_pack, &fixture.first_head, 1)
            .unwrap();

        let replica = engine.repository_path(&repo);
        assert_eq!(git_head(&replica), fixture.second_head);
        assert_eq!(engine.cache.applied_sequence(&repo, &replica), Some(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn separate_engines_do_not_reuse_a_predecessor_incarnation_cache() {
        let predecessor_engine = test_engine("repository-engine-recreated-multi-engine");
        let root = predecessor_engine.cache_root().to_path_buf();
        let recreated_engine = RepositoryEngine::new(root.clone(), 1024 * 1024 * 1024).unwrap();
        let fixture = incremental_push_fixture(&predecessor_engine);
        let predecessor = RepositoryIncarnation::new("owner/repo", "repoi_predecessor").unwrap();
        let recreated = RepositoryIncarnation::new("owner/repo", "repoi_recreated").unwrap();

        predecessor_engine
            .sync_after_push(&predecessor, &fixture.first_pack, &fixture.first_head, 1)
            .unwrap();
        recreated_engine
            .sync_after_push(&recreated, &fixture.first_pack, &fixture.first_head, 1)
            .unwrap();

        let predecessor_path = predecessor_engine.repository_path(&predecessor);
        let recreated_path = recreated_engine.repository_path(&recreated);
        assert_ne!(predecessor_path, recreated_path);
        assert!(predecessor_path.exists());
        assert!(recreated_path.exists());
        assert_eq!(
            recreated_engine
                .cache
                .applied_sequence(&recreated, &recreated_path),
            Some(1)
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn git_head(repo: &Path) -> String {
        String::from_utf8(
            run_git_output(
                Some(repo),
                &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
                "read test head",
            )
            .unwrap()
            .stdout,
        )
        .unwrap()
        .trim()
        .to_string()
    }
}
