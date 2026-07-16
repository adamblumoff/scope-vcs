use crate::{domain::store::SourceBlob, error::ApiError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod settlement;
pub use settlement::{ResolutionDisposition, allowed_resolution_dispositions, settlement_for};
mod policy;
pub use policy::{
    RequestMergeability, RequestMergeabilityStatus, RequestPermissions, request_actor_role,
    request_mergeability, request_permissions, request_visible_to_access,
};
mod discussions;
mod submission;
pub use discussions::{
    CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
    CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
    MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput,
    ReopenRequestDiscussionInput, RequestDiscussion, RequestDiscussionMutation,
    RequestDiscussionReadState, RequestDiscussionReply, RequestDiscussionStatus,
    ResolveRequestDiscussionInput, create_request_discussion, create_request_discussion_reply,
    mark_request_discussion_read, reopen_and_reply_to_request_discussion,
    reopen_request_discussion, resolve_request_discussion,
};
mod description;
pub use description::{UpdateRequestDescriptionInput, update_request_description};
use settlement::{CreditSettlementIds, settle_request_credits, u32_to_i32};
pub use submission::{
    RecordWorkingRequestUploadInput, StartRequestInput, StartRequestMutation, SubmitRequestInput,
    SubmitRequestMutation, WorkingRequestUploadMutation, record_working_request_upload,
    start_request, submit_request, validate_request_name,
};

pub const REQUEST_REF_PREFIX: &str = "refs/heads/";
pub const REQUEST_DISCUSSION_BODY_MAX_BYTES: usize = 64 * 1024;
pub const REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES: usize = 128;
pub const REQUEST_DESCRIPTION_MAX_BYTES: usize = 256 * 1024;
const REPO_DELETE_REFUND_LEDGER_ENTRY_PREFIX: &str = "repo_delete_refund:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestActorRole {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestAudience {
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestState {
    Working,
    Submitted,
    NeedsResponse,
    Resolved,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestDisposition {
    Accepted,
    UsefulNotMerged,
    HiddenContext,
    NotAligned,
    Duplicate,
    Abandoned,
    LowQuality,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RequestSettlement {
    pub disposition: RequestDisposition,
    pub stake_credits: u32,
    pub refunded_credits: u32,
    pub reward_credits: u32,
    pub burned_credits: u32,
    pub settled_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: String,
    pub head_oid: String,
    pub git_snapshot: Option<SourceBlob>,
    pub title: String,
    pub description_markdown: String,
    pub state: RequestState,
    pub activity_version: u64,
    pub stake_credits: u32,
    pub disposition: Option<RequestDisposition>,
    pub settlement: Option<RequestSettlement>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestEventKind {
    Started,
    Submitted,
    RevisionPushed,
    NeedsResponse,
    ContributorResponded,
    Merged,
    Resolved,
    Settled,
    Withdrawn,
    DescriptionEdited,
    DiscussionResolved,
    DiscussionReopened,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestEventPayload {
    Started {
        title: String,
        description_markdown: String,
    },
    Submitted {
        head_oid: String,
    },
    RevisionPushed {
        old_head_oid: String,
        new_head_oid: String,
        note: Option<String>,
    },
    NeedsResponse {
        body: String,
        head_oid: String,
    },
    ContributorResponded {
        body: Option<String>,
        head_oid: String,
    },
    Merged {
        body: Option<String>,
        head_oid: String,
    },
    Resolved {
        body: Option<String>,
        head_oid: String,
        disposition: RequestDisposition,
    },
    Settled {
        settlement: RequestSettlement,
    },
    Withdrawn {
        head_oid: String,
    },
    DescriptionEdited {
        previous_markdown: String,
        new_markdown: String,
    },
    DiscussionResolved {
        discussion_id: String,
    },
    DiscussionReopened {
        discussion_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEvent {
    pub id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub kind: RequestEventKind,
    pub position: u64,
    pub payload: RequestEventPayload,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreditAccount {
    pub user_id: String,
    pub balance_credits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum CreditLedgerEntryKind {
    StarterGrant,
    RequestStakeDebit,
    StakeRefund,
    RequestReward,
    AdminAdjustment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditLedgerEntry {
    pub id: String,
    pub user_id: String,
    pub request_id: Option<String>,
    pub kind: CreditLedgerEntryKind,
    pub amount_credits: i32,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct GrantUserCreditsInput {
    pub ledger_entry_id: String,
    pub user_id: String,
    pub amount_credits: u32,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditAccountMutation {
    pub account: UserCreditAccount,
    pub ledger_entry: CreditLedgerEntry,
}

#[derive(Clone, Debug)]
pub struct RecordRequestRevisionInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: Option<SourceBlob>,
    pub event_id: String,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRevisionMutation {
    pub request: Request,
    pub event: RequestEvent,
    pub orphan_objects: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct MarkRequestNeedsResponseInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub body: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RespondToRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestTimelineMutation {
    pub request: Request,
    pub event: RequestEvent,
}

#[derive(Clone, Debug)]
pub struct ResolveRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub disposition: RequestDisposition,
    pub event_id: String,
    pub settlement_event_id: String,
    pub refund_ledger_entry_id: Option<String>,
    pub reward_ledger_entry_id: Option<String>,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveRequestMutation {
    pub request: Request,
    pub resolved_event: RequestEvent,
    pub settled_event: RequestEvent,
    pub account: Option<UserCreditAccount>,
    pub ledger_entries: Vec<CreditLedgerEntry>,
}

#[derive(Clone, Debug)]
pub struct MergeRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub expected_main_oid: String,
    pub current_main_oid: String,
    pub expected_head_oid: String,
    pub event_id: String,
    pub settlement_event_id: String,
    pub refund_ledger_entry_id: Option<String>,
    pub reward_ledger_entry_id: Option<String>,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequestMutation {
    pub request: Request,
    pub merged_event: RequestEvent,
    pub settled_event: RequestEvent,
    pub account: Option<UserCreditAccount>,
    pub ledger_entries: Vec<CreditLedgerEntry>,
}

#[derive(Clone, Debug)]
pub struct DeleteRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_delete: bool,
    pub event_id: String,
    pub refund_ledger_entry_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteRequestMutation {
    DeletedWorking {
        request: Request,
        events: Vec<RequestEvent>,
        orphan_objects: Vec<SourceBlob>,
    },
    Withdrawn {
        request: Box<Request>,
        event: RequestEvent,
        account: Option<UserCreditAccount>,
        ledger_entry: Option<CreditLedgerEntry>,
    },
}

pub fn grant_user_credits(
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: GrantUserCreditsInput,
) -> Result<CreditAccountMutation, ApiError> {
    validate_required_id("ledger entry id", &input.ledger_entry_id)?;
    validate_required_id("user id", &input.user_id)?;
    if input.amount_credits == 0 {
        return Err(ApiError::bad_request(
            "credit grant amount must be positive",
        ));
    }
    ensure_ledger_entry_id_available(ledger_entries, &input.ledger_entry_id)?;
    let current_balance = accounts
        .get(&input.user_id)
        .map(|account| account.balance_credits)
        .unwrap_or(0);
    let balance_credits = current_balance
        .checked_add(input.amount_credits)
        .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
    u32_to_i32(balance_credits)?;
    let amount_credits = u32_to_i32(input.amount_credits)?;

    let account = UserCreditAccount {
        user_id: input.user_id.clone(),
        balance_credits,
    };
    let ledger_entry = CreditLedgerEntry {
        id: input.ledger_entry_id,
        user_id: input.user_id,
        request_id: None,
        kind: CreditLedgerEntryKind::StarterGrant,
        amount_credits,
        created_at_unix: input.now_unix,
    };
    accounts.insert(account.user_id.clone(), account.clone());
    ledger_entries.insert(ledger_entry.id.clone(), ledger_entry.clone());
    Ok(CreditAccountMutation {
        account,
        ledger_entry,
    })
}

pub fn record_request_revision(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: RecordRequestRevisionInput,
) -> Result<RequestRevisionMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("head oid", &input.new_head_oid)?;
    validate_required_id("event id", &input.event_id)?;
    ensure_event_id_available(events, &input.event_id)?;

    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !input.actor_can_edit {
        return Err(ApiError::forbidden("request branch edit access required"));
    }
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is closed"));
    }
    match input.expected_old_head_oid.as_deref() {
        Some(expected_old_head_oid) if request.head_oid != expected_old_head_oid => {
            return Err(ApiError::conflict(
                "request branch changed since push started; fetch and retry",
            ));
        }
        None if request.git_snapshot.is_some() => {
            return Err(ApiError::conflict(
                "request branch changed since push started; fetch and retry",
            ));
        }
        _ => {}
    }

    let old_head_oid = request.head_oid.clone();
    let old_git_snapshot = input
        .git_snapshot
        .as_ref()
        .and_then(|_| request.git_snapshot.clone());
    request.head_oid = input.new_head_oid.clone();
    if input.git_snapshot.is_some() {
        request.git_snapshot = input.git_snapshot.clone();
    }
    request.updated_at_unix = input.now_unix;
    if request.state == RequestState::NeedsResponse {
        request.state = RequestState::Submitted;
    }
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::RevisionPushed,
        position,
        payload: RequestEventPayload::RevisionPushed {
            old_head_oid,
            new_head_oid: input.new_head_oid,
            note: input.body,
        },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestRevisionMutation {
        request,
        event,
        orphan_objects: old_git_snapshot.into_iter().collect(),
    })
}

pub fn delete_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: DeleteRequestInput,
) -> Result<DeleteRequestMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    let request = requests
        .get(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !input.actor_can_delete {
        return Err(ApiError::forbidden("request delete access required"));
    }
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is already closed"));
    }
    if request.state == RequestState::Working {
        let request = requests
            .remove(&input.request_id)
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let removed_events = events
            .keys()
            .filter(|event_id| {
                events
                    .get(*event_id)
                    .is_some_and(|event| event.request_id == request.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let events = removed_events
            .into_iter()
            .filter_map(|event_id| events.remove(&event_id))
            .collect::<Vec<_>>();
        return Ok(DeleteRequestMutation::DeletedWorking {
            orphan_objects: request.git_snapshot.clone().into_iter().collect(),
            request,
            events,
        });
    }

    ensure_event_id_available(events, &input.event_id)?;
    let refund = if request.stake_credits == 0 {
        None
    } else {
        let refund_ledger_entry_id = input
            .refund_ledger_entry_id
            .clone()
            .ok_or_else(|| ApiError::bad_request("refund ledger entry id is required"))?;
        ensure_ledger_entry_id_available(ledger_entries, &refund_ledger_entry_id)?;
        let refund_amount = u32_to_i32(request.stake_credits)?;
        let current_balance = accounts
            .get(&request.author_user_id)
            .map(|account| account.balance_credits)
            .unwrap_or(0);
        let balance_credits = current_balance
            .checked_add(request.stake_credits)
            .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
        u32_to_i32(balance_credits)?;
        let account = UserCreditAccount {
            user_id: request.author_user_id.clone(),
            balance_credits,
        };
        let ledger_entry = CreditLedgerEntry {
            id: refund_ledger_entry_id,
            user_id: request.author_user_id.clone(),
            request_id: Some(request.id.clone()),
            kind: CreditLedgerEntryKind::StakeRefund,
            amount_credits: refund_amount,
            created_at_unix: input.now_unix,
        };
        Some((account, ledger_entry))
    };

    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    request.state = RequestState::Withdrawn;
    request.updated_at_unix = input.now_unix;
    request.resolved_at_unix = Some(input.now_unix);
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Withdrawn,
        position,
        payload: RequestEventPayload::Withdrawn {
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    let (account, ledger_entry) = match refund {
        Some((account, ledger_entry)) => {
            accounts.insert(account.user_id.clone(), account.clone());
            ledger_entries.insert(ledger_entry.id.clone(), ledger_entry.clone());
            (Some(account), Some(ledger_entry))
        }
        None => (None, None),
    };
    Ok(DeleteRequestMutation::Withdrawn {
        request: Box::new(request),
        event,
        account,
        ledger_entry,
    })
}

pub fn mark_request_needs_response(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: MarkRequestNeedsResponseInput,
) -> Result<RequestTimelineMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    validate_required_body("needs-response body", &input.body)?;
    ensure_event_id_available(events, &input.event_id)?;
    let request = open_request_mut(requests, &input.request_id)?;
    if request.state != RequestState::Submitted {
        return Err(ApiError::conflict(
            "request must be submitted before asking for a response",
        ));
    }
    request.state = RequestState::NeedsResponse;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::NeedsResponse,
        position,
        payload: RequestEventPayload::NeedsResponse {
            body: input.body,
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestTimelineMutation { request, event })
}

pub fn respond_to_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: RespondToRequestInput,
) -> Result<RequestTimelineMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    let request = open_request_mut(requests, &input.request_id)?;
    if request.author_user_id != input.actor_user_id {
        return Err(ApiError::forbidden("request author required"));
    }
    if request.state != RequestState::NeedsResponse {
        return Err(ApiError::conflict(
            "request is not waiting on the contributor",
        ));
    }
    request.state = RequestState::Submitted;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::ContributorResponded,
        position,
        payload: RequestEventPayload::ContributorResponded {
            body: input.body,
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestTimelineMutation { request, event })
}

pub fn resolve_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: ResolveRequestInput,
) -> Result<ResolveRequestMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    validate_required_id("settlement event id", &input.settlement_event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    ensure_event_id_available(events, &input.settlement_event_id)?;
    if input.event_id == input.settlement_event_id {
        return Err(ApiError::bad_request("settlement event id must be unique"));
    }

    let request = requests
        .get(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is already closed"));
    }
    if !matches!(
        request.state,
        RequestState::Submitted | RequestState::NeedsResponse
    ) {
        return Err(ApiError::conflict(
            "request must be submitted before it can be resolved",
        ));
    }
    if request.settlement.is_some() {
        return Err(ApiError::conflict("request is already settled"));
    }
    if input.disposition == RequestDisposition::Abandoned
        && request.state != RequestState::NeedsResponse
    {
        return Err(ApiError::conflict(
            "abandonment requires the request to be waiting on the contributor",
        ));
    }
    if input.disposition == RequestDisposition::Accepted {
        return Err(ApiError::bad_request(
            "accepted requests must be merged through the merge flow",
        ));
    }

    let settlement = settlement_for(request.stake_credits, input.disposition, input.now_unix);
    let credit_mutation = settle_request_credits(
        accounts,
        ledger_entries,
        request,
        &settlement,
        CreditSettlementIds {
            refund_ledger_entry_id: input.refund_ledger_entry_id.clone(),
            reward_ledger_entry_id: input.reward_ledger_entry_id.clone(),
        },
        input.now_unix,
    )?;

    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    request.state = RequestState::Resolved;
    request.disposition = Some(input.disposition);
    request.settlement = Some(settlement.clone());
    request.updated_at_unix = input.now_unix;
    request.resolved_at_unix = Some(input.now_unix);
    let resolved_position = advance_request_activity(request)?;
    let settled_position = advance_request_activity(request)?;
    let request = request.clone();

    let resolved_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id.clone(),
        kind: RequestEventKind::Resolved,
        position: resolved_position,
        payload: RequestEventPayload::Resolved {
            body: input.body,
            head_oid: request.head_oid.clone(),
            disposition: input.disposition,
        },
        created_at_unix: input.now_unix,
    };
    let settled_event = RequestEvent {
        id: input.settlement_event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Settled,
        position: settled_position,
        payload: RequestEventPayload::Settled {
            settlement: settlement.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(resolved_event.id.clone(), resolved_event.clone());
    events.insert(settled_event.id.clone(), settled_event.clone());

    Ok(ResolveRequestMutation {
        request,
        resolved_event,
        settled_event,
        account: credit_mutation.account,
        ledger_entries: credit_mutation.ledger_entries,
    })
}

pub fn merge_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: MergeRequestInput,
) -> Result<MergeRequestMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("expected main oid", &input.expected_main_oid)?;
    validate_required_id("current main oid", &input.current_main_oid)?;
    validate_required_id("expected head oid", &input.expected_head_oid)?;
    validate_required_id("event id", &input.event_id)?;
    validate_required_id("settlement event id", &input.settlement_event_id)?;
    ensure_event_id_available(events, &input.event_id)?;
    ensure_event_id_available(events, &input.settlement_event_id)?;
    if input.event_id == input.settlement_event_id {
        return Err(ApiError::bad_request("settlement event id must be unique"));
    }
    if input.expected_main_oid != input.current_main_oid {
        return Err(ApiError::conflict("main changed since merge was confirmed"));
    }

    let request = requests
        .get(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is already closed"));
    }
    if request.state != RequestState::Submitted {
        return Err(ApiError::conflict(
            "request must be submitted before it can be merged",
        ));
    }
    if request.settlement.is_some() {
        return Err(ApiError::conflict("request is already settled"));
    }
    if request.head_oid != input.expected_head_oid {
        return Err(ApiError::conflict(
            "request changed since merge was confirmed",
        ));
    }

    let settlement = settlement_for(
        request.stake_credits,
        RequestDisposition::Accepted,
        input.now_unix,
    );
    let credit_mutation = settle_request_credits(
        accounts,
        ledger_entries,
        request,
        &settlement,
        CreditSettlementIds {
            refund_ledger_entry_id: input.refund_ledger_entry_id.clone(),
            reward_ledger_entry_id: input.reward_ledger_entry_id.clone(),
        },
        input.now_unix,
    )?;

    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    request.state = RequestState::Resolved;
    request.disposition = Some(RequestDisposition::Accepted);
    request.settlement = Some(settlement.clone());
    request.updated_at_unix = input.now_unix;
    request.resolved_at_unix = Some(input.now_unix);
    let merged_position = advance_request_activity(request)?;
    let settled_position = advance_request_activity(request)?;
    let request = request.clone();

    let merged_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id.clone(),
        kind: RequestEventKind::Merged,
        position: merged_position,
        payload: RequestEventPayload::Merged {
            body: input.body,
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    let settled_event = RequestEvent {
        id: input.settlement_event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Settled,
        position: settled_position,
        payload: RequestEventPayload::Settled {
            settlement: settlement.clone(),
        },
        created_at_unix: input.now_unix,
    };
    events.insert(merged_event.id.clone(), merged_event.clone());
    events.insert(settled_event.id.clone(), settled_event.clone());

    Ok(MergeRequestMutation {
        request,
        merged_event,
        settled_event,
        account: credit_mutation.account,
        ledger_entries: credit_mutation.ledger_entries,
    })
}

pub fn canonical_request_ref(request_name: &str) -> String {
    format!("{REQUEST_REF_PREFIX}{request_name}")
}

fn validate_required_id(label: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(format!("{label} is required")));
    }
    Ok(())
}

fn validate_required_body(label: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(format!("{label} is required")));
    }
    Ok(())
}

pub(super) fn validate_body_size(
    label: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ApiError> {
    if value.len() > max_bytes {
        return Err(ApiError::bad_request(format!(
            "{label} exceeds {max_bytes} bytes"
        )));
    }
    Ok(())
}

pub(super) fn advance_request_activity(request: &mut Request) -> Result<u64, ApiError> {
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("request activity version overflow"))?;
    Ok(request.activity_version)
}

pub(super) fn open_request_mut<'a>(
    requests: &'a mut BTreeMap<String, Request>,
    request_id: &str,
) -> Result<&'a mut Request, ApiError> {
    let request = requests
        .get_mut(request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if matches!(
        request.state,
        RequestState::Resolved | RequestState::Withdrawn
    ) {
        return Err(ApiError::conflict("request is closed"));
    }
    Ok(request)
}

pub(super) fn ensure_event_id_available(
    events: &BTreeMap<String, RequestEvent>,
    event_id: &str,
) -> Result<(), ApiError> {
    if events.contains_key(event_id) {
        Err(ApiError::conflict("request event already exists"))
    } else {
        Ok(())
    }
}

fn ensure_request_name_available(
    requests: &BTreeMap<String, Request>,
    repo_id: &str,
    request_name: &str,
) -> Result<(), ApiError> {
    if requests
        .values()
        .any(|request| request.repo_id == repo_id && request.name == request_name)
    {
        Err(ApiError::conflict("request name already exists"))
    } else {
        Ok(())
    }
}

fn ensure_ledger_entry_id_available(
    ledger_entries: &BTreeMap<String, CreditLedgerEntry>,
    ledger_entry_id: &str,
) -> Result<(), ApiError> {
    validate_required_id("credit ledger entry id", ledger_entry_id)?;
    if ledger_entry_id.starts_with(REPO_DELETE_REFUND_LEDGER_ENTRY_PREFIX) {
        return Err(ApiError::bad_request(
            "credit ledger entry id uses a working internal prefix",
        ));
    }
    if ledger_entries.contains_key(ledger_entry_id) {
        Err(ApiError::conflict("credit ledger entry already exists"))
    } else {
        Ok(())
    }
}

fn ensure_new_ledger_entry_id(
    ledger_entries: &BTreeMap<String, CreditLedgerEntry>,
    reserved_ledger_entry_ids: &mut BTreeSet<String>,
    ledger_entry_id: &str,
) -> Result<(), ApiError> {
    ensure_ledger_entry_id_available(ledger_entries, ledger_entry_id)?;
    if !reserved_ledger_entry_ids.insert(ledger_entry_id.to_string()) {
        return Err(ApiError::bad_request(
            "credit ledger entry id must be unique",
        ));
    }
    Ok(())
}
