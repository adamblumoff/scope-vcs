use super::requests::*;
use crate::content::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn author_submits_a_pushed_draft_exactly_once() {
    let request = pushed_draft(RequestActorRole::Public);
    let submitted = submit_request(&request, submit_input()).unwrap();

    assert_eq!(submitted.request.state(), RequestState::Open);
    assert_eq!(submitted.request.submitted_at_unix, Some(20));
    assert_eq!(submitted.events.len(), 1);
    assert!(matches!(
        submitted.events[0].payload,
        RequestEventPayload::Submitted { ref head_oid } if head_oid == "head"
    ));
    assert!(submit_request(&submitted.request, submit_input()).is_err());
}

#[test]
fn submission_requires_the_author_and_a_pushed_snapshot() {
    let mut input = submit_input();
    input.actor_user_id = "maintainer".to_string();
    input.actor_is_author = false;
    assert!(submit_request(&pushed_draft(RequestActorRole::Member), input).is_err());

    let draft = start_request(
        &mut BTreeMap::new(),
        StartRequestInput {
            id: "request_1".to_string(),
            repo_id: "owner/repo".to_string(),
            name: "fix-parser".to_string(),
            author_user_id: "author".to_string(),
            title: Some("Fix parser".to_string()),
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid: "base".to_string(),
            event_id: "event_started".to_string(),
            now_unix: 10,
        },
    )
    .unwrap()
    .request;
    assert!(submit_request(&draft, submit_input()).is_err());
}

#[test]
fn merge_is_maintainer_only_and_terminal() {
    let open = open_request();
    let mut denied = merge_input();
    denied.actor_is_maintainer = false;
    assert!(merge_request(&open, denied).is_err());

    let merged = merge_request(&open, merge_input()).unwrap();
    assert_eq!(merged.request.state(), RequestState::Merged);
    assert_eq!(
        merged.request.merged_by_user_id.as_deref(),
        Some("maintainer")
    );
    assert_eq!(merged.events.len(), 1);
    assert_eq!(merged.events[0].kind, RequestEventKind::Merged);
    assert!(merge_request(&merged.request, merge_input()).is_err());
}

fn submit_input() -> SubmitRequestInput {
    SubmitRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_is_author: true,
        actor_can_submit: true,
        event_id: "event_submitted".to_string(),
        now_unix: 20,
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

fn pushed_draft(role: RequestActorRole) -> Request {
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

fn open_request() -> Request {
    submit_request(&pushed_draft(RequestActorRole::Public), submit_input())
        .unwrap()
        .request
}
