use super::*;
use crate::{
    content_ref::ContentRef,
    runs::{
        job::{
            RunRunnerSummary, create_run_jobs, reconcile_run, request_run_cancellation, retry_run,
            summarize_run_runners,
        },
        resources::JobResources,
        runner::{
            RUNNER_PROTOCOL_VERSION, Runner, RunnerCapabilities, RunnerGrant,
            RunnerMaxConcurrentJobs, RunnerName,
        },
        workflow::{
            CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowPath,
            WorkflowStep, WorkflowTriggers,
        },
    },
    store::SourceBlob,
};

fn workflow(jobs: &[(&str, &[&str])]) -> WorkflowRevision {
    let identity = WorkflowIdentity::new(
        "repo-1",
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
    )
    .unwrap();
    let jobs = jobs
        .iter()
        .map(|(id, needs)| {
            WorkflowJob::new(
                WorkflowJobId::parse(*id).unwrap(),
                needs
                    .iter()
                    .map(|id| WorkflowJobId::parse(*id).unwrap())
                    .collect(),
                RunnerSelector::Any,
                ContainerSpec::new("rust:latest").unwrap(),
                JobResources::new(1_000, 1024 * 1024 * 1024).unwrap(),
                600,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            )
            .unwrap()
        })
        .collect();
    WorkflowRevision::new(
        identity,
        CompiledWorkflow::new("Test", WorkflowTriggers::new(true, false).unwrap(), jobs).unwrap(),
    )
    .unwrap()
}

fn run(revision: &WorkflowRevision) -> Run {
    Run::new(
        "run-1",
        "manual:test",
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user-1".into()),
        RunSource::ephemeral_git_bundle(SourceBlob {
            content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
            sha256: "c".repeat(64),
            git_oid: "d".repeat(40),
            git_file_mode: "100644".into(),
            size_bytes: 42,
        })
        .unwrap(),
        None,
        10,
    )
    .unwrap()
}

fn runner() -> Runner {
    Runner::new(
        "runner-1",
        "user-1",
        "a".repeat(64),
        "1.0.0",
        RUNNER_PROTOCOL_VERSION,
        RunnerCapabilities::v1(),
        RunnerMaxConcurrentJobs::new(1).unwrap(),
        1,
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

#[test]
fn roots_queue_and_dependents_promote_in_deterministic_topological_order() {
    let revision = workflow(&[
        ("build", &[]),
        ("lint", &[]),
        ("package", &["build", "lint"]),
    ]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    assert_eq!(jobs[0].state, RunJobState::Queued);
    assert_eq!(jobs[1].state, RunJobState::Queued);
    assert_eq!(jobs[2].state, RunJobState::Blocked);

    jobs[0].state = RunJobState::Succeeded;
    jobs[0].completed_at_unix = Some(20);
    jobs[0].updated_at_unix = 20;
    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();
    assert_eq!(jobs[2].state, RunJobState::Blocked);
    jobs[1].state = RunJobState::Succeeded;
    jobs[1].completed_at_unix = Some(21);
    jobs[1].updated_at_unix = 21;
    reconcile_run(&mut run, &mut jobs, &revision, 21).unwrap();
    assert_eq!(jobs[2].state, RunJobState::Queued);
    assert_eq!(run.state, RunState::Queued);
}

#[test]
fn completed_upstream_keeps_run_running_while_dependent_waits() {
    let revision = workflow(&[("build", &[]), ("package", &["build"])]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    run.state = RunState::Running;
    run.updated_at_unix = 20;
    jobs[0].state = RunJobState::Succeeded;
    jobs[0].updated_at_unix = 20;
    jobs[0].completed_at_unix = Some(20);

    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();

    assert_eq!(jobs[1].state, RunJobState::Queued);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.completed_at_unix, None);
}

#[test]
fn effective_runner_summary_distinguishes_any_named_and_mixed_jobs() {
    let revision = workflow(&[("build", &[]), ("lint", &[])]);
    let run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    assert_eq!(
        summarize_run_runners(&run, &jobs).unwrap(),
        RunRunnerSummary::Any
    );

    let named = RunnerSelector::named("linux-one").unwrap();
    jobs[0].desired_runner = named.clone();
    jobs[1].desired_runner = named;
    assert_eq!(
        summarize_run_runners(&run, &jobs).unwrap(),
        RunRunnerSummary::Named("linux-one".to_string())
    );

    jobs[1].desired_runner = RunnerSelector::named("linux-two").unwrap();
    assert_eq!(
        summarize_run_runners(&run, &jobs).unwrap(),
        RunRunnerSummary::Mixed
    );
}

#[test]
fn effective_runner_summary_rejects_missing_or_foreign_jobs() {
    let revision = workflow(&[("build", &[])]);
    let run = run(&revision);
    assert!(summarize_run_runners(&run, &[]).is_err());

    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    jobs[0].run_id = "run-2".to_string();
    assert!(summarize_run_runners(&run, &jobs).is_err());
}

#[test]
fn manual_any_override_cannot_widen_a_named_job_runner() {
    let identity = WorkflowIdentity::new(
        "repo-1",
        WorkflowPath::parse("/.scope/runs/named.yml").unwrap(),
    )
    .unwrap();
    let definition = CompiledWorkflow::new(
        "Named",
        WorkflowTriggers::new(true, false).unwrap(),
        vec![
            WorkflowJob::new(
                WorkflowJobId::parse("checks").unwrap(),
                vec![],
                RunnerSelector::named("linux-one").unwrap(),
                ContainerSpec::new("rust:latest").unwrap(),
                JobResources::new(1_000, 1024 * 1024 * 1024).unwrap(),
                600,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let revision = WorkflowRevision::new(identity, definition).unwrap();
    let template = run(&revision);
    let widened = Run::new(
        "run-widened",
        "manual:widened",
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user-1".into()),
        template.source,
        Some(RunnerSelector::Any),
        10,
    )
    .unwrap();

    assert!(widened.validate_workflow_revision(&revision).is_err());
}

#[test]
fn failure_skips_every_downstream_level_in_one_reconciliation() {
    let revision = workflow(&[
        ("build", &[]),
        ("package", &["build"]),
        ("publish", &["package"]),
    ]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    jobs[0].state = RunJobState::Failed;
    jobs[0].updated_at_unix = 20;
    jobs[0].completed_at_unix = Some(20);
    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();
    assert_eq!(jobs[1].state, RunJobState::Skipped);
    assert_eq!(jobs[2].state, RunJobState::Skipped);
    assert_eq!(run.state, RunState::Failed);
}

#[test]
fn canceled_upstream_cancels_run_and_skips_every_downstream_job() {
    let revision = workflow(&[
        ("build", &[]),
        ("package", &["build"]),
        ("publish", &["package"]),
    ]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    jobs[0].state = RunJobState::Canceled;
    jobs[0].updated_at_unix = 20;
    jobs[0].completed_at_unix = Some(20);

    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();

    assert!(run.cancellation_requested);
    assert_eq!(jobs[1].state, RunJobState::Skipped);
    assert_eq!(jobs[2].state, RunJobState::Skipped);
    assert_eq!(run.state, RunState::Canceled);
}

#[test]
fn canceled_job_terminalizes_non_active_siblings_but_preserves_active_work() {
    let revision = workflow(&[
        ("build", &[]),
        ("lint", &[]),
        ("docs", &[]),
        ("package", &["build"]),
    ]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    jobs[0].state = RunJobState::Canceled;
    jobs[0].updated_at_unix = 20;
    jobs[0].completed_at_unix = Some(20);
    jobs[1].state = RunJobState::Running;
    jobs[1].updated_at_unix = 30;

    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();

    assert!(run.cancellation_requested);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.updated_at_unix, 30);
    assert_eq!(jobs[1].state, RunJobState::Running);
    assert_eq!(jobs[1].updated_at_unix, 30);
    assert_eq!(jobs[2].state, RunJobState::Canceled);
    assert_eq!(jobs[3].state, RunJobState::Skipped);
}

#[test]
fn sibling_with_later_timestamp_does_not_reject_independent_reconciliation() {
    let revision = workflow(&[("build", &[]), ("lint", &[])]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    jobs[0].state = RunJobState::Succeeded;
    jobs[0].updated_at_unix = 20;
    jobs[0].completed_at_unix = Some(20);
    jobs[1].state = RunJobState::Running;
    jobs[1].updated_at_unix = 30;
    run.state = RunState::Running;
    run.updated_at_unix = 30;

    reconcile_run(&mut run, &mut jobs, &revision, 20).unwrap();

    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.updated_at_unix, 30);
    assert_eq!(jobs[1].updated_at_unix, 30);
}

#[test]
fn cancellation_uses_each_mutated_entitys_monotonic_time() {
    let revision = workflow(&[("build", &[]), ("lint", &[])]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    run.state = RunState::Running;
    run.updated_at_unix = 30;
    jobs[0].state = RunJobState::Running;
    jobs[0].updated_at_unix = 30;

    assert!(request_run_cancellation(&mut run, &mut jobs, 20).unwrap());

    assert!(run.cancellation_requested);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.updated_at_unix, 30);
    assert_eq!(jobs[0].state, RunJobState::Running);
    assert_eq!(jobs[0].updated_at_unix, 30);
    assert_eq!(jobs[1].state, RunJobState::Canceled);
    assert_eq!(jobs[1].updated_at_unix, 20);
}

#[test]
fn unacknowledged_cancellation_does_not_mask_terminal_job_outcomes() {
    let revision = workflow(&[("checks", &[])]);
    for (job_state, expected_run_state) in [
        (RunJobState::Succeeded, RunState::Succeeded),
        (RunJobState::Failed, RunState::Failed),
        (RunJobState::Lost, RunState::Lost),
    ] {
        let mut run = run(&revision);
        let mut jobs = create_run_jobs(&run, &revision).unwrap();
        run.state = RunState::Running;
        jobs[0].state = RunJobState::Running;

        assert!(request_run_cancellation(&mut run, &mut jobs, 20).unwrap());
        assert!(run.cancellation_requested);
        assert_eq!(jobs[0].state, RunJobState::Running);

        jobs[0].state = job_state;
        jobs[0].updated_at_unix = 30;
        jobs[0].completed_at_unix = Some(30);
        reconcile_run(&mut run, &mut jobs, &revision, 30).unwrap();

        assert_eq!(run.state, expected_run_state);
        assert!(!run.cancellation_requested);
    }
}

#[test]
fn identical_terminal_completion_is_idempotent_after_current_attempt_is_cleared() {
    let revision = workflow(&[("checks", &[])]);
    let run = run(&revision);
    let definition = &revision.definition().jobs()[0];
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, mut steps) = job
        .claim(
            &run,
            definition,
            &runner(),
            &grant(),
            "attempt-1",
            "e".repeat(64),
            20,
            80,
        )
        .unwrap();
    let conclusion = AttemptConclusion::SetupFailed {
        exit_code: 1,
        message: "container failed".into(),
    };
    attempt
        .complete(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            conclusion.clone(),
            false,
            21,
        )
        .unwrap();
    assert!(job.current_attempt_id.is_none());
    attempt
        .complete(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            conclusion,
            false,
            22,
        )
        .unwrap();
}

#[test]
fn truncation_records_the_step_that_exhausted_the_attempt_log_budget() {
    let revision = workflow(&[("checks", &[])]);
    let run = run(&revision);
    let definition = &revision.definition().jobs()[0];
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, mut steps) = job
        .claim(
            &run,
            definition,
            &runner(),
            &grant(),
            "attempt-1",
            "e".repeat(64),
            20,
            80,
        )
        .unwrap();
    steps.push(RunAttemptStep::pending("attempt-1", 1).unwrap());
    job.pin_container_image(
        PinnedContainerImage::parse(format!("alpine@sha256:{}", "a".repeat(64))).unwrap(),
        21,
    )
    .unwrap();
    attempt
        .start_step(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            0,
            22,
        )
        .unwrap();
    attempt.log_bytes = MAX_RUN_LOG_BYTES_PER_ATTEMPT - 1;
    let chunk = RunLogChunk::new("attempt-1", 0, 1, "too much", 22).unwrap();

    assert!(!attempt.accept_log_chunk(&steps, &chunk).unwrap());
    assert_eq!(attempt.first_truncated_step_index, Some(0));
    attempt
        .complete_step(
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            0,
            StepConclusion::Succeeded,
            true,
            23,
        )
        .unwrap();
    attempt
        .start_step(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            1,
            24,
        )
        .unwrap();
    attempt
        .complete_step(
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            1,
            StepConclusion::Succeeded,
            true,
            25,
        )
        .unwrap();
    assert_eq!(attempt.first_truncated_step_index, Some(0));
}

#[test]
fn interrupted_attempt_persists_active_step_log_truncation_before_completion() {
    let revision = workflow(&[("checks", &[])]);
    let run = run(&revision);
    let definition = &revision.definition().jobs()[0];
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, mut steps) = job
        .claim(
            &run,
            definition,
            &runner(),
            &grant(),
            "attempt-1",
            "e".repeat(64),
            20,
            80,
        )
        .unwrap();
    job.pin_container_image(
        PinnedContainerImage::parse(format!("alpine@sha256:{}", "a".repeat(64))).unwrap(),
        21,
    )
    .unwrap();
    attempt
        .start_step(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            0,
            22,
        )
        .unwrap();

    attempt
        .complete(
            &run,
            &mut job,
            &mut steps,
            "runner-1",
            &"e".repeat(64),
            AttemptConclusion::TimedOut,
            true,
            23,
        )
        .unwrap();

    assert_eq!(attempt.first_truncated_step_index, Some(0));
    assert_eq!(
        attempt.terminal_reason,
        Some(AttemptTerminalReason::TimedOut {
            step_index: Some(0),
        })
    );
}

#[test]
fn retry_rejects_revision_job_set_mismatch_and_backward_time() {
    let revision = workflow(&[("checks", &[])]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    run.state = RunState::Failed;
    run.updated_at_unix = 30;
    run.completed_at_unix = Some(30);
    jobs[0].state = RunJobState::Failed;
    jobs[0].updated_at_unix = 30;
    jobs[0].completed_at_unix = Some(30);
    assert!(retry_run(&mut run, &mut jobs, &revision, 29).is_err());

    let other = workflow(&[("build", &[])]);
    assert!(retry_run(&mut run, &mut jobs, &other, 31).is_err());

    jobs[0].key = WorkflowJobId::parse("missing").unwrap();
    assert!(retry_run(&mut run, &mut jobs, &revision, 31).is_err());
}

#[test]
fn retry_retains_each_jobs_pinned_container_image() {
    let revision = workflow(&[("build", &[]), ("package", &["build"])]);
    let mut run = run(&revision);
    let mut jobs = create_run_jobs(&run, &revision).unwrap();
    let build_image =
        PinnedContainerImage::parse(format!("registry.example/build@sha256:{}", "a".repeat(64)))
            .unwrap();
    let package_image = PinnedContainerImage::parse(format!(
        "registry.example/package@sha256:{}",
        "b".repeat(64)
    ))
    .unwrap();
    jobs[0].pinned_container_image = Some(build_image.clone());
    jobs[1].pinned_container_image = Some(package_image.clone());
    for job in &mut jobs {
        job.state = RunJobState::Failed;
        job.updated_at_unix = 20;
        job.completed_at_unix = Some(20);
    }
    run.state = RunState::Failed;
    run.updated_at_unix = 20;
    run.completed_at_unix = Some(20);

    retry_run(&mut run, &mut jobs, &revision, 30).unwrap();

    assert_eq!(jobs[0].pinned_container_image, Some(build_image));
    assert_eq!(jobs[1].pinned_container_image, Some(package_image));
    assert_eq!(jobs[0].state, RunJobState::Queued);
    assert_eq!(jobs[1].state, RunJobState::Blocked);
}
