use super::{MetadataStore, runs_tests};
use scope_domain::runs::{
    cache::WorkflowCache,
    cutover::{
        RUNNER_PROTOCOL_CANARY_CACHE_NAME, RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS,
        RunnerProtocolCanaryPhase, RunnerProtocolCanaryStatus, RunnerProtocolCutoverState,
    },
    run::{PinnedContainerImage, RunTrigger, StepConclusion},
    runner::{RUNNER_PROTOCOL_VERSION, RunnerCapabilities},
    workflow::{
        CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowIdentity, WorkflowPath,
        WorkflowRevision, WorkflowStep, WorkflowTriggers,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use sha2::{Digest, Sha256};

const CANARY_IMAGE: &str = "registry.example/runner-canary@sha256:1111111111111111111111111111111111111111111111111111111111111111";

#[tokio::test]
async fn fenced_v4_dispatches_only_the_canary_suite_then_opens() {
    let store = runs_tests::postgres_store();
    runs_tests::register_runner(&store, "runner-1", "linux-box").await;
    runs_tests::enqueue(
        &store,
        runs_tests::run("run-general", "manual:general"),
        runs_tests::revision(),
    )
    .await;
    set_fenced(&store).await;

    assert!(
        store
            .runs()
            .enqueue_run(
                runs_tests::run("run-arbitrary", "manual:arbitrary"),
                runs_tests::revision()
            )
            .await
            .is_err()
    );
    for (phase, run_id) in [
        (RunnerProtocolCanaryPhase::ColdWrite, "run-cold-old"),
        (RunnerProtocolCanaryPhase::ColdWrite, "run-cold"),
        (RunnerProtocolCanaryPhase::WarmRead, "run-warm"),
        (RunnerProtocolCanaryPhase::Evict, "run-evict"),
    ] {
        enqueue_canary(&store, phase, run_id, CANARY_IMAGE, "linux-box").await;
    }
    enqueue_canary(
        &store,
        RunnerProtocolCanaryPhase::WarmRead,
        "run-warm-wrong-image",
        &format!("registry.example/runner-canary@sha256:{}", "2".repeat(64)),
        "linux-box",
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

    store
        .admin()
        .create_runner_protocol_canary(
            RunnerProtocolCanaryPhase::ColdWrite,
            "runner-1",
            "run-cold-old",
            19,
        )
        .await
        .unwrap();
    let reassigned = store
        .admin()
        .create_runner_protocol_canary(
            RunnerProtocolCanaryPhase::ColdWrite,
            "runner-1",
            "run-cold",
            20,
        )
        .await
        .unwrap();
    assert_eq!(reassigned.canaries.len(), 1);
    assert_eq!(reassigned.canaries[0].run_id(), "run-cold");

    for (phase, run_id) in [
        (RunnerProtocolCanaryPhase::ColdWrite, "run-cold"),
        (RunnerProtocolCanaryPhase::WarmRead, "run-warm"),
        (RunnerProtocolCanaryPhase::Evict, "run-evict"),
    ] {
        if phase != RunnerProtocolCanaryPhase::ColdWrite {
            if phase == RunnerProtocolCanaryPhase::WarmRead {
                assert!(
                    store
                        .admin()
                        .create_runner_protocol_canary(
                            phase,
                            "runner-1",
                            "run-warm-wrong-image",
                            20,
                        )
                        .await
                        .is_err()
                );
            }
            store
                .admin()
                .create_runner_protocol_canary(phase, "runner-1", run_id, 20)
                .await
                .unwrap();
        }
        assert_eq!(
            store
                .runs()
                .next_dispatchable_run("runner-1")
                .await
                .unwrap()
                .unwrap()
                .id,
            run_id
        );
        let (attempt_id, token_hash) = complete_canary_attempt(&store, run_id).await;
        let running = store.admin().runner_protocol_cutover().await.unwrap();
        assert_eq!(
            running.canaries.last().unwrap().status(),
            RunnerProtocolCanaryStatus::Running
        );
        store
            .runs()
            .finalize_runner_protocol_canary_cache(&attempt_id, &token_hash, true, 25)
            .await
            .unwrap();
    }

    let fenced = store.admin().runner_protocol_cutover().await.unwrap();
    assert_eq!(fenced.cutover.state(), RunnerProtocolCutoverState::V4Fenced);
    assert_eq!(fenced.canary_generation, 1);
    assert!(
        fenced
            .canaries
            .iter()
            .all(|canary| canary.status() == RunnerProtocolCanaryStatus::Succeeded)
    );
    let opened = store
        .admin()
        .advance_runner_protocol_cutover(RunnerProtocolCutoverState::V4Open, 50)
        .await
        .unwrap();
    assert_eq!(opened.cutover.state(), RunnerProtocolCutoverState::V4Open);
    assert_eq!(
        store
            .runs()
            .next_dispatchable_run("runner-1")
            .await
            .unwrap()
            .unwrap()
            .id,
        "run-cold-old"
    );
}

#[tokio::test]
async fn open_v4_hot_paths_do_not_take_the_cutover_row_lock() {
    let store = runs_tests::postgres_store();
    runs_tests::register_runner(&store, "runner-1", "linux-box").await;
    let revision = runs_tests::revision();
    runs_tests::enqueue(
        &store,
        runs_tests::run("run-1", "manual:one"),
        revision.clone(),
    )
    .await;
    runs_tests::enqueue(&store, runs_tests::run("run-2", "manual:two"), revision).await;
    store
        .runs()
        .claim_run("run-1", "runner-1", "attempt-1", &"a".repeat(64), 20, 80)
        .await
        .unwrap();

    let lock = store.db.begin().await.unwrap();
    lock.execute(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT 1 FROM scope_runner_protocol_cutover WHERE key = 'current' FOR UPDATE".to_string(),
    ))
    .await
    .unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        store
            .runs()
            .heartbeat_attempt("attempt-1", "runner-1", &"a".repeat(64), 21, 90),
    )
    .await
    .expect("heartbeat should not wait for the cutover singleton")
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        store
            .runs()
            .claim_run("run-2", "runner-1", "attempt-2", &"b".repeat(64), 22, 90),
    )
    .await
    .expect("claim should not wait for the cutover singleton")
    .unwrap();
    lock.rollback().await.unwrap();
}

#[tokio::test]
async fn failed_canary_starts_a_bounded_replacement_generation() {
    let store = runs_tests::postgres_store();
    runs_tests::register_runner(&store, "runner-1", "linux-box").await;
    set_fenced(&store).await;
    enqueue_canary(
        &store,
        RunnerProtocolCanaryPhase::ColdWrite,
        "run-1",
        CANARY_IMAGE,
        "linux-box",
    )
    .await;
    enqueue_canary(
        &store,
        RunnerProtocolCanaryPhase::ColdWrite,
        "run-2",
        CANARY_IMAGE,
        "linux-box",
    )
    .await;
    store
        .admin()
        .create_runner_protocol_canary(
            RunnerProtocolCanaryPhase::ColdWrite,
            "runner-1",
            "run-1",
            20,
        )
        .await
        .unwrap();
    let (attempt_id, token_hash) = complete_canary_attempt(&store, "run-1").await;
    assert!(
        store
            .admin()
            .create_runner_protocol_canary(
                RunnerProtocolCanaryPhase::ColdWrite,
                "runner-1",
                "run-2",
                22,
            )
            .await
            .is_err()
    );
    store
        .runs()
        .finalize_runner_protocol_canary_cache(&attempt_id, &token_hash, false, 23)
        .await
        .unwrap();

    let replacement = store
        .admin()
        .create_runner_protocol_canary(
            RunnerProtocolCanaryPhase::ColdWrite,
            "runner-1",
            "run-2",
            24,
        )
        .await
        .unwrap();
    assert_eq!(replacement.canary_generation, 2);
    assert_eq!(replacement.canaries.len(), 1);
    assert_eq!(replacement.canaries[0].generation().get(), 2);
    assert_eq!(replacement.canaries[0].run_id(), "run-2");
}

#[tokio::test]
async fn owned_v3_runner_upgrade_rotates_credentials_atomically() {
    let store = runs_tests::postgres_store();
    runs_tests::register_runner(&store, "runner-1", "linux-box").await;
    store
        .db
        .execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "UPDATE scope_runners SET protocol_version = 3, enabled = FALSE WHERE id = 'runner-1'"
                .to_string(),
        ))
        .await
        .unwrap();
    set_fenced(&store).await;

    let old_hash = "1".repeat(64);
    let new_hash = "9".repeat(64);
    assert!(
        store
            .runs()
            .authenticate_runner(&old_hash, 20)
            .await
            .is_err()
    );
    assert!(
        store
            .runs()
            .upgrade_runner_registration(
                "runner-1",
                "user_other",
                new_hash.clone(),
                "2.0.0".to_string(),
                RUNNER_PROTOCOL_VERSION,
                RunnerCapabilities::v1(),
            )
            .await
            .is_err()
    );
    let upgraded = store
        .runs()
        .upgrade_runner_registration(
            "runner-1",
            "user_owner",
            new_hash.clone(),
            "2.0.0".to_string(),
            RUNNER_PROTOCOL_VERSION,
            RunnerCapabilities::v1(),
        )
        .await
        .unwrap();
    assert_eq!(upgraded.version, "2.0.0");
    assert_eq!(upgraded.protocol_version, RUNNER_PROTOCOL_VERSION);
    assert!(upgraded.enabled);
    assert!(
        store
            .runs()
            .authenticate_runner(&old_hash, 21)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .runs()
            .authenticate_runner(&new_hash, 21)
            .await
            .unwrap()
            .id,
        "runner-1"
    );
}

async fn set_fenced(store: &MetadataStore) {
    store
        .db
        .execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "UPDATE scope_runner_protocol_cutover SET state = 'v4-fenced' WHERE key = 'current'"
                .to_string(),
        ))
        .await
        .unwrap();
}

async fn complete_canary_attempt(store: &MetadataStore, run_id: &str) -> (String, String) {
    let attempt_id = format!("attempt-{run_id}");
    let token_hash = hex::encode(Sha256::digest(run_id.as_bytes()));
    let claim = store
        .runs()
        .claim_run(run_id, "runner-1", &attempt_id, &token_hash, 21, 90)
        .await
        .unwrap();
    assert!(claim.canary_phase.is_some());
    store
        .runs()
        .pin_attempt_container_image(
            &attempt_id,
            "runner-1",
            &token_hash,
            PinnedContainerImage::parse(CANARY_IMAGE).unwrap(),
            22,
        )
        .await
        .unwrap();
    store
        .runs()
        .start_attempt_step(&attempt_id, "runner-1", &token_hash, 0, 23)
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt_step(
            &attempt_id,
            "runner-1",
            &token_hash,
            0,
            StepConclusion::Succeeded,
            24,
        )
        .await
        .unwrap();
    (attempt_id, token_hash)
}

async fn enqueue_canary(
    store: &MetadataStore,
    phase: RunnerProtocolCanaryPhase,
    run_id: &str,
    image: &str,
    runner_name: &str,
) {
    let revision = canary_revision(phase, image, runner_name);
    let run = runs_tests::run_for_revision(
        run_id,
        &format!("canary:{run_id}"),
        &revision,
        RunnerSelector::named(runner_name).unwrap(),
        RunTrigger::Manual,
        Some("user_owner".to_string()),
    );
    runs_tests::enqueue(store, run, revision).await;
}

fn canary_revision(
    phase: RunnerProtocolCanaryPhase,
    image: &str,
    runner_name: &str,
) -> WorkflowRevision {
    WorkflowRevision::new(
        WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse(format!(
                "/.scope/runs/canary-{}.yml",
                match phase {
                    RunnerProtocolCanaryPhase::ColdWrite => "cold",
                    RunnerProtocolCanaryPhase::WarmRead => "warm",
                    RunnerProtocolCanaryPhase::Evict => "evict",
                }
            ))
            .unwrap(),
        )
        .unwrap(),
        CompiledWorkflow::new(
            phase.workflow_name(),
            WorkflowTriggers::new(true, false).unwrap(),
            RunnerSelector::named(runner_name).unwrap(),
            ContainerSpec::new(image).unwrap(),
            RUNNER_PROTOCOL_CANARY_TIMEOUT_SECONDS,
            vec![WorkflowCache::parse(RUNNER_PROTOCOL_CANARY_CACHE_NAME).unwrap()],
            vec![WorkflowStep::new(phase.step_name(), phase.step_command()).unwrap()],
        )
        .unwrap(),
    )
    .unwrap()
}
