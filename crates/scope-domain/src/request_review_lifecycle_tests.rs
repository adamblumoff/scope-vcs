use super::requests::*;
use crate::store::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn public_ready_is_free_and_enforces_concurrency_cap() {
    let request = working_request(RequestActorRole::Public);
    let ready = mark_request_ready(&request, ready_input(2)).unwrap();
    assert_eq!(ready.request.state, RequestState::ReadyForReview);
    assert_eq!(ready.request.first_ready_at_unix, Some(20));
    assert_eq!(ready.request.ready_at_unix, Some(20));
    assert!(matches!(
        ready.events[0].payload,
        RequestEventPayload::ReadyForReview { .. }
    ));

    assert!(
        mark_request_ready(&request, ready_input(PUBLIC_READY_REQUEST_LIMIT))
            .unwrap_err()
            .message
            .contains("at most 3")
    );
}

#[test]
fn maintainer_authored_requests_are_not_subject_to_public_cap() {
    let request = working_request(RequestActorRole::Member);
    let ready = mark_request_ready(&request, ready_input(usize::MAX)).unwrap();
    assert_eq!(ready.request.state, RequestState::ReadyForReview);
}

#[test]
fn returning_to_working_clears_hold_and_preserves_first_publication() {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer-a".to_string());
    request.updated_at_unix = 21;
    let returned = return_request_to_working(
        &request,
        exit_input(
            RequestReviewExitReason::ChangesRequested,
            "maintainer-b",
            true,
        ),
    )
    .unwrap();
    assert_eq!(returned.request.state, RequestState::Working);
    assert_eq!(returned.request.first_ready_at_unix, Some(20));
    assert_eq!(returned.request.ready_at_unix, None);
    assert_eq!(returned.request.held_at_unix, None);
}

#[test]
fn held_review_blocks_author_but_maintainer_can_invalidate_it() {
    let mut request = ready_request();
    request.held_at_unix = Some(21);
    request.held_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 21;
    assert!(
        return_request_to_working(
            &request,
            exit_input(RequestReviewExitReason::RevisionPushed, "author", false),
        )
        .unwrap_err()
        .message
        .contains("held")
    );
    let returned = return_request_to_working(
        &request,
        exit_input(RequestReviewExitReason::ContentEdited, "maintainer", true),
    )
    .unwrap();
    assert_eq!(returned.request.state, RequestState::Working);
}

#[test]
fn hold_is_maintainer_controlled_and_idempotent() {
    let held = set_request_hold(&ready_request(), hold_input("maintainer-a", true, 21)).unwrap();
    assert_eq!(held.events.len(), 1);
    let unchanged = set_request_hold(&held.request, hold_input("maintainer-b", true, 22)).unwrap();
    assert!(unchanged.events.is_empty());
    let released = set_request_hold(&held.request, hold_input("maintainer-b", false, 22)).unwrap();
    assert_eq!(released.request.held_at_unix, None);
}

#[test]
fn assessment_completes_once_and_rejected_requires_reason() {
    for outcome in [
        RequestAssessmentOutcome::Accepted,
        RequestAssessmentOutcome::Neutral,
        RequestAssessmentOutcome::Rejected,
    ] {
        let assessed = assess_request(&ready_request(), assessment_input(outcome)).unwrap();
        assert_eq!(assessed.request.state, RequestState::Completed);
        assert_eq!(assessed.request.assessment_outcome, Some(outcome));
        assert_eq!(assessed.events.len(), 1);
        assert!(assess_request(&assessed.request, assessment_input(outcome)).is_err());
    }

    let mut invalid = assessment_input(RequestAssessmentOutcome::Rejected);
    invalid.body_markdown = Some("  ".to_string());
    assert!(assess_request(&ready_request(), invalid).is_err());
}

#[test]
fn merge_from_ready_records_assessment_then_merge() {
    let merged = merge_request(&ready_request(), merge_input()).unwrap();
    assert_eq!(merged.request.state, RequestState::Completed);
    assert_eq!(
        merged.request.assessment_outcome,
        Some(RequestAssessmentOutcome::Accepted)
    );
    assert_eq!(merged.events.len(), 2);
    assert_eq!(merged.events[1].kind, RequestEventKind::Merged);
}

#[test]
fn repeat_ready_cycles_preserve_first_publication() {
    let first =
        mark_request_ready(&working_request(RequestActorRole::Public), ready_input(0)).unwrap();
    let returned = return_request_to_working(
        &first.request,
        exit_input(RequestReviewExitReason::AuthorReturned, "author", false),
    )
    .unwrap();
    let mut input = ready_input(0);
    input.event_id = "event_ready_2".to_string();
    input.now_unix = 22;
    let second = mark_request_ready(&returned.request, input).unwrap();
    assert_eq!(second.request.first_ready_at_unix, Some(20));
    assert_eq!(second.request.ready_at_unix, Some(22));
}

fn ready_input(count: usize) -> MarkRequestReadyInput {
    MarkRequestReadyInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_is_author: true,
        actor_can_mutate: true,
        public_ready_count: count,
        event_id: "event_ready".to_string(),
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

fn hold_input(actor: &str, held: bool, now_unix: u64) -> SetRequestHoldInput {
    SetRequestHoldInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_maintainer: true,
        held,
        event_id: format!("event_hold_{held}_{now_unix}"),
        now_unix,
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
        now_unix: 30,
    }
}

fn merge_input() -> MergeRequestInput {
    MergeRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "maintainer".to_string(),
        actor_is_maintainer: true,
        merged_head_oid: "head".to_string(),
        merged_main_oid: "main-after".to_string(),
        merged_event_id: "event_merged".to_string(),
        assessed_event_id: "event_assessed".to_string(),
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
        content_ref: crate::content_ref::ContentRef::git_bundle_sha256("head"),
        sha256: "sha256-head".to_string(),
        git_oid: "head".to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    });
    request.updated_at_unix = 11;
    request
}

fn ready_request() -> Request {
    mark_request_ready(&working_request(RequestActorRole::Public), ready_input(0))
        .unwrap()
        .request
}
