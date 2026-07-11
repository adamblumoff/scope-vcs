use crate::domain::requests::{
    Request, RequestDisposition, RequestEvent, RequestSettlement, allowed_resolution_dispositions,
    settlement_for,
};
use scope_api_contract::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(ts_rs::TS))]
pub(crate) struct RequestEditorRequest {
    pub(crate) user_id: String,
}

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
        title: request.title,
        author_user_id: request.author_user_id,
        editor_user_ids: request.editor_user_ids.into_iter().collect(),
        author_role: request.author_role,
        base_audience: request.base_audience,
        target_branch: request.target_branch,
        request_ref: request.request_ref,
        base_main_oid: super::git_oid_response(request.base_main_oid)?,
        head_oid: super::git_oid_response(request.head_oid)?,
        state: request.state,
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

pub(crate) fn request_event_response(
    event: RequestEvent,
) -> Result<RequestEventResponse, crate::error::ApiError> {
    Ok(RequestEventResponse {
        id: event.id,
        actor_user_id: event.actor_user_id,
        kind: event.kind,
        body: event.body,
        old_head_oid: event
            .old_head_oid
            .map(super::git_oid_response)
            .transpose()?,
        new_head_oid: event
            .new_head_oid
            .map(super::git_oid_response)
            .transpose()?,
        created_at_unix: event.created_at_unix,
    })
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
