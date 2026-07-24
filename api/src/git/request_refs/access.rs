use crate::{
    domain::{
        requests::{Request, RequestViewer, request_policy},
        store::RepositoryAccess,
    },
    error::ApiError,
    state::{AppState, find_repo},
};

pub(super) fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    access: RepositoryAccess,
    is_invitee: bool,
) -> bool {
    request_policy(
        request,
        RequestViewer::new(access, Some(actor_user_id), is_invitee),
    )
    .branch_mutable
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
    let is_invitee = state
        .metadata
        .request_is_invitee(&request.id, actor_user_id)
        .await?;
    if !request_actor_can_edit_ref(&request, actor_user_id, access, is_invitee) {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}
