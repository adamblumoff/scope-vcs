mod discussion_commands;

use super::super::MetadataStore;
use super::super::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand, DiscussionTransition,
    ReopenAndReplyToRequestDiscussionCommand, TransitionRequestDiscussionCommand,
};
use super::*;
use scope_domain::{
    account::UserAccount,
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    requests::{
        RecordRequestRevisionInput, RequestActorRole, RequestAudience, RequestDiscussionAnchor,
        RequestDiscussionStatus, RequestState,
    },
};

#[tokio::test]
async fn revision_window_bounds_recent_rows_and_keeps_an_explicit_older_revision() {
    let store = postgres_store();
    start_public_request(&store).await;

    let mut old_head = "head".to_string();
    let mut revision_ids = Vec::new();
    for number in 1..=3 {
        let new_head = format!("head_{number}");
        let mutation = store
            .requests()
            .record_request_revision(
                RecordRequestRevisionInput {
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_public".to_string(),
                    actor_can_edit: true,
                    expected_old_head_oid: Some(old_head),
                    new_head_oid: new_head.clone(),
                    git_snapshot: source_blob(&new_head),
                    event_id: format!("revision_event_{number}"),
                    body: None,
                    now_unix: 3 + number,
                },
                &super::super::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        old_head = new_head;
        revision_ids.push(mutation.revision.id);
    }

    let window = store
        .requests()
        .request_revision_window("req_1", Some(&revision_ids[0]), 1)
        .await
        .unwrap();
    assert!(window.has_earlier_revisions);
    assert_eq!(
        window
            .revisions
            .iter()
            .map(|revision| revision.id.as_str())
            .collect::<Vec<_>>(),
        [revision_ids[0].as_str(), revision_ids[2].as_str()]
    );
}

#[tokio::test]
async fn discussion_transactions_are_idempotent_atomic_and_self_read() {
    let store = postgres_store();
    start_public_request(&store).await;

    let first = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            anchor: None,
            now_unix: 10,
        })
        .await
        .unwrap();
    assert!(first.created);
    let mut request = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    request.submitted_at_unix = Some(11);
    request.updated_at_unix = 11;
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_before_retry".to_string(),
            actor_user_id: "user_owner".to_string(),
            client_reply_id: "client_before_retry".to_string(),
            body_markdown: "Maintainer reply".to_string(),
            reply_to_reply_id: None,
            now_unix: 11,
        })
        .await
        .unwrap();
    let retried = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            id: "discussion_retry_id".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            anchor: None,
            now_unix: 12,
        })
        .await
        .unwrap();
    assert!(!retried.created);
    assert_eq!(retried.discussion.id, first.discussion.id);
    assert_eq!(
        retried.read_state.read_through_position,
        first.discussion.opened_position
    );
    let unread_after_retry = store
        .requests()
        .request_discussion("req_1", &first.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unread_after_retry.0.unread_count, 1);

    let resolved = store
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_discussion_resolved".to_string(),
            now_unix: 13,
            transition: DiscussionTransition::Resolve,
        })
        .await
        .unwrap();
    assert_eq!(resolved.status, RequestDiscussionStatus::Resolved);
    let reply_error = store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_rejected".to_string(),
            actor_user_id: "user_public".to_string(),
            client_reply_id: "client_rejected".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            now_unix: 14,
        })
        .await
        .unwrap_err();
    assert_eq!(reply_error.kind, crate::error::PostgresErrorKind::Conflict);

    let reopened = store
        .requests()
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id,
            reply_id: "reply_1".to_string(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_discussion_reopened".to_string(),
            client_reply_id: "client_reply".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            now_unix: 15,
        })
        .await
        .unwrap();
    assert_eq!(reopened.discussion.status, RequestDiscussionStatus::Open);
    assert_eq!(
        reopened.activity_event.as_ref().unwrap().position,
        reopened.reply.position
    );
    let batch = store
        .requests()
        .request_discussion("req_1", &reopened.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(batch.0.unread_count, 0);
}

#[tokio::test]
async fn completed_private_discussion_transitions_persist_nothing() {
    let store = postgres_store();
    let mut input = public_start_input();
    input.author_user_id = "user_owner".to_string();
    input.author_role = RequestActorRole::Owner;
    input.audience = RequestAudience::Private;
    store.requests().start_request(input).await.unwrap();
    let mut upload = public_upload_input();
    upload.actor_user_id = "user_owner".to_string();
    store
        .requests()
        .record_working_request_upload(upload, &super::super::generated_ids::test_generated_id)
        .await
        .unwrap();
    for id in [
        "discussion_open",
        "discussion_resolved",
        "discussion_retried",
    ] {
        store
            .requests()
            .create_request_discussion(CreateRequestDiscussionCommand {
                request_id: "req_1".to_string(),
                id: id.to_string(),
                actor_user_id: "user_owner".to_string(),
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".to_string(),
                anchor: None,
                now_unix: 4,
            })
            .await
            .unwrap();
    }
    store
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "user_owner".to_string(),
            event_id: "event_initial_resolve".to_string(),
            now_unix: 5,
            transition: DiscussionTransition::Resolve,
        })
        .await
        .unwrap();
    store
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: "discussion_retried".to_string(),
            actor_user_id: "user_owner".to_string(),
            event_id: "event_retry_initial_resolve".to_string(),
            now_unix: 5,
            transition: DiscussionTransition::Resolve,
        })
        .await
        .unwrap();
    let retry_input = ReopenAndReplyToRequestDiscussionCommand {
        request_id: "req_1".to_string(),
        discussion_id: "discussion_retried".to_string(),
        reply_id: "reply_before_completion".to_string(),
        actor_user_id: "user_owner".to_string(),
        event_id: "event_retry_reopened".to_string(),
        client_reply_id: "client_retry_reopened".to_string(),
        body_markdown: "Reopen before completion".to_string(),
        reply_to_reply_id: None,
        now_unix: 6,
    };
    store
        .requests()
        .reopen_and_reply_to_request_discussion(retry_input.clone())
        .await
        .unwrap();
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.submitted_at_unix = Some(7);
            request.closed_at_unix = Some(8);
            request.closed_by_user_id = Some("user_owner".to_string());
            request.updated_at_unix = 8;
        })
        .await
        .unwrap();
    let expected_request = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    let expected_open = store
        .requests()
        .request_discussion("req_1", "discussion_open", Some("user_owner"))
        .await
        .unwrap()
        .unwrap()
        .0;
    let expected_resolved = store
        .requests()
        .request_discussion("req_1", "discussion_resolved", Some("user_owner"))
        .await
        .unwrap()
        .unwrap()
        .0;
    let expected_events = store
        .requests()
        .request_events_by_request_id("req_1")
        .await
        .unwrap();

    let resolve_error = store
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: "discussion_open".to_string(),
            actor_user_id: "user_owner".to_string(),
            event_id: "event_rejected_resolve".to_string(),
            now_unix: 8,
            transition: DiscussionTransition::Resolve,
        })
        .await
        .unwrap_err();
    assert_eq!(
        resolve_error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    let reopen_error = store
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: "discussion_resolved".to_string(),
            actor_user_id: "user_owner".to_string(),
            event_id: "event_rejected_reopen".to_string(),
            now_unix: 9,
            transition: DiscussionTransition::Reopen,
        })
        .await
        .unwrap_err();
    assert_eq!(
        reopen_error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    let retry_error = store
        .requests()
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionCommand {
            now_unix: 10,
            ..retry_input
        })
        .await
        .unwrap_err();
    assert_eq!(
        retry_error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );

    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap(),
        expected_request
    );
    let actual_open = store
        .requests()
        .request_discussion("req_1", "discussion_open", Some("user_owner"))
        .await
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(actual_open.discussion, expected_open.discussion);
    assert_eq!(actual_open.unread_count, expected_open.unread_count);
    let actual_resolved = store
        .requests()
        .request_discussion("req_1", "discussion_resolved", Some("user_owner"))
        .await
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(actual_resolved.discussion, expected_resolved.discussion);
    assert_eq!(actual_resolved.unread_count, expected_resolved.unread_count);
    assert_eq!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap(),
        expected_events
    );
}

#[tokio::test]
async fn discussion_replies_are_read_as_flat_chronological_pages() {
    let store = postgres_store();
    start_public_request(&store).await;
    let discussion = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            id: "discussion_tree".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_tree".to_string(),
            body_markdown: "Tree shape".to_string(),
            anchor: None,
            now_unix: 10,
        })
        .await
        .unwrap();
    create_test_reply(&store, &discussion.discussion.id, "root_a", None, 11).await;
    create_test_reply(&store, &discussion.discussion.id, "root_b", None, 12).await;
    create_test_reply(
        &store,
        &discussion.discussion.id,
        "child_a",
        Some("root_a"),
        13,
    )
    .await;
    create_test_reply(
        &store,
        &discussion.discussion.id,
        "child_b",
        Some("root_a"),
        14,
    )
    .await;
    create_test_reply(
        &store,
        &discussion.discussion.id,
        "grandchild",
        Some("child_a"),
        15,
    )
    .await;

    let summary = store
        .requests()
        .request_discussion("req_1", &discussion.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(summary.reply_count, 5);
    assert_eq!(summary.latest_replies.len(), 3);
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["child_a", "child_b", "grandchild"]
    );
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .find(|model| model.reply.id == "child_a")
            .unwrap()
            .reply_to
            .as_ref()
            .unwrap()
            .id,
        "root_a"
    );
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .find(|model| model.reply.id == "grandchild")
            .unwrap()
            .reply_to
            .as_ref()
            .unwrap()
            .body_markdown,
        "Reply child_a"
    );
    assert_eq!(
        summary.read_through_position,
        summary.discussion.last_activity_position
    );

    let (replies, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, None, 10)
        .await
        .unwrap();
    assert_eq!(
        replies
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["root_a", "root_b", "child_a", "child_b", "grandchild"]
    );
    let (newest, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, None, 3)
        .await
        .unwrap();
    assert_eq!(
        newest
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["child_a", "child_b", "grandchild"]
    );
    let (older, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, Some(newest[0].reply.position), 3)
        .await
        .unwrap();
    assert_eq!(
        older
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["root_a", "root_b"]
    );
}

#[tokio::test]
async fn close_draft_request_deletes_request_and_events() {
    let store = postgres_store();
    start_public_request(&store).await;
    store
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                actor_can_edit: false,
                expected_old_head_oid: Some("head".to_string()),
                new_head_oid: "head-2".to_string(),
                git_snapshot: source_blob("head-2"),
                event_id: "event_revision".to_string(),
                body: None,
                now_unix: 4,
            },
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            id: "discussion_revision".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_revision".to_string(),
            body_markdown: "Review this revision".to_string(),
            anchor: Some(RequestDiscussionAnchor {
                revision_id: "event_revision".to_string(),
                commit_oid: None,
                path: None,
            }),
            now_unix: 5,
        })
        .await
        .unwrap();

    let mutation = store
        .requests()
        .close_request(
            CloseRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                actor_is_author: false,
                actor_is_maintainer: false,
                event_id: "event_closed".to_string(),
                now_unix: 6,
            },
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    assert!(matches!(
        mutation,
        CloseRequestMutation::DeletedDraft { .. }
    ));
    assert!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .requests()
            .request_events_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
    let (_, pending_blobs) = store.cleanup().pending_cleanup_queues().await.unwrap();
    let pending_refs = pending_blobs
        .iter()
        .map(|blob| blob.content_ref.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        pending_refs,
        [
            scope_domain::content_ref::ContentRef::git_bundle_sha256("sha256-head"),
            scope_domain::content_ref::ContentRef::git_bundle_sha256("sha256-head-2"),
        ]
        .into_iter()
        .collect()
    );
    let referenced = super::super::object_references::referenced_content_refs(store.db.as_ref())
        .await
        .unwrap();
    assert!(
        !referenced.contains(&scope_domain::content_ref::ContentRef::git_bundle_sha256(
            "head"
        ))
    );
    assert!(
        !referenced.contains(&scope_domain::content_ref::ContentRef::git_bundle_sha256(
            "head-2"
        ))
    );
}

#[tokio::test]
async fn maintainer_cannot_delete_another_authors_draft() {
    let store = postgres_store();
    start_public_request(&store).await;

    let error = store
        .requests()
        .close_request(
            CloseRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_owner".to_string(),
                actor_is_author: false,
                actor_is_maintainer: false,
                event_id: "event_closed_by_maintainer".to_string(),
                now_unix: 4,
            },
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap_err();

    assert!(error.message.contains("only the request author"));
    assert!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn close_open_request_persists_exact_closer() {
    let store = postgres_store();
    let mut request = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    request.submitted_at_unix = Some(3);
    request.updated_at_unix = 3;
    save_request_row(store.db.as_ref(), &request).await.unwrap();

    let mutation = store
        .requests()
        .close_request(
            CloseRequestInput {
                request_id: request.id.clone(),
                actor_user_id: request.author_user_id.clone(),
                actor_is_author: false,
                actor_is_maintainer: false,
                event_id: "event_closed".to_string(),
                now_unix: 4,
            },
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    assert!(matches!(mutation, CloseRequestMutation::Closed { .. }));
    let stored = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state(), RequestState::Closed);
    assert_eq!(stored.closed_at_unix, Some(4));
    assert_eq!(stored.closed_by_user_id.as_deref(), Some("user_public"));
}

pub(crate) fn postgres_store() -> MetadataStore {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    store
        .admin()
        .seed_catalog_for_tests(catalog_with_repo())
        .unwrap();
    store
}

fn catalog_with_repo() -> crate::db::CatalogFixture {
    let owner = UserAccount {
        id: "user_owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let public_user = UserAccount {
        id: "user_public".to_string(),
        handle: "public".to_string(),
        email: "public@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", Visibility::Public, "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;

    let mut catalog = crate::db::CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public_user.id.clone(), public_user);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    catalog
}

pub(crate) async fn start_public_request(store: &MetadataStore) {
    store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap();
    store
        .requests()
        .record_working_request_upload(
            public_upload_input(),
            &super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
}

async fn create_test_reply(
    store: &MetadataStore,
    discussion_id: &str,
    id: &str,
    parent_id: Option<&str>,
    now_unix: u64,
) {
    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: "req_1".to_string(),
            discussion_id: discussion_id.to_string(),
            id: id.to_string(),
            actor_user_id: "user_public".to_string(),
            client_reply_id: format!("client_{id}"),
            body_markdown: format!("Reply {id}"),
            reply_to_reply_id: parent_id.map(str::to_string),
            now_unix,
        })
        .await
        .unwrap();
}

fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "user_public".to_string(),
        title: Some("Fix parser crash".to_string()),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 2,
    }
}

fn public_upload_input() -> RecordWorkingRequestUploadInput {
    RecordWorkingRequestUploadInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        actor_can_edit: true,
        expected_old_head_oid: None,
        new_head_oid: "head".to_string(),
        git_snapshot: source_blob("head"),
        now_unix: 3,
    }
}

fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        content_ref: scope_domain::content_ref::ContentRef::git_bundle_sha256(format!(
            "sha256-{git_oid}"
        )),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

mod authorization_locks;
