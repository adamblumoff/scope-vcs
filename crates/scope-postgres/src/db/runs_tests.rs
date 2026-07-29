use super::{CatalogFixture, MetadataStore, TestDatabaseTarget, entities};
use crate::error::PostgresErrorKind;
use scope_domain::{
    policy::Visibility,
    runs::{
        run::{
            AttemptConclusion, AttemptState, PinnedContainerImage, Run, RunLogChunk, RunSource,
            RunState, RunTrigger,
        },
        runner::{RUNNER_PROTOCOL_VERSION, Runner, RunnerCapabilities, RunnerGrant, RunnerName},
        workflow::{
            CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowIdentity, WorkflowPath,
            WorkflowRevision, WorkflowStep, WorkflowTriggers,
        },
    },
    store::{RepoPublicationState, StoredRepository, UserAccount},
};
use sea_orm::EntityTrait;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::{sync::Barrier, task::JoinSet};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_create_exactly_one_active_attempt() {
    let store = Arc::new(postgres_store());
    register_runner(&store, "runner-1", "linux-one").await;
    register_runner(&store, "runner-2", "linux-two").await;
    enqueue(&store, run("run-1", "manual:one"), revision()).await;

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = JoinSet::new();
    for index in 1..=2 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .runs()
                .claim_run(
                    "run-1",
                    &format!("runner-{index}"),
                    &format!("attempt-{index}"),
                    &format!("{index:064x}"),
                    20,
                    80,
                )
                .await
        });
    }

    let mut claims = Vec::new();
    let mut conflicts = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(claim) => claims.push(claim),
            Err(error) if error.kind == PostgresErrorKind::Conflict => conflicts += 1,
            Err(error) => panic!("unexpected claim failure: {}", error.message),
        }
    }

    assert_eq!(claims.len(), 1);
    assert_eq!(conflicts, 1);
    let stored = store.runs().run("run-1").await.unwrap().unwrap();
    assert_eq!(stored.state, RunState::Leased);
    assert_eq!(
        stored.current_attempt_id.as_deref(),
        Some(claims[0].attempt.id.as_str())
    );
}

#[tokio::test]
async fn active_cancellation_is_intent_until_runner_acknowledges() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-1", "manual:cancel"), revision()).await;
    let claim = store
        .runs()
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
        .await
        .unwrap();

    let canceling = store
        .runs()
        .request_run_cancellation("run-1", 30)
        .await
        .unwrap();
    assert_eq!(canceling.state, RunState::Leased);
    assert!(canceling.cancellation_requested);
    assert_eq!(
        store
            .runs()
            .start_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 35)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    assert!(
        store
            .runs()
            .heartbeat_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 40, 100,)
            .await
            .unwrap()
    );
    let completed = store
        .runs()
        .complete_attempt(
            &claim.attempt.id,
            "runner-1",
            &"a".repeat(64),
            AttemptConclusion::Canceled,
            50,
        )
        .await
        .unwrap();
    assert_eq!(completed.run.state, RunState::Canceled);
    assert_eq!(completed.attempt.state, AttemptState::Canceled);
    assert_eq!(
        store
            .runs()
            .heartbeat_attempt(&claim.attempt.id, "runner-1", &"a".repeat(64), 60, 120,)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
}

#[tokio::test]
async fn lease_recovery_requeues_only_before_execution_and_rejects_stale_attempts() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    enqueue(&store, run("run-1", "manual:lease"), revision()).await;
    let first = store
        .runs()
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
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
        .claim_run("run-1", "runner-1", "attempt-2", &"b".repeat(64), 81, 140)
        .await
        .unwrap();
    assert_eq!(second.attempt.number, 2);
    let stale = store
        .runs()
        .complete_attempt(
            &first.attempt.id,
            "runner-1",
            &"a".repeat(64),
            AttemptConclusion::Failed { exit_code: 1 },
            90,
        )
        .await
        .unwrap_err();
    assert_eq!(stale.kind, PostgresErrorKind::Conflict);

    pin_attempt(&store, "attempt-2", "runner-1", &"b".repeat(64), 89).await;
    store
        .runs()
        .start_attempt("attempt-2", "runner-1", &"b".repeat(64), 90)
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
async fn terminal_run_retention_deletes_metadata_and_queues_its_source_atomically() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    let revision = revision();
    let revision_digest = revision.digest().to_string();
    enqueue(&store, run("run-1", "manual:retention"), revision).await;
    store
        .runs()
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
        .await
        .unwrap();
    pin_attempt(&store, "attempt-1", "runner-1", &"a".repeat(64), 21).await;
    store
        .runs()
        .start_attempt("attempt-1", "runner-1", &"a".repeat(64), 22)
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt(
            "attempt-1",
            "runner-1",
            &"a".repeat(64),
            AttemptConclusion::Succeeded,
            30,
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .runs()
            .prune_terminal_runs(29, 40, 10, &super::generated_ids::test_generated_id)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .runs()
            .prune_terminal_runs(30, 40, 10, &super::generated_ids::test_generated_id)
            .await
            .unwrap(),
        1
    );
    assert!(store.runs().run("run-1").await.unwrap().is_none());
    assert!(
        entities::workflow_revision::Entity::find_by_id(revision_digest)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .is_none()
    );
    let cleanup = store
        .cleanup()
        .source_blob_cleanup_batch(400, &super::generated_ids::test_generated_id)
        .await
        .unwrap();
    assert_eq!(cleanup.pending.len(), 1);
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
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
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
            .claim_run("run-1", "runner-1", "attempt-2", &"b".repeat(64), 81, 140,)
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
            .next_dispatchable_run("runner-2")
            .await
            .unwrap()
            .unwrap()
            .id,
        "run-1"
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
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
        .await
        .unwrap();
    pin_attempt(&store, "attempt-1", "runner-1", &"a".repeat(64), 20).await;
    store
        .runs()
        .start_attempt("attempt-1", "runner-1", &"a".repeat(64), 21)
        .await
        .unwrap();
    let first = RunLogChunk::new("attempt-1", 1, "first\n", 22).unwrap();
    let stored = store
        .runs()
        .append_attempt_log(first.clone(), &"a".repeat(64), 22)
        .await
        .unwrap();
    let retry = store
        .runs()
        .append_attempt_log(
            RunLogChunk::new("attempt-1", 1, first.text, 23).unwrap(),
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
                RunLogChunk::new("attempt-1", 1, "different\n", 22).unwrap(),
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
                RunLogChunk::new("attempt-1", 3, "gap\n", 23).unwrap(),
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
            RunLogChunk::new("attempt-1", 2, "second\n", 23).unwrap(),
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
            .next_dispatchable_run("runner-1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .runs()
            .next_dispatchable_run("runner-2")
            .await
            .unwrap()
            .unwrap()
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
            .next_dispatchable_run("runner-2")
            .await
            .unwrap()
            .is_none()
    );
}

fn postgres_store() -> MetadataStore {
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
    repository.record.publication_state = RepoPublicationState::Published;
    let mut other_repository =
        StoredRepository::new(&owner, "repo-two", Visibility::Private).unwrap();
    other_repository.record.publication_state = RepoPublicationState::Published;
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repository.record.id.clone(), repository);
    catalog
        .repositories
        .insert(other_repository.record.id.clone(), other_repository);
    catalog
}

async fn register_runner(store: &MetadataStore, id: &str, name: &str) {
    let runner = runner(id);
    store.runs().register_runner(runner.clone()).await.unwrap();
    store
        .runs()
        .grant_runner(
            RunnerGrant::new(
                "owner/repo",
                runner.id,
                RunnerName::parse(name).unwrap(),
                "user_owner",
                10,
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

fn runner(id: &str) -> Runner {
    let hash_byte = if id.ends_with('1') { '1' } else { '2' };
    Runner::new(
        id,
        "user_owner",
        hash_byte.to_string().repeat(64),
        "1.0.0",
        RUNNER_PROTOCOL_VERSION,
        RunnerCapabilities::v1(),
        10,
    )
    .unwrap()
}

fn revision() -> WorkflowRevision {
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
            runner,
            ContainerSpec::new("rust:1.90").unwrap(),
            20 * 60,
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
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

fn run(id: &str, idempotency_key: &str) -> Run {
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

fn run_for_revision(
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
        desired_runner,
        10,
    )
    .unwrap()
}

async fn enqueue(store: &MetadataStore, run: Run, revision: WorkflowRevision) {
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
