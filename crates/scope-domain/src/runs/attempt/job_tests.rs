use super::*;
use crate::{
    content::SourceBlob,
    content_ref::ContentRef,
    runs::{
        job::{create_run_jobs, reconcile_run, request_run_cancellation},
        run::RunState,
        source::{RunSource, RunTrigger},
        step::StepConclusion,
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

fn workflow() -> WorkflowRevision {
    let identity = WorkflowIdentity::new(
        "repo-1",
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
        10,
    )
    .unwrap()
}

#[test]
fn cloud_dispatch_pins_the_workflow_image_and_rotates_the_bootstrap_credential() {
    let revision = workflow();
    let mut run = run(&revision);
    let definition = revision.definition().only_job().unwrap();
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, steps) = job
        .dispatch(
            &run,
            definition,
            "attempt-1",
            "b".repeat(64),
            "runtime-1",
            11,
            911,
        )
        .unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 11).unwrap();

    assert_eq!(job.state, RunJobState::Dispatching);
    assert_eq!(run.state, RunState::Dispatching);
    assert_eq!(job.pinned_container_image.digest(), IMAGE_DIGEST);
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt.authorize_cache_access(&job, 12).unwrap();
    assert!(
        attempt
            .claim_runtime(&job, &"b".repeat(64), "e".repeat(64), 13, 103)
            .is_err()
    );
    assert_eq!(steps.len(), 1);
}

#[test]
fn successful_attempt_finishes_only_after_runtime_finalization() {
    let revision = workflow();
    let mut run = run(&revision);
    let definition = revision.definition().only_job().unwrap();
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, mut steps) = job
        .dispatch(
            &run,
            definition,
            "attempt-1",
            "b".repeat(64),
            "runtime-1",
            11,
            911,
        )
        .unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 11).unwrap();
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt
        .start_step(&run, &mut job, &mut steps, &"a".repeat(64), 0, 13)
        .unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 13).unwrap();
    attempt
        .complete_step(
            &mut job,
            &mut steps,
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
            14,
        )
        .unwrap();

    assert_eq!(steps[0].state, StepState::Succeeded);
    assert_eq!(attempt.state, AttemptState::Running);
    assert_eq!(job.state, RunJobState::Running);

    let mut canceled_run = run.clone();
    let mut canceled_job = job.clone();
    let mut canceled_attempt = attempt.clone();
    let mut canceled_steps = steps.clone();
    request_run_cancellation(
        &mut canceled_run,
        std::slice::from_mut(&mut canceled_job),
        15,
    )
    .unwrap();
    canceled_attempt
        .complete(
            &canceled_run,
            &mut canceled_job,
            &mut canceled_steps,
            &"a".repeat(64),
            AttemptConclusion::Succeeded,
            false,
            16,
        )
        .unwrap();
    reconcile_run(
        &mut canceled_run,
        std::slice::from_mut(&mut canceled_job),
        &revision,
        16,
    )
    .unwrap();
    assert_eq!(canceled_attempt.state, AttemptState::Canceled);
    assert_eq!(canceled_job.state, RunJobState::Canceled);
    assert_eq!(canceled_run.state, RunState::Canceled);

    attempt
        .complete(
            &run,
            &mut job,
            &mut steps,
            &"a".repeat(64),
            AttemptConclusion::Succeeded,
            false,
            15,
        )
        .unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 15).unwrap();

    assert_eq!(attempt.state, AttemptState::Succeeded);
    assert_eq!(job.state, RunJobState::Succeeded);
    assert_eq!(run.state, RunState::Succeeded);
    assert!(attempt.authorize_cache_access(&job, 15).is_err());
}

#[test]
fn provider_confirmed_abort_terminalizes_a_running_attempt_as_canceled() {
    let revision = workflow();
    let mut run = run(&revision);
    let definition = revision.definition().only_job().unwrap();
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (mut attempt, mut steps) = job
        .dispatch(
            &run,
            definition,
            "attempt-1",
            "b".repeat(64),
            "runtime-1",
            11,
            911,
        )
        .unwrap();
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt
        .start_step(&run, &mut job, &mut steps, &"a".repeat(64), 0, 13)
        .unwrap();
    request_run_cancellation(&mut run, std::slice::from_mut(&mut job), 14).unwrap();

    attempt
        .confirm_provider_cancellation(&run, &mut job, &mut steps, 15)
        .unwrap();
    attempt
        .confirm_provider_cancellation(&run, &mut job, &mut steps, 15)
        .unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 15).unwrap();

    assert_eq!(attempt.state, AttemptState::Canceled);
    assert_eq!(job.state, RunJobState::Canceled);
    assert_eq!(run.state, RunState::Canceled);
    assert_eq!(steps[0].state, StepState::Canceled);
}

#[test]
fn maximum_attempt_age_expires_even_a_renewed_lease() {
    let revision = workflow();
    let run = run(&revision);
    let definition = revision.definition().only_job().unwrap();
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let created_at = 11;
    let (mut attempt, mut steps) = job
        .dispatch(
            &run,
            definition,
            "attempt-1",
            "b".repeat(64),
            "runtime-1",
            created_at,
            created_at + MAX_RUN_ATTEMPT_AGE_SECONDS + 100,
        )
        .unwrap();

    assert!(
        attempt
            .expire(
                &run,
                &mut job,
                &mut steps,
                created_at + MAX_RUN_ATTEMPT_AGE_SECONDS - 1,
            )
            .is_err()
    );
    attempt
        .expire(
            &run,
            &mut job,
            &mut steps,
            created_at + MAX_RUN_ATTEMPT_AGE_SECONDS,
        )
        .unwrap();

    assert_eq!(attempt.state, AttemptState::Lost);
    assert_eq!(job.state, RunJobState::Queued);
}

#[test]
fn final_pre_start_loss_is_terminal_but_penultimate_loss_requeues() {
    for number in [MAX_RUN_ATTEMPTS - 1, MAX_RUN_ATTEMPTS] {
        let revision = workflow();
        let mut run = run(&revision);
        let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
        job.last_attempt_number = number - 1;
        let (mut attempt, mut steps) = job
            .dispatch(
                &run,
                revision.definition().only_job().unwrap(),
                "last-attempt",
                "b".repeat(64),
                "runtime",
                11,
                12,
            )
            .unwrap();
        attempt.expire(&run, &mut job, &mut steps, 12).unwrap();
        reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 12).unwrap();
        attempt.validate_execution(&steps).unwrap();
        if number == MAX_RUN_ATTEMPTS {
            assert_eq!(job.state, RunJobState::Lost);
            assert_eq!(run.state, RunState::Lost);
            assert!(!crate::runs::job::can_retry_run(
                &run,
                std::slice::from_ref(&job)
            ));
            assert_eq!(job.completed_at_unix, Some(12));
            assert_eq!(
                attempt.terminal_reason,
                Some(AttemptTerminalReason::DispatchAttemptsExhausted)
            );
        } else {
            assert_eq!(job.state, RunJobState::Queued);
            assert_eq!(run.state, RunState::Queued);
            assert_eq!(job.completed_at_unix, None);
            assert_eq!(
                attempt.terminal_reason,
                Some(AttemptTerminalReason::ExecutionLost { step_index: None })
            );
        }
        assert_eq!(job.current_attempt_id, None);
    }
}

#[test]
fn cancellation_wins_over_pre_start_attempt_exhaustion() {
    let revision = workflow();
    let mut run = run(&revision);
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    job.last_attempt_number = MAX_RUN_ATTEMPTS - 1;
    let (mut attempt, mut steps) = job
        .dispatch(
            &run,
            revision.definition().only_job().unwrap(),
            "last-attempt",
            "b".repeat(64),
            "runtime",
            11,
            12,
        )
        .unwrap();
    request_run_cancellation(&mut run, std::slice::from_mut(&mut job), 11).unwrap();
    attempt.expire(&run, &mut job, &mut steps, 12).unwrap();
    reconcile_run(&mut run, std::slice::from_mut(&mut job), &revision, 12).unwrap();
    assert_eq!(job.state, RunJobState::Canceled);
    assert_eq!(run.state, RunState::Canceled);
    assert!(!crate::runs::job::can_retry_run(&run, &[job]));
}
