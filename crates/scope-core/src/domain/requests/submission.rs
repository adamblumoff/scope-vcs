use super::settlement::{maximum_request_reward, u32_to_i32};
use super::{
    CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestActorRole, RequestAudience,
    RequestEvent, RequestEventKind, RequestState, UserCreditAccount, ensure_event_id_available,
    ensure_ledger_entry_id_available, ensure_request_name_available, validate_required_id,
};
use crate::{domain::store::SourceBlob, error::ApiError};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct StartRequestInput {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: String,
    pub title: Option<String>,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequestMutation {
    pub request: Request,
}

#[derive(Clone, Debug)]
pub struct RecordWorkingRequestUploadInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingRequestUploadMutation {
    pub request: Request,
    pub source_blobs_to_delete: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct SubmitRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub expected_head_oid: String,
    pub stake_credits: u32,
    pub stake_ledger_entry_id: Option<String>,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitRequestMutation {
    pub request: Request,
    pub event: RequestEvent,
    pub account: Option<UserCreditAccount>,
    pub ledger_entry: Option<CreditLedgerEntry>,
}

pub fn start_request(
    requests: &mut BTreeMap<String, Request>,
    input: StartRequestInput,
) -> Result<StartRequestMutation, ApiError> {
    validate_start_request_input(&input)?;
    if requests.contains_key(&input.id) {
        return Err(ApiError::conflict("request already exists"));
    }
    ensure_request_name_available(requests, &input.repo_id, &input.name)?;

    let title = input.title.unwrap_or_else(|| input.name.clone());

    let request = Request {
        id: input.id,
        repo_id: input.repo_id,
        name: input.name,
        author_user_id: input.author_user_id,
        author_role: input.author_role,
        audience: input.audience,
        base_main_oid: input.base_main_oid.clone(),
        head_oid: input.base_main_oid,
        git_snapshot: None,
        title,
        state: RequestState::Working,
        stake_credits: 0,
        disposition: None,
        settlement: None,
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
        resolved_at_unix: None,
    };
    requests.insert(request.id.clone(), request.clone());
    Ok(StartRequestMutation { request })
}

pub fn record_working_request_upload(
    requests: &mut BTreeMap<String, Request>,
    input: RecordWorkingRequestUploadInput,
) -> Result<WorkingRequestUploadMutation, ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("head oid", &input.new_head_oid)?;
    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if !input.actor_can_edit {
        return Err(ApiError::forbidden("request branch edit access required"));
    }
    if request.state != RequestState::Working {
        return Err(ApiError::conflict("request is already submitted"));
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

    let old_git_snapshot = request.git_snapshot.clone();
    request.head_oid = input.new_head_oid;
    request.git_snapshot = Some(input.git_snapshot);
    request.updated_at_unix = input.now_unix;
    let request = request.clone();
    Ok(WorkingRequestUploadMutation {
        request,
        source_blobs_to_delete: old_git_snapshot.into_iter().collect(),
    })
}

pub fn submit_request(
    requests: &mut BTreeMap<String, Request>,
    events: &mut BTreeMap<String, RequestEvent>,
    accounts: &mut BTreeMap<String, UserCreditAccount>,
    ledger_entries: &mut BTreeMap<String, CreditLedgerEntry>,
    input: SubmitRequestInput,
) -> Result<SubmitRequestMutation, ApiError> {
    validate_submit_request_input(&input)?;
    ensure_event_id_available(events, &input.event_id)?;
    let request = requests
        .get(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    if request.author_user_id != input.actor_user_id {
        return Err(ApiError::forbidden("request author required"));
    }
    if request.state != RequestState::Working {
        return Err(ApiError::conflict("request is already submitted"));
    }
    if request.git_snapshot.is_none() {
        return Err(ApiError::conflict(
            "request branch must be pushed before submit finalization",
        ));
    }
    if request.head_oid != input.expected_head_oid {
        return Err(ApiError::conflict(
            "request branch changed before submit finalization; retry submit",
        ));
    }
    validate_request_stake_rules(request.author_role, request.audience, input.stake_credits)?;

    let account_and_ledger_entry = if input.stake_credits == 0 {
        None
    } else {
        let ledger_entry_id = input
            .stake_ledger_entry_id
            .clone()
            .ok_or_else(|| ApiError::bad_request("stake ledger entry id is required"))?;
        ensure_ledger_entry_id_available(ledger_entries, &ledger_entry_id)?;
        let stake_amount = u32_to_i32(input.stake_credits)?;
        let account = accounts
            .get(&input.actor_user_id)
            .ok_or_else(|| ApiError::conflict("insufficient credits"))?;
        if account.balance_credits < input.stake_credits {
            return Err(ApiError::conflict("insufficient credits"));
        }
        let maximum_rewarded_balance = account
            .balance_credits
            .checked_add(maximum_request_reward(input.stake_credits))
            .ok_or_else(|| ApiError::bad_request("credit balance overflow"))?;
        u32_to_i32(maximum_rewarded_balance)?;
        let account = UserCreditAccount {
            user_id: account.user_id.clone(),
            balance_credits: account.balance_credits - input.stake_credits,
        };
        u32_to_i32(account.balance_credits)?;
        let entry = CreditLedgerEntry {
            id: ledger_entry_id,
            user_id: input.actor_user_id.clone(),
            request_id: Some(input.request_id.clone()),
            kind: CreditLedgerEntryKind::RequestStakeDebit,
            amount_credits: -stake_amount,
            created_at_unix: input.now_unix,
        };
        Some((account, entry))
    };

    let request = requests
        .get_mut(&input.request_id)
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    request.stake_credits = input.stake_credits;
    request.state = RequestState::Submitted;
    request.updated_at_unix = input.now_unix;
    let request = request.clone();
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::Submitted,
        body: None,
        old_head_oid: None,
        new_head_oid: Some(request.head_oid.clone()),
        created_at_unix: input.now_unix,
    };
    let account = account_and_ledger_entry
        .as_ref()
        .map(|(account, _)| account.clone());
    let ledger_entry = account_and_ledger_entry
        .as_ref()
        .map(|(_, entry)| entry.clone());
    if let Some((account, entry)) = account_and_ledger_entry {
        accounts.insert(account.user_id.clone(), account);
        ledger_entries.insert(entry.id.clone(), entry);
    }
    events.insert(event.id.clone(), event.clone());
    Ok(SubmitRequestMutation {
        request,
        event,
        account,
        ledger_entry,
    })
}

fn validate_start_request_input(input: &StartRequestInput) -> Result<(), ApiError> {
    validate_required_id("request id", &input.id)?;
    validate_required_id("repo id", &input.repo_id)?;
    validate_required_id("author user id", &input.author_user_id)?;
    validate_request_name(&input.name)?;
    if let Some(title) = &input.title {
        validate_required_id("title", title)?;
    }
    validate_required_id("base main oid", &input.base_main_oid)?;
    validate_request_audience_rules(input.author_role, input.audience)
}

fn validate_submit_request_input(input: &SubmitRequestInput) -> Result<(), ApiError> {
    validate_required_id("request id", &input.request_id)?;
    validate_required_id("actor user id", &input.actor_user_id)?;
    validate_required_id("expected head oid", &input.expected_head_oid)?;
    validate_required_id("event id", &input.event_id)?;
    Ok(())
}

fn validate_request_stake_rules(
    author_role: RequestActorRole,
    audience: RequestAudience,
    stake_credits: u32,
) -> Result<(), ApiError> {
    validate_request_audience_rules(author_role, audience)?;
    if author_role == RequestActorRole::Public && stake_credits == 0 {
        return Err(ApiError::bad_request(
            "public requests require credit stake",
        ));
    }
    if author_role != RequestActorRole::Public && stake_credits != 0 {
        return Err(ApiError::bad_request(
            "member and owner requests do not use credit stake",
        ));
    }
    Ok(())
}

fn validate_request_audience_rules(
    author_role: RequestActorRole,
    audience: RequestAudience,
) -> Result<(), ApiError> {
    if author_role == RequestActorRole::Public && audience != RequestAudience::Public {
        return Err(ApiError::bad_request(
            "public contributors can only create public requests",
        ));
    }
    Ok(())
}

pub fn validate_request_name(name: &str) -> Result<(), ApiError> {
    validate_required_id("request name", name)?;
    if name.len() > 48
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
    {
        return Err(ApiError::bad_request(
            "request name must match [a-z0-9][a-z0-9-]{0,47}",
        ));
    }
    if matches!(name, "main" | "head" | "scope") {
        return Err(ApiError::bad_request("request name is reserved"));
    }
    Ok(())
}
