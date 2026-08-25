use super::{definition::*, identity::*, observation::*};
use crate::runs::workflow::{definition::WorkflowJobId, identity::WorkflowPath};

fn workflow_namespace(path: &str, job: &str) -> CacheNamespace {
    CacheNamespace::workflow(
        &WorkflowPath::parse(path).unwrap(),
        &WorkflowJobId::parse(job).unwrap(),
    )
}

fn cache(name: &str) -> WorkflowCache {
    WorkflowCache::new(
        name,
        format!("/scope/cache/{name}"),
        "v1",
        CacheKeyInputs::default(),
        CacheKeyInputs::default(),
    )
    .unwrap()
}

fn identity(
    repository: &str,
    path: &str,
    job: &str,
    cache: WorkflowCache,
    group: char,
    exact: char,
) -> CacheIdentity {
    CacheIdentity::new(
        repository,
        workflow_namespace(path, job),
        cache,
        CachePlatform::LinuxAmd64,
        group.to_string().repeat(64),
        exact.to_string().repeat(64),
    )
    .unwrap()
}

#[test]
fn workflow_cache_names_and_mount_paths_are_validated() {
    let cache = cache("cargo");
    assert_eq!(cache.as_str(), "cargo");
    assert_eq!(cache.mount_path(), "/scope/cache/cargo");

    for invalid in [
        "",
        "Cargo",
        "cargo_target",
        "-cargo",
        "cargo-",
        "cargo--target",
        "scope-internal",
    ] {
        assert!(
            WorkflowCache::new(
                invalid,
                "/scope/cache/valid",
                "v1",
                Default::default(),
                Default::default()
            )
            .is_err(),
            "{invalid}"
        );
    }
    for invalid in [
        "",
        "relative",
        "/",
        "/cache/../escape",
        "/cache/./same",
        "/cache,readonly",
        "/cache/\"quoted\"",
        "/cache/nul\0byte",
        "/cache/new\nline",
        "/cache/carriage\rreturn",
        "/workspace/target",
        "/workspace/target/debug",
    ] {
        assert!(
            WorkflowCache::new(
                "cargo",
                invalid,
                "v1",
                Default::default(),
                Default::default()
            )
            .is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn identity_is_partitioned_by_every_semantic_component() {
    let workflow_cache = cache("cargo");
    let base = identity(
        "repo-1",
        "/.scope/runs/test.yml",
        "checks",
        workflow_cache.clone(),
        'a',
        'b',
    );
    let other_repo = identity(
        "repo-2",
        "/.scope/runs/test.yml",
        "checks",
        workflow_cache.clone(),
        'a',
        'b',
    );
    let other_cache = identity(
        "repo-1",
        "/.scope/runs/test.yml",
        "checks",
        cache("cargo-target"),
        'a',
        'b',
    );
    let other_workflow = identity(
        "repo-1",
        "/.scope/runs/release.yml",
        "checks",
        cache("cargo"),
        'a',
        'b',
    );
    let other_job = identity(
        "repo-1",
        "/.scope/runs/test.yml",
        "release",
        cache("cargo"),
        'a',
        'b',
    );
    let other_group = identity(
        "repo-1",
        "/.scope/runs/test.yml",
        "checks",
        workflow_cache.clone(),
        'c',
        'b',
    );
    let other_exact = identity(
        "repo-1",
        "/.scope/runs/test.yml",
        "checks",
        workflow_cache,
        'a',
        'c',
    );

    assert_eq!(base.exact_digest(), base.exact_digest());
    assert_eq!(base.exact_digest().len(), 64);
    assert_ne!(base.exact_digest(), other_repo.exact_digest());
    assert_ne!(base.exact_digest(), other_cache.exact_digest());
    assert_ne!(base.exact_digest(), other_workflow.exact_digest());
    assert_ne!(base.exact_digest(), other_job.exact_digest());
    assert_ne!(base.exact_digest(), other_group.exact_digest());
    assert_ne!(base.exact_digest(), other_exact.exact_digest());
    assert_eq!(
        base.compatibility_group_digest(),
        other_exact.compatibility_group_digest()
    );
    assert!(
        CacheIdentity::new(
            " ",
            workflow_namespace("/.scope/runs/test.yml", "checks"),
            cache("cargo"),
            CachePlatform::LinuxAmd64,
            "a".repeat(64),
            "b".repeat(64),
        )
        .is_err()
    );
}

#[test]
fn attempt_cache_observation_accepts_exact_retries_and_rejects_conflicts() {
    let cold_timing = AttemptCachePreparationTiming::new(7, 10, 0, 0, 0, 0, 17).unwrap();
    let mut observation = AttemptCacheObservation::prepared(
        "attempt-1",
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        WorkflowJobId::parse("checks").unwrap(),
        "cargo",
        "a".repeat(64),
        CachePreparation::Cold {
            reason: CacheColdReason::MetadataMissing,
        },
        cold_timing,
    )
    .unwrap();

    assert!(observation.finalize(CacheFinalState::Ready, 9).unwrap());
    assert!(!observation.finalize(CacheFinalState::Ready, 9).unwrap());
    assert!(observation.finalize(CacheFinalState::Evicted, 9).is_err());
    assert!(
        AttemptCacheObservation::prepared(
            "attempt-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
            WorkflowJobId::parse("checks").unwrap(),
            "cargo",
            "A".repeat(64),
            CachePreparation::Exact,
            AttemptCachePreparationTiming::new(1, 0, 1, 0, 0, 0, 1).unwrap(),
        )
        .is_err()
    );
}

#[test]
fn cache_timing_requires_truthful_phase_totals_and_setup_wall_time() {
    assert!(AttemptCachePreparationTiming::new(1, 2, 3, 4, 5, 6, 17).is_err());
    assert!(
        AttemptCachePreparationTiming::new(1, 2, MAX_CACHE_OBSERVATION_SIZE_BYTES + 1, 0, 0, 0, 3,)
            .is_err()
    );
    assert!(AttemptCacheSetupObservation::new("attempt-1", 5, 4).is_err());
    assert_eq!(
        AttemptCacheSetupObservation::new("attempt-1", 4, 5).unwrap(),
        AttemptCacheSetupObservation {
            attempt_id: "attempt-1".to_string(),
            authorization_ms: 4,
            wall_ms: 5,
        }
    );
}
