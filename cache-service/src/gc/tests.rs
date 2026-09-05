use super::*;
use crate::auth::GrantVerifier;
use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
};
use scope_object_store::{S3ObjectStore, S3ObjectStoreSettings, S3Presigner};
use scope_postgres::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Fixture {
    state: AppState,
    repository_id: String,
    now: u64,
    requests: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let requests = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let app = axum::Router::new().fallback(axum::routing::delete({
            let requests = requests.clone();
            let peak = peak.clone();
            let fail = fail.clone();
            move || {
                let requests = requests.clone();
                let active = active.clone();
                let peak = peak.clone();
                let fail = fail.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    requests.fetch_add(1, Ordering::SeqCst);
                    active.fetch_sub(1, Ordering::SeqCst);
                    if fail.load(Ordering::SeqCst) {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        axum::http::StatusCode::NO_CONTENT
                    }
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let metadata =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let owner = UserAccount {
            id: "user_gc".into(),
            handle: "cache-gc".into(),
            email: "gc@example.com".into(),
            email_verified: true,
        };
        let mut repository =
            Repository::new(&owner, "gc", Visibility::Private, "repoi_test").unwrap();
        repository.record.lifecycle_state = RepoLifecycleState::Ready;
        let repository_id = repository.record.id.clone();
        let mut catalog = CatalogFixture::default();
        catalog.users.insert(owner.id.clone(), owner);
        catalog
            .repositories
            .insert(repository_id.clone(), repository);
        metadata.admin().seed_catalog_for_tests(catalog).unwrap();
        let mut settings = S3ObjectStoreSettings::new(
            endpoint,
            "gc-test".into(),
            "local".into(),
            "access".into(),
            "secret".into(),
        );
        settings.force_path_style = true;
        let object_store = tokio::task::spawn_blocking({
            let settings = settings.clone();
            move || S3ObjectStore::new(settings).unwrap()
        })
        .await
        .unwrap();
        let state = AppState {
            metadata, object_store: Arc::new(object_store), presigner: Arc::new(S3Presigner::new(&settings)),
            verifier: Arc::new(GrantVerifier::new("-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n", "test-local".into()).unwrap()),
            http: reqwest::Client::new(), backend: Arc::from("test-local"),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            state,
            repository_id,
            now,
            requests,
            peak,
            fail,
            server,
        }
    }

    async fn upload(&self, index: usize, created_at: u64, committed: bool) {
        let digest = format!("{index:064x}");
        let upload_id = format!("upload-{index}");
        self.state
            .metadata
            .caches()
            .prepare_upload(
                &self.repository_id,
                &digest,
                &"f".repeat(64),
                &digest,
                1,
                "test-local",
                &upload_id,
                created_at,
            )
            .await
            .unwrap();
        if committed {
            self.state
                .metadata
                .caches()
                .commit_upload(&upload_id, created_at + 1)
                .await
                .unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn expired_uploads_drain_multiple_batches_with_bounded_deletions() {
    let fixture = Fixture::new().await;
    for index in 1..=201 {
        fixture.upload(index, fixture.now - 3600, false).await;
    }
    reconcile(&fixture.state).await.unwrap();
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 201);
    assert!((2..=DELETE_CONCURRENCY).contains(&fixture.peak.load(Ordering::SeqCst)));
    reconcile(&fixture.state).await.unwrap();
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 201);
    assert!(
        fixture
            .state
            .metadata
            .caches()
            .expire_uploads(fixture.now, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn failed_uploads_are_retried_on_the_next_sweep() {
    let fixture = Fixture::new().await;
    for index in 1..=101 {
        fixture.upload(index, fixture.now - 3600, false).await;
    }
    fixture.fail.store(true, Ordering::SeqCst);
    reconcile(&fixture.state).await.unwrap();
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 100);
    fixture.fail.store(false, Ordering::SeqCst);
    reconcile(&fixture.state).await.unwrap();
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 201);
    assert!(
        fixture
            .state
            .metadata
            .caches()
            .expire_uploads(fixture.now, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn object_deletions_drain_batches_without_shortening_reference_grace() {
    let fixture = Fixture::new().await;
    let old = fixture.now - 8 * 24 * 60 * 60;
    for index in 1..=201 {
        fixture.upload(index, old, true).await;
    }
    assert_eq!(
        fixture
            .state
            .metadata
            .caches()
            .expire_references(fixture.now - 3601, 1000)
            .await
            .unwrap(),
        201
    );
    // This additional reference only becomes expired during the sweep, retaining its grace period.
    fixture.upload(202, old, true).await;
    reconcile(&fixture.state).await.unwrap();
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 201);
    assert_eq!(
        fixture
            .state
            .metadata
            .caches()
            .expire_committed_uploads(fixture.now, 1000)
            .await
            .unwrap(),
        0
    );
    let after_sweep = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        fixture
            .state
            .metadata
            .caches()
            .claim_deletions(after_sweep + 3601, after_sweep + 7200, 1000)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "manual sustained cache expiry experiment"]
async fn measure_expired_upload_backlog() {
    let fixture = Fixture::new().await;
    for round in 0..3 {
        for item in 0..200 {
            fixture
                .upload(round * 200 + item + 1, fixture.now - 3600, false)
                .await;
        }
        let started = std::time::Instant::now();
        reconcile(&fixture.state).await.unwrap();
        eprintln!(
            "GC round {round}: created {}, deleted {}, remaining {}, elapsed {:?}",
            (round + 1) * 200,
            fixture.requests.load(Ordering::SeqCst),
            (round + 1) * 200 - fixture.requests.load(Ordering::SeqCst),
            started.elapsed()
        );
    }
}
