use crate::domain::requests::{
    Request, RequestDisposition, RequestEvent, RequestSettlement, allowed_resolution_dispositions,
    request_list_mergeability, settlement_for,
};
use crate::domain::store::RepositoryAccess;
use scope_api_contract::*;
use scope_core::db::RequestListRow;

fn settlement_preview(
    stake_credits: u32,
    disposition: RequestDisposition,
) -> RequestSettlementPreviewResponse {
    let settlement = settlement_for(stake_credits, disposition, 0);
    RequestSettlementPreviewResponse {
        stake_credits,
        refunded_credits: settlement.refunded_credits,
        reward_credits: settlement.reward_credits,
        burned_credits: settlement.burned_credits,
    }
}

pub(crate) fn request_summary_response(
    request: Request,
    permissions: RequestPermissionsResponse,
    mergeability: RequestMergeabilityResponse,
) -> Result<RequestSummaryResponse, crate::error::ApiError> {
    let resolution_options = allowed_resolution_dispositions(request.state)
        .iter()
        .copied()
        .map(|disposition| RequestResolutionOptionResponse {
            disposition,
            settlement: settlement_preview(request.stake_credits, disposition.into()),
        })
        .collect();
    let merge_settlement_preview =
        settlement_preview(request.stake_credits, RequestDisposition::Accepted);
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
        stake_credits: request.stake_credits,
        disposition: request.disposition,
        settlement: request.settlement.map(request_settlement_response),
        created_at_unix: request.created_at_unix,
        updated_at_unix: request.updated_at_unix,
        resolved_at_unix: request.resolved_at_unix,
        permissions,
        mergeability,
        resolution_options,
        merge_settlement_preview,
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
        author_role: request.author_role,
        audience: request.audience,
        head_oid: request_head_oid.clone(),
        state: request.state,
        stake_credits: request.stake_credits,
        disposition: request.disposition,
        settlement: request.settlement.map(request_settlement_response),
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

fn request_settlement_response(settlement: RequestSettlement) -> RequestSettlementResponse {
    RequestSettlementResponse {
        disposition: settlement.disposition,
        stake_credits: settlement.stake_credits,
        refunded_credits: settlement.refunded_credits,
        reward_credits: settlement.reward_credits,
        burned_credits: settlement.burned_credits,
        settled_at_unix: settlement.settled_at_unix,
    }
}
