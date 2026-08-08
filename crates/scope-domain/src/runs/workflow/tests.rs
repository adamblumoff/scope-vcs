use super::*;

fn job(
    id: &str,
    needs: &[&str],
    caches: Vec<WorkflowCache>,
    steps: Vec<WorkflowStep>,
) -> WorkflowJob {
    WorkflowJob::new(
        WorkflowJobId::parse(id).unwrap(),
        needs
            .iter()
            .map(|need| WorkflowJobId::parse(*need).unwrap())
            .collect(),
        RunnerSelector::Any,
        ContainerSpec::new("rust:1.90").unwrap(),
        20 * 60,
        caches,
        steps,
    )
    .unwrap()
}

fn compiled_workflow() -> CompiledWorkflow {
    CompiledWorkflow::new(
        "Test",
        WorkflowTriggers::new(true, true).unwrap(),
        vec![job(
            "checks",
            &[],
            vec![
                WorkflowCache::parse("cargo-target").unwrap(),
                WorkflowCache::parse("cargo").unwrap(),
            ],
            vec![
                WorkflowStep::new("Format", "cargo fmt --check").unwrap(),
                WorkflowStep::new("Test", "cargo test --workspace").unwrap(),
            ],
        )],
    )
    .unwrap()
}

#[test]
fn workflow_paths_are_exact_and_stable() {
    let path = WorkflowPath::parse("/.scope/runs/test-api.yaml").unwrap();
    assert_eq!(path.name(), "test-api");
    for invalid in [
        ".scope/runs/test.yml",
        "/.scope/runs/Test.yml",
        "/.scope/runs/test.json",
        "/.scope/runs/nested/test.yml",
        "/.scope/runs/-test.yml",
        "/.scope/runs/test--api.yml",
        "/.scope/runs/test_.yml",
    ] {
        assert!(
            matches!(
                WorkflowPath::parse(invalid),
                Err(WorkflowError::InvalidPath)
            ),
            "{invalid} should be rejected"
        );
    }
}

#[test]
fn revisions_are_deterministic_and_identity_is_separate() {
    let definition = compiled_workflow();
    let left = WorkflowRevision::new(
        WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap(),
        definition.clone(),
    )
    .unwrap();
    let right = WorkflowRevision::new(
        WorkflowIdentity::new(
            "repo-2",
            WorkflowPath::parse("/.scope/runs/other.yml").unwrap(),
        )
        .unwrap(),
        definition,
    )
    .unwrap();

    assert_eq!(left.digest(), right.digest());
    assert_ne!(left.workflow(), right.workflow());
    assert_eq!(left.digest().len(), 64);
    assert_eq!(
        left.definition()
            .only_job()
            .unwrap()
            .caches()
            .iter()
            .map(WorkflowCache::as_str)
            .collect::<Vec<_>>(),
        ["cargo", "cargo-target"]
    );
}

#[test]
fn persisted_definitions_revalidate_invariants() {
    let definition = compiled_workflow();
    let json = serde_json::to_value(&definition).unwrap();
    assert_eq!(
        serde_json::from_value::<CompiledWorkflow>(json).unwrap(),
        definition
    );

    let mut invalid = serde_json::to_value(&definition).unwrap();
    invalid["jobs"][0]["timeout_seconds"] = serde_json::json!(0);
    assert!(serde_json::from_value::<CompiledWorkflow>(invalid).is_err());
}

#[test]
fn compiled_workflow_enforces_behavior_invariants() {
    let duplicate_steps = WorkflowJob::new(
        WorkflowJobId::parse("checks").unwrap(),
        vec![],
        RunnerSelector::named("linux-box").unwrap(),
        ContainerSpec::new("rust:1.90").unwrap(),
        60,
        vec![],
        vec![
            WorkflowStep::new("Test", "cargo test").unwrap(),
            WorkflowStep::new("Test", "cargo test --all").unwrap(),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_steps,
        WorkflowError::DuplicateStepName(name) if name == "Test"
    ));
    assert!(matches!(
        WorkflowTriggers::new(false, false),
        Err(WorkflowError::MissingTrigger)
    ));
    assert!(matches!(
        RunnerSelector::named("any"),
        Err(WorkflowError::InvalidRunnerName)
    ));
    assert!(matches!(
        WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            RunnerSelector::Named("any".to_string()),
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        ),
        Err(WorkflowError::InvalidRunnerName)
    ));

    let dependency = WorkflowJobId::parse("checks").unwrap();
    assert!(matches!(
        WorkflowJob::new(
            dependency.clone(),
            vec![dependency.clone()],
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        ),
        Err(WorkflowError::SelfDependency { job }) if job == "checks"
    ));
    assert!(matches!(
        WorkflowJob::new(
            WorkflowJobId::parse("web").unwrap(),
            vec![dependency.clone(), dependency],
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        ),
        Err(WorkflowError::DuplicateDependency { job, dependency })
            if job == "web" && dependency == "checks"
    ));

    let duplicate_job = job(
        "checks",
        &[],
        vec![],
        vec![WorkflowStep::new("Test", "cargo test").unwrap()],
    );
    assert!(matches!(
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![duplicate_job.clone(), duplicate_job],
        ),
        Err(WorkflowError::DuplicateJobId(id)) if id == "checks"
    ));
}

#[test]
fn cache_and_job_order_are_canonical_in_the_v3_digest() {
    let identity = || {
        WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap()
    };
    let definition = |caches| {
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![job(
                "checks",
                &[],
                caches,
                vec![WorkflowStep::new("Test", "cargo test").unwrap()],
            )],
        )
        .unwrap()
    };
    let cargo = WorkflowCache::parse("cargo").unwrap();
    let target = WorkflowCache::parse("cargo-target").unwrap();
    let left =
        WorkflowRevision::new(identity(), definition(vec![target.clone(), cargo.clone()])).unwrap();
    let right = WorkflowRevision::new(identity(), definition(vec![cargo.clone(), target])).unwrap();
    let without_cache = WorkflowRevision::new(identity(), definition(vec![])).unwrap();

    assert_eq!(left.digest(), right.digest());
    assert_ne!(left.digest(), without_cache.digest());
    assert!(matches!(
        WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            vec![cargo.clone(), cargo],
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        ),
        Err(WorkflowError::DuplicateCacheName(name)) if name == "cargo"
    ));
    let excessive = (0..=MAX_WORKFLOW_CACHES)
        .map(|index| WorkflowCache::parse(format!("cache-{index}")))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(matches!(
        WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            RunnerSelector::Any,
            ContainerSpec::new("rust:1.90").unwrap(),
            60,
            excessive,
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        ),
        Err(WorkflowError::TooManyCaches)
    ));

    let first = CompiledWorkflow::new(
        "Graph",
        WorkflowTriggers::new(true, false).unwrap(),
        vec![
            job(
                "web",
                &[],
                vec![],
                vec![WorkflowStep::new("Web", "true").unwrap()],
            ),
            job(
                "backend",
                &[],
                vec![],
                vec![WorkflowStep::new("Backend", "true").unwrap()],
            ),
        ],
    )
    .unwrap();
    let second = CompiledWorkflow::new(
        "Graph",
        WorkflowTriggers::new(true, false).unwrap(),
        first.jobs().iter().cloned().rev().collect(),
    )
    .unwrap();
    assert_eq!(
        WorkflowRevision::new(identity(), first).unwrap().digest(),
        WorkflowRevision::new(identity(), second).unwrap().digest()
    );
}

#[test]
fn job_graph_rejects_missing_dependencies_and_cycles() {
    let missing = CompiledWorkflow::new(
        "Graph",
        WorkflowTriggers::new(true, false).unwrap(),
        vec![job(
            "web",
            &["backend"],
            vec![],
            vec![WorkflowStep::new("Web", "true").unwrap()],
        )],
    )
    .unwrap_err();
    assert!(matches!(
        missing,
        WorkflowError::MissingDependency { job, dependency }
            if job == "web" && dependency == "backend"
    ));

    let cycle = CompiledWorkflow::new(
        "Graph",
        WorkflowTriggers::new(true, false).unwrap(),
        vec![
            job(
                "backend",
                &["web"],
                vec![],
                vec![WorkflowStep::new("Backend", "true").unwrap()],
            ),
            job(
                "web",
                &["backend"],
                vec![],
                vec![WorkflowStep::new("Web", "true").unwrap()],
            ),
        ],
    )
    .unwrap_err();
    assert!(matches!(cycle, WorkflowError::DependencyCycle));
}

#[test]
fn serial_jobs_use_a_deterministic_topological_order() {
    let workflow = CompiledWorkflow::new(
        "Graph",
        WorkflowTriggers::new(true, false).unwrap(),
        vec![
            job(
                "integration",
                &["web", "backend"],
                vec![],
                vec![WorkflowStep::new("Integration", "true").unwrap()],
            ),
            job(
                "web",
                &[],
                vec![],
                vec![WorkflowStep::new("Web", "true").unwrap()],
            ),
            job(
                "backend",
                &[],
                vec![],
                vec![WorkflowStep::new("Backend", "true").unwrap()],
            ),
        ],
    )
    .unwrap();

    assert_eq!(
        workflow
            .serial_jobs()
            .into_iter()
            .map(|job| job.id().as_str())
            .collect::<Vec<_>>(),
        ["backend", "web", "integration"]
    );
    assert!(workflow.only_job().is_none());
}

#[test]
fn persisted_flat_definitions_are_not_accepted() {
    let mut json = serde_json::to_value(compiled_workflow()).unwrap();
    let jobs = json.as_object_mut().unwrap().remove("jobs").unwrap();
    let job = &jobs[0];
    for field in ["runner", "container", "timeout_seconds", "caches", "steps"] {
        json[field] = job[field].clone();
    }
    assert!(serde_json::from_value::<CompiledWorkflow>(json).is_err());
}
