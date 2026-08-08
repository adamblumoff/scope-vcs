use super::{CatalogFixture, MetadataStore, TestDatabaseTarget, entities};
use crate::error::PostgresErrorKind;
use scope_domain::{
    policy::Visibility,
    runs::{
        run::{
            AttemptConclusion, AttemptState, PinnedContainerImage, Run, RunLogChunk, RunSource,
            RunState, RunTrigger, StepState,
        },
        runner::{
            RUNNER_PROTOCOL_VERSION, Runner, RunnerCapabilities, RunnerGrant,
            RunnerMaxConcurrentJobs, RunnerName,
        },
        workflow::{
            CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowIdentity, WorkflowJob,
            WorkflowJobId, WorkflowPath, WorkflowRevision, WorkflowStep, WorkflowTriggers,
        },
    },
    store::{
        RepoLifecycleState, RepositoryMember, RepositoryMemberPermissions, StoredRepository,
        UserAccount,
    },
};
use sea_orm::EntityTrait;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

mod job_scheduler;
mod retention;
pub(crate) use job_scheduler::parallel_revision;
mod runner_fixtures;
use runner_fixtures::runner;
pub(super) use runner_fixtures::{
    register_runner, register_runner_with_capacity, runner_with_capacity,
};
pub(super) mod workflow_fixtures;

#[tokio::test]
async fn lease_recovery_requeues_only_before_execution_and_rejects_stale_attempts() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-1", "manual:lease"), revision()).await;
    let first = store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-1",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    assert_eq!(
        store.runs().expired_attempt_ids(79, 10).await.unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        store.runs().expired_attempt_ids(80, 10).await.unwrap(),
        vec!["attempt-1"]
    );
    let recovered = store.runs().expire_attempt("attempt-1", 80).await.unwrap();
    assert_eq!(recovered.run.state, RunState::Queued);

    let second = store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-2",
            &"b".repeat(64),
            81,
            140,
        )
        .await
        .unwrap();
    assert_eq!(second.attempt.number, 2);
    assert!(
        second
            .steps
            .iter()
            .all(|step| step.state == StepState::Pending)
    );
    let details = store.runs().run_attempt_details("run-1").await.unwrap();
    assert_eq!(
        details
            .iter()
            .map(|detail| detail.attempt.id.as_str())
            .collect::<Vec<_>>(),
        vec!["attempt-2", "attempt-1"]
    );
    assert!(
        details[0]
            .steps
            .iter()
            .all(|step| step.state == StepState::Pending)
    );
    assert!(
        details[1]
            .steps
            .iter()
            .all(|step| step.state == StepState::Skipped)
    );
    let stale = store
        .runs()
        .complete_attempt(
            &first.attempt.id,
            &"a".repeat(64),
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "setup failed".to_string(),
            },
            90,
        )
        .await
        .unwrap_err();
    assert_eq!(stale.kind, PostgresErrorKind::Conflict);

    pin_attempt(&store, "attempt-2", "runner-1", &"b".repeat(64), 89).await;
    store
        .runs()
        .start_attempt_step("attempt-2", "runner-1", &"b".repeat(64), 0, 90)
        .await
        .unwrap();
    assert_eq!(
        store.runs().expired_attempt_ids(140, 10).await.unwrap(),
        vec!["attempt-2"]
    );
    let lost = store.runs().expire_attempt("attempt-2", 140).await.unwrap();
    assert_eq!(lost.run.state, RunState::Lost);
    assert_eq!(lost.attempt.state, AttemptState::Lost);
}

#[tokio::test]
async fn repository_deletion_queues_run_sources_and_removes_orphaned_workflows() {
    let store = postgres_store();
    let revision = revision();
    let revision_digest = revision.digest().to_string();
    enqueue(&store, run("run-delete", "manual:repo-delete"), revision).await;

    store
        .repositories()
        .delete_repo(
            "owner",
            "repo",
            "user_owner",
            40,
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    assert!(store.runs().run("run-delete").await.unwrap().is_none());
    assert!(
        entities::workflow_revision::Entity::find_by_id(revision_digest)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .is_none()
    );
    let references = entities::object_reference::Entity::find()
        .all(store.db.as_ref())
        .await
        .unwrap();
    assert!(
        references
            .iter()
            .all(|reference| reference.ref_kind != "run_source")
    );
    let cleanup = store
        .cleanup()
        .pending_source_blob_cleanups_for_tests()
        .await
        .unwrap();
    assert_eq!(cleanup.len(), 1);
}

#[tokio::test]
async fn names_are_repository_scoped_revisions_are_idempotent_and_revocation_stops_heartbeats() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    let second = runner("runner-2");
    store.runs().register_runner(second.clone()).await.unwrap();
    let duplicate_name = RunnerGrant::new(
        "owner/repo",
        &second.id,
        RunnerName::parse("linux-box").unwrap(),
        "user_owner",
        10,
    )
    .unwrap();
    assert_eq!(
        store
            .runs()
            .grant_runner(duplicate_name)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );

    let first = store
        .runs()
        .enqueue_run(run("run-1", "manual:idempotent"), revision())
        .await
        .unwrap();
    let mut retry = run("run-2", "manual:idempotent");
    retry.source = first.source.clone();
    let duplicate = store.runs().enqueue_run(retry, revision()).await.unwrap();
    assert_eq!(duplicate.id, first.id);
    let conflicting_retry = store
        .runs()
        .enqueue_run(run("run-conflict", "manual:idempotent"), revision())
        .await
        .unwrap_err();
    assert_eq!(conflicting_retry.kind, PostgresErrorKind::Conflict);
    let other_repo = store
        .runs()
        .enqueue_run(
            run_for_repository("run-3", "manual:idempotent", "owner/repo-two"),
            revision_for_repository("owner/repo-two"),
        )
        .await
        .unwrap();
    assert_eq!(other_repo.id, "run-3");

    let claim = store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-1",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    store
        .runs()
        .revoke_runner_grant("owner/repo", "runner-1", 30)
        .await
        .unwrap();
    let reattach_active_runner = RunnerGrant::new(
        "owner/repo",
        "runner-1",
        RunnerName::parse("linux-returned").unwrap(),
        "user_owner",
        31,
    )
    .unwrap();
    assert_eq!(
        store
            .runs()
            .grant_runner(reattach_active_runner)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    assert_eq!(
        store
            .runs()
            .heartbeat_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 40, 100,)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    let reconciled = store.runs().expire_attempt("attempt-1", 80).await.unwrap();
    assert_eq!(reconciled.run.state, RunState::Queued);
    assert_eq!(
        store
            .runs()
            .claim_job(
                "run-1",
                "checks",
                "runner-1",
                "attempt-2",
                &"b".repeat(64),
                81,
                140,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    store
        .runs()
        .grant_runner(
            RunnerGrant::new(
                "owner/repo",
                "runner-1",
                RunnerName::parse("linux-returned").unwrap(),
                "user_owner",
                81,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .runs()
        .revoke_runner_grant("owner/repo", "runner-1", 82)
        .await
        .unwrap();

    let reused_name = RunnerGrant::new(
        "owner/repo",
        &second.id,
        RunnerName::parse("linux-box").unwrap(),
        "user_owner",
        83,
    )
    .unwrap();
    store.runs().grant_runner(reused_name).await.unwrap();
    assert_eq!(
        store
            .runs()
            .next_dispatchable_job("runner-2", 83)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        "run-1"
    );
}

#[tokio::test]
async fn removing_repository_member_revokes_that_members_runner_grants_atomically() {
    let store = postgres_store();
    let member_runner = Runner::new(
        "member-runner",
        "user_member",
        "3".repeat(64),
        "1.0.0",
        RUNNER_PROTOCOL_VERSION,
        RunnerCapabilities::v1(),
        RunnerMaxConcurrentJobs::new(1).unwrap(),
        10,
    )
    .unwrap();
    store
        .runs()
        .register_runner(member_runner.clone())
        .await
        .unwrap();
    store
        .runs()
        .grant_runner(
            RunnerGrant::new(
                "owner/repo",
                &member_runner.id,
                RunnerName::parse("member-linux").unwrap(),
                "user_member",
                10,
            )
            .unwrap(),
        )
        .await
        .unwrap();

    store
        .repositories()
        .remove_repository_member(
            "owner",
            "repo",
            "user_owner",
            "user_member",
            20,
            &super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    let grants = store.runs().runner_grants(&member_runner.id).await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].revoked_at_unix, Some(20));
    assert!(
        store
            .runs()
            .next_dispatchable_job(&member_runner.id, 20)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn machine_authentication_and_attempt_logs_are_narrow_and_idempotent() {
    let store = postgres_store();
    let runner = runner("runner-1");
    let runner_hash = runner.secret_hash.clone();
    store.runs().register_runner(runner).await.unwrap();
    store
        .runs()
        .grant_runner(
            RunnerGrant::new(
                "owner/repo",
                "runner-1",
                RunnerName::parse("linux-box").unwrap(),
                "user_owner",
                10,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .runs()
            .authenticate_runner(&runner_hash, 11)
            .await
            .unwrap()
            .last_seen_at_unix,
        Some(11)
    );
    assert_eq!(
        store
            .runs()
            .authenticate_runner(&"f".repeat(64), 11)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Unauthenticated
    );

    enqueue(&store, run("run-1", "manual:logs"), revision()).await;
    store
        .runs()
        .claim_job(
            "run-1",
            "checks",
            "runner-1",
            "attempt-1",
            &"a".repeat(64),
            20,
            80,
        )
        .await
        .unwrap();
    pin_attempt(&store, "attempt-1", "runner-1", &"a".repeat(64), 20).await;
    store
        .runs()
        .start_attempt_step("attempt-1", "runner-1", &"a".repeat(64), 0, 21)
        .await
        .unwrap();
    let first = RunLogChunk::new("attempt-1", 0, 1, "first\n", 22).unwrap();
    let stored = store
        .runs()
        .append_attempt_log(first.clone(), &"a".repeat(64), 22)
        .await
        .unwrap();
    let retry = store
        .runs()
        .append_attempt_log(
            RunLogChunk::new("attempt-1", 0, 1, first.text, 23).unwrap(),
            &"a".repeat(64),
            23,
        )
        .await
        .unwrap();
    assert_eq!(retry.position, stored.position);
    assert_eq!(
        store
            .runs()
            .append_attempt_log(
                RunLogChunk::new("attempt-1", 0, 1, "different\n", 22).unwrap(),
                &"a".repeat(64),
                23,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    assert_eq!(
        store
            .runs()
            .append_attempt_log(
                RunLogChunk::new("attempt-1", 0, 3, "gap\n", 23).unwrap(),
                &"a".repeat(64),
                23,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    let second = store
        .runs()
        .append_attempt_log(
            RunLogChunk::new("attempt-1", 0, 2, "second\n", 23).unwrap(),
            &"a".repeat(64),
            23,
        )
        .await
        .unwrap();
    assert!(second.position > stored.position);
    assert_eq!(
        store
            .runs()
            .next_attempt_log_sequence("attempt-1")
            .await
            .unwrap(),
        3
    );
    let attempt = store
        .runs()
        .authenticate_attempt("attempt-1", &"a".repeat(64), 23)
        .await
        .unwrap()
        .attempt;
    assert_eq!(attempt.log_bytes, 13);
    assert!(!attempt.logs_truncated);
    let logs = store
        .runs()
        .run_logs_after("run-1", stored.position, 100)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].chunk.text, "second\n");
    let step_logs = store
        .runs()
        .attempt_step_logs_after("run-1", "attempt-1", 0, stored.position, 100)
        .await
        .unwrap();
    assert_eq!(step_logs.logs.len(), 1);
    assert_eq!(step_logs.logs[0].chunk.step_index, 0);
    assert!(!step_logs.logs_truncated);
}

#[tokio::test]
async fn enqueue_enforces_workflow_trigger_and_runner_policy() {
    let store = postgres_store();
    let disabled_trigger = run_with_options(
        "run-1",
        "push:disabled",
        "owner/repo",
        RunnerSelector::Any,
        RunTrigger::PushMain,
        None,
    );
    assert_eq!(
        store
            .runs()
            .enqueue_run(disabled_trigger, revision())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::InvalidInput
    );
    assert!(store.runs().run("run-1").await.unwrap().is_none());

    let named_revision = revision_for_repository_with_runner(
        "owner/repo",
        RunnerSelector::named("linux-one").unwrap(),
    );
    let widened_manual = run_for_revision(
        "run-2",
        "manual:widened",
        &named_revision,
        RunnerSelector::Any,
        RunTrigger::Manual,
        Some("user_owner".to_string()),
    );
    assert_eq!(
        store
            .runs()
            .enqueue_run(widened_manual, named_revision.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::InvalidInput
    );

    let named_override = run_for_revision(
        "run-3",
        "manual:override",
        &named_revision,
        RunnerSelector::named("linux-two").unwrap(),
        RunTrigger::Manual,
        Some("user_owner".to_string()),
    );
    assert_eq!(
        store
            .runs()
            .enqueue_run(named_override, named_revision)
            .await
            .unwrap()
            .id,
        "run-3"
    );
}

#[tokio::test]
async fn dispatch_candidates_respect_runner_names_and_enabled_state() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    enqueue(
        &store,
        run_for_selector(
            "run-1",
            "manual:routed",
            RunnerSelector::named("linux-two").unwrap(),
        ),
        revision(),
    )
    .await;

    assert!(
        store
            .runs()
            .next_dispatchable_job("runner-1", 10)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .runs()
            .next_dispatchable_job("runner-2", 10)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        "run-1"
    );

    store
        .runs()
        .set_runner_enabled("runner-2", false)
        .await
        .unwrap();
    assert!(
        store
            .runs()
            .next_dispatchable_job("runner-2", 10)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn repository_operations_keep_all_active_runs_beyond_the_recent_limit() {
    let store = postgres_store();
    enqueue(&store, run("run-terminal", "manual:terminal"), revision()).await;
    store
        .runs()
        .request_run_cancellation("run-terminal", 11)
        .await
        .unwrap();
    for index in 0..21 {
        enqueue(
            &store,
            run(
                &format!("run-active-{index:02}"),
                &format!("manual:active:{index:02}"),
            ),
            revision(),
        )
        .await;
    }

    let runs = store
        .runs()
        .repository_operations_runs("owner/repo", 20)
        .await
        .unwrap();
    assert_eq!(runs.len(), 21);
    assert!(runs.iter().all(|run| !run.run.state.is_terminal()));
    assert_eq!(
        runs.iter()
            .map(|run| &run.run.id)
            .collect::<BTreeSet<_>>()
            .len(),
        runs.len()
    );
}

pub(super) fn postgres_store() -> MetadataStore {
    let store = MetadataStore::connect_fresh_for_tests(
        &TestDatabaseTarget::required().expect("test database target"),
    )
    .expect("connect test database");
    store
        .admin()
        .seed_catalog_for_tests(catalog_with_repo())
        .unwrap();
    store
}

fn catalog_with_repo() -> CatalogFixture {
    let owner = UserAccount {
        id: "user_owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repository = StoredRepository::new(&owner, "repo", Visibility::Private).unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let member = UserAccount {
        id: "user_member".to_string(),
        handle: "member".to_string(),
        email: "member@example.com".to_string(),
        email_verified: true,
    };
    repository.members.push(RepositoryMember {
        repo_id: repository.record.id.clone(),
        user_id: member.id.clone(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let mut other_repository =
        StoredRepository::new(&owner, "repo-two", Visibility::Private).unwrap();
    other_repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(member.id.clone(), member);
    catalog
        .repositories
        .insert(repository.record.id.clone(), repository);
    catalog
        .repositories
        .insert(other_repository.record.id.clone(), other_repository);
    catalog
}

pub(super) fn revision() -> WorkflowRevision {
    revision_for_repository("owner/repo")
}

fn revision_for_repository(repository_id: &str) -> WorkflowRevision {
    revision_for_repository_with_runner(repository_id, RunnerSelector::Any)
}

fn revision_for_repository_with_runner(
    repository_id: &str,
    runner: RunnerSelector,
) -> WorkflowRevision {
    WorkflowRevision::new(
        workflow_identity_for(repository_id),
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![],
                    runner,
                    ContainerSpec::new("rust:1.90").unwrap(),
                    20 * 60,
                    vec![],
                    vec![WorkflowStep::new("Test", "cargo test").unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn workflow_identity_for(repository_id: &str) -> WorkflowIdentity {
    WorkflowIdentity::new(
        repository_id,
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
    )
    .unwrap()
}

pub(super) fn run(id: &str, idempotency_key: &str) -> Run {
    run_for_selector(id, idempotency_key, RunnerSelector::Any)
}

fn run_for_selector(id: &str, idempotency_key: &str, desired_runner: RunnerSelector) -> Run {
    run_for_repository_and_selector(id, idempotency_key, "owner/repo", desired_runner)
}

fn run_for_repository(id: &str, idempotency_key: &str, repository_id: &str) -> Run {
    run_for_repository_and_selector(id, idempotency_key, repository_id, RunnerSelector::Any)
}

fn run_for_repository_and_selector(
    id: &str,
    idempotency_key: &str,
    repository_id: &str,
    desired_runner: RunnerSelector,
) -> Run {
    run_with_options(
        id,
        idempotency_key,
        repository_id,
        desired_runner,
        RunTrigger::Manual,
        Some("user_owner".to_string()),
    )
}

fn run_with_options(
    id: &str,
    idempotency_key: &str,
    repository_id: &str,
    desired_runner: RunnerSelector,
    trigger: RunTrigger,
    requested_by_user_id: Option<String>,
) -> Run {
    let revision = revision_for_repository(repository_id);
    run_for_revision(
        id,
        idempotency_key,
        &revision,
        desired_runner,
        trigger,
        requested_by_user_id,
    )
}

pub(super) fn run_for_revision(
    id: &str,
    idempotency_key: &str,
    revision: &WorkflowRevision,
    desired_runner: RunnerSelector,
    trigger: RunTrigger,
    requested_by_user_id: Option<String>,
) -> Run {
    let source_digest = hex::encode(Sha256::digest(id.as_bytes()));
    Run::new(
        id,
        idempotency_key,
        revision.workflow().clone(),
        revision.digest(),
        trigger,
        requested_by_user_id,
        RunSource::ephemeral_git_bundle(scope_domain::store::SourceBlob {
            content_ref: scope_domain::content_ref::ContentRef::git_bundle_sha256(
                source_digest.clone(),
            ),
            sha256: source_digest.clone(),
            git_oid: source_digest[..40].to_string(),
            git_file_mode: scope_domain::store::DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 42,
        })
        .unwrap(),
        Some(desired_runner),
        10,
    )
    .unwrap()
}

pub(super) async fn enqueue(store: &MetadataStore, run: Run, revision: WorkflowRevision) {
    store.runs().enqueue_run(run, revision).await.unwrap();
}

async fn pin_attempt(
    store: &MetadataStore,
    attempt_id: &str,
    runner_id: &str,
    token_hash: &str,
    now_unix: u64,
) {
    store
        .runs()
        .pin_attempt_container_image(
            attempt_id,
            runner_id,
            token_hash,
            PinnedContainerImage::parse(format!("registry.example/job@sha256:{}", "1".repeat(64)))
                .unwrap(),
            now_unix,
        )
        .await
        .unwrap();
}
