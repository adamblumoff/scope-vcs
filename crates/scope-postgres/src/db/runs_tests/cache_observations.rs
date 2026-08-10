use super::*;

#[tokio::test]
async fn cache_observation_reports_are_authenticated_and_exactly_idempotent() {
    let store = postgres_store();
    register_runner(&store, "runner-1", "linux-box").await;
    let cache = WorkflowCache::new("cargo", "/workspace/target").unwrap();
    let revision = WorkflowRevision::new(
        workflow_identity_for("owner/repo"),
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![],
                    RunnerSelector::Any,
                    ContainerSpec::new("rust:1.90").unwrap(),
                    20 * 60,
                    vec![cache.clone()],
                    vec![WorkflowStep::new("Test", "cargo test").unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    enqueue(
        &store,
        run_for_revision(
            "run-cache",
            "manual:cache",
            &revision,
            RunnerSelector::Any,
            RunTrigger::Manual,
            Some("user_owner".to_string()),
        ),
        revision,
    )
    .await;
    let token_hash = "a".repeat(64);
    store
        .runs()
        .claim_job(
            "run-cache",
            "checks",
            "runner-1",
            "attempt-cache",
            &token_hash,
            20,
            80,
        )
        .await
        .unwrap();
    pin_attempt(&store, "attempt-cache", "runner-1", &token_hash, 21).await;
    let image =
        PinnedContainerImage::parse(format!("registry.example/job@sha256:{}", "1".repeat(64)))
            .unwrap();
    let identity_digest = CacheIdentity::new(
        "owner/repo",
        CacheNamespace::workflow(
            &WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
            &WorkflowJobId::parse("checks").unwrap(),
        ),
        cache,
        &image,
        CachePlatform::LinuxAmd64,
    )
    .unwrap()
    .digest();
    let preparation = AttemptCachePreparationCommand {
        cache_name: "cargo".to_string(),
        identity_digest: identity_digest.clone(),
        preparation: CachePreparation::Cold {
            reason: CacheColdReason::MetadataMissing,
        },
        prepare_ms: 12,
    };

    assert_eq!(
        store
            .runs()
            .report_attempt_cache_preparations(
                "attempt-cache",
                &"b".repeat(64),
                vec![preparation.clone()],
                22,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Unauthenticated
    );

    let runs = store.runs();
    let first_retry = runs.report_attempt_cache_preparations(
        "attempt-cache",
        &token_hash,
        vec![preparation.clone()],
        22,
    );
    let second_retry = runs.report_attempt_cache_preparations(
        "attempt-cache",
        &token_hash,
        vec![preparation.clone()],
        22,
    );
    let (first_retry, second_retry) = tokio::join!(first_retry, second_retry);
    first_retry.unwrap();
    second_retry.unwrap();
    let mut conflicting_preparation = preparation;
    conflicting_preparation.prepare_ms = 13;
    assert_eq!(
        store
            .runs()
            .report_attempt_cache_preparations(
                "attempt-cache",
                &token_hash,
                vec![conflicting_preparation],
                22,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    store
        .runs()
        .abandon_attempt("attempt-cache", "runner-1", &token_hash, 23)
        .await
        .unwrap();
    let finalization = AttemptCacheFinalizationCommand {
        identity_digest: identity_digest.clone(),
        final_state: CacheFinalState::Evicted,
        finalize_ms: 8,
    };
    for _ in 0..2 {
        store
            .runs()
            .report_attempt_cache_finalizations(
                "attempt-cache",
                &token_hash,
                vec![finalization.clone()],
                24,
            )
            .await
            .unwrap();
    }
    store
        .runs()
        .report_attempt_cache_preparations(
            "attempt-cache",
            &token_hash,
            vec![AttemptCachePreparationCommand {
                cache_name: "cargo".to_string(),
                identity_digest: finalization.identity_digest.clone(),
                preparation: CachePreparation::Cold {
                    reason: CacheColdReason::MetadataMissing,
                },
                prepare_ms: 12,
            }],
            24,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .runs()
            .report_attempt_cache_finalizations(
                "attempt-cache",
                &token_hash,
                vec![AttemptCacheFinalizationCommand {
                    identity_digest: identity_digest.clone(),
                    final_state: CacheFinalState::Ready,
                    finalize_ms: 8,
                }],
                24,
            )
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    let detail = store.runs().run_detail("run-cache").await.unwrap().unwrap();
    assert_eq!(detail.attempts.len(), 1);
    assert_eq!(detail.attempts[0].caches.len(), 1);
    let cache = &detail.attempts[0].caches[0];
    assert_eq!(cache.attempt_id, "attempt-cache");
    assert_eq!(cache.cache_name, "cargo");
    assert_eq!(cache.identity_digest, identity_digest);
    assert_eq!(cache.final_state, CacheFinalState::Evicted);
    assert_eq!(cache.finalize_ms, Some(8));
}
