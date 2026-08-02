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
    let ready = mark_request_ready(
        &working_request(RequestActorRole::Member),
        ready_input(usize::MAX),
    )
    .unwrap();
    assert_eq!(ready.request.state, RequestState::ReadyForReview);
}

#[test]
fn returning_to_working_preserves_first_publication() {
    let returned = return_request_to_working(
        &ready_request(),
        exit_input(RequestReviewExitReason::AuthorReturned, "author"),
    )
    .unwrap();
    assert_eq!(returned.request.state, RequestState::Working);
    assert_eq!(returned.request.first_ready_at_unix, Some(20));
    assert_eq!(returned.request.ready_at_unix, None);
}

#[test]
fn merge_from_ready_completes_directly_with_one_event() {
    let merged = merge_request(&ready_request(), merge_input()).unwrap();
    assert_eq!(merged.request.state, RequestState::Completed);
    assert_eq!(
        merged.request.completed_by_user_id.as_deref(),
        Some("maintainer")
    );
    assert_eq!(merged.events.len(), 1);
    assert_eq!(merged.events[0].kind, RequestEventKind::Merged);
}

#[test]
fn merge_rejects_working_and_completed_requests() {
    assert!(merge_request(&working_request(RequestActorRole::Public), merge_input()).is_err());
    let completed = merge_request(&ready_request(), merge_input())
        .unwrap()
        .request;
    assert!(merge_request(&completed, merge_input()).is_err());
}

#[test]
fn repeat_ready_cycles_preserve_first_publication() {
    let first =
        mark_request_ready(&working_request(RequestActorRole::Public), ready_input(0)).unwrap();
    let returned = return_request_to_working(
        &first.request,
        exit_input(RequestReviewExitReason::AuthorReturned, "author"),
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

fn exit_input(reason: RequestReviewExitReason, actor: &str) -> ReturnRequestToWorkingInput {
    ReturnRequestToWorkingInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_author: actor == "author",
        actor_can_mutate: true,
        reason,
        event_id: "event_working".to_string(),
        now_unix: 22,
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
