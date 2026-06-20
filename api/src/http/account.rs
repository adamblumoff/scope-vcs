use crate::domain::policy::ScopePath;
use crate::{
    auth::shoo::{ensure_user_for_identity, http_identity, principal_for_repo},
    error::ApiError,
    http::responses::{
        AccountSessionResponse, HealthResponse, SessionCapabilities, SessionIdentity, SessionRepo,
        SessionResponse, UserResponse,
    },
    state::AppState,
    state::{can_read_path, can_write_path, ensure_repo_read, find_repo, role_for_principal},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

pub(crate) async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "api",
    })
}

pub(crate) async fn get_account_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountSessionResponse>, ApiError> {
    let identity = http_identity(&state, &headers).await?;
    let user = match identity.as_ref() {
        Some(identity) => Some(UserResponse::from(ensure_user_for_identity(
            &state, identity,
        )?)),
        None => None,
    };

    Ok(Json(AccountSessionResponse {
        identity: identity.as_ref().map(SessionIdentity::from),
        user,
    }))
}

pub(crate) async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<SessionResponse>, ApiError> {
    let repo = find_repo(&state, &owner, &repo_name)?;
    let identity = http_identity(&state, &headers).await?;
    let principal = principal_for_repo(&state, &repo, identity.as_ref())?;
    ensure_repo_read(&state, &repo, &principal)?;
    let root = ScopePath::root();
    let role = role_for_principal(&state, &repo, &principal)?;

    Ok(Json(SessionResponse {
        identity: identity.as_ref().map(SessionIdentity::from),
        repo: SessionRepo {
            id: repo.record.id.clone(),
            publication_state: repo.record.publication_state,
            role,
        },
        capabilities: SessionCapabilities {
            read: can_read_path(&state, &repo, &principal, &root)?,
            write: can_write_path(&state, &repo, &principal, &root)?,
        },
        principal_id: principal.id,
    }))
}
