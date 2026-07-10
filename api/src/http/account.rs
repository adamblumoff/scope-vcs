use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user},
    domain::policy::ScopePath,
    error::ApiError,
    http::responses::{
        AccountSessionResponse, HealthResponse, ReadinessCheckResponse, ReadinessResponse,
        SessionIdentity, SessionRepo, SessionResponse, UserResponse, repository_access_response,
        session_capabilities_response,
    },
    state::AppState,
    state::{access_for_principal, can_read_path, ensure_repo_read, find_repo},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

pub(crate) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "api",
    })
}

pub(crate) async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let database_ready = state.metadata.readiness_check().await.is_ok();
    let object_store_ready = state.object_store.readiness_check().is_ok();
    let ready = database_ready && object_store_ready;

    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(ReadinessResponse {
            status: if ready { "ok" } else { "unavailable" },
            service: "api",
            checks: vec![
                ReadinessCheckResponse {
                    name: "database",
                    status: if database_ready { "ok" } else { "unavailable" },
                },
                ReadinessCheckResponse {
                    name: "object_store",
                    status: if object_store_ready {
                        "ok"
                    } else {
                        "unavailable"
                    },
                },
            ],
        }),
    )
}

pub(crate) async fn get_account_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountSessionResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;

    Ok(Json(AccountSessionResponse {
        identity: user.as_ref().map(SessionIdentity::from),
        user: user.map(UserResponse::from),
    }))
}

pub(crate) async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let user = optional_scope_user(&state, &headers).await?;
    let principal = principal_for_scope_user(&repo, user.as_ref());
    ensure_repo_read(&state, &repo, &principal)?;
    let root = ScopePath::root();
    let access = access_for_principal(&state, &repo, &principal)?;
    let can_read = can_read_path(&state, &repo, &principal, &root)?;

    Ok(Json(SessionResponse {
        identity: user.as_ref().map(SessionIdentity::from),
        repo: SessionRepo {
            id: repo.record.id.clone(),
            publication_state: repo.record.publication_state,
            access: repository_access_response(access),
        },
        capabilities: session_capabilities_response(can_read, access),
        principal_id: principal.id,
    }))
}
