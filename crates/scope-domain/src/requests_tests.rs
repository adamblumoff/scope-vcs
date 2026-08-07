use super::requests::*;
use crate::store::{DEFAULT_GIT_FILE_MODE, RepositoryAccess, RepositoryActor, SourceBlob};
use std::collections::BTreeMap;

#[test]
fn new_request_is_an_unsubmitted_draft() {
    let mutation = start_request(&mut BTreeMap::new(), public_start_input()).unwrap();

    assert_eq!(mutation.request.state(), RequestState::Draft);
    assert!(!mutation.request.is_submitted());
    assert!(!policy_for(&mutation.request, ViewerKind::Anonymous).counts_as_open);
    assert_eq!(mutation.request.closed_at_unix, None);
    mutation.request.validate_facts().unwrap();
}

#[test]
fn started_event_identity_is_bounded_at_maximum_request_sizes() {
    let title = "t".repeat(REQUEST_TITLE_MAX_BYTES);
    let description = "d".repeat(REQUEST_DESCRIPTION_MAX_BYTES);
    let identity = request_identity_audit_fact(&title, &description).unwrap();
    let payload = RequestEventPayload::Started {
        identity: identity.clone(),
    };

    assert_eq!(identity.title_byte_count, REQUEST_TITLE_MAX_BYTES as u64);
    assert_eq!(
        identity.description_byte_count,
        REQUEST_DESCRIPTION_MAX_BYTES as u64
    );
    assert_eq!(identity.title_sha256.len(), 64);
    assert_eq!(identity.description_sha256.len(), 64);
    assert!(serde_json::to_vec(&payload).unwrap().len() < 512);
}

#[test]
fn request_name_rules_and_repository_uniqueness_remain_domain_owned() {
    for invalid in [
        "main",
        "HEAD",
        "two words",
        "nested/name",
        "-leading",
        "UPPER",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let mut input = public_start_input();
        input.name = invalid.to_string();
        assert!(start_request(&mut BTreeMap::new(), input).is_err());
    }

    let mut requests = BTreeMap::new();
    start_request(&mut requests, public_start_input()).unwrap();
    let mut duplicate = public_start_input();
    duplicate.id = "request_2".to_string();
    assert!(start_request(&mut requests, duplicate).is_err());
    assert_eq!(canonical_request_ref("fix-parser"), "refs/heads/fix-parser");
}

#[test]
fn terminal_facts_require_submission_and_are_mutually_exclusive() {
    let mut request = open_request();
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("author".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    assert_eq!(request.state(), RequestState::Closed);

    request.merged_at_unix = Some(31);
    request.merged_by_user_id = Some("maintainer".to_string());
    request.merged_head_oid = Some("head".to_string());
    request.merged_main_oid = Some("main-after".to_string());
    request.updated_at_unix = 31;
    assert!(request.validate_facts().is_err());

    let mut invalid = pushed_draft();
    invalid.closed_at_unix = Some(30);
    invalid.closed_by_user_id = Some("author".to_string());
    invalid.updated_at_unix = 30;
    assert!(invalid.validate_facts().is_err());
}

#[test]
fn open_request_edits_and_revisions_stay_open() {
    let request = open_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let revised = record_request_revision(
        &mut requests,
        &mut BTreeMap::new(),
        RecordRequestRevisionInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "head-2".to_string(),
            git_snapshot: source_blob("head-2"),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 22,
        },
    )
    .unwrap();

    assert_eq!(revised.request.state(), RequestState::Open);
    assert_eq!(revised.request.submitted_at_unix, Some(20));
}

#[test]
fn policy_keeps_drafts_private_and_open_requests_visible_and_mutable() {
    let draft = pushed_draft();
    assert!(!policy_for(&draft, ViewerKind::Anonymous).exact_visible);
    let author = policy_for(&draft, ViewerKind::Author).permissions;
    assert!(author.can_submit && author.can_push_branch && author.can_close);
    assert!(
        !policy_for(&draft, ViewerKind::Maintainer)
            .permissions
            .can_close
    );

    let open = open_request();
    assert!(policy_for(&open, ViewerKind::Anonymous).exact_visible);
    assert!(policy_for(&open, ViewerKind::Anonymous).counts_as_open);
    assert!(
        policy_for(&open, ViewerKind::Author)
            .permissions
            .can_push_branch
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_push_branch
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_merge
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_close
    );
}

#[test]
fn draft_close_deletes_and_open_close_preserves_exact_actor() {
    let draft = pushed_draft();
    let mut requests = BTreeMap::from([(draft.id.clone(), draft)]);
    let mut events = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    assert!(matches!(
        close_request(
            &mut requests,
            &mut events,
            &mut revisions,
            close_input("author", true, false),
        )
        .unwrap(),
        CloseRequestMutation::DeletedDraft { .. }
    ));
    assert!(requests.is_empty());

    let open = open_request();
    let mut requests = BTreeMap::from([(open.id.clone(), open)]);
    let mutation = close_request(
        &mut requests,
        &mut events,
        &mut revisions,
        close_input("maintainer", false, true),
    )
    .unwrap();
    let CloseRequestMutation::Closed { request, event } = mutation else {
        panic!("submitted request must remain as closed history");
    };
    assert_eq!(request.state(), RequestState::Closed);
    assert_eq!(request.closed_by_user_id.as_deref(), Some("maintainer"));
    assert_eq!(event.actor_user_id, "maintainer");
}

#[test]
fn discussion_moderation_does_not_change_request_lifecycle() {
    let request = open_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    let opened = create_request_discussion(
        &mut requests,
        &mut discussions,
        CreateRequestDiscussionInput {
            request_id: "request_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Review this invariant".to_string(),
            anchor: None,
            now_unix: 21,
        },
    )
    .unwrap();
    resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: opened.discussion.id,
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_discussion_resolved".to_string(),
            now_unix: 22,
        },
    )
    .unwrap();
    assert_eq!(requests["request_1"].state(), RequestState::Open);
}

#[test]
fn completed_private_discussion_transitions_are_rejected_before_mutation() {
    let mut request = open_request();
    request.audience = RequestAudience::Private;
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    for id in ["discussion_open", "discussion_resolved"] {
        create_request_discussion(
            &mut requests,
            &mut discussions,
            CreateRequestDiscussionInput {
                request_id: "request_1".to_string(),
                id: id.to_string(),
                actor_user_id: "author".to_string(),
                actor_can_participate: true,
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".to_string(),
                anchor: None,
                now_unix: 21,
            },
        )
        .unwrap();
    }
    resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_initial_resolve".to_string(),
            now_unix: 22,
        },
    )
    .unwrap();
    let request = requests.get_mut("request_1").unwrap();
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    let expected_requests = requests.clone();
    let expected_discussions = discussions.clone();

    let resolve_error = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_open".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_rejected_resolve".to_string(),
            now_unix: 31,
        },
    )
    .unwrap_err();
    assert_eq!(resolve_error.kind, crate::error::DomainErrorKind::Conflict);
    assert_eq!(requests, expected_requests);
    assert_eq!(discussions, expected_discussions);

    let reopen_error = reopen_request_discussion(
        &mut requests,
        &mut discussions,
        ReopenRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_rejected_reopen".to_string(),
            now_unix: 32,
        },
    )
    .unwrap_err();
    assert_eq!(reopen_error.kind, crate::error::DomainErrorKind::Conflict);
    assert_eq!(requests, expected_requests);
    assert_eq!(discussions, expected_discussions);
}

#[test]
fn completed_public_discussion_transitions_remain_allowed() {
    let request = open_request();
    let mut requests = BTreeMap::from([(request.id.clone(), request)]);
    let mut discussions = BTreeMap::new();
    for id in ["discussion_open", "discussion_resolved"] {
        create_request_discussion(
            &mut requests,
            &mut discussions,
            CreateRequestDiscussionInput {
                request_id: "request_1".to_string(),
                id: id.to_string(),
                actor_user_id: "author".to_string(),
                actor_can_participate: true,
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".to_string(),
                anchor: None,
                now_unix: 21,
            },
        )
        .unwrap();
    }
    resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_initial_resolve".to_string(),
            now_unix: 22,
        },
    )
    .unwrap();
    let request = requests.get_mut("request_1").unwrap();
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();

    let resolved = resolve_request_discussion(
        &mut requests,
        &mut discussions,
        ResolveRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_open".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_completed_resolve".to_string(),
            now_unix: 31,
        },
    )
    .unwrap();
    assert_eq!(resolved.request.state(), RequestState::Closed);
    assert_eq!(
        resolved.discussion.status,
        RequestDiscussionStatus::Resolved
    );

    let reopened = reopen_request_discussion(
        &mut requests,
        &mut discussions,
        ReopenRequestDiscussionInput {
            request_id: "request_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_is_maintainer: true,
            actor_can_transition: true,
            event_id: "event_completed_reopen".to_string(),
            now_unix: 32,
        },
    )
    .unwrap();
    assert_eq!(reopened.request.state(), RequestState::Closed);
    assert_eq!(reopened.discussion.status, RequestDiscussionStatus::Open);
}

#[derive(Clone, Copy)]
enum ViewerKind {
    Anonymous,
    Author,
    Maintainer,
}

fn policy_for(request: &Request, viewer: ViewerKind) -> RequestPolicyDecision {
    let access = match viewer {
        ViewerKind::Maintainer => maintainer_access(),
        _ => RepositoryAccess::public(),
    };
    let user_id = match viewer {
        ViewerKind::Anonymous => None,
        ViewerKind::Author => Some(request.author_user_id.as_str()),
        ViewerKind::Maintainer => Some("maintainer"),
    };
    request_policy(request, RequestViewer::new(access, user_id, false))
}

fn public_start_input() -> StartRequestInput {
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
    }
}

pub(super) fn working_request() -> Request {
    start_request(&mut BTreeMap::new(), public_start_input())
        .unwrap()
        .request
}

fn pushed_draft() -> Request {
    let mut request = working_request();
    request.head_oid = "head".to_string();
    request.git_snapshot = Some(source_blob("head"));
    request.updated_at_unix = 11;
    request
}

pub(super) fn open_request() -> Request {
    submit_request(
        &pushed_draft(),
        SubmitRequestInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_is_author: true,
            actor_can_submit: true,
            event_id: "event_submitted".to_string(),
            now_unix: 20,
        },
    )
    .unwrap()
    .request
}

fn close_input(actor: &str, actor_is_author: bool, actor_is_maintainer: bool) -> CloseRequestInput {
    CloseRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_author,
        actor_is_maintainer,
        event_id: "event_closed".to_string(),
        now_unix: 30,
    }
}

fn maintainer_access() -> RepositoryAccess {
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: true,
        can_push: true,
        can_change_file_visibility: false,
        can_apply_changes: false,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        content_ref: crate::content_ref::ContentRef::blob_sha256(git_oid),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}
