use crate::domain::requests::{Request, RequestEvent, request_list_mergeability};
use crate::domain::store::RepositoryAccess;
use scope_api_contract::*;
use scope_core::db::RequestListRow;

pub(crate) fn request_summary_response(
    request: Request,
    permissions: RequestPermissionsResponse,
    mergeability: RequestMergeabilityResponse,
) -> Result<RequestSummaryResponse, crate::error::ApiError> {
    Ok(RequestSummaryResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        description_markdown: request.description_markdown,
        author_user_id: request.author_user_id,
        author_role: request.author_role,
        audience: request.audience,
        base_main_oid: super::git_oid_response(request.base_main_oid)?,
        head_oid: super::git_oid_response(request.head_oid)?,
        state: request.state,
        activity_version: request.activity_version,
        current_stake_credits: request.current_stake_credits,
        first_ready_at_unix: request.first_ready_at_unix,
        ready_at_unix: request.ready_at_unix,
        held_at_unix: request.held_at_unix,
        held_by_user_id: request.held_by_user_id,
        assessment_outcome: request.assessment_outcome,
        assessment_body_markdown: request.assessment_body_markdown,
        assessed_at_unix: request.assessed_at_unix,
        assessed_by_user_id: request.assessed_by_user_id,
        completed_at_unix: request.completed_at_unix,
        completed_by_user_id: request.completed_by_user_id,
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
        permissions,
        mergeability,
    })
}

pub(crate) fn request_list_item_response(
    request: RequestListRow,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
) -> Result<RequestListItemResponse, crate::error::ApiError> {
    let decision = request_list_mergeability(
        request.state,
        request.assessment_outcome,
        request.has_git_snapshot,
        request.is_held,
        request.is_merged,
        access,
    );
    let request_head_oid = super::git_oid_response(request.head_oid)?;
    Ok(RequestListItemResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        author_role: request.author_role,
        audience: request.audience,
        head_oid: request_head_oid.clone(),
        state: request.state,
        current_stake_credits: request.current_stake_credits,
        assessment_outcome: request.assessment_outcome,
        updated_at_unix: request.updated_at_unix,
        mergeability: RequestMergeabilityResponse {
            status: decision.status,
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
        kind: event.kind,
        payload: event.payload,
        created_at_unix: event.created_at_unix,
    }
}
