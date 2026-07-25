use super::requests::*;
use crate::domain::store::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn public_ready_debits_stake_and_enforces_range_balance_and_cap() {
    let request = working_request(RequestActorRole::Public);
    let credits = account(100);
    let result = mark_request_ready(&request, Some(&credits), ready_input(25, 2)).unwrap();
    assert_eq!(result.request.state, RequestState::ReadyForReview);
    assert_eq!(result.request.current_stake_credits, 25);
    assert_eq!(result.credit_account.unwrap().balance_credits, 75);
    assert_eq!(result.ledger_entries[0].amount_credits, -25);
    assert_eq!(
        result.ledger_entries[0].kind,
        CreditLedgerEntryKind::ReviewStakeDebit
    );

    for stake in [0, 26] {
        assert!(mark_request_ready(&request, Some(&credits), ready_input(stake, 0)).is_err());
    }
    assert!(
        mark_request_ready(&request, Some(&account(2)), ready_input(3, 0))
            .unwrap_err()
            .message
            .contains("insufficient")
    );
    assert!(
        mark_request_ready(&request, Some(&credits), ready_input(1, 3))
            .unwrap_err()
            .message
            .contains("at most 3")
    );
}

#[test]
fn maintainer_authored_requests_are_unlimited_and_never_use_credits() {
    let request = working_request(RequestActorRole::Member);
    let mut input = ready_input(1, usize::MAX);
    input.stake_credits = None;
    input.stake_ledger_entry_id = None;
    let result = mark_request_ready(&request, None, input).unwrap();
    assert_eq!(result.request.current_stake_credits, 0);
    assert!(result.credit_account.is_none());
    assert!(result.ledger_entries.is_empty());
}

#[test]
fn returning_to_working_refunds_once_clears_hold_and_preserves_publication() {
    let mut request = ready_request(10);
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer-a".to_string());
    request.updated_at_unix = 21;
    let first_ready_at = request.first_ready_at_unix;
    let result = return_request_to_working(
        &request,
        Some(&account(90)),
        exit_input(
            RequestReviewExitReason::ChangesRequested,
            "maintainer-b",
            true,
        ),
    )
    .unwrap();
    assert_eq!(result.request.state, RequestState::Working);
    assert_eq!(result.request.first_ready_at_unix, first_ready_at);
    assert_eq!(result.request.ready_at_unix, None);
    assert_eq!(result.request.held_at_unix, None);
    assert_eq!(result.credit_account.unwrap().balance_credits, 100);
    assert_eq!(
        result.ledger_entries[0].kind,
        CreditLedgerEntryKind::ReviewStakeRefund
    );
    assert!(
        return_request_to_working(
            &result.request,
            Some(&account(100)),
            exit_input(RequestReviewExitReason::AuthorReturned, "author", false)
        )
        .is_err()
    );
}

#[test]
fn held_review_blocks_author_but_maintainer_can_invalidate_it() {
    let mut request = ready_request(10);
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    assert!(
        return_request_to_working(
            &request,
            Some(&account(90)),
            exit_input(RequestReviewExitReason::RevisionPushed, "author", false)
        )
        .unwrap_err()
        .message
        .contains("held")
    );
    let result = return_request_to_working(
        &request,
        Some(&account(90)),
        exit_input(RequestReviewExitReason::ContentEdited, "maintainer", true),
    )
    .unwrap();
    assert_eq!(result.request.state, RequestState::Working);
    assert_eq!(result.request.held_at_unix, None);
}

#[test]
fn hold_is_group_controlled_and_idempotent() {
    let request = ready_request(10);
    let held = set_request_hold(&request, hold_input("maintainer-a", true, 21)).unwrap();
    assert_eq!(held.events.len(), 1);
    let unchanged = set_request_hold(&held.request, hold_input("maintainer-b", true, 22)).unwrap();
    assert!(unchanged.events.is_empty());
    let released = set_request_hold(&held.request, hold_input("maintainer-b", false, 22)).unwrap();
    assert_eq!(released.request.held_at_unix, None);
    assert_eq!(released.events[0].kind, RequestEventKind::HoldReleased);
}

#[test]
fn assessment_settles_exactly_and_is_immutable() {
    let cases = [
        (RequestAssessmentOutcome::Accepted, 110, 2),
        (RequestAssessmentOutcome::Neutral, 100, 1),
        (RequestAssessmentOutcome::Rejected, 90, 0),
    ];
    for (outcome, expected_balance, expected_entries) in cases {
        let request = ready_request(10);
        let result =
            assess_request(&request, Some(&account(90)), assessment_input(outcome)).unwrap();
        assert_eq!(result.request.state, RequestState::Completed);
        assert_eq!(result.request.assessment_outcome, Some(outcome));
        assert_eq!(
            result.credit_account.unwrap().balance_credits,
            expected_balance
        );
        assert_eq!(result.ledger_entries.len(), expected_entries);
        assert_eq!(
            result.events.last().unwrap().kind,
            RequestEventKind::Settled
        );
        assert!(
            assess_request(
                &result.request,
                None,
                assessment_input(RequestAssessmentOutcome::Accepted)
            )
            .is_err()
        );
    }
}

#[test]
fn rejected_assessment_requires_written_reason() {
    let mut input = assessment_input(RequestAssessmentOutcome::Rejected);
    input.body_markdown = Some("  ".to_string());
    assert!(
        assess_request(&ready_request(10), Some(&account(90)), input)
            .unwrap_err()
            .message
            .contains("written reason")
    );
}

#[test]
fn assessment_body_rejects_oversized_markdown() {
    let mut input = assessment_input(RequestAssessmentOutcome::Accepted);
    input.body_markdown = Some("x".repeat(REQUEST_ASSESSMENT_BODY_MAX_BYTES + 1));
    assert!(
        assess_request(&ready_request(10), Some(&account(90)), input)
            .unwrap_err()
            .message
            .contains("assessment body exceeds")
    );
}

#[test]
fn merge_from_ready_accepts_once_and_later_accepted_merge_has_no_credit_effect() {
    let merged = merge_request(&ready_request(10), Some(&account(90)), merge_input(false)).unwrap();
    assert_eq!(merged.request.state, RequestState::Completed);
    assert_eq!(
        merged.request.assessment_outcome,
        Some(RequestAssessmentOutcome::Accepted)
    );
    assert_eq!(merged.credit_account.unwrap().balance_credits, 110);
    assert_eq!(merged.ledger_entries.len(), 2);
    assert_eq!(merged.events.last().unwrap().kind, RequestEventKind::Merged);

    let accepted = assess_request(
        &ready_request(10),
        Some(&account(90)),
        assessment_input(RequestAssessmentOutcome::Accepted),
    )
    .unwrap()
    .request;
    let later = merge_request(&accepted, None, merge_input(true)).unwrap();
    assert!(later.credit_account.is_none());
    assert!(later.ledger_entries.is_empty());
    assert!(later.settlement.is_none());
    assert_eq!(later.events.len(), 1);
}

#[test]
fn repeat_ready_cycles_keep_distinct_stake_facts() {
    let first = mark_request_ready(
        &working_request(RequestActorRole::Public),
        Some(&account(100)),
        ready_input(5, 0),
    )
    .unwrap();
    let working = return_request_to_working(
        &first.request,
        first.credit_account.as_ref(),
        exit_input(RequestReviewExitReason::AuthorReturned, "author", false),
    )
    .unwrap();
    let mut input = ready_input(20, 0);
    input.event_id = "event_ready_2".to_string();
    input.stake_ledger_entry_id = Some("ledger_stake_2".to_string());
    input.now_unix = 22;
    let second =
        mark_request_ready(&working.request, working.credit_account.as_ref(), input).unwrap();
    assert_eq!(second.request.first_ready_at_unix, Some(20));
    assert_eq!(second.request.ready_at_unix, Some(22));
    assert_eq!(second.request.current_stake_credits, 20);
    assert!(matches!(
        first.events[0].payload,
        RequestEventPayload::ReadyForReview {
            stake_credits: 5,
            ..
        }
    ));
    assert!(matches!(
        working.events[0].payload,
        RequestEventPayload::ReturnedToWorking {
            stake_credits: 5,
            ..
        }
    ));
    assert!(matches!(
        second.events[0].payload,
        RequestEventPayload::ReadyForReview {
            stake_credits: 20,
            ..
        }
    ));
}

fn ready_input(stake: u32, count: usize) -> MarkRequestReadyInput {
    MarkRequestReadyInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_is_author: true,
        actor_can_mutate: true,
        stake_credits: Some(stake),
        public_ready_count: count,
        ready_queue_version: 1,
        event_id: "event_ready".to_string(),
        stake_ledger_entry_id: Some("ledger_stake".to_string()),
        now_unix: 20,
    }
}

fn exit_input(
    reason: RequestReviewExitReason,
    actor: &str,
    maintainer: bool,
) -> ReturnRequestToWorkingInput {
    ReturnRequestToWorkingInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_author: actor == "author",
        actor_is_maintainer: maintainer,
        actor_can_mutate: true,
        reason,
        event_id: "event_working".to_string(),
        now_unix: 22,
    }
}

fn hold_input(actor: &str, held: bool, now: u64) -> SetRequestHoldInput {
    SetRequestHoldInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_maintainer: true,
        held,
        event_id: format!("event_hold_{held}_{now}"),
        now_unix: now,
    }
}

fn assessment_input(outcome: RequestAssessmentOutcome) -> AssessRequestInput {
    AssessRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "maintainer".to_string(),
        actor_is_maintainer: true,
        outcome,
        body_markdown: (outcome == RequestAssessmentOutcome::Rejected)
            .then(|| "Concrete rejection reason".to_string()),
        assessed_event_id: "event_assessed".to_string(),
        settled_event_id: Some("event_settled".to_string()),
        refund_ledger_entry_id: (outcome != RequestAssessmentOutcome::Rejected)
            .then(|| "ledger_refund".to_string()),
        reward_ledger_entry_id: (outcome == RequestAssessmentOutcome::Accepted)
            .then(|| "ledger_reward".to_string()),
        now_unix: 30,
    }
}

fn merge_input(later: bool) -> MergeRequestInput {
    MergeRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "maintainer".to_string(),
        actor_is_maintainer: true,
        merged_head_oid: "head".to_string(),
        merged_main_oid: "main-after".to_string(),
        merged_event_id: "event_merged".to_string(),
        assessed_event_id: "event_assessed".to_string(),
        settled_event_id: (!later).then(|| "event_settled".to_string()),
        refund_ledger_entry_id: (!later).then(|| "ledger_refund".to_string()),
        reward_ledger_entry_id: (!later).then(|| "ledger_reward".to_string()),
        now_unix: 31,
    }
}

fn working_request(role: RequestActorRole) -> Request {
    let mut request = start_request(
        &mut BTreeMap::new(),
        StartRequestInput {
            id: "request_1".to_string(),
            repo_id: "owner/repo".to_string(),
            name: "fix-parser".to_string(),
            author_user_id: "author".to_string(),
            title: Some("Fix parser".to_string()),
            author_role: role,
            audience: RequestAudience::Public,
            base_main_oid: "base".to_string(),
            event_id: "event_started".to_string(),
            now_unix: 10,
        },
    )
    .unwrap()
    .request;
    request.head_oid = "head".to_string();
    request.git_snapshot = Some(SourceBlob {
        object_key: "objects/head".to_string(),
        sha256: "sha256-head".to_string(),
        git_oid: "head".to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    });
    request.updated_at_unix = 11;
    request
}

fn ready_request(stake: u32) -> Request {
    mark_request_ready(
        &working_request(RequestActorRole::Public),
        Some(&account(100)),
        ready_input(stake, 0),
    )
    .unwrap()
    .request
}

fn account(balance_credits: u32) -> UserCreditAccount {
    UserCreditAccount {
        user_id: "author".to_string(),
        balance_credits,
    }
}
