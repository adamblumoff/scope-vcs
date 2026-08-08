use super::MetadataStore;
use scope_domain::runs::runner::{
    RUNNER_PROTOCOL_VERSION, Runner, RunnerCapabilities, RunnerGrant, RunnerMaxConcurrentJobs,
    RunnerName,
};

pub(in crate::db) async fn register_runner(store: &MetadataStore, id: &str, name: &str) {
    register_runner_with_capacity(store, id, name, 1).await;
}

pub(in crate::db) async fn register_runner_with_capacity(
    store: &MetadataStore,
    id: &str,
    name: &str,
    max_concurrent_jobs: u8,
) {
    let runner = runner_with_capacity(id, max_concurrent_jobs);
    store.runs().register_runner(runner.clone()).await.unwrap();
    store
        .runs()
        .grant_runner(
            RunnerGrant::new(
                "owner/repo",
                runner.id,
                RunnerName::parse(name).unwrap(),
                "user_owner",
                10,
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

pub(super) fn runner(id: &str) -> Runner {
    runner_with_capacity(id, 1)
}

pub(in crate::db) fn runner_with_capacity(id: &str, max_concurrent_jobs: u8) -> Runner {
    let hash_byte = if id.ends_with('1') { '1' } else { '2' };
    Runner::new(
        id,
        "user_owner",
        hash_byte.to_string().repeat(64),
        "1.0.0",
        RUNNER_PROTOCOL_VERSION,
        RunnerCapabilities::v1(),
        RunnerMaxConcurrentJobs::new(max_concurrent_jobs).unwrap(),
        10,
    )
    .unwrap()
}
