use super::{
    CreditLedgerEntry, CreditLedgerEntryKind, REQUEST_MAX_STAKE_CREDITS, Request, RequestActorRole,
    RequestAssessmentOutcome, RequestEvent, RequestEventKind, RequestEventPayload,
    RequestReviewExitReason, RequestSettlement, RequestState, UserCreditAccount, settlement_for,
    validate_assessment_body, validate_required_id,
};
use crate::error::ApiError;

pub const PUBLIC_READY_REQUEST_LIMIT: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestReviewMutation {
    pub request: Request,
    pub events: Vec<RequestEvent>,
    pub credit_account: Option<UserCreditAccount>,
    pub ledger_entries: Vec<CreditLedgerEntry>,
    pub settlement: Option<RequestSettlement>,
}

#[derive(Clone, Debug)]
pub struct MarkRequestReadyInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_author: bool,
    pub actor_can_mutate: bool,
    pub stake_credits: Option<u32>,
    pub public_ready_count: usize,
    pub ready_queue_version: u64,
    pub event_id: String,
    pub stake_ledger_entry_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ReturnRequestToWorkingInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_author: bool,
    pub actor_is_maintainer: bool,
    pub actor_can_mutate: bool,
    pub reason: RequestReviewExitReason,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct SetRequestHoldInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub held: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct AssessRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub outcome: RequestAssessmentOutcome,
    pub body_markdown: Option<String>,
    pub assessed_event_id: String,
    pub settled_event_id: Option<String>,
    pub refund_ledger_entry_id: Option<String>,
    pub reward_ledger_entry_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct MergeRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub merged_head_oid: String,
    pub merged_main_oid: String,
    pub merged_event_id: String,
    pub assessed_event_id: String,
    pub settled_event_id: Option<String>,
    pub refund_ledger_entry_id: Option<String>,
    pub reward_ledger_entry_id: Option<String>,
    pub now_unix: u64,
}

pub fn mark_request_ready(
    request: &Request,
    account: Option<&UserCreditAccount>,
    input: MarkRequestReadyInput,
) -> Result<RequestReviewMutation, ApiError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if !input.actor_is_author || request.author_user_id != input.actor_user_id {
        return Err(ApiError::forbidden(
            "only the request author can mark it ready for review",
        ));
    }
    if !input.actor_can_mutate {
        return Err(ApiError::forbidden("request mutation access required"));
    }
    if request.state != RequestState::Working {
        return Err(ApiError::conflict(
            "only working requests can be marked ready for review",
        ));
    }
    if request.git_snapshot.is_none() {
        return Err(ApiError::conflict(
            "request branch must be pushed before review",
        ));
    }

    let public = request.author_role == RequestActorRole::Public;
    let stake = if public {
        let stake = input
            .stake_credits
            .ok_or_else(|| ApiError::bad_request("review stake is required"))?;
        if !(1..=REQUEST_MAX_STAKE_CREDITS).contains(&stake) {
            return Err(ApiError::bad_request(format!(
                "review stake must be between 1 and {REQUEST_MAX_STAKE_CREDITS} credits"
            )));
        }
        if input.public_ready_count >= PUBLIC_READY_REQUEST_LIMIT {
            return Err(ApiError::conflict(format!(
                "public contributors may have at most {PUBLIC_READY_REQUEST_LIMIT} ready requests per repository"
            )));
        }
        stake
    } else {
        reject_maintainer_credit_inputs(
            account,
            input.stake_credits.is_some() || input.stake_ledger_entry_id.is_some(),
        )?;
        0
    };

    let (credit_account, ledger_entries) = if public {
        let account = require_account(request, account)?;
        if account.balance_credits < stake {
            return Err(ApiError::conflict("insufficient review credits"));
        }
        let id = require_id(
            "stake ledger entry id",
            input.stake_ledger_entry_id.as_deref(),
        )?;
        (
            Some(UserCreditAccount {
                user_id: account.user_id.clone(),
                balance_credits: account.balance_credits - stake,
            }),
            vec![ledger_entry(
                request,
                id,
                CreditLedgerEntryKind::ReviewStakeDebit,
                -to_i32(stake)?,
                input.now_unix,
            )],
        )
    } else {
        (None, Vec::new())
    };

    let mut next = request.clone();
    if input.ready_queue_version == 0 {
        return Err(ApiError::internal_message(
            "ready queue version must be positive",
        ));
    }
    next.ready_queue_version = Some(input.ready_queue_version);
    next.state = RequestState::ReadyForReview;
    next.current_stake_credits = stake;
    next.first_ready_at_unix.get_or_insert(input.now_unix);
    next.ready_at_unix = Some(input.now_unix);
    clear_hold(&mut next);
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::ReadyForReview {
        head_oid: next.head_oid.clone(),
        stake_credits: stake,
    };
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReadyForReview,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(
        next,
        vec![event],
        credit_account,
        ledger_entries,
        None,
    ))
}

pub fn return_request_to_working(
    request: &Request,
    account: Option<&UserCreditAccount>,
    input: ReturnRequestToWorkingInput,
) -> Result<RequestReviewMutation, ApiError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if request.state != RequestState::ReadyForReview {
        return Err(ApiError::conflict(
            "only ready requests can return to working",
        ));
    }
    authorize_exit(request, &input)?;
    let stake = request.current_stake_credits;
    let (credit_account, ledger_entries) =
        refund(request, account, &input.event_id, input.now_unix)?;

    let mut next = request.clone();
    next.state = RequestState::Working;
    next.current_stake_credits = 0;
    next.ready_at_unix = None;
    clear_hold(&mut next);
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::ReturnedToWorking {
        head_oid: next.head_oid.clone(),
        stake_credits: stake,
        reason: input.reason,
    };
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        RequestEventKind::ReturnedToWorking,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(
        next,
        vec![event],
        credit_account,
        ledger_entries,
        None,
    ))
}

pub fn set_request_hold(
    request: &Request,
    input: SetRequestHoldInput,
) -> Result<RequestReviewMutation, ApiError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("request event id", &input.event_id)?;
    if !input.actor_is_maintainer {
        return Err(ApiError::forbidden("repo maintainer required"));
    }
    if request.state != RequestState::ReadyForReview {
        return Err(ApiError::conflict("only ready requests can be held"));
    }
    if request.held_at_unix.is_some() == input.held {
        return Ok(mutation(
            request.clone(),
            Vec::new(),
            None,
            Vec::new(),
            None,
        ));
    }

    let mut next = request.clone();
    let (kind, payload) = if input.held {
        next.held_at_unix = Some(input.now_unix);
        // Provenance only: any maintainer may release the group hold.
        next.held_by_user_id = Some(input.actor_user_id.clone());
        (
            RequestEventKind::Held,
            RequestEventPayload::Held {
                head_oid: next.head_oid.clone(),
            },
        )
    } else {
        clear_hold(&mut next);
        (
            RequestEventKind::HoldReleased,
            RequestEventPayload::HoldReleased {
                head_oid: next.head_oid.clone(),
            },
        )
    };
    next.updated_at_unix = input.now_unix;
    let event = append_event(
        &mut next,
        input.event_id,
        input.actor_user_id,
        kind,
        payload,
        input.now_unix,
    )?;
    next.validate_facts()?;
    Ok(mutation(next, vec![event], None, Vec::new(), None))
}

pub fn assess_request(
    request: &Request,
    account: Option<&UserCreditAccount>,
    input: AssessRequestInput,
) -> Result<RequestReviewMutation, ApiError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("assessed event id", &input.assessed_event_id)?;
    if !input.actor_is_maintainer {
        return Err(ApiError::forbidden("repo maintainer required"));
    }
    if request.state != RequestState::ReadyForReview {
        return Err(ApiError::conflict("only ready requests can be assessed"));
    }
    validate_assessment_body(input.outcome, input.body_markdown.as_deref())?;
    let settlement = settlement_for(request.current_stake_credits, input.outcome, input.now_unix);
    let (credit_account, ledger_entries) = settle(
        request,
        account,
        &settlement,
        input.refund_ledger_entry_id.as_deref(),
        input.reward_ledger_entry_id.as_deref(),
        input.now_unix,
    )?;

    let stake = request.current_stake_credits;
    let mut next = request.clone();
    next.state = RequestState::Completed;
    next.current_stake_credits = 0;
    next.ready_at_unix = None;
    clear_hold(&mut next);
    next.assessment_outcome = Some(input.outcome);
    next.assessment_body_markdown = input.body_markdown.clone();
    next.assessed_at_unix = Some(input.now_unix);
    next.assessed_by_user_id = Some(input.actor_user_id.clone());
    next.completed_at_unix = Some(input.now_unix);
    next.completed_by_user_id = Some(input.actor_user_id.clone());
    next.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::Assessed {
        head_oid: next.head_oid.clone(),
        outcome: input.outcome,
        body_markdown: input.body_markdown,
        stake_credits: stake,
    };
    let assessed = append_event(
        &mut next,
        input.assessed_event_id,
        input.actor_user_id.clone(),
        RequestEventKind::Assessed,
        payload,
        input.now_unix,
    )?;
    let mut events = vec![assessed];
    if stake > 0 {
        let id = require_id("settled event id", input.settled_event_id.as_deref())?;
        let settled_payload = RequestEventPayload::Settled {
            settlement: settlement.clone(),
        };
        events.push(append_event(
            &mut next,
            id,
            input.actor_user_id,
            RequestEventKind::Settled,
            settled_payload,
            input.now_unix,
        )?);
    } else {
        reject_maintainer_credit_inputs(
            account,
            input.settled_event_id.is_some()
                || input.refund_ledger_entry_id.is_some()
                || input.reward_ledger_entry_id.is_some(),
        )?;
    }
    next.validate_facts()?;
    Ok(mutation(
        next,
        events,
        credit_account,
        ledger_entries,
        (stake > 0).then_some(settlement),
    ))
}

pub fn merge_request(
    request: &Request,
    account: Option<&UserCreditAccount>,
    input: MergeRequestInput,
) -> Result<RequestReviewMutation, ApiError> {
    validate_command(request, &input.request_id, &input.actor_user_id)?;
    validate_required_id("merged event id", &input.merged_event_id)?;
    validate_required_id("merged head oid", &input.merged_head_oid)?;
    validate_required_id("merged main oid", &input.merged_main_oid)?;
    if !input.actor_is_maintainer {
        return Err(ApiError::forbidden("repo maintainer required"));
    }
    if request.merged_at_unix.is_some() {
        return Err(ApiError::conflict("request is already merged"));
    }
    if input.merged_head_oid != request.head_oid {
        return Err(ApiError::conflict(
            "request branch changed before merge completed",
        ));
    }

    let mut result = match request.state {
        RequestState::ReadyForReview => assess_request(
            request,
            account,
            AssessRequestInput {
                request_id: input.request_id.clone(),
                actor_user_id: input.actor_user_id.clone(),
                actor_is_maintainer: true,
                outcome: RequestAssessmentOutcome::Accepted,
                body_markdown: None,
                assessed_event_id: input.assessed_event_id,
                settled_event_id: input.settled_event_id,
                refund_ledger_entry_id: input.refund_ledger_entry_id,
                reward_ledger_entry_id: input.reward_ledger_entry_id,
                now_unix: input.now_unix,
            },
        )?,
        RequestState::Completed
            if request.assessment_outcome == Some(RequestAssessmentOutcome::Accepted) =>
        {
            reject_maintainer_credit_inputs(
                account,
                input.settled_event_id.is_some()
                    || input.refund_ledger_entry_id.is_some()
                    || input.reward_ledger_entry_id.is_some(),
            )?;
            mutation(request.clone(), Vec::new(), None, Vec::new(), None)
        }
        RequestState::Completed => {
            return Err(ApiError::conflict(
                "only accepted completed requests can be merged",
            ));
        }
        RequestState::Working => {
            return Err(ApiError::conflict(
                "only ready or accepted requests can be merged",
            ));
        }
    };

    result.request.merged_at_unix = Some(input.now_unix);
    result.request.merged_by_user_id = Some(input.actor_user_id.clone());
    result.request.merged_head_oid = Some(input.merged_head_oid.clone());
    result.request.merged_main_oid = Some(input.merged_main_oid.clone());
    result.request.updated_at_unix = input.now_unix;
    let payload = RequestEventPayload::Merged {
        head_oid: input.merged_head_oid,
        main_oid: input.merged_main_oid,
    };
    result.events.push(append_event(
        &mut result.request,
        input.merged_event_id,
        input.actor_user_id,
        RequestEventKind::Merged,
        payload,
        input.now_unix,
    )?);
    result.request.validate_facts()?;
    Ok(result)
}

fn authorize_exit(request: &Request, input: &ReturnRequestToWorkingInput) -> Result<(), ApiError> {
    match input.reason {
        RequestReviewExitReason::AuthorReturned => {
            if !input.actor_is_author || request.author_user_id != input.actor_user_id {
                return Err(ApiError::forbidden(
                    "only the request author can return it to working",
                ));
            }
            if !input.actor_can_mutate {
                return Err(ApiError::forbidden("request mutation access required"));
            }
            if request.held_at_unix.is_some() {
                return Err(ApiError::conflict(
                    "held request cannot be returned by its author",
                ));
            }
        }
        RequestReviewExitReason::ChangesRequested if !input.actor_is_maintainer => {
            return Err(ApiError::forbidden("repo maintainer required"));
        }
        RequestReviewExitReason::RevisionPushed | RequestReviewExitReason::ContentEdited => {
            if !input.actor_can_mutate {
                return Err(ApiError::forbidden("request mutation access required"));
            }
            if request.held_at_unix.is_some() && !input.actor_is_maintainer {
                return Err(ApiError::conflict("request cannot be changed while held"));
            }
        }
        RequestReviewExitReason::ChangesRequested => {}
    }
    Ok(())
}

fn refund(
    request: &Request,
    account: Option<&UserCreditAccount>,
    event_id: &str,
    now: u64,
) -> Result<(Option<UserCreditAccount>, Vec<CreditLedgerEntry>), ApiError> {
    let stake = request.current_stake_credits;
    if stake == 0 {
        reject_maintainer_credit_inputs(account, false)?;
        return Ok((None, Vec::new()));
    }
    let account = require_account(request, account)?;
    let balance = account
        .balance_credits
        .checked_add(stake)
        .ok_or_else(|| ApiError::conflict("credit balance overflow"))?;
    let entry = ledger_entry(
        request,
        format!("{event_id}:stake-refund"),
        CreditLedgerEntryKind::ReviewStakeRefund,
        to_i32(stake)?,
        now,
    );
    Ok((
        Some(UserCreditAccount {
            user_id: account.user_id.clone(),
            balance_credits: balance,
        }),
        vec![entry],
    ))
}

fn settle(
    request: &Request,
    account: Option<&UserCreditAccount>,
    settlement: &RequestSettlement,
    refund_id: Option<&str>,
    reward_id: Option<&str>,
    now: u64,
) -> Result<(Option<UserCreditAccount>, Vec<CreditLedgerEntry>), ApiError> {
    if settlement.stake_credits == 0 {
        reject_maintainer_credit_inputs(account, refund_id.is_some() || reward_id.is_some())?;
        return Ok((None, Vec::new()));
    }
    let account = require_account(request, account)?;
    let credit = settlement
        .refunded_credits
        .checked_add(settlement.reward_credits)
        .ok_or_else(|| ApiError::conflict("credit settlement overflow"))?;
    let balance = account
        .balance_credits
        .checked_add(credit)
        .ok_or_else(|| ApiError::conflict("credit balance overflow"))?;
    let mut entries = Vec::new();
    if settlement.refunded_credits > 0 {
        entries.push(ledger_entry(
            request,
            require_id("refund ledger entry id", refund_id)?,
            CreditLedgerEntryKind::ReviewStakeRefund,
            to_i32(settlement.refunded_credits)?,
            now,
        ));
    } else if refund_id.is_some() {
        return Err(ApiError::bad_request(
            "rejected assessment does not refund credits",
        ));
    }
    if settlement.reward_credits > 0 {
        entries.push(ledger_entry(
            request,
            require_id("reward ledger entry id", reward_id)?,
            CreditLedgerEntryKind::AssessmentReward,
            to_i32(settlement.reward_credits)?,
            now,
        ));
    } else if reward_id.is_some() {
        return Err(ApiError::bad_request(
            "assessment outcome does not reward credits",
        ));
    }
    Ok((
        Some(UserCreditAccount {
            user_id: account.user_id.clone(),
            balance_credits: balance,
        }),
        entries,
    ))
}

fn validate_command(request: &Request, request_id: &str, actor: &str) -> Result<(), ApiError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("actor user id", actor)?;
    if request.id != request_id {
        Err(ApiError::not_found("request not found"))
    } else {
        Ok(())
    }
}

fn append_event(
    request: &mut Request,
    id: String,
    actor: String,
    kind: RequestEventKind,
    payload: RequestEventPayload,
    now: u64,
) -> Result<RequestEvent, ApiError> {
    validate_required_id("request event id", &id)?;
    request.activity_version = request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("request activity version overflow"))?;
    Ok(RequestEvent {
        id,
        request_id: request.id.clone(),
        actor_user_id: actor,
        kind,
        position: request.activity_version,
        payload,
        created_at_unix: now,
    })
}

fn mutation(
    request: Request,
    events: Vec<RequestEvent>,
    credit_account: Option<UserCreditAccount>,
    ledger_entries: Vec<CreditLedgerEntry>,
    settlement: Option<RequestSettlement>,
) -> RequestReviewMutation {
    RequestReviewMutation {
        request,
        events,
        credit_account,
        ledger_entries,
        settlement,
    }
}

fn ledger_entry(
    request: &Request,
    id: String,
    kind: CreditLedgerEntryKind,
    amount: i32,
    now: u64,
) -> CreditLedgerEntry {
    CreditLedgerEntry {
        id,
        user_id: request.author_user_id.clone(),
        request_id: Some(request.id.clone()),
        kind,
        amount_credits: amount,
        created_at_unix: now,
    }
}

fn require_account<'a>(
    request: &Request,
    account: Option<&'a UserCreditAccount>,
) -> Result<&'a UserCreditAccount, ApiError> {
    let account =
        account.ok_or_else(|| ApiError::conflict("request author credit account is missing"))?;
    if account.user_id != request.author_user_id {
        Err(ApiError::conflict(
            "credit account does not belong to the request author",
        ))
    } else {
        Ok(account)
    }
}

fn reject_maintainer_credit_inputs(
    account: Option<&UserCreditAccount>,
    has_ids: bool,
) -> Result<(), ApiError> {
    if account.is_some() || has_ids {
        Err(ApiError::bad_request(
            "maintainer-authored requests do not use review credits",
        ))
    } else {
        Ok(())
    }
}

fn require_id(label: &str, id: Option<&str>) -> Result<String, ApiError> {
    let id = id.ok_or_else(|| ApiError::bad_request(format!("{label} is required")))?;
    validate_required_id(label, id)?;
    Ok(id.to_string())
}

fn to_i32(value: u32) -> Result<i32, ApiError> {
    i32::try_from(value).map_err(|_| ApiError::conflict("credit amount exceeds i32 range"))
}

fn clear_hold(request: &mut Request) {
    request.held_at_unix = None;
    request.held_by_user_id = None;
}
