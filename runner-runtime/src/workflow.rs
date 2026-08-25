use scope_domain::runs::{
    cache::definition::{CacheKeyInputs, WorkflowCache},
    workflow::definition::{ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep},
};

pub(crate) fn domain_workflow_job(
    job: &scope_api_contract::WorkflowJob,
) -> anyhow::Result<WorkflowJob> {
    let caches = job
        .caches
        .iter()
        .map(|cache| {
            WorkflowCache::new(
                cache.name.clone(),
                cache.path.clone(),
                cache.format.clone(),
                CacheKeyInputs::new(
                    cache.compatibility.files.clone(),
                    cache.compatibility.environment.clone(),
                    cache.compatibility.source,
                )?,
                CacheKeyInputs::new(
                    cache.exact.files.clone(),
                    cache.exact.environment.clone(),
                    cache.exact.source,
                )?,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    WorkflowJob::new(
        WorkflowJobId::parse(job.id.clone())?,
        job.needs
            .iter()
            .cloned()
            .map(WorkflowJobId::parse)
            .collect::<Result<Vec<_>, _>>()?,
        ContainerSpec::new(job.container.image.clone())?,
        job.timeout_seconds,
        caches,
        job.environment.clone(),
        job.steps
            .iter()
            .map(|step| WorkflowStep::new(step.name.clone(), step.run.clone()))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::{
        WorkflowCache as WireWorkflowCache, WorkflowCacheKeyInputs, WorkflowContainer,
        WorkflowJob as WireWorkflowJob, WorkflowStep as WireWorkflowStep,
    };
    use std::collections::BTreeMap;

    #[test]
    fn workflow_job_wire_data_is_validated_at_the_runner_edge() {
        let image = format!("ubuntu@sha256:{}", "01".repeat(32));
        let wire = WireWorkflowJob {
            id: "verify".to_string(),
            needs: vec!["build".to_string()],
            container: WorkflowContainer {
                image: image.clone(),
            },
            timeout_seconds: 600,
            caches: vec![WireWorkflowCache {
                name: "cargo".to_string(),
                path: "/cache/cargo".to_string(),
                format: "tar-zstd".to_string(),
                compatibility: WorkflowCacheKeyInputs {
                    files: vec!["Cargo.lock".to_string()],
                    environment: vec!["RUSTFLAGS".to_string()],
                    source: false,
                },
                exact: WorkflowCacheKeyInputs {
                    files: vec!["Cargo.lock".to_string()],
                    environment: vec!["RUSTFLAGS".to_string()],
                    source: true,
                },
            }],
            environment: BTreeMap::from([("RUSTFLAGS".to_string(), "-Dwarnings".to_string())]),
            steps: vec![WireWorkflowStep {
                name: "test".to_string(),
                run: "cargo test".to_string(),
            }],
        };

        let job = domain_workflow_job(&wire).unwrap();

        assert_eq!(job.id().as_str(), "verify");
        assert_eq!(job.needs()[0].as_str(), "build");
        assert_eq!(job.container().image(), image);
        assert_eq!(job.timeout_seconds(), 600);
        assert_eq!(job.caches()[0].as_str(), "cargo");
        assert_eq!(job.environment()["RUSTFLAGS"], "-Dwarnings");
        assert_eq!(job.steps()[0].name(), "test");
        assert_eq!(job.steps()[0].run(), "cargo test");

        let mut invalid = wire;
        invalid.id = "not valid".to_string();
        assert!(domain_workflow_job(&invalid).is_err());
    }
}
