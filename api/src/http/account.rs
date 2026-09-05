use crate::{
    auth::scope::optional_scope_user,
    error::ApiError,
    http::responses::{
        HealthResponse, ReadinessCheckResponse, ReadinessResponse, SessionRepo, SessionResponse,
        repository_access_response, session_capabilities_response, user_response,
    },
    repo_access::find_read_access,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use scope_api_contract::{AccountSessionResponse, SessionIdentity};
use scope_domain::{policy::Principal, repository::access::RepositoryActor};

pub(crate) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "api",
    })
}

pub(crate) async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let database_ready = state.metadata.admin().readiness_check().await.is_ok();
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
        user: user.map(user_response),
    }))
}

pub(crate) async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionResponse>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let repo = find_read_access(
        &state,
        &owner,
        &repo_name,
        user.as_ref().map(|user| user.id.as_str()),
    )
    .await?;
    let access = repo.access;
    let can_read_root = repo.can_read_root();
    let principal_id = if access.actor == RepositoryActor::Public {
        Principal::public().id
    } else {
        user.as_ref()
            .expect("repository member is authenticated")
            .id
            .clone()
    };

    Ok(Json(SessionResponse {
        identity: user.as_ref().map(SessionIdentity::from),
        repo: SessionRepo {
            id: repo.record.id.clone(),
            lifecycle_state: repo.record.lifecycle_state.into(),
            access: repository_access_response(access),
        },
        capabilities: session_capabilities_response(can_read_root, access),
        principal_id,
    }))
}
