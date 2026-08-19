mod auth;
mod config;
mod error;
mod gc;
mod handlers;

pub use config::Settings;

use auth::GrantVerifier;
use axum::{
    Router,
    routing::{get, post},
};
use scope_cache_contract::{
    COMMIT_CACHE_UPLOAD_PATH, PREPARE_CACHE_UPLOAD_PATH, RESTORE_CACHE_PATH,
};
use scope_object_store::{S3ObjectStore, S3Presigner};
use scope_postgres::db::MetadataStore;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    metadata: MetadataStore,
    object_store: Arc<S3ObjectStore>,
    presigner: Arc<S3Presigner>,
    verifier: Arc<GrantVerifier>,
    http: reqwest::Client,
    backend: Arc<str>,
}

impl AppState {
    pub async fn from_settings(settings: Settings) -> anyhow::Result<Self> {
        let metadata = MetadataStore::connect_worker(settings.database_url).await?;
        let object_store_settings = settings.object_store.clone();
        let object_store = Arc::new(
            tokio::task::spawn_blocking(move || S3ObjectStore::new(object_store_settings))
                .await??,
        );
        let presigner = Arc::new(S3Presigner::new(&settings.object_store));
        let verifier = Arc::new(GrantVerifier::new(
            &settings.grant_public_key_pem,
            settings.backend.clone(),
        )?);
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            metadata,
            object_store,
            presigner,
            verifier,
            http,
            backend: Arc::from(settings.backend),
        })
    }

    pub fn start_reconciler(&self) {
        gc::start(self.clone());
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route(RESTORE_CACHE_PATH, post(handlers::restore))
        .route(PREPARE_CACHE_UPLOAD_PATH, post(handlers::prepare_upload))
        .route(COMMIT_CACHE_UPLOAD_PATH, post(handlers::commit_upload))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
