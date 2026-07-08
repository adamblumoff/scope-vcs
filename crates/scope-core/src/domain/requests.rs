use crate::{domain::store::SourceBlob, error::ApiError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod settlement;
pub use settlement::settlement_for;
mod submission;
use settlement::{CreditSettlementIds, settle_request_credits, settlement_event_body, u32_to_i32};
pub use submission::{
    FinalizeReservedRequestInput, FinalizeReservedRequestMutation,
    RecordReservedRequestUploadInput, ReserveRequestInput, ReserveRequestMutation,
    ReservedRequestUploadMutation, finalize_reserved_request, record_reserved_request_upload,
    reserve_request,
};

const MAIN_BRANCH: &str = "main";
pub const REQUEST_REF_PREFIX: &str = "refs/scope/requests/";
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
pub enum RequestBaseAudience {
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum RequestState {
    Reserved,
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
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub base_audience: RequestBaseAudience,
    pub target_branch: String,
    pub request_ref: String,
    pub base_main_oid: String,
    pub head_oid: String,
    pub git_snapshot: Option<SourceBlob>,
    pub title: String,
    pub state: RequestState,
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
    Created,
    RevisionPushed,
    Commented,
    NeedsResponse,
    ContributorResponded,
    Merged,
    Resolved,
    Settled,
    Withdrawn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEvent {
    pub id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub kind: RequestEventKind,
    pub body: Option<String>,
    pub old_head_oid: Option<String>,
    pub new_head_oid: Option<String>,
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
    pub source_blobs_to_delete: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct CommentRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub body: String,
    pub now_unix: u64,
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
    if request.author_user_id != input.actor_user_id {
        return Err(ApiError::forbidden("request author required"));
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
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::RevisionPushed,
        body: input.body,
        old_head_oid: Some(old_head_oid),
        new_head_oid: Some(input.new_head_oid),
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestRevisionMutation {
        request,
        event,
        source_blobs_to_delete: old_git_snapshot.into_iter().collect(),
    })
}

pub fn comment_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    input: CommentRequestInput,
) -> Result<RequestTimelineMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("event id", &input.event_id)?;
    validate_required_body("comment body", &input.body)?;
    ensure_event_id_available(events, &input.event_id)?;
    let request = open_request_mut(requests, &input.request_id)?;
    request.updated_at_unix = input.now_unix;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Commented,
        body: Some(input.body),
        old_head_oid: None,
        new_head_oid: None,
        created_at_unix: input.now_unix,
    };
    events.insert(event.id.clone(), event.clone());
    Ok(RequestTimelineMutation { request, event })
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
    request.state = RequestState::NeedsResponse;
    request.updated_at_unix = input.now_unix;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::NeedsResponse,
        body: Some(input.body),
        old_head_oid: None,
        new_head_oid: Some(request.head_oid.clone()),
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
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::ContributorResponded,
        body: input.body,
        old_head_oid: None,
        new_head_oid: Some(request.head_oid.clone()),
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
    let request = request.clone();

    let resolved_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id.clone(),
        kind: RequestEventKind::Resolved,
        body: input.body,
        old_head_oid: None,
        new_head_oid: Some(request.head_oid.clone()),
        created_at_unix: input.now_unix,
    };
    let settled_event = RequestEvent {
        id: input.settlement_event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Settled,
        body: Some(settlement_event_body(&settlement)),
        old_head_oid: None,
        new_head_oid: None,
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
    let request = request.clone();

    let merged_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id.clone(),
        kind: RequestEventKind::Merged,
        body: input.body,
        old_head_oid: None,
        new_head_oid: Some(request.head_oid.clone()),
        created_at_unix: input.now_unix,
    };
    let settled_event = RequestEvent {
        id: input.settlement_event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Settled,
        body: Some(settlement_event_body(&settlement)),
        old_head_oid: None,
        new_head_oid: None,
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

pub fn canonical_request_ref(request_id: &str) -> String {
    format!("{REQUEST_REF_PREFIX}{request_id}")
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

fn open_request_mut<'a>(
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

fn ensure_event_id_available(
    events: &BTreeMap<String, RequestEvent>,
    event_id: &str,
) -> Result<(), ApiError> {
    if events.contains_key(event_id) {
        Err(ApiError::conflict("request event already exists"))
    } else {
        Ok(())
    }
}

fn ensure_request_ref_available(
    requests: &BTreeMap<String, Request>,
    request_ref: &str,
) -> Result<(), ApiError> {
    if requests
        .values()
        .any(|request| request.request_ref == request_ref)
    {
        Err(ApiError::conflict("request ref already exists"))
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
            "credit ledger entry id uses a reserved internal prefix",
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
