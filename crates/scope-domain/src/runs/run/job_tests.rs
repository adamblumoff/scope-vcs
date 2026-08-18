use super::*;
use crate::{
    content_ref::ContentRef,
    runs::{
        job::{create_run_jobs, reconcile_run, request_run_cancellation},
        workflow::{
            CompiledWorkflow, ContainerSpec, WorkflowIdentity, WorkflowJob, WorkflowJobId,
            WorkflowPath, WorkflowRevision, WorkflowStep, WorkflowTriggers,
        },
    },
    store::SourceBlob,
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
            ExecutionProvider::Northflank,
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
            ExecutionProvider::Northflank,
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
            ExecutionProvider::Northflank,
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
