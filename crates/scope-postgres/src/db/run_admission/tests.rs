use super::*;
use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
        workflow::{
            definition::{
                CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
                WorkflowTriggers,
            },
            identity::{WorkflowIdentity, WorkflowPath},
            revision::WorkflowRevision,
        },
    },
};
const IMAGE_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn workflow(repository_id: &str) -> WorkflowRevision {
    let identity = WorkflowIdentity::new(
        repository_id,
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
    )
    .unwrap();
    let job = WorkflowJob::new(
        WorkflowJobId::parse("checks").unwrap(),
        vec![],
        ContainerSpec::new(format!("rust@sha256:{IMAGE_DIGEST}")).unwrap(),
        600,
        vec![],
        Default::default(),
        vec![WorkflowStep::new("Test", "cargo test").unwrap()],
    )
    .unwrap();
    WorkflowRevision::new(
        identity,
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![job],
        )
        .unwrap(),
    )
    .unwrap()
}

fn run(revision: &WorkflowRevision, id: &str) -> Run {
    Run::new(
        id,
        id,
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user_cache_owner".into()),
        RunSource::ephemeral_git_bundle(SourceBlob {
            content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
            sha256: "c".repeat(64),
            git_oid: "d".repeat(40),
            git_file_mode: "100644".into(),
            size_bytes: 42,
        })
        .unwrap(),
        10,
    )
    .unwrap()
}

fn seed_repository(store: &MetadataStore) -> String {
    let owner = UserAccount {
        id: "user_cache_owner".to_string(),
        handle: "cache-owner".to_string(),
        email: "cache-owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repository = Repository::new(&owner, "cache-repo", Visibility::Private, "repoi_test")
        .expect("test repository is valid");
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repository_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repository_id.clone(), repository);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    repository_id
}

async fn fixture(count: usize) -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let repository_id = seed_repository(&store);
    let revision = workflow(&repository_id);
    for index in 0..count {
        store
            .runs()
            .enqueue_run(run(&revision, &format!("run-{index:03}")), revision.clone())
            .await
            .unwrap();
    }
    store
}

#[tokio::test]
async fn concurrent_admission_obeys_global_capacity() {
    for limit in [0, 1, 3] {
        let store = fixture(8).await;
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..8 {
            let runs = store.runs();
            tasks.spawn(async move {
                runs.admit_next_job(
                    limit,
                    &format!("attempt-{index}"),
                    &format!("{index:064x}"),
                    "runtime",
                    11,
                    20,
                )
                .await
                .unwrap()
            });
        }
        let mut admitted = 0;
        while let Some(outcome) = tasks.join_next().await {
            match outcome.unwrap() {
                DispatchAdmission::Admitted(_) => admitted += 1,
                DispatchAdmission::AtCapacity => {}
                other => panic!("unexpected admission result: {other:?}"),
            }
        }
        assert_eq!(admitted, limit);
        let row = store.runs().db.query_one(Statement::from_string(DatabaseBackend::Postgres,
            "SELECT COUNT(*)::bigint AS active FROM scope_run_attempts WHERE state IN ('dispatching', 'running')"))
            .await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "active").unwrap(), limit as i64);
    }
}

#[tokio::test]
async fn exhausted_oldest_job_is_repaired_and_healthy_job_progresses() {
    let store = fixture(2).await;
    let runs = store.runs();
    // Reproduce the persisted state from the former final pre-start expiry behavior.
    runs.db
        .execute_unprepared(
            "UPDATE scope_run_jobs SET last_attempt_number = 99 WHERE run_id = 'run-000'",
        )
        .await
        .unwrap();
    let claim = runs
        .dispatch_job(
            "run-000",
            "checks",
            "last-attempt",
            &"b".repeat(64),
            "runtime",
            11,
            12,
        )
        .await
        .unwrap();
    runs.expire_attempt(&claim.attempt.id, 12).await.unwrap();
    runs.db.execute_unprepared("UPDATE scope_run_jobs SET state = 'queued', completed_at_unix = NULL WHERE run_id = 'run-000'; UPDATE scope_runs SET state = 'queued', completed_at_unix = NULL WHERE id = 'run-000'; UPDATE scope_run_attempts SET terminal_reason = '{\"kind\":\"execution-lost\",\"step_index\":null}' WHERE id = 'last-attempt'").await.unwrap();
    let outcome = runs
        .admit_next_job(1, "unused", &"c".repeat(64), "runtime", 13, 20)
        .await
        .unwrap();
    let DispatchAdmission::Exhausted(repaired) = outcome else {
        panic!("expected exhaustion repair");
    };
    assert_eq!(repaired.run.state, scope_domain::runs::run::RunState::Lost);
    assert_eq!(
        repaired.attempt.terminal_reason,
        Some(scope_domain::runs::step::AttemptTerminalReason::DispatchAttemptsExhausted)
    );
    let outcome = runs
        .admit_next_job(1, "healthy", &"d".repeat(64), "runtime", 13, 20)
        .await
        .unwrap();
    let DispatchAdmission::Admitted(claim) = outcome else {
        panic!("expected healthy admission");
    };
    assert_eq!(claim.run.id, "run-001");
}

#[tokio::test]
async fn retries_wait_for_provider_cleanup() {
    let store = fixture(1).await;
    let runs = store.runs();
    let DispatchAdmission::Admitted(claim) = runs
        .admit_next_job(1, "attempt-1", &"b".repeat(64), "runtime", 11, 12)
        .await
        .unwrap()
    else {
        panic!("expected admission");
    };
    runs.expire_attempt(&claim.attempt.id, 12).await.unwrap();
    assert!(matches!(
        runs.admit_next_job(1, "attempt-2", &"c".repeat(64), "runtime", 13, 20)
            .await
            .unwrap(),
        DispatchAdmission::Empty
    ));
    runs.complete_cloud_task_absence(&claim.attempt.id, 13)
        .await
        .unwrap();
    assert!(matches!(
        runs.admit_next_job(1, "attempt-2", &"c".repeat(64), "runtime", 13, 20)
            .await
            .unwrap(),
        DispatchAdmission::Admitted(_)
    ));
}

#[tokio::test]
async fn canceled_jobs_are_not_admitted() {
    let store = fixture(2).await;
    let runs = store.runs();
    runs.request_run_cancellation("run-000", 11).await.unwrap();
    let DispatchAdmission::Admitted(claim) = runs
        .admit_next_job(1, "healthy", &"b".repeat(64), "runtime", 12, 20)
        .await
        .unwrap()
    else {
        panic!("expected healthy admission");
    };
    assert_eq!(claim.run.id, "run-001");
}

#[tokio::test]
async fn uncertain_start_reserves_capacity_until_expiry_and_retains_cleanup_work() {
    let store = fixture(2).await;
    let runs = store.runs();
    let DispatchAdmission::Admitted(claim) = runs
        .admit_next_job(1, "uncertain", &"b".repeat(64), "runtime", 11, 12)
        .await
        .unwrap()
    else {
        panic!("expected admission");
    };
    // An ambiguous provider response leaves the reservation active.
    assert!(matches!(
        runs.admit_next_job(1, "next", &"c".repeat(64), "runtime", 11, 20)
            .await
            .unwrap(),
        DispatchAdmission::AtCapacity
    ));
    runs.expire_attempt(&claim.attempt.id, 12).await.unwrap();
    let cleanup = runs.claim_terminal_cloud_task_stops(13, 10).await.unwrap();
    assert_eq!(cleanup.len(), 1);
    assert_eq!(cleanup[0].attempt_id, "uncertain");
    let DispatchAdmission::Admitted(next) = runs
        .admit_next_job(1, "next", &"c".repeat(64), "runtime", 13, 20)
        .await
        .unwrap()
    else {
        panic!("expected admission");
    };
    assert_eq!(next.run.id, "run-001");
}

#[tokio::test]
async fn rejected_start_releases_capacity_and_completes_absent_task_cleanup() {
    let store = fixture(2).await;
    let runs = store.runs();
    let token_hash = "b".repeat(64);
    let DispatchAdmission::Admitted(claim) = runs
        .admit_next_job(1, "rejected", &token_hash, "runtime", 11, 20)
        .await
        .unwrap()
    else {
        panic!("expected admission");
    };
    runs.complete_attempt(
        &claim.attempt.id,
        &token_hash,
        scope_domain::runs::step::AttemptConclusion::SetupFailed {
            exit_code: 69,
            message: "provider rejected dispatch".into(),
        },
        false,
        12,
    )
    .await
    .unwrap();
    runs.complete_cloud_task_absence(&claim.attempt.id, 12)
        .await
        .unwrap();
    assert!(
        runs.claim_terminal_cloud_task_stops(13, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let DispatchAdmission::Admitted(next) = runs
        .admit_next_job(1, "next", &"c".repeat(64), "runtime", 13, 20)
        .await
        .unwrap()
    else {
        panic!("expected admission");
    };
    assert_eq!(next.run.id, "run-001");
}
