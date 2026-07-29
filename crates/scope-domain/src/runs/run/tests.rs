use super::*;
use crate::runs::{
    runner::{RUNNER_PROTOCOL_VERSION, RunnerCapabilities, RunnerName},
    workflow::WorkflowPath,
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
    Run::new(
        "run-1",
        "manual:repo-1:test:oid",
        WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap(),
        "b".repeat(64),
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
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
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
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
    attempt
        .complete(
            &mut run,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::Failed { exit_code: 1 },
            21,
        )
        .unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.pinned_container_image.is_none());
}

#[test]
fn claim_start_heartbeat_and_completion_preserve_attempt_identity() {
    let mut run = run();
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
    assert_eq!(run.state, RunState::Leased);
    assert!(run.pin_container_image(pinned_image('1'), 25).unwrap());

    attempt
        .start(&mut run, "runner-1", &"e".repeat(64), 30)
        .unwrap();
    assert_eq!(run.state, RunState::Running);
    assert!(
        !attempt
            .heartbeat(&run, "runner-1", &"e".repeat(64), 40, 100)
            .unwrap()
    );
    attempt
        .complete(
            &mut run,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::Succeeded,
            50,
        )
        .unwrap();
    assert_eq!(run.state, RunState::Succeeded);
    assert_eq!(attempt.state, AttemptState::Succeeded);
    let retry = attempt
        .complete(
            &mut run,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::Succeeded,
            60,
        )
        .unwrap_err();
    assert_eq!(retry.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn attempt_log_budget_is_cumulative_and_fail_closed() {
    let mut run = run();
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
    let chunk_text = "x".repeat(MAX_RUN_LOG_CHUNK_BYTES);
    for sequence in 1..=(MAX_RUN_LOG_BYTES_PER_ATTEMPT / MAX_RUN_LOG_CHUNK_BYTES as u64) {
        let chunk = RunLogChunk::new("attempt-1", sequence, chunk_text.clone(), 21).unwrap();
        assert!(attempt.accept_log_chunk(&chunk).unwrap());
    }
    assert_eq!(attempt.log_bytes, MAX_RUN_LOG_BYTES_PER_ATTEMPT);

    let extra = RunLogChunk::new("attempt-1", 161, "extra", 21).unwrap();
    assert!(!attempt.accept_log_chunk(&extra).unwrap());
    assert!(attempt.logs_truncated);
    assert!(!attempt.accept_log_chunk(&extra).unwrap());
    assert_eq!(attempt.log_bytes, MAX_RUN_LOG_BYTES_PER_ATTEMPT);

    assert!(RunLogChunk::new("attempt-1", 1, "x".repeat(MAX_RUN_LOG_CHUNK_BYTES + 1), 21).is_err());
}

#[test]
fn cancellation_waits_for_active_runner_acknowledgement() {
    let mut run = run();
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();

    assert!(run.request_cancellation(30).unwrap());
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
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::Canceled,
            50,
        )
        .unwrap();
    assert_eq!(run.state, RunState::Canceled);
}

#[test]
fn lease_loss_requeues_only_before_user_code_starts() {
    let mut queued = run();
    let mut leased = queued
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
    leased.expire(&mut queued, 80).unwrap();
    assert_eq!(queued.state, RunState::Queued);

    let mut running = run();
    let mut started = running
        .claim(&runner(), &grant(), "attempt-2", "f".repeat(64), 20, 80)
        .unwrap();
    running.pin_container_image(pinned_image('1'), 25).unwrap();
    started
        .start(&mut running, "runner-1", &"f".repeat(64), 30)
        .unwrap();
    started.expire(&mut running, 80).unwrap();
    assert_eq!(running.state, RunState::Lost);
}

#[test]
fn runner_restart_abandonment_uses_the_same_safe_recovery_boundary() {
    let mut run = run();
    let mut before_start = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
    before_start
        .abandon(&mut run, "runner-1", &"e".repeat(64), 30)
        .unwrap();
    assert_eq!(run.state, RunState::Queued);
    assert_eq!(before_start.state, AttemptState::Lost);

    let mut after_start = run
        .claim(&runner(), &grant(), "attempt-2", "f".repeat(64), 31, 90)
        .unwrap();
    run.pin_container_image(pinned_image('1'), 32).unwrap();
    after_start
        .start(&mut run, "runner-1", &"f".repeat(64), 33)
        .unwrap();
    after_start
        .abandon(&mut run, "runner-1", &"f".repeat(64), 34)
        .unwrap();
    assert_eq!(run.state, RunState::Lost);
    assert_eq!(after_start.state, AttemptState::Lost);
}

#[test]
fn stale_or_expired_attempt_credentials_cannot_mutate_a_run() {
    let mut run = run();
    let mut attempt = run
        .claim(&runner(), &grant(), "attempt-1", "e".repeat(64), 20, 80)
        .unwrap();
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
