use super::*;
use crate::{
    content_ref::ContentRef,
    projection::ProjectionViewKey,
    runs::{
        runner::{RUNNER_PROTOCOL_VERSION, RunnerCapabilities, RunnerName},
        workflow::{CompiledWorkflow, ContainerSpec, WorkflowPath, WorkflowStep, WorkflowTriggers},
    },
    store::SourceBlob,
};

fn runner() -> Runner {
    Runner::new(
        "runner-1",
        "user-1",
        "a".repeat(64),
        "1.0.0",
        RUNNER_PROTOCOL_VERSION,
        RunnerCapabilities::v1(),
        1,
    )
    .unwrap()
}

fn run() -> Run {
    let revision = workflow_revision();
    Run::new(
        "run-1",
        "manual:repo-1:test:oid",
        WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user-1".to_string()),
        RunSource::ephemeral_git_bundle(SourceBlob {
            content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
            sha256: "c".repeat(64),
            git_oid: "d".repeat(40),
            git_file_mode: "100644".to_string(),
            size_bytes: 42,
        })
        .unwrap(),
        RunnerSelector::Any,
        10,
    )
    .unwrap()
}

fn workflow_revision() -> WorkflowRevision {
    WorkflowRevision::new(
        WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap(),
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            RunnerSelector::Any,
            ContainerSpec::new("rust:latest").unwrap(),
            600,
            vec![
                WorkflowStep::new("Format", "cargo fmt --check").unwrap(),
                WorkflowStep::new("Test", "cargo test").unwrap(),
                WorkflowStep::new("Build", "cargo build").unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn claim(
    run: &mut Run,
    attempt_id: &str,
    token: char,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> (RunAttempt, Vec<RunAttemptStep>) {
    run.claim(
        &runner(),
        &grant(),
        &workflow_revision(),
        attempt_id,
        token.to_string().repeat(64),
        now_unix,
        lease_expires_at_unix,
    )
    .unwrap()
}

fn grant() -> RunnerGrant {
    RunnerGrant::new(
        "repo-1",
        "runner-1",
        RunnerName::parse("linux-box").unwrap(),
        "user-1",
        1,
    )
    .unwrap()
}

fn pinned_image(digit: char) -> PinnedContainerImage {
    PinnedContainerImage::parse(format!(
        "registry.example/job@sha256:{}",
        digit.to_string().repeat(64)
    ))
    .unwrap()
}

#[test]
fn accepted_sources_pin_manifest_snapshot_audience_and_cutoff() {
    let manifest = SourceBlob {
        content_ref: ContentRef::git_manifest_sha256("a".repeat(64)),
        sha256: "a".repeat(64),
        git_oid: "b".repeat(40),
        git_file_mode: "100644".to_string(),
        size_bytes: 12,
    };
    let snapshot = SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
        sha256: "c".repeat(64),
        git_oid: "b".repeat(40),
        git_file_mode: "100644".to_string(),
        size_bytes: 34,
    };
    let source = RunSource::accepted_revision(
        7,
        manifest.clone(),
        snapshot.clone(),
        ProjectionViewKey::Private,
    )
    .unwrap();
    assert_eq!(source.git_oid(), "b".repeat(40));
    assert_eq!(source.digest(), "c".repeat(64));
    assert_eq!(source.retained_objects(), vec![&manifest, &snapshot]);
    assert!(source.is_private_only());

    assert!(
        RunSource::accepted_revision(
            0,
            manifest.clone(),
            snapshot.clone(),
            ProjectionViewKey::Private,
        )
        .is_err()
    );
    let mut wrong_head = snapshot;
    wrong_head.git_oid = "d".repeat(40);
    assert!(
        RunSource::accepted_revision(7, manifest, wrong_head, ProjectionViewKey::Public,).is_err()
    );
}

#[test]
fn image_pin_is_required_before_start_and_is_compare_and_swap() {
    let mut run = run();
    let (mut attempt, _) = claim(&mut run, "attempt-1", 'e', 20, 80);
    assert!(
        attempt
            .start(&mut run, "runner-1", &"e".repeat(64), 21)
            .is_err()
    );
    assert!(run.pin_container_image(pinned_image('1'), 21).unwrap());
    assert!(!run.pin_container_image(pinned_image('1'), 22).unwrap());
    assert!(run.pin_container_image(pinned_image('2'), 23).is_err());
    attempt
        .start(&mut run, "runner-1", &"e".repeat(64), 24)
        .unwrap();
    assert_eq!(
        run.pinned_container_image.as_ref().unwrap().as_str(),
        format!("registry.example/job@sha256:{}", "1".repeat(64))
    );
}

#[test]
fn setup_failure_can_finish_before_an_image_is_pinned() {
    let mut run = run();
    let (mut attempt, mut steps) = claim(&mut run, "attempt-1", 'e', 20, 80);
    attempt
        .complete(
            &mut run,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::SetupFailed {
                exit_code: 1,
                message: "container image could not start".to_string(),
            },
            21,
        )
        .unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.pinned_container_image.is_none());
    assert!(steps.iter().all(|step| step.state == StepState::Skipped));
    assert_eq!(
        attempt.terminal_reason,
        Some(AttemptTerminalReason::RunnerSetupFailed {
            exit_code: 1,
            message: "container image could not start".to_string(),
        })
    );
    let mut invalid_reason = attempt.clone();
    invalid_reason.terminal_reason = Some(AttemptTerminalReason::RunnerSetupFailed {
        exit_code: 0,
        message: "invalid".to_string(),
    });
    assert!(invalid_reason.validate_execution(&steps).is_err());
}

#[test]
fn steps_execute_in_order_and_final_success_concludes_the_run() {
    let mut run = run();
    let (mut attempt, mut steps) = claim(&mut run, "attempt-1", 'e', 20, 80);
    assert_eq!(run.state, RunState::Leased);
    assert!(run.pin_container_image(pinned_image('1'), 25).unwrap());

    assert!(
        !attempt
            .heartbeat(&run, "runner-1", &"e".repeat(64), 30, 100)
            .unwrap()
    );
    assert!(
        attempt
            .start_step(&mut run, &mut steps, "runner-1", &"e".repeat(64), 1, 31,)
            .is_err()
    );
    for index in 0..3 {
        attempt
            .start_step(
                &mut run,
                &mut steps,
                "runner-1",
                &"e".repeat(64),
                index,
                42 + u64::from(index) * 2,
            )
            .unwrap();
        if index == 2 {
            assert!(run.request_cancellation(47).unwrap());
        }
        attempt
            .complete_step(
                &mut run,
                &mut steps,
                "runner-1",
                &"e".repeat(64),
                index,
                StepConclusion::Succeeded,
                43 + u64::from(index) * 2,
            )
            .unwrap();
        if index == 0 {
            let mut canceled_run = run.clone();
            let mut canceled_attempt = attempt.clone();
            let mut canceled_steps = steps.clone();
            assert!(canceled_run.request_cancellation(44).unwrap());
            assert!(
                canceled_attempt
                    .start_step(
                        &mut canceled_run,
                        &mut canceled_steps,
                        "runner-1",
                        &"e".repeat(64),
                        1,
                        45,
                    )
                    .is_err()
            );
            assert_eq!(canceled_steps[1].state, StepState::Pending);
        }
    }
    assert_eq!(run.state, RunState::Succeeded);
    assert_eq!(attempt.state, AttemptState::Succeeded);
    assert!(!run.cancellation_requested);
    assert!(attempt.terminal_reason.is_none());
    assert!(steps.iter().all(|step| step.state == StepState::Succeeded));

    attempt
        .complete_step(
            &mut run,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            2,
            StepConclusion::Succeeded,
            101,
        )
        .unwrap();
    let conflicting = attempt
        .complete_step(
            &mut run,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            2,
            StepConclusion::Failed { exit_code: 1 },
            101,
        )
        .unwrap_err();
    assert_eq!(conflicting.kind, crate::error::DomainErrorKind::Conflict);

    let late_log = RunLogChunk::new("attempt-1", 2, 1, "late", 60).unwrap();
    let late_log = attempt.accept_log_chunk(&steps, &late_log).unwrap_err();
    assert_eq!(late_log.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn step_failure_skips_the_rest_and_records_a_typed_reason() {
    let mut run = run();
    let (mut attempt, mut steps) = claim(&mut run, "attempt-1", 'e', 20, 80);
    run.pin_container_image(pinned_image('1'), 21).unwrap();
    attempt
        .start(&mut run, "runner-1", &"e".repeat(64), 22)
        .unwrap();
    attempt
        .start_step(&mut run, &mut steps, "runner-1", &"e".repeat(64), 0, 23)
        .unwrap();
    assert!(run.request_cancellation(24).unwrap());
    attempt
        .complete_step(
            &mut run,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            0,
            StepConclusion::Failed { exit_code: 7 },
            25,
        )
        .unwrap();

    assert_eq!(attempt.state, AttemptState::Failed);
    assert_eq!(run.state, RunState::Failed);
    assert!(!run.cancellation_requested);
    assert_eq!(steps[0].state, StepState::Failed);
    assert!(
        steps[1..]
            .iter()
            .all(|step| step.state == StepState::Skipped)
    );
    assert_eq!(
        attempt.terminal_reason,
        Some(AttemptTerminalReason::StepFailed {
            step_index: 0,
            exit_code: 7,
        })
    );
    attempt.validate_execution(&steps).unwrap();

    let mut mismatched_reason = attempt.clone();
    mismatched_reason.terminal_reason = Some(AttemptTerminalReason::StepFailed {
        step_index: 0,
        exit_code: 8,
    });
    assert!(mismatched_reason.validate_execution(&steps).is_err());

    let mut unskipped_steps = steps.clone();
    unskipped_steps[1] = RunAttemptStep::pending("attempt-1", 1).unwrap();
    assert!(attempt.validate_execution(&unskipped_steps).is_err());

    run.last_attempt_number = MAX_RUN_ATTEMPTS;
    assert!(!run.can_retry());
    assert!(run.retry(26).is_err());
}

#[test]
fn attempt_log_budget_is_cumulative_and_logs_belong_to_the_running_step() {
    let mut run = run();
    let (mut attempt, mut steps) = claim(&mut run, "attempt-1", 'e', 20, 80);
    run.pin_container_image(pinned_image('1'), 21).unwrap();
    attempt
        .start(&mut run, "runner-1", &"e".repeat(64), 22)
        .unwrap();
    attempt
        .start_step(&mut run, &mut steps, "runner-1", &"e".repeat(64), 0, 23)
        .unwrap();
    let chunk_text = "x".repeat(MAX_RUN_LOG_CHUNK_BYTES);
    for sequence in 1..=(MAX_RUN_LOG_BYTES_PER_ATTEMPT / MAX_RUN_LOG_CHUNK_BYTES as u64) {
        let chunk = RunLogChunk::new("attempt-1", 0, sequence, chunk_text.clone(), 24).unwrap();
        assert!(attempt.accept_log_chunk(&steps, &chunk).unwrap());
    }
    assert_eq!(attempt.log_bytes, MAX_RUN_LOG_BYTES_PER_ATTEMPT);

    let wrong_step = RunLogChunk::new("attempt-1", 1, 161, "wrong", 24).unwrap();
    assert!(attempt.accept_log_chunk(&steps, &wrong_step).is_err());
    let extra = RunLogChunk::new("attempt-1", 0, 161, "extra", 24).unwrap();
    assert!(!attempt.accept_log_chunk(&steps, &extra).unwrap());
    assert!(attempt.logs_truncated);
    assert!(!attempt.accept_log_chunk(&steps, &extra).unwrap());
    assert_eq!(attempt.log_bytes, MAX_RUN_LOG_BYTES_PER_ATTEMPT);

    assert!(
        RunLogChunk::new(
            "attempt-1",
            0,
            1,
            "x".repeat(MAX_RUN_LOG_CHUNK_BYTES + 1),
            24,
        )
        .is_err()
    );
}

#[test]
fn cancellation_waits_for_active_runner_acknowledgement() {
    let mut run = run();
    let (mut attempt, mut steps) = claim(&mut run, "attempt-1", 'e', 20, 80);

    assert!(run.can_request_cancellation());
    assert!(!run.can_retry());
    assert!(run.request_cancellation(30).unwrap());
    assert!(!run.can_request_cancellation());
    assert_eq!(run.state, RunState::Leased);
    assert!(!run.request_cancellation(40).unwrap());
    assert_eq!(run.updated_at_unix, 30);
    assert!(
        attempt
            .start(&mut run, "runner-1", &"e".repeat(64), 40)
            .is_err()
    );
    assert_eq!(run.state, RunState::Leased);
    assert!(
        attempt
            .heartbeat(&run, "runner-1", &"e".repeat(64), 40, 100)
            .unwrap()
    );
    attempt
        .complete(
            &mut run,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::Canceled,
            50,
        )
        .unwrap();
    assert_eq!(run.state, RunState::Canceled);
    assert!(steps.iter().all(|step| step.state == StepState::Skipped));
    assert!(!run.can_request_cancellation());
    assert!(run.can_retry());
    run.retry(60).unwrap();
    assert!(run.can_request_cancellation());
    assert!(!run.can_retry());
}

#[test]
fn lease_loss_requeues_only_before_user_code_starts() {
    let mut queued = run();
    let (mut leased, mut leased_steps) = claim(&mut queued, "attempt-1", 'e', 20, 80);
    leased.expire(&mut queued, &mut leased_steps, 80).unwrap();
    assert_eq!(queued.state, RunState::Queued);
    assert!(
        leased_steps
            .iter()
            .all(|step| step.state == StepState::Skipped)
    );

    let mut running = run();
    let (mut started, mut started_steps) = claim(&mut running, "attempt-2", 'f', 20, 80);
    running.pin_container_image(pinned_image('1'), 25).unwrap();
    started
        .start(&mut running, "runner-1", &"f".repeat(64), 30)
        .unwrap();
    started
        .start_step(
            &mut running,
            &mut started_steps,
            "runner-1",
            &"f".repeat(64),
            0,
            31,
        )
        .unwrap();
    started
        .expire(&mut running, &mut started_steps, 80)
        .unwrap();
    assert_eq!(running.state, RunState::Lost);
    assert_eq!(started_steps[0].state, StepState::Lost);
    assert!(
        started_steps[1..]
            .iter()
            .all(|step| step.state == StepState::Skipped)
    );
}

#[test]
fn runner_restart_abandonment_uses_the_same_safe_recovery_boundary() {
    let mut run = run();
    let (mut before_start, mut before_steps) = claim(&mut run, "attempt-1", 'e', 20, 80);
    before_start
        .abandon(&mut run, &mut before_steps, "runner-1", &"e".repeat(64), 30)
        .unwrap();
    assert_eq!(run.state, RunState::Queued);
    assert_eq!(before_start.state, AttemptState::Lost);

    let (mut after_start, mut after_steps) = claim(&mut run, "attempt-2", 'f', 31, 90);
    run.pin_container_image(pinned_image('1'), 32).unwrap();
    after_start
        .start(&mut run, "runner-1", &"f".repeat(64), 33)
        .unwrap();
    after_start
        .abandon(&mut run, &mut after_steps, "runner-1", &"f".repeat(64), 34)
        .unwrap();
    assert_eq!(run.state, RunState::Lost);
    assert_eq!(after_start.state, AttemptState::Lost);
}

#[test]
fn stale_or_expired_attempt_credentials_cannot_mutate_a_run() {
    let mut run = run();
    let (mut attempt, _) = claim(&mut run, "attempt-1", 'e', 20, 80);
    assert!(
        attempt
            .start(&mut run, "other-runner", &"e".repeat(64), 30)
            .is_err()
    );
    assert!(
        attempt
            .start(&mut run, "runner-1", &"0".repeat(64), 30)
            .is_err()
    );
    assert!(
        attempt
            .start(&mut run, "runner-1", &"e".repeat(64), 80)
            .is_err()
    );
}
