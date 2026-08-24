use crate::{AppState, auth::require_cache, error::ServiceError};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use scope_cache_contract::{
    CommitCacheUploadRequest, CommitCacheUploadResponse, PrepareCacheUploadRequest,
    PrepareCacheUploadResponse, RestoreCacheRequest, RestoreCacheResponse, SignedCacheGrantClaims,
};
use scope_cache_domain::{CacheDigest, UploadLeaseId};
use scope_object_store::ObjectStore;
use scope_postgres::db::{CacheCommitResult, CachePrepareResult};
use std::{collections::BTreeMap, time::Duration};

const SIGNED_URL_TTL_SECONDS: u32 = 15 * 60;

pub(crate) async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

pub(crate) async fn readyz(State(state): State<AppState>) -> Result<StatusCode, ServiceError> {
    state.metadata.admin().readiness_check().await?;
    let store = state.object_store.clone();
    tokio::task::spawn_blocking(move || store.readiness_check())
        .await
        .map_err(|error| ServiceError::internal(error.to_string()))?
        .map_err(|error| ServiceError::unavailable(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RestoreCacheRequest>,
) -> Result<Json<RestoreCacheResponse>, ServiceError> {
    let now = unix_now()?;
    let claims = authorize_grant(&state, &headers, now).await?;
    require_cache(
        &claims,
        &request.exact_identity_digest,
        &request.compatibility_group_digest,
        now,
    )?;
    let signed_url_ttl = signed_url_ttl(&claims, now)?;
    let object = state
        .metadata
        .caches()
        .restore(
            claims.repository_id.as_str(),
            request.exact_identity_digest.as_str(),
            request.compatibility_group_digest.as_str(),
            now,
        )
        .await?;
    let Some(object) = object else {
        return Ok(Json(RestoreCacheResponse::Miss));
    };
    if object.object.storage_backend != state.backend.as_ref() {
        return Err(ServiceError::internal(
            "cache reference targets a different storage backend",
        ));
    }
    let url = state
        .presigner
        .presign("GET", &object.object.object_key, signed_url_ttl)
        .map_err(|error| ServiceError::internal(error.to_string()))?;
    Ok(Json(RestoreCacheResponse::Hit {
        source: match object.source {
            scope_postgres::db::CacheRestoreKind::Exact => {
                scope_cache_contract::CacheRestoreSource::Exact
            }
            scope_postgres::db::CacheRestoreKind::Compatible => {
                scope_cache_contract::CacheRestoreSource::Compatible
            }
        },
        object_digest: CacheDigest::parse(object.object.checksum_sha256)?,
        size_bytes: object.object.size_bytes,
        download_url: url,
        expires_at_unix: checked_add(now, u64::from(signed_url_ttl))?,
    }))
}

pub(crate) async fn prepare_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PrepareCacheUploadRequest>,
) -> Result<Json<PrepareCacheUploadResponse>, ServiceError> {
    let now = unix_now()?;
    let claims = authorize_grant(&state, &headers, now).await?;
    require_cache(
        &claims,
        &request.exact_identity_digest,
        &request.compatibility_group_digest,
        now,
    )?;
    let signed_url_ttl = signed_url_ttl(&claims, now)?;
    let upload_id = UploadLeaseId::parse(random_upload_id()?)?;
    let result = state
        .metadata
        .caches()
        .prepare_upload(
            claims.repository_id.as_str(),
            request.exact_identity_digest.as_str(),
            request.compatibility_group_digest.as_str(),
            request.object_digest.as_str(),
            request.size_bytes,
            state.backend.as_ref(),
            upload_id.as_str(),
            now,
        )
        .await?;
    match result {
        CachePrepareResult::UseObject {
            object,
            expires_at_unix,
            ..
        } => Ok(Json(PrepareCacheUploadResponse::UseObject {
            object_digest: CacheDigest::parse(object.checksum_sha256)?,
            expires_at_unix,
        })),
        CachePrepareResult::Upload(upload) => {
            let signed = state
                .presigner
                .presign_checksum_bound_put(
                    &upload.object_key,
                    signed_url_ttl,
                    &upload.checksum_sha256,
                    upload.size_bytes,
                )
                .map_err(|error| ServiceError::internal(error.to_string()))?;
            Ok(Json(PrepareCacheUploadResponse::Upload {
                lease_id: UploadLeaseId::parse(upload.upload_id)?,
                upload_url: signed.url,
                upload_headers: BTreeMap::from_iter(signed.headers),
                expires_at_unix: upload.expires_at_unix,
            }))
        }
    }
}

pub(crate) async fn commit_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CommitCacheUploadRequest>,
) -> Result<Json<CommitCacheUploadResponse>, ServiceError> {
    let now = unix_now()?;
    let claims = authorize_grant(&state, &headers, now).await?;
    let upload = state
        .metadata
        .caches()
        .upload(request.lease_id.as_str())
        .await?;
    let identity = CacheDigest::parse(upload.identity_digest.clone())?;
    let compatibility_group = CacheDigest::parse(upload.compatibility_group_digest.clone())?;
    require_cache(&claims, &identity, &compatibility_group, now)?;
    if claims.repository_id.as_str() != upload.repository_id
        || upload.storage_backend != state.backend.as_ref()
        || request.object_digest.as_str() != upload.checksum_sha256
        || request.size_bytes != upload.size_bytes
    {
        return Err(ServiceError::forbidden(
            "cache upload does not belong to this grant or content claim",
        ));
    }
    verify_uploaded_object(&state, &upload).await?;
    let result = state
        .metadata
        .caches()
        .commit_upload(request.lease_id.as_str(), now)
        .await?;
    let (object, expires_at_unix) = match result {
        CacheCommitResult::Committed {
            object,
            expires_at_unix,
            ..
        }
        | CacheCommitResult::AlreadyCommitted {
            object,
            expires_at_unix,
            ..
        } => (object, expires_at_unix),
        CacheCommitResult::Stale {
            orphaned_object_key,
        } => {
            let store = state.object_store.clone();
            match tokio::task::spawn_blocking(move || store.delete(&orphaned_object_key)).await {
                Ok(Ok(())) => {
                    state
                        .metadata
                        .caches()
                        .complete_upload_cleanup(request.lease_id.as_str())
                        .await?;
                }
                Ok(Err(error)) => {
                    state
                        .metadata
                        .caches()
                        .retry_upload_cleanup(request.lease_id.as_str())
                        .await?;
                    tracing::warn!(%error, "stale cache upload deletion will be retried");
                }
                Err(error) => {
                    state
                        .metadata
                        .caches()
                        .retry_upload_cleanup(request.lease_id.as_str())
                        .await?;
                    tracing::warn!(%error, "stale cache upload deletion task failed");
                }
            }
            return Err(ServiceError::conflict("cache upload lease is stale"));
        }
    };
    Ok(Json(CommitCacheUploadResponse {
        exact_identity_digest: identity,
        object_digest: CacheDigest::parse(object.checksum_sha256)?,
        expires_at_unix,
    }))
}

async fn verify_uploaded_object(
    state: &AppState,
    upload: &scope_postgres::db::CacheUploadRecord,
) -> Result<(), ServiceError> {
    let url = state
        .presigner
        .presign("HEAD", &upload.object_key, SIGNED_URL_TTL_SECONDS)
        .map_err(|error| ServiceError::internal(error.to_string()))?;
    let response = state
        .http
        .head(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| ServiceError::unavailable(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ServiceError::conflict(format!(
            "uploaded cache object is unavailable ({})",
            response.status()
        )));
    }
    let uploaded_size = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if uploaded_size != Some(upload.size_bytes) {
        return Err(ServiceError::conflict(format!(
            "uploaded cache object size does not match its lease (expected {}, received {uploaded_size:?})",
            upload.size_bytes
        )));
    }
    let checksum = response
        .headers()
        .get("x-amz-meta-scope-sha256")
        .and_then(|value| value.to_str().ok());
    if checksum != Some(upload.checksum_sha256.as_str()) {
        return Err(ServiceError::conflict(
            "uploaded cache object checksum metadata does not match its lease",
        ));
    }
    Ok(())
}

fn random_upload_id() -> Result<String, ServiceError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| ServiceError::internal(error.to_string()))?;
    Ok(format!("cache_upload_{}", hex::encode(bytes)))
}

fn unix_now() -> Result<u64, ServiceError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| ServiceError::internal(error.to_string()))
}

fn checked_add(now: u64, seconds: u64) -> Result<u64, ServiceError> {
    now.checked_add(seconds)
        .ok_or_else(|| ServiceError::internal("cache timestamp overflow"))
}

async fn authorize_grant(
    state: &AppState,
    headers: &HeaderMap,
    now_unix: u64,
) -> Result<SignedCacheGrantClaims, ServiceError> {
    let claims = state.verifier.verify(headers, now_unix)?;
    if !state
        .metadata
        .runs()
        .authorize_cache_grant(&claims.attempt_id, claims.repository_id.as_str(), now_unix)
        .await?
    {
        return Err(ServiceError::unauthorized(
            "cache grant is no longer attached to an active attempt",
        ));
    }
    Ok(claims)
}

fn signed_url_ttl(claims: &SignedCacheGrantClaims, now_unix: u64) -> Result<u32, ServiceError> {
    let remaining = claims
        .expires_at_unix
        .checked_sub(now_unix)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(|| ServiceError::unauthorized("cache grant is expired"))?;
    Ok(
        u32::try_from(remaining.min(u64::from(SIGNED_URL_TTL_SECONDS)))
            .expect("signed URL TTL is capped to a u32 constant"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppState, GrantVerifier, router};
    use axum::{
        body::{Body, to_bytes},
        extract::State,
        http::{Method, Request, Response, StatusCode, header::AUTHORIZATION},
        routing::any,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use scope_cache_contract::{
        AuthorizedCache, COMMIT_CACHE_UPLOAD_PATH, PREPARE_CACHE_UPLOAD_PATH, RESTORE_CACHE_PATH,
        SignedCacheGrantClaims,
    };
    use scope_cache_domain::RepositoryId;
    use scope_domain::{
        content_ref::ContentRef,
        policy::Visibility,
        runs::{
            run::{Run, RunSource, RunTrigger},
            workflow::{
                CompiledWorkflow, ContainerSpec, WorkflowIdentity, WorkflowJob, WorkflowJobId,
                WorkflowPath, WorkflowRevision, WorkflowStep, WorkflowTriggers,
            },
        },
        store::{RepoLifecycleState, StoredRepository, UserAccount},
    };
    use scope_object_store::{S3ObjectStore, S3ObjectStoreSettings, S3Presigner};
    use scope_postgres::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
    use sha2::{Digest as _, Sha256};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt as _;

    #[test]
    fn signed_urls_never_outlive_the_attempt_grant() {
        let claims = SignedCacheGrantClaims {
            attempt_id: "attempt-1".to_string(),
            repository_id: RepositoryId::parse("repo-1").unwrap(),
            allowed_caches: vec![],
            backend: "test-local".to_string(),
            expires_at_unix: 100,
        };
        assert_eq!(signed_url_ttl(&claims, 50).unwrap(), 50);
        assert!(signed_url_ttl(&claims, 100).is_err());

        let long_grant = SignedCacheGrantClaims {
            expires_at_unix: 2_000,
            ..claims
        };
        assert_eq!(
            signed_url_ttl(&long_grant, 0).unwrap(),
            SIGNED_URL_TTL_SECONDS
        );
    }

    #[tokio::test]
    async fn real_service_round_trip_is_cold_then_warm_then_unchanged() {
        let endpoint = start_object_store().await;
        let metadata = MetadataStore::connect_fresh_for_tests(
            &TestDatabaseTarget::required().expect("test database target"),
        )
        .expect("connect test metadata");
        let repository_id = seed_repository(&metadata);
        let (attempt_id, attempt_token_hash) = seed_active_attempt(&metadata, &repository_id).await;
        let terminal_metadata = metadata.clone();
        let identity = CacheDigest::parse("1".repeat(64)).unwrap();
        let group = CacheDigest::parse("2".repeat(64)).unwrap();
        let object_bytes = b"real cache-service round trip".to_vec();
        let object_digest = CacheDigest::parse(hex::encode(Sha256::digest(&object_bytes))).unwrap();
        let mut settings = S3ObjectStoreSettings::new(
            endpoint,
            "scope-cache-e2e".to_string(),
            "us-east-1".to_string(),
            "minioadmin".to_string(),
            "minioadmin".to_string(),
        );
        settings.force_path_style = true;
        let object_store = tokio::task::spawn_blocking({
            let settings = settings.clone();
            move || S3ObjectStore::new(settings)
        })
        .await
        .unwrap()
        .unwrap();
        let state = AppState {
            metadata,
            object_store: Arc::new(object_store),
            presigner: Arc::new(S3Presigner::new(&settings)),
            verifier: Arc::new(
                GrantVerifier::new(TEST_PUBLIC_KEY, "test-local".to_string()).unwrap(),
            ),
            http: reqwest::Client::new(),
            backend: Arc::from("test-local"),
        };
        let token = grant(&attempt_id, &repository_id, identity.clone(), group.clone());
        let app = router(state);

        let first = post_json(
            &app,
            PREPARE_CACHE_UPLOAD_PATH,
            &token,
            &PrepareCacheUploadRequest {
                exact_identity_digest: identity.clone(),
                compatibility_group_digest: group.clone(),
                object_digest: object_digest.clone(),
                size_bytes: object_bytes.len() as u64,
            },
        )
        .await;
        let PrepareCacheUploadResponse::Upload {
            lease_id,
            upload_url,
            upload_headers,
            ..
        } = serde_json::from_slice(&first).unwrap()
        else {
            panic!("first prepare must upload");
        };
        let mut put = reqwest::Client::new()
            .put(upload_url)
            .header(reqwest::header::CONTENT_LENGTH, object_bytes.len())
            .body(object_bytes.clone());
        for (name, value) in upload_headers {
            put = put.header(name, value);
        }
        let put_response = put.send().await.unwrap();
        assert!(put_response.status().is_success(), "{put_response:?}");

        let commit_request = CommitCacheUploadRequest {
            lease_id,
            object_digest: object_digest.clone(),
            size_bytes: object_bytes.len() as u64,
        };
        post_json(&app, COMMIT_CACHE_UPLOAD_PATH, &token, &commit_request).await;
        post_json(&app, COMMIT_CACHE_UPLOAD_PATH, &token, &commit_request).await;
        let restored = post_json(
            &app,
            RESTORE_CACHE_PATH,
            &token,
            &RestoreCacheRequest {
                exact_identity_digest: identity.clone(),
                compatibility_group_digest: group.clone(),
            },
        )
        .await;
        let RestoreCacheResponse::Hit { download_url, .. } =
            serde_json::from_slice(&restored).unwrap()
        else {
            panic!("committed cache must restore");
        };
        assert_eq!(
            reqwest::get(download_url)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            object_bytes
        );

        let unchanged = post_json(
            &app,
            PREPARE_CACHE_UPLOAD_PATH,
            &token,
            &PrepareCacheUploadRequest {
                exact_identity_digest: identity,
                compatibility_group_digest: group.clone(),
                object_digest,
                size_bytes: object_bytes.len() as u64,
            },
        )
        .await;
        assert!(matches!(
            serde_json::from_slice(&unchanged).unwrap(),
            PrepareCacheUploadResponse::UseObject { .. }
        ));

        terminal_metadata
            .runs()
            .abandon_attempt(&attempt_id, &attempt_token_hash, unix_now().unwrap())
            .await
            .unwrap();
        let rejected = app
            .oneshot(
                Request::post(RESTORE_CACHE_PATH)
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&RestoreCacheRequest {
                            exact_identity_digest: CacheDigest::parse("1".repeat(64)).unwrap(),
                            compatibility_group_digest: group,
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    #[derive(Clone, Default)]
    struct TestObjectStore {
        object: Arc<Mutex<Option<TestObject>>>,
    }

    struct TestObject {
        bytes: Vec<u8>,
        checksum: String,
    }

    async fn start_object_store() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .fallback(any(test_object_request))
            .with_state(TestObjectStore::default());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    async fn test_object_request(
        State(store): State<TestObjectStore>,
        request: Request<Body>,
    ) -> Response<Body> {
        match *request.method() {
            Method::PUT => {
                let checksum = request
                    .headers()
                    .get("x-amz-meta-scope-sha256")
                    .and_then(|value| value.to_str().ok())
                    .unwrap()
                    .to_string();
                let checksum_base64 = request
                    .headers()
                    .get("x-amz-checksum-sha256")
                    .and_then(|value| value.to_str().ok())
                    .unwrap()
                    .to_string();
                let bytes = to_bytes(request.into_body(), 1024 * 1024)
                    .await
                    .unwrap()
                    .to_vec();
                if checksum != hex::encode(Sha256::digest(&bytes))
                    || checksum_base64 != BASE64.encode(Sha256::digest(&bytes))
                {
                    return Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::empty())
                        .unwrap();
                }
                *store.object.lock().unwrap() = Some(TestObject { bytes, checksum });
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::empty())
                    .unwrap()
            }
            Method::HEAD => {
                let object = store.object.lock().unwrap();
                let Some(object) = object.as_ref() else {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Body::empty())
                        .unwrap();
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .header(reqwest::header::CONTENT_LENGTH, object.bytes.len())
                    .header("x-amz-meta-scope-sha256", &object.checksum)
                    .body(Body::empty())
                    .unwrap()
            }
            Method::GET => {
                let object = store.object.lock().unwrap();
                let Some(object) = object.as_ref() else {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Body::empty())
                        .unwrap();
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .header(reqwest::header::CONTENT_LENGTH, object.bytes.len())
                    .body(Body::from(object.bytes.clone()))
                    .unwrap()
            }
            _ => Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Body::empty())
                .unwrap(),
        }
    }

    async fn post_json<T: serde::Serialize>(
        app: &axum::Router,
        path: &str,
        token: &str,
        body: &T,
    ) -> Vec<u8> {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(
            status.is_success(),
            "{status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        bytes.to_vec()
    }

    fn grant(
        attempt_id: &str,
        repository_id: &str,
        identity: CacheDigest,
        group: CacheDigest,
    ) -> String {
        encode(
            &Header::new(Algorithm::EdDSA),
            &SignedCacheGrantClaims {
                attempt_id: attempt_id.to_string(),
                repository_id: RepositoryId::parse(repository_id).unwrap(),
                allowed_caches: vec![AuthorizedCache {
                    exact_identity_digest: identity,
                    compatibility_group_digest: group,
                }],
                backend: "test-local".to_string(),
                expires_at_unix: unix_now().unwrap() + 3_600,
            },
            &EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn seed_repository(store: &MetadataStore) -> String {
        let owner = UserAccount {
            id: "user_cache_service".to_string(),
            handle: "cache-service".to_string(),
            email: "cache-service@example.com".to_string(),
            email_verified: true,
        };
        let mut repository = StoredRepository::new(&owner, "e2e", Visibility::Private).unwrap();
        repository.record.lifecycle_state = RepoLifecycleState::Ready;
        let repository_id = repository.record.id.clone();
        let mut catalog = CatalogFixture::default();
        catalog.users.insert(owner.id.clone(), owner);
        catalog
            .repositories
            .insert(repository_id.clone(), repository);
        store.admin().seed_catalog_for_tests(catalog).unwrap();
        repository_id
    }

    async fn seed_active_attempt(store: &MetadataStore, repository_id: &str) -> (String, String) {
        let workflow = WorkflowIdentity::new(
            repository_id,
            WorkflowPath::parse("/.scope/runs/cache-service.yml").unwrap(),
        )
        .unwrap();
        let definition = CompiledWorkflow::new(
            "Cache service",
            WorkflowTriggers::new(true, false).unwrap(),
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![],
                    ContainerSpec::new(format!("alpine@sha256:{}", "a".repeat(64))).unwrap(),
                    600,
                    vec![],
                    Default::default(),
                    vec![WorkflowStep::new("Test", "true").unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let revision = WorkflowRevision::new(workflow.clone(), definition).unwrap();
        let source_digest = "c".repeat(64);
        let source = RunSource::ephemeral_git_bundle(scope_domain::store::SourceBlob {
            content_ref: ContentRef::git_bundle_sha256(source_digest.clone()),
            sha256: source_digest,
            git_oid: "d".repeat(40),
            git_file_mode: "100644".to_string(),
            size_bytes: 1,
        })
        .unwrap();
        let now = unix_now().unwrap();
        store
            .runs()
            .enqueue_run(
                Run::new(
                    "cache-service-run",
                    "cache-service-test",
                    workflow,
                    revision.digest(),
                    RunTrigger::Manual,
                    Some("user_cache_service".to_string()),
                    source,
                    now,
                )
                .unwrap(),
                revision,
            )
            .await
            .unwrap();
        let offer = store.runs().next_dispatchable_job().await.unwrap().unwrap();
        let attempt_id = "attempt-cache-service".to_string();
        let attempt_token_hash = "e".repeat(64);
        store
            .runs()
            .dispatch_job(
                &offer.run.id,
                offer.job.key.as_str(),
                &attempt_id,
                &attempt_token_hash,
                "test-runtime",
                now,
                now + 3_600,
            )
            .await
            .unwrap();
        (attempt_id, attempt_token_hash)
    }

    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";
    const TEST_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";
}
