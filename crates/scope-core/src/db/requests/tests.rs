use super::*;
use crate::domain::{
    policy::Visibility,
    requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput, CreditLedgerEntryKind,
        REQUEST_LIST_MAX_PAGE_SIZE, ReopenAndReplyToRequestDiscussionInput, RequestActorRole,
        RequestAudience, RequestDiscussionStatus, RequestDisposition, RequestEventKind,
        RequestState,
    },
    store::{
        AppCatalog, DEFAULT_GIT_FILE_MODE, RepoPublicationState, SourceBlob, StoredRepository,
        UserAccount, app_catalog,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[tokio::test]
async fn request_list_page_projects_visible_rows_in_stable_id_pages() {
    let store = postgres_store();

    let mut private = public_start_input();
    private.id = "req_a".to_string();
    private.name = "private-request".to_string();
    private.author_user_id = "user_owner".to_string();
    private.author_role = RequestActorRole::Owner;
    private.audience = RequestAudience::Private;
    private.event_id = "event_a".to_string();
    store.start_request(private).await.unwrap();

    let mut public_b = public_start_input();
    public_b.id = "req_b".to_string();
    public_b.name = "public-b".to_string();
    public_b.event_id = "event_b".to_string();
    store.start_request(public_b).await.unwrap();
    let mut upload_b = public_upload_input();
    upload_b.request_id = "req_b".to_string();
    upload_b.new_head_oid = "head_b".to_string();
    upload_b.git_snapshot = source_blob("head_b");
    store.record_working_request_upload(upload_b).await.unwrap();

    let mut public_c = public_start_input();
    public_c.id = "req_c".to_string();
    public_c.name = "public-c".to_string();
    public_c.event_id = "event_c".to_string();
    store.start_request(public_c).await.unwrap();

    let audiences = [RequestAudience::Public, RequestAudience::Private];
    let first_page = store
        .request_list_page("owner/repo", &audiences, None, 2)
        .await
        .unwrap();
    assert_eq!(
        first_page
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        vec!["req_a", "req_b"]
    );
    assert!(!first_page[0].has_git_snapshot);
    assert!(first_page[1].has_git_snapshot);
    assert_eq!(first_page[1].head_oid, "head_b");
    assert_eq!(first_page[1].state, RequestState::Working);

    let second_page = store
        .request_list_page("owner/repo", &audiences, Some("req_b"), 2)
        .await
        .unwrap();
    assert_eq!(
        second_page
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        vec!["req_c"]
    );

    let public_only = store
        .request_list_page("owner/repo", &[RequestAudience::Public], None, 10)
        .await
        .unwrap();
    assert_eq!(
        public_only
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        vec!["req_b", "req_c"]
    );
    assert!(
        store
            .request_list_page("owner/repo", &[], None, 10)
            .await
            .unwrap()
            .is_empty()
    );

    for index in 0..(REQUEST_LIST_MAX_PAGE_SIZE + 5) {
        let mut input = public_start_input();
        input.id = format!("zz_req_{index:03}");
        input.name = format!("bounded-{index}");
        input.event_id = format!("event_bounded_{index}");
        store.start_request(input).await.unwrap();
    }
    let bounded = store
        .request_list_page("owner/repo", &audiences, None, u64::MAX)
        .await
        .unwrap();
    assert_eq!(bounded.len(), REQUEST_LIST_MAX_PAGE_SIZE + 1);
}

#[tokio::test]
async fn request_submission_and_resolution_update_credit_facts() {
    let store = postgres_store();

    grant_public_credits(&store).await;
    submit_public_request(&store).await;
    let unauthorized = store
        .resolve_request(ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            disposition: RequestDisposition::Accepted,
            event_id: "rejected".to_string(),
            settlement_event_id: "rejected-settlement".to_string(),
            refund_ledger_entry_id: None,
            reward_ledger_entry_id: None,
            body: None,
            now_unix: 2,
        })
        .await
        .unwrap_err();
    assert!(unauthorized.message.contains("repo maintainer required"));
    store
        .record_request_revision(RecordRequestRevisionInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "head_2".to_string(),
            git_snapshot: source_blob("head_2"),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 2,
        })
        .await
        .unwrap();
    let revision_thread = store
        .request_discussion("req_1", "thread_event_revision", Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revision_thread.0.unread_count, 0);
    let mutation = store
        .resolve_request(ResolveRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: "user_owner".to_string(),
            disposition: RequestDisposition::UsefulNotMerged,
            event_id: "event_resolved".to_string(),
            settlement_event_id: "event_settled".to_string(),
            refund_ledger_entry_id: Some("ledger_refund".to_string()),
            reward_ledger_entry_id: Some("ledger_reward".to_string()),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();

    assert_eq!(mutation.request.settlement.unwrap().reward_credits, 2);
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        22
    );
    let request = store.request_for_tests("req_1").await.unwrap().unwrap();
    assert_eq!(request.resolved_at_unix, Some(3));
    assert_eq!(request.head_oid, "head_2");
    let mut events = store.request_events_for_tests().await.unwrap();
    events.sort_by_key(|event| event.position);
    assert_eq!(
        events
            .into_iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![
            RequestEventKind::Started,
            RequestEventKind::Submitted,
            RequestEventKind::RevisionPushed,
            RequestEventKind::Resolved,
            RequestEventKind::Settled,
        ]
    );
    assert_eq!(
        store.credit_ledger_entries_for_tests().await.unwrap().len(),
        4
    );
}

#[tokio::test]
async fn public_user_cannot_choose_owner_role_to_skip_stake() {
    let store = postgres_store();
    let mut start_input = public_start_input();
    start_input.audience = RequestAudience::Private;
    let started = store.start_request(start_input).await.unwrap();
    assert_eq!(started.request.author_role, RequestActorRole::Public);
    assert_eq!(started.request.audience, RequestAudience::Public);
    store
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
    let mut input = public_submit_input();
    input.stake_credits = 0;
    input.stake_ledger_entry_id = None;

    let error = store.submit_request(input).await.unwrap_err();

    assert!(
        error
            .message
            .contains("public requests require credit stake")
    );
    assert_eq!(
        store
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state,
        RequestState::Working
    );
    let events = store.request_events_for_tests().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, RequestEventKind::Started);
    assert!(
        store
            .credit_ledger_entries_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn discussion_transactions_are_idempotent_atomic_and_self_read() {
    let store = postgres_store();
    grant_public_credits(&store).await;
    submit_public_request(&store).await;
    let submitted_thread = store
        .request_discussion("req_1", "thread_event_created", Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(submitted_thread.0.unread_count, 0);

    let first = store
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
    store
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
        .request_discussion("req_1", &first.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unread_after_retry.0.unread_count, 1);

    let resolved = store
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
    assert_eq!(reply_error.kind, crate::error::ErrorKind::Conflict);

    let reopened = store
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id,
            reply_id: "reply_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_maintainer: false,
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
        .request_discussion("req_1", &reopened.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(batch.0.unread_count, 0);
}

#[tokio::test]
async fn discussion_replies_are_read_as_paginated_tree_levels() {
    let store = postgres_store();
    grant_public_credits(&store).await;
    submit_public_request(&store).await;
    let discussion = store
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
            .request_discussion_reply_child_count("root_a")
            .await
            .unwrap(),
        2
    );
    let (grandchildren, _) = store
        .request_discussion_replies(&discussion.discussion.id, Some("child_a"), None, 10)
        .await
        .unwrap();
    assert_eq!(grandchildren.len(), 1);
    assert_eq!(grandchildren[0].reply.id, "grandchild");
}

#[tokio::test]
async fn owner_submission_preserves_explicit_public_audience_without_credits() {
    let store = postgres_store();
    let mut input = public_start_input();
    input.id = "req_owner".to_string();
    input.name = "owner-request".to_string();
    input.author_user_id = "user_owner".to_string();
    input.author_role = RequestActorRole::Public;
    input.audience = RequestAudience::Public;
    let start = store.start_request(input).await.unwrap();
    store
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: start.request.id.clone(),
            actor_user_id: "user_owner".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "head".to_string(),
            git_snapshot: source_blob("head"),
            now_unix: 2,
        })
        .await
        .unwrap();
    let mutation = store
        .submit_request(SubmitRequestInput {
            request_id: start.request.id,
            actor_user_id: "user_owner".to_string(),
            expected_head_oid: "head".to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_owner".to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();

    assert_eq!(mutation.request.author_role, RequestActorRole::Owner);
    assert_eq!(mutation.request.audience, RequestAudience::Public);
    assert!(mutation.account.is_none());
    assert!(mutation.ledger_entry.is_none());
    assert_eq!(
        store
            .request_by_name("owner/repo", "owner-request")
            .await
            .unwrap()
            .unwrap()
            .id,
        "req_owner"
    );
}

#[tokio::test]
async fn repo_delete_waits_for_resolution_and_does_not_refund_settled_stake_twice() {
    assert_delete_serializes_with_credit_mutation(CreditMutation::Resolve, 22).await;
}

#[tokio::test]
async fn repo_delete_waits_for_submission_and_refunds_the_committed_stake() {
    assert_delete_serializes_with_credit_mutation(CreditMutation::Submit, 20).await;
}

enum CreditMutation {
    Submit,
    Resolve,
}

async fn assert_delete_serializes_with_credit_mutation(
    mutation: CreditMutation,
    expected_balance: u32,
) {
    let store = postgres_store();
    grant_public_credits(&store).await;
    match mutation {
        CreditMutation::Submit => {
            store.start_request(public_start_input()).await.unwrap();
            store
                .record_working_request_upload(public_upload_input())
                .await
                .unwrap();
        }
        CreditMutation::Resolve => submit_public_request(&store).await,
    }

    let credit_guard = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&credit_guard, "user-credit", "user_public")
        .await
        .unwrap();

    let mutation_store = store.clone();
    let mutation = tokio::spawn(async move {
        match mutation {
            CreditMutation::Submit => mutation_store
                .submit_request(public_submit_input())
                .await
                .map(|_| ()),
            CreditMutation::Resolve => mutation_store
                .resolve_request(ResolveRequestInput {
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_owner".to_string(),
                    disposition: RequestDisposition::UsefulNotMerged,
                    event_id: "event_resolved".to_string(),
                    settlement_event_id: "event_settled".to_string(),
                    refund_ledger_entry_id: Some("ledger_refund".to_string()),
                    reward_ledger_entry_id: Some("ledger_reward".to_string()),
                    body: None,
                    now_unix: 5,
                })
                .await
                .map(|_| ()),
        }
    });
    wait_until_aggregate_lock_is_held(&store, "repository", "owner/repo").await;
    let delete_store = store.clone();
    let delete = tokio::spawn(async move {
        delete_store
            .delete_repo("owner", "repo", "user_owner")
            .await
    });
    wait_until_aggregate_lock_is_waited_on(&store, "repository").await;

    assert!(
        !mutation.is_finished(),
        "credit mutation should wait for credit lock"
    );
    assert!(
        !delete.is_finished(),
        "deletion should wait behind the repository lock"
    );
    credit_guard.commit().await.unwrap();

    mutation.await.unwrap().unwrap();
    delete.await.unwrap().unwrap();
    assert!(
        store
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        expected_balance
    );
    assert_eq!(
        store
            .credit_ledger_entries_for_tests()
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == CreditLedgerEntryKind::StakeRefund)
            .count(),
        1
    );
}

async fn grant_public_credits(store: &MetadataStore) {
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "ledger_grant".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 20,
            now_unix: 1,
        })
        .await
        .unwrap();
}

fn postgres_store() -> MetadataStore {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    store.seed_catalog_for_tests(catalog_with_repo()).unwrap();
    store
}

async fn wait_until_aggregate_lock_is_held(store: &MetadataStore, namespace: &str, id: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let probe = store.db.begin().await.unwrap();
        probe
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "SET LOCAL lock_timeout = '10ms'",
            ))
            .await
            .unwrap();
        match acquire_aggregate_lock(&probe, namespace, id).await {
            Ok(()) => {
                probe.rollback().await.unwrap();
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for {namespace}:{id} to be locked"
                );
                tokio::task::yield_now().await;
            }
            Err(error) if error.message.contains("lock timeout") => {
                probe.rollback().await.unwrap();
                return;
            }
            Err(error) => {
                let _ = probe.rollback().await;
                panic!("failed to probe {namespace}:{id} lock: {}", error.message);
            }
        }
    }
}

async fn wait_until_aggregate_lock_is_waited_on(store: &MetadataStore, namespace: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let waiting = store
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (\
                    SELECT 1 FROM pg_stat_activity AS activity \
                    JOIN pg_locks AS relation_lock ON relation_lock.pid = activity.pid \
                    WHERE activity.application_name = $1 \
                      AND activity.wait_event_type = 'Lock' \
                      AND relation_lock.relation = to_regclass('scope_metadata_locks')::oid\
                ) AS waiting",
                [format!("scope-test-lock:{namespace}").into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<bool>("", "waiting")
            .unwrap();
        if waiting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for a blocked {namespace} aggregate lock"
        );
        tokio::task::yield_now().await;
    }
}

fn catalog_with_repo() -> AppCatalog {
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
    repo.record.publication_state = RepoPublicationState::Published;

    let mut catalog = app_catalog();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public_user.id.clone(), public_user);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    catalog
}

async fn submit_public_request(store: &MetadataStore) {
    store.start_request(public_start_input()).await.unwrap();
    store
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
    store.submit_request(public_submit_input()).await.unwrap();
}

async fn create_test_reply(
    store: &MetadataStore,
    discussion_id: &str,
    id: &str,
    parent_id: Option<&str>,
    now_unix: u64,
) {
    store
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

fn public_submit_input() -> SubmitRequestInput {
    SubmitRequestInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        expected_head_oid: "head".to_string(),
        stake_credits: 10,
        stake_ledger_entry_id: Some("ledger_stake".to_string()),
        event_id: "event_created".to_string(),
        now_unix: 4,
    }
}

fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        object_key: format!("objects/{git_oid}"),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}
