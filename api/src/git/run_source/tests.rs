use super::*;
use crate::git::import::{git_push_from_repo, run_git, run_git_output};
use scope_domain::{
    account::UserAccount, policy::Visibility, projection::ProjectionViewKey,
    repository::git::GitHead, runs::source::RunSource,
};

#[tokio::test]
async fn concurrent_git_head_materializations_share_one_build_and_reuse_the_pinned_bundle() {
    let mut state = AppState::test_state();
    let owner = UserAccount {
        id: "user-owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog
        .create_repository(&owner, "repo", Visibility::Private)
        .unwrap();
    catalog.users.insert(owner.id.clone(), owner);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    let repository = tempfile::tempdir().unwrap();
    run_git(
        None,
        &["init", "-b", "main", repository.path().to_str().unwrap()],
        "initialize source repo",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    let content = (0_u64..32_768)
        .flat_map(|index| Sha256::digest(index.to_le_bytes()))
        .collect::<Vec<_>>();
    fs::write(repository.path().join("README.md"), content).unwrap();
    run_git(
        Some(repository.path()),
        &["add", "README.md"],
        "stage source file",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-m", "pin source"],
        "commit source file",
    )
    .unwrap();

    let pushed = git_push_from_repo(&state, "owner/repo", repository.path(), None)
        .await
        .unwrap();
    let source = RunSource::accepted_git_head(
        "owner/repo",
        GitHead {
            head_oid: pushed.stored.head.head_oid.clone(),
            push_sequence: pushed.stored.head.push_sequence,
            change_version: 1,
            manifest: pushed.stored.head.manifest.clone(),
        },
        vec![pushed.stored.pack_span.clone()],
        ProjectionViewKey::Private,
    )
    .unwrap();

    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    let incarnation = state
        .metadata
        .repositories()
        .git_push_context("owner", "repo", "user-owner")
        .await
        .unwrap()
        .unwrap()
        .incarnation;
    state.runtime_budgets =
        std::sync::Arc::new(crate::runtime_budgets::RuntimeBudgets::from_config(
            crate::runtime_budgets::RuntimeBudgetConfig {
                git_materialization_concurrency: 1,
                ..Default::default()
            },
        ));
    let started = Instant::now();
    let results = futures_util::future::join_all((0..8).map(|_| {
        materialize_accepted_git_head_bundle(&state, &incarnation, &source, 4 * 1024 * 1024)
    }))
    .await;
    let cold_elapsed = started.elapsed();
    let mut results = results.into_iter();
    let materialized = results.next().unwrap().unwrap();
    assert!(!materialized.bytes.is_empty());
    for concurrent in results {
        let concurrent = concurrent.unwrap();
        assert_eq!(concurrent.bytes, materialized.bytes);
        assert_eq!(concurrent.sha256, materialized.sha256);
    }
    assert_eq!(
        materialized.sha256,
        hex::encode(Sha256::digest(&materialized.bytes))
    );

    // Warm reads require neither Git admission nor the original remote pack.
    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    state
        .git_segment_store
        .delete_remote(&scope_git_storage::object_key(
            "owner/repo",
            &pushed.stored.pack_span.segment.segment_id,
        ))
        .await
        .unwrap();
    let _busy = state.runtime_budgets.try_git_materialization().unwrap();
    let started = Instant::now();
    let warm = materialize_accepted_git_head_bundle(&state, &incarnation, &source, 4 * 1024 * 1024)
        .await
        .unwrap();
    eprintln!(
        "run source eight cold followers: {:?}; warm read: {:?}; bundle bytes: {}",
        cold_elapsed,
        started.elapsed(),
        warm.bytes.len()
    );
    assert_eq!(warm.bytes, materialized.bytes);
    assert_eq!(warm.sha256, materialized.sha256);
    assert!(
        materialize_accepted_git_head_bundle(&state, &incarnation, &source, 1)
            .await
            .is_err()
    );
    let recreated = RepositoryIncarnation::new("owner/repo", "different-incarnation").unwrap();
    assert!(
        materialize_accepted_git_head_bundle(&state, &recreated, &source, 4 * 1024 * 1024)
            .await
            .is_err()
    );

    let bundle = repository.path().join("source.bundle");
    fs::write(&bundle, materialized.bytes).unwrap();
    let output = std::process::Command::new("git")
        .args(["bundle", "list-heads", bundle.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(&pushed.stored.head.head_oid));
}

async fn git_head_fixture(state: &AppState) -> (RunSource, TemporarySourceDirectory) {
    let owner = UserAccount {
        id: "user-owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog
        .create_repository(&owner, "repo", Visibility::Private)
        .unwrap();
    catalog.users.insert(owner.id.clone(), owner);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    let repository = TemporarySourceDirectory::new(&state.data_dir.join("run-source")).unwrap();
    fs::create_dir_all(repository.path()).unwrap();
    run_git(
        None,
        &["init", "-b", "main", repository.path().to_str().unwrap()],
        "initialize source repo",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    fs::write(repository.path().join("README.md"), "pinned run source").unwrap();
    run_git(
        Some(repository.path()),
        &["add", "README.md"],
        "stage source file",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-m", "pin source"],
        "commit source file",
    )
    .unwrap();

    let pushed = git_push_from_repo(state, "owner/repo", repository.path(), None)
        .await
        .unwrap();
    let source = RunSource::accepted_git_head(
        "owner/repo",
        GitHead {
            head_oid: pushed.stored.head.head_oid.clone(),
            push_sequence: pushed.stored.head.push_sequence,
            change_version: 1,
            manifest: pushed.stored.head.manifest.clone(),
        },
        vec![pushed.stored.pack_span.clone()],
        ProjectionViewKey::Private,
    )
    .unwrap();

    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    (source, repository)
}

#[tokio::test]
async fn cancelled_index_and_bundle_requests_keep_repository_and_capacity_until_exit() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    // Restore children are cleanup, init, index, update-ref, fsck, symbolic-ref,
    // followed by bundle creation. Pause inside the selected blocking child.
    for phase in [3, 7] {
        for outcome in ["success", "failure", "panic"] {
            let state = AppState::test_state();
            let (source, _fixture) = git_head_fixture(&state).await;
            let _other_permit = state.runtime_budgets.try_git_materialization().unwrap();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let started_tx = Mutex::new(Some(started_tx));
            let (release_tx, release_rx) = mpsc::channel();
            let release_rx = Mutex::new(release_rx);
            let count = AtomicUsize::new(0);
            let repo_path = Arc::new(Mutex::new(PathBuf::new()));
            let path_for_hook = repo_path.clone();
            let owner = operation::with_hook(
                &state,
                Box::new(move || {
                    if count.fetch_add(1, Ordering::SeqCst) + 1 != phase {
                        return;
                    }
                    started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    let path = path_for_hook.lock().unwrap();
                    assert!(path.exists(), "repository removed beneath blocking child");
                    match outcome {
                        "panic" => panic!("injected blocking child panic"),
                        "failure" => {
                            if phase == 3 {
                                for entry in fs::read_dir(&*path).unwrap() {
                                    let entry = entry.unwrap();
                                    if entry.file_name().to_string_lossy().ends_with(".pack.tmp") {
                                        fs::remove_file(entry.path()).unwrap();
                                    }
                                }
                            } else {
                                fs::remove_file(path.join("refs/heads/main")).unwrap();
                            }
                        }
                        _ => {}
                    }
                }),
            );
            // Observe the path without keeping the operation resources alive.
            let path = operation::repository(&owner);
            *repo_path.lock().unwrap() = path.clone();
            let operation_state = state.clone();
            let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
            let request = tokio::spawn(async move {
                operation::supervise(async move {
                    let result = materialize_owned_git_head_bundle(
                        &operation_state,
                        &source,
                        4 * 1024 * 1024,
                        owner,
                    )
                    .await;
                    completed_tx.send(result.is_ok()).unwrap();
                    result
                })
                .await
            });
            tokio::time::timeout(Duration::from_secs(10), started_rx)
                .await
                .unwrap()
                .unwrap();
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            assert!(path.exists());
            assert!(state.runtime_budgets.try_git_materialization().is_err());
            release_tx.send(()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), completed_rx)
                    .await
                    .unwrap()
                    .unwrap(),
                outcome == "success",
            );
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(permit) = state.runtime_budgets.try_git_materialization() {
                        assert!(
                            !path.exists(),
                            "capacity returned before repository cleanup"
                        );
                        drop(permit);
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

#[test]
fn manual_bundle_inspection_reads_the_requested_workflow_at_the_pinned_commit() {
    let state = AppState::test_state();
    let source = tempfile::tempdir().unwrap();
    run_git(
        None,
        &["init", "-b", "main", source.path().to_str().unwrap()],
        "initialize manual run source",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    fs::create_dir_all(source.path().join(".scope/runs")).unwrap();
    fs::write(
        source.path().join(".scope/runs/checks.yml"),
        format!(
            "name: Checks\non:\n  manual: true\ncaches: []\ncontainer:\n  image: alpine@sha256:{}\ntimeout: 5m\njobs:\n  checks:\n    steps:\n      - name: Test\n        run: 'true'\n",
            "a".repeat(64)
        ),
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["add", "."],
        "stage manual run source",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["commit", "-m", "manual run source"],
        "commit manual run source",
    )
    .unwrap();
    let git_oid = String::from_utf8(
        run_git_output(
            Some(source.path()),
            &["rev-parse", "HEAD"],
            "read manual run commit",
        )
        .unwrap()
        .stdout,
    )
    .unwrap();
    let git_oid = git_oid.trim();
    let bundle_path = source.path().join("source.bundle");
    run_git(
        Some(source.path()),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create manual run bundle",
    )
    .unwrap();

    let parsed = inspect_manual_run_bundle(
        &state.data_dir.join("manual-run-inspection-test"),
        &fs::read(bundle_path).unwrap(),
        git_oid,
        "checks",
    )
    .unwrap();
    let revision = parsed.into_revision("owner/repo").unwrap();

    assert!(revision.definition().triggers().manual());
    assert_eq!(
        revision.workflow().path().as_str(),
        "/.scope/runs/checks.yml"
    );
}
