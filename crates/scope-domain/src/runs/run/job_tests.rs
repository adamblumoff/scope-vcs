use super::*;
use crate::{
    content_ref::ContentRef,
    runs::{
        job::{create_run_jobs, reconcile_run, retry_run},
        runner::{RUNNER_PROTOCOL_VERSION, Runner, RunnerCapabilities, RunnerGrant, RunnerName},
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

    assert!(!run.cancellation_requested);
    assert_eq!(jobs[1].state, RunJobState::Skipped);
    assert_eq!(jobs[2].state, RunJobState::Skipped);
    assert_eq!(run.state, RunState::Canceled);
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
            22,
        )
        .unwrap();
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
