use super::{MetadataStore, TestDatabaseTarget};
use crate::db::{AddRequestInviteeCommand, RemoveRequestInviteeCommand};
use crate::domain::{
    policy::Visibility,
    requests::{
        GrantUserCreditsInput, MarkRequestReadyInput, RecordRequestRevisionInput,
        RecordWorkingRequestUploadInput, RequestActorRole, RequestAudience, RequestEventKind,
        RequestState, SetRequestHoldInput, StartRequestInput, UpdateRequestDescriptionInput,
    },
    store::{
        DEFAULT_GIT_FILE_MODE, RepoPublicationState, SourceBlob, StoredRepository, UserAccount,
        app_catalog,
    },
};
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test]
async fn held_maintainer_content_and_branch_mutations_invalidate_and_refund_atomically() {
    let store = postgres_store();
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "starter:user_public".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 100,
            now_unix: 1,
        })
        .await
        .unwrap();
    start_uploaded(&store).await;
    ready_and_hold(&store, "one", 10, 10).await;

    let author_error = store
        .update_request_description_with_review_invalidation(UpdateRequestDescriptionInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit_description: false,
            event_id: "event_author_edit".to_string(),
            description_markdown: "Author edit".to_string(),
            now_unix: 20,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        author_error.kind,
        crate::error::ErrorKind::Forbidden | crate::error::ErrorKind::Conflict
    ));

    let edited = store
        .update_request_description_with_review_invalidation(UpdateRequestDescriptionInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_can_edit_description: false,
            event_id: "event_maintainer_edit".to_string(),
            description_markdown: "Maintainer edit".to_string(),
            now_unix: 21,
        })
        .await
        .unwrap();
    assert_eq!(edited.request.state, RequestState::Working);
    assert_eq!(edited.request.held_at_unix, None);
    assert_eq!(edited.request.description_markdown, "Maintainer edit");
    assert_balance(&store, 100).await;

    ready_and_hold(&store, "two", 20, 30).await;
    let revised = store
        .record_request_revision_with_review_invalidation(RecordRequestRevisionInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_can_edit: false,
            expected_old_head_oid: Some("head-one".to_string()),
            new_head_oid: "head-two".to_string(),
            git_snapshot: source_blob("head-two"),
            event_id: "event_revision_two".to_string(),
            body: None,
            now_unix: 40,
        })
        .await
        .unwrap();
    assert_eq!(revised.request.state, RequestState::Working);
    assert_eq!(revised.request.held_at_unix, None);
    assert_eq!(revised.request.head_oid, "head-two");
    assert_balance(&store, 100).await;

    let events = store
        .request_events_by_request_id("req_invalidate")
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == RequestEventKind::ReturnedToWorking)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == RequestEventKind::RevisionPushed)
            .count(),
        1
    );
}

#[tokio::test]
async fn unrelated_public_user_cannot_edit_description_or_invalidate_ready() {
    let store = postgres_store();
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "starter:user_public".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 100,
            now_unix: 1,
        })
        .await
        .unwrap();
    start_uploaded(&store).await;
    store
        .mark_request_ready(MarkRequestReadyInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: Some(10),
            public_ready_count: usize::MAX,
            ready_queue_version: 0,
            event_id: "event_ready_unrelated_edit".to_string(),
            stake_ledger_entry_id: Some("ledger_stake_unrelated_edit".to_string()),
            now_unix: 10,
        })
        .await
        .unwrap();

    let error = store
        .update_request_description_with_review_invalidation(UpdateRequestDescriptionInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_collaborator".to_string(),
            actor_can_edit_description: true,
            event_id: "event_unrelated_edit".to_string(),
            description_markdown: "unrelated edit".to_string(),
            now_unix: 20,
        })
        .await
        .unwrap_err();

    assert!(error.message.contains("mutation access"));
    let request = store
        .request_for_tests("req_invalidate")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.state, RequestState::ReadyForReview);
    assert!(request.description_markdown.is_empty());
    assert_balance(&store, 90).await;
}

#[tokio::test]
async fn public_invitee_revision_invalidates_ready_and_refunds_atomically() {
    let store = postgres_store();
    store
        .grant_user_credits(GrantUserCreditsInput {
            ledger_entry_id: "starter:user_public".to_string(),
            user_id: "user_public".to_string(),
            amount_credits: 100,
            now_unix: 1,
        })
        .await
        .unwrap();
    start_uploaded(&store).await;
    store
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            target_handle: "collaborator".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    store
        .mark_request_ready(MarkRequestReadyInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: Some(10),
            public_ready_count: usize::MAX,
            ready_queue_version: 0,
            event_id: "event_ready_collaborator".to_string(),
            stake_ledger_entry_id: Some("ledger_stake_collaborator".to_string()),
            now_unix: 10,
        })
        .await
        .unwrap();

    let revised = store
        .record_request_revision_with_review_invalidation(RecordRequestRevisionInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_collaborator".to_string(),
            actor_can_edit: false,
            expected_old_head_oid: Some("head-one".to_string()),
            new_head_oid: "head-collaborator".to_string(),
            git_snapshot: source_blob("head-collaborator"),
            event_id: "event_revision_collaborator".to_string(),
            body: None,
            now_unix: 20,
        })
        .await
        .unwrap();

    assert_eq!(revised.request.state, RequestState::Working);
    assert_eq!(revised.request.head_oid, "head-collaborator");
    assert_balance(&store, 100).await;
    let events = store
        .request_events_by_request_id("req_invalidate")
        .await
        .unwrap();
    assert!(events.iter().any(|event| {
        event.kind == RequestEventKind::ReturnedToWorking
            && event.actor_user_id == "user_collaborator"
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invitee_revocation_serializes_with_final_revision_authorization() {
    let store = postgres_store();
    start_uploaded(&store).await;
    store
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            target_handle: "collaborator".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let remove_store = store.clone();
    let remove_barrier = Arc::clone(&barrier);
    let remove = tokio::spawn(async move {
        remove_barrier.wait().await;
        remove_store
            .remove_request_invitee(RemoveRequestInviteeCommand {
                request_id: "req_invalidate".to_string(),
                actor_user_id: "user_public".to_string(),
                target_handle: "collaborator".to_string(),
            })
            .await
    });
    let revision_store = store.clone();
    let revision_barrier = Arc::clone(&barrier);
    let revision = tokio::spawn(async move {
        revision_barrier.wait().await;
        revision_store
            .record_request_revision_with_review_invalidation(RecordRequestRevisionInput {
                request_id: "req_invalidate".to_string(),
                actor_user_id: "user_collaborator".to_string(),
                actor_can_edit: true,
                expected_old_head_oid: Some("head-one".to_string()),
                new_head_oid: "head-racing-invitee".to_string(),
                git_snapshot: source_blob("head-racing-invitee"),
                event_id: "event_revision_racing_invitee".to_string(),
                body: None,
                now_unix: 20,
            })
            .await
    });

    remove.await.unwrap().unwrap();
    let revision = revision.await.unwrap();
    assert!(
        revision.is_ok()
            || revision
                .as_ref()
                .unwrap_err()
                .message
                .contains("mutation access")
    );
    assert!(
        !store
            .request_is_invitee("req_invalidate", "user_collaborator")
            .await
            .unwrap()
    );
    let request = store
        .request_for_tests("req_invalidate")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request.head_oid,
        if revision.is_ok() {
            "head-racing-invitee"
        } else {
            "head-one"
        }
    );
}

fn postgres_store() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = user("user_owner", "owner");
    let public = user("user_public", "public");
    let collaborator = user("user_collaborator", "collaborator");
    let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
    repo.record.publication_state = RepoPublicationState::Published;
    let mut catalog = app_catalog();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public.id.clone(), public);
    catalog.users.insert(collaborator.id.clone(), collaborator);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.seed_catalog_for_tests(catalog).unwrap();
    store
}

fn user(id: &str, handle: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: handle.to_string(),
        email: format!("{handle}@example.com"),
        email_verified: true,
    }
}

async fn start_uploaded(store: &MetadataStore) {
    store
        .start_request(StartRequestInput {
            id: "req_invalidate".to_string(),
            repo_id: "owner/repo".to_string(),
            name: "invalidate".to_string(),
            author_user_id: "user_public".to_string(),
            title: None,
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid: "base".to_string(),
            event_id: "event_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    store
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit: false,
            expected_old_head_oid: None,
            new_head_oid: "head-one".to_string(),
            git_snapshot: source_blob("head-one"),
            now_unix: 3,
        })
        .await
        .unwrap();
}

async fn ready_and_hold(store: &MetadataStore, suffix: &str, stake: u32, now: u64) {
    store
        .mark_request_ready(MarkRequestReadyInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_is_author: false,
            actor_can_mutate: false,
            stake_credits: Some(stake),
            public_ready_count: usize::MAX,
            ready_queue_version: 0,
            event_id: format!("event_ready_{suffix}"),
            stake_ledger_entry_id: Some(format!("ledger_stake_{suffix}")),
            now_unix: now,
        })
        .await
        .unwrap();
    store
        .set_request_hold(SetRequestHoldInput {
            request_id: "req_invalidate".to_string(),
            actor_user_id: "user_owner".to_string(),
            actor_is_maintainer: false,
            held: true,
            event_id: format!("event_hold_{suffix}"),
            now_unix: now + 1,
        })
        .await
        .unwrap();
}

async fn assert_balance(store: &MetadataStore, expected: u32) {
    assert_eq!(
        store
            .credit_account_for_tests("user_public")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        expected
    );
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
