use super::super::MetadataStore;
use super::*;
use scope_domain::{
    policy::Visibility,
    requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput,
        ReopenAndReplyToRequestDiscussionInput, RequestActorRole, RequestAudience,
        RequestDiscussionStatus, RequestState,
    },
    store::{DEFAULT_GIT_FILE_MODE, RepoLifecycleState, SourceBlob, StoredRepository, UserAccount},
};

#[tokio::test]
async fn discussion_transactions_are_idempotent_atomic_and_self_read() {
    let store = postgres_store();
    start_public_request(&store).await;

    let first = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: false,
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 10,
        })
        .await
        .unwrap();
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
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_before_retry".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_can_participate: false,
            client_reply_id: "client_before_retry".to_string(),
            body_markdown: "Maintainer reply".to_string(),
            reply_to_reply_id: None,
            now_unix: 11,
        })
        .await
        .unwrap();
    let retried = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_retry_id".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: false,
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            now_unix: 12,
        })
        .await
        .unwrap();
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
        .resolve_request_discussion(
            "req_1".to_string(),
            first.discussion.id.clone(),
            "user_public".to_string(),
            "event_discussion_resolved".to_string(),
            13,
        )
        .await
        .unwrap();
    assert_eq!(resolved.status, RequestDiscussionStatus::Resolved);
    let reply_error = store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_rejected".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: false,
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
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id,
            reply_id: "reply_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_maintainer: false,
            actor_can_transition: false,
            actor_can_participate: false,
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
            .create_request_discussion(CreateRequestDiscussionInput {
                request_id: "req_1".to_string(),
                id: id.to_string(),
                actor_user_id: "user_owner".to_string(),
                actor_can_participate: false,
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".to_string(),
                now_unix: 4,
            })
            .await
            .unwrap();
    }
    store
        .requests()
        .resolve_request_discussion(
            "req_1".to_string(),
            "discussion_resolved".to_string(),
            "user_owner".to_string(),
            "event_initial_resolve".to_string(),
            5,
        )
        .await
        .unwrap();
    store
        .requests()
        .resolve_request_discussion(
            "req_1".to_string(),
            "discussion_retried".to_string(),
            "user_owner".to_string(),
            "event_retry_initial_resolve".to_string(),
            5,
        )
        .await
        .unwrap();
    let retry_input = ReopenAndReplyToRequestDiscussionInput {
        request_id: "req_1".to_string(),
        discussion_id: "discussion_retried".to_string(),
        reply_id: "reply_before_completion".to_string(),
        actor_user_id: "user_owner".to_string(),
        actor_is_maintainer: false,
        actor_can_transition: false,
        actor_can_participate: false,
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
        .resolve_request_discussion(
            "req_1".to_string(),
            "discussion_open".to_string(),
            "user_owner".to_string(),
            "event_rejected_resolve".to_string(),
            8,
        )
        .await
        .unwrap_err();
    assert_eq!(
        resolve_error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    let reopen_error = store
        .requests()
        .reopen_request_discussion(
            "req_1".to_string(),
            "discussion_resolved".to_string(),
            "user_owner".to_string(),
            "event_rejected_reopen".to_string(),
            9,
        )
        .await
        .unwrap_err();
    assert_eq!(
        reopen_error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    let retry_error = store
        .requests()
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
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
async fn discussion_replies_are_read_as_paginated_tree_levels() {
    let store = postgres_store();
    start_public_request(&store).await;
    let discussion = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_tree".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: false,
            client_discussion_id: "client_tree".to_string(),
            body_markdown: "Tree shape".to_string(),
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
    assert_eq!(summary.latest_replies.len(), 4);
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["root_a", "child_a", "child_b", "grandchild"]
    );
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .find(|model| model.reply.id == "root_a")
            .unwrap()
            .child_reply_count,
        2
    );

    let (roots, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, None, None, 10)
        .await
        .unwrap();
    assert_eq!(
        roots
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["root_a", "root_b"]
    );
    let (children, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, Some("root_a"), None, 10)
        .await
        .unwrap();
    assert_eq!(
        children
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["child_a", "child_b"]
    );
    assert_eq!(children[0].child_reply_count, 1);
    assert_eq!(children[1].child_reply_count, 0);
    assert_eq!(
        store
            .requests()
            .request_discussion_reply_child_count("root_a")
            .await
            .unwrap(),
        2
    );
    let (grandchildren, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, Some("child_a"), None, 10)
        .await
        .unwrap();
    assert_eq!(grandchildren.len(), 1);
    assert_eq!(grandchildren[0].reply.id, "grandchild");
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

    let mutation = store
        .requests()
        .close_request(
            CloseRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                actor_is_author: false,
                actor_is_maintainer: false,
                event_id: "event_closed".to_string(),
                now_unix: 5,
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
    let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
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
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: discussion_id.to_string(),
            id: id.to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_participate: false,
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
