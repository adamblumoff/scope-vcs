use crate::{
    domain::{
        requests::{Request, request_permissions, request_visible_to_access},
        store::RepositoryAccess,
    },
    error::ApiError,
    state::{AppState, find_repo},
};

fn request_is_closed(request: &Request) -> bool {
    matches!(
        request.state,
        crate::domain::requests::RequestState::Resolved
            | crate::domain::requests::RequestState::Withdrawn
    )
}

pub(super) fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    access: RepositoryAccess,
) -> bool {
    request_permissions(request, access, Some(actor_user_id)).can_push_branch
}

pub(super) async fn ensure_request_ref_update_allowed(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    request_name: &str,
) -> Result<Request, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let access = repo.access_for_user_id(actor_user_id);
    let request = state
        .metadata
        .request_by_name(&repo.record.id, request_name)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !request_visible_to_access(&request, access) {
        return Err(ApiError::not_found("request not found"));
    }
    if request_is_closed(&request) {
        return Err(ApiError::conflict("request is closed"));
    }
    if !request_actor_can_edit_ref(&request, actor_user_id, access) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}
