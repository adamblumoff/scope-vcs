use super::workflow_identity_for;
use scope_domain::runs::resources::JobResources;
use scope_domain::runs::workflow::{
    CompiledWorkflow, ContainerSpec, RunnerSelector, WorkflowJob, WorkflowJobId, WorkflowRevision,
    WorkflowStep, WorkflowTriggers,
};

pub(in crate::db) fn revision_with_jobs(job_ids: &[&str]) -> WorkflowRevision {
    let job = |id: &str| {
        WorkflowJob::new(
            WorkflowJobId::parse(id).unwrap(),
            vec![],
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            scope_domain::runs::resources::JobResources::new(1_000, 1024 * 1024 * 1024).unwrap(),
            20 * 60,
            vec![],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        )
        .unwrap()
    };
    WorkflowRevision::new(
        workflow_identity_for("owner/repo"),
        CompiledWorkflow::new(
            "Parallel",
            WorkflowTriggers::new(true, false).unwrap(),
            job_ids.iter().map(|id| job(id)).collect(),
        )
        .unwrap(),
    )
    .unwrap()
}

pub(in crate::db) fn revision_with_resources(jobs: &[(&str, JobResources)]) -> WorkflowRevision {
    let jobs = jobs
        .iter()
        .map(|(id, resources)| {
            WorkflowJob::new(
                WorkflowJobId::parse(*id).unwrap(),
                vec![],
                RunnerSelector::Any,
                ContainerSpec::new("rust:1.90").unwrap(),
                *resources,
                20 * 60,
                vec![],
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            )
            .unwrap()
        })
        .collect();
    WorkflowRevision::new(
        workflow_identity_for("owner/repo"),
        CompiledWorkflow::new(
            "Resource requests",
            WorkflowTriggers::new(true, false).unwrap(),
            jobs,
        )
        .unwrap(),
    )
    .unwrap()
}
