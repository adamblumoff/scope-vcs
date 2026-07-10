use crate::{
    domain::{
        requests::{Request, RequestActorRole, RequestBaseAudience, RequestState},
        store::RepositoryActor,
    },
    error::ApiError,
    state::{AppState, find_repo},
};

fn request_is_closed(request: &Request) -> bool {
    matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    )
}

fn request_is_open_for_current_actor(request: &Request, current_actor: RepositoryActor) -> bool {
    if request_is_closed(request) {
        return false;
    }
    match current_actor {
        RepositoryActor::Public => {
            request.author_role == RequestActorRole::Public
                && request.base_audience == RequestBaseAudience::Public
        }
        RepositoryActor::Member | RepositoryActor::Owner => true,
    }
}

pub(super) fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    current_actor: RepositoryActor,
) -> bool {
    if !request_is_open_for_current_actor(request, current_actor) {
        return false;
    }
    match current_actor {
        RepositoryActor::Public => {
            request.author_user_id == actor_user_id
                || request.editor_user_ids.contains(actor_user_id)
        }
        RepositoryActor::Member | RepositoryActor::Owner => true,
    }
}

pub(super) async fn ensure_request_ref_update_allowed(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    request_ref: &str,
) -> Result<Request, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let request = state
        .metadata
        .request_by_ref(request_ref)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.repo_id != repo.record.id {
        return Err(ApiError::not_found("request not found"));
    }
    let current_actor = repo.access_for_user_id(actor_user_id).actor;
    if !request_is_open_for_current_actor(&request, current_actor) {
        if request_is_closed(&request)
            && (request.author_user_id == actor_user_id
                || request.editor_user_ids.contains(actor_user_id))
        {
            return Err(ApiError::conflict("request is closed"));
        }
        return Err(ApiError::not_found("request not found"));
    }
    if !request_actor_can_edit_ref(&request, actor_user_id, current_actor) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}
