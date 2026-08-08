use super::workflow_identity_for;
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
