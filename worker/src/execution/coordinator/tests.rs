use super::*;
use crate::execution::fake::{FakeEcs, TEST_IMAGE};
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
use scope_postgres::db::{CatalogFixture, TestDatabaseTarget};

#[test]
fn bootstrap_tokens_use_the_runtime_auth_prefix() {
    let token = random_token("scope_bootstrap_").unwrap();
    assert!(token.starts_with("scope_bootstrap_"));
    assert_eq!(token.len(), "scope_bootstrap_".len() + 64);
}

#[tokio::test]
async fn interrupted_provider_starts_and_cleanup_remain_owned_after_worker_restart() {
    let metadata = queued_runs(3).await;
    let provider = FakeEcs::new().await;
    let coordinator = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        ecs: provider.client.clone(),
        origin_id: "worker-before-restart".into(),
        settings: provider.settings(),
    };
    let now = crate::unix_now().unwrap();
    let mut dispatch = tokio::task::JoinSet::new();
    dispatch.spawn(async move { coordinator.dispatch_available(now).await });
    // Actual coordinator admissions commit before concurrent HTTP RunTask calls.
    provider.wait_for("RunTask", 3).await;
    assert_eq!(
        metadata.runs().active_cloud_attempt_count().await.unwrap(),
        3
    );
    dispatch.shutdown().await;
    // AWS may finish those requests even though this worker lost the response.
    provider.starts.add_permits(3);
    let expired_at = now + DISPATCH_LEASE.as_secs() + 1;
    let expired = metadata
        .runs()
        .expired_attempt_ids(expired_at, 10)
        .await
        .unwrap();
    assert_eq!(expired.len(), 3);
    for attempt in &expired {
        let expired_claim = metadata
            .runs()
            .expire_attempt(attempt, expired_at)
            .await
            .unwrap();
        assert_eq!(
            expired_claim.job.state,
            scope_domain::runs::job::RunJobState::Queued
        );
    }
    assert!(
        metadata
            .runs()
            .next_dispatchable_job()
            .await
            .unwrap()
            .is_none(),
        "a retry cannot bypass cleanup of the previous uncertain provider task"
    );

    let restarted = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        ecs: provider.client.clone(),
        origin_id: "worker-after-restart".into(),
        settings: provider.settings(),
    };
    let mut cleanup = tokio::task::JoinSet::new();
    cleanup.spawn(async move { restarted.cleanup_terminal(expired_at).await });
    provider.wait_for("ListTasks", 3).await;
    cleanup.shutdown().await;
    assert!(
        metadata
            .runs()
            .claim_terminal_cloud_task_stops(expired_at + 1, 10)
            .await
            .unwrap()
            .is_empty()
    );
    // A second restart cannot lose the unknown outcome or prematurely free its fence.
    let reclaimed = metadata
        .runs()
        .claim_terminal_cloud_task_stops(expired_at + 901, 10)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 3);
    for task in reclaimed {
        assert!(expired.contains(&task.attempt_id));
        assert!(task.external_run_id.is_none());
    }
}

async fn queued_runs(count: usize) -> MetadataStore {
    let metadata =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "owner".into(),
        handle: "worker-owner".into(),
        email: "owner@example.test".into(),
        email_verified: true,
    };
    let mut repository = Repository::new(
        &owner,
        "worker-test",
        Visibility::Private,
        "repoi_worker_test",
    )
    .unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repo_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog.repositories.insert(repo_id.clone(), repository);
    metadata.admin().seed_catalog_for_tests(catalog).unwrap();
    let identity = WorkflowIdentity::new(
        repo_id,
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
    )
    .unwrap();
    let job = WorkflowJob::new(
        WorkflowJobId::parse("checks").unwrap(),
        vec![],
        ContainerSpec::new(TEST_IMAGE).unwrap(),
        600,
        vec![],
        Default::default(),
        vec![WorkflowStep::new("Test", "true").unwrap()],
    )
    .unwrap();
    let revision = WorkflowRevision::new(
        identity,
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![job],
        )
        .unwrap(),
    )
    .unwrap();
    let source = RunSource::ephemeral_git_bundle(SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
        sha256: "c".repeat(64),
        git_oid: "d".repeat(40),
        git_file_mode: "100644".into(),
        size_bytes: 42,
    })
    .unwrap();
    for id in 0..count {
        let run = Run::new(
            format!("run-{id}"),
            format!("manual:{id}"),
            revision.workflow().clone(),
            revision.digest(),
            RunTrigger::Manual,
            Some(owner.id.clone()),
            source.clone(),
            crate::unix_now().unwrap(),
        )
        .unwrap();
        metadata
            .runs()
            .enqueue_run(run, revision.clone())
            .await
            .unwrap();
    }
    metadata
}
