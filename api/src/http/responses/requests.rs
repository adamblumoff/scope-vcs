use scope_api_contract::{
    RequestActorSummaryResponse, RequestEventResponse, RequestInviteeResponse,
    RequestListItemResponse, RequestMergeabilityResponse, RequestPermissionsResponse,
    RequestSummaryResponse,
};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{Request, RequestEvent, request_list_mergeability},
};
use scope_postgres::db::RequestListRow;

pub(crate) fn request_summary_response(
    request: Request,
    invitees: Vec<RequestInviteeResponse>,
    permissions: RequestPermissionsResponse,
    mergeability: RequestMergeabilityResponse,
) -> Result<RequestSummaryResponse, crate::error::ApiError> {
    let state = request.state();
    Ok(RequestSummaryResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        description_markdown: request.description_markdown,
        author_user_id: request.author_user_id,
        author_role: request.author_role.into(),
        audience: request.audience.into(),
        base_main_oid: super::git_oid_response(request.base_main_oid)?,
        head_oid: super::git_oid_response(request.head_oid)?,
        state: state.into(),
        activity_version: request.activity_version,
        submitted_at_unix: request.submitted_at_unix,
        closed_at_unix: request.closed_at_unix,
        closed_by_user_id: request.closed_by_user_id,
        merged_at_unix: request.merged_at_unix,
        merged_by_user_id: request.merged_by_user_id,
        merged_head_oid: request
            .merged_head_oid
            .map(super::git_oid_response)
            .transpose()?,
        merged_main_oid: request
            .merged_main_oid
            .map(super::git_oid_response)
            .transpose()?,
        created_at_unix: request.created_at_unix,
        updated_at_unix: request.updated_at_unix,
        invitees,
        permissions,
        mergeability,
    })
}

pub(crate) fn request_list_item_response(
    request: RequestListRow,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
) -> Result<RequestListItemResponse, crate::error::ApiError> {
    let decision = request_list_mergeability(request.state, request.has_git_snapshot, access);
    let request_head_oid = super::git_oid_response(request.head_oid)?;
    Ok(RequestListItemResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        author_role: request.author_role.into(),
        audience: request.audience.into(),
        head_oid: request_head_oid.clone(),
        state: request.state.into(),
        submitted_at_unix: request.submitted_at_unix,
        updated_at_unix: request.updated_at_unix,
        mergeability: RequestMergeabilityResponse {
            status: decision.status.into(),
            current_main_oid: current_main_oid.map(super::git_oid_response).transpose()?,
            request_head_oid,
            reason: decision.reason.map(str::to_string),
        },
    })
}

pub(crate) fn request_event_response(
    event: RequestEvent,
    actor: RequestActorSummaryResponse,
) -> RequestEventResponse {
    RequestEventResponse {
        id: event.id,
        position: event.position,
        actor,
        kind: event.kind.into(),
        payload: event.payload.into(),
        created_at_unix: event.created_at_unix,
    }
}
