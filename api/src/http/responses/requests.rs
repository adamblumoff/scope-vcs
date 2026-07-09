use crate::domain::requests::{
    Request, RequestActorRole, RequestBaseAudience, RequestDisposition, RequestEvent,
    RequestEventKind, RequestSettlement, RequestState,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RepoRequestPermissionsResponse {
    pub(crate) can_submit_request: bool,
    pub(crate) uses_credit_stake: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestListResponse {
    pub(crate) requests: Vec<RequestSummaryResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestDetailResponse {
    pub(crate) request: RequestSummaryResponse,
    pub(crate) events: Vec<RequestEventResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestMutationResponse {
    pub(crate) request: RequestSummaryResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestDeleteResponse {
    pub(crate) deleted: bool,
    pub(crate) request: Option<RequestSummaryResponse>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct StartRequestRequest {
    pub(crate) title: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestEditorRequest {
    pub(crate) user_id: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestSummaryResponse {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) author_user_id: String,
    pub(crate) editor_user_ids: Vec<String>,
    pub(crate) author_role: RequestActorRole,
    pub(crate) base_audience: RequestBaseAudience,
    pub(crate) target_branch: String,
    pub(crate) request_ref: String,
    pub(crate) base_main_oid: String,
    pub(crate) head_oid: String,
    pub(crate) state: RequestState,
    pub(crate) stake_credits: u32,
    pub(crate) disposition: Option<RequestDisposition>,
    pub(crate) settlement: Option<RequestSettlementResponse>,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    pub(crate) resolved_at_unix: Option<u64>,
    pub(crate) permissions: RequestPermissionsResponse,
    pub(crate) mergeability: RequestMergeabilityResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestPermissionsResponse {
    pub(crate) can_comment: bool,
    pub(crate) can_pull_branch: bool,
    pub(crate) can_push_branch: bool,
    pub(crate) can_delete: bool,
    pub(crate) can_invite_editor: bool,
    pub(crate) can_mark_needs_response: bool,
    pub(crate) can_respond: bool,
    pub(crate) can_resolve: bool,
    pub(crate) can_merge: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum RequestMergeabilityStatus {
    Ready,
    Closed,
    NotReady,
    NotMaintainer,
    MissingRequestBranch,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestMergeabilityResponse {
    pub(crate) status: RequestMergeabilityStatus,
    pub(crate) current_main_oid: Option<String>,
    pub(crate) request_head_oid: String,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestSettlementResponse {
    pub(crate) disposition: RequestDisposition,
    pub(crate) stake_credits: u32,
    pub(crate) refunded_credits: u32,
    pub(crate) reward_credits: u32,
    pub(crate) burned_credits: u32,
    pub(crate) settled_at_unix: u64,
}

impl From<RequestSettlement> for RequestSettlementResponse {
    fn from(settlement: RequestSettlement) -> Self {
        Self {
            disposition: settlement.disposition,
            stake_credits: settlement.stake_credits,
            refunded_credits: settlement.refunded_credits,
            reward_credits: settlement.reward_credits,
            burned_credits: settlement.burned_credits,
            settled_at_unix: settlement.settled_at_unix,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RequestEventResponse {
    pub(crate) id: String,
    pub(crate) actor_user_id: String,
    pub(crate) kind: RequestEventKind,
    pub(crate) body: Option<String>,
    pub(crate) old_head_oid: Option<String>,
    pub(crate) new_head_oid: Option<String>,
    pub(crate) created_at_unix: u64,
}

impl From<RequestEvent> for RequestEventResponse {
    fn from(event: RequestEvent) -> Self {
        Self {
            id: event.id,
            actor_user_id: event.actor_user_id,
            kind: event.kind,
            body: event.body,
            old_head_oid: event.old_head_oid,
            new_head_oid: event.new_head_oid,
            created_at_unix: event.created_at_unix,
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SubmitRequestRequest {
    pub(crate) head_oid: String,
    pub(crate) stake_credits: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct CommentRequestRequest {
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct NeedsResponseRequest {
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RespondRequestRequest {
    pub(crate) body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ResolveRequestRequest {
    pub(crate) disposition: RequestDisposition,
    pub(crate) body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct MergeRequestRequest {
    pub(crate) expected_main_oid: String,
    pub(crate) expected_head_oid: String,
    pub(crate) body: Option<String>,
}

pub(crate) fn request_summary_response(
    request: Request,
    permissions: RequestPermissionsResponse,
    mergeability: RequestMergeabilityResponse,
) -> RequestSummaryResponse {
    RequestSummaryResponse {
        id: request.id,
        title: request.title,
        author_user_id: request.author_user_id,
        editor_user_ids: request.editor_user_ids.into_iter().collect(),
        author_role: request.author_role,
        base_audience: request.base_audience,
        target_branch: request.target_branch,
        request_ref: request.request_ref,
        base_main_oid: request.base_main_oid,
        head_oid: request.head_oid,
        state: request.state,
        stake_credits: request.stake_credits,
        disposition: request.disposition,
        settlement: request.settlement.map(Into::into),
        created_at_unix: request.created_at_unix,
        updated_at_unix: request.updated_at_unix,
        resolved_at_unix: request.resolved_at_unix,
        permissions,
        mergeability,
    }
}
