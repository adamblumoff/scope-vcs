use super::*;
use crate::error::PostgresErrorKind;

#[tokio::test]
async fn discussion_commands_derive_actor_permissions_from_persisted_private_request() {
    let store = postgres_store();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.author_role = RequestActorRole::Owner;
    start.audience = RequestAudience::Private;
    store.requests().start_request(start).await.unwrap();
    let mut upload = public_upload_input();
    upload.actor_user_id = "user_owner".into();
    store
        .requests()
        .record_working_request_upload(
            upload,
            &super::super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    let create = CreateRequestDiscussionCommand {
        request_id: "req_1".into(),
        id: "discussion_policy".into(),
        actor_user_id: "user_public".into(),
        client_discussion_id: "client_policy".into(),
        body_markdown: "Review policy".into(),
        anchor: None,
        now_unix: 4,
    };
    assert_eq!(
        store
            .requests()
            .create_request_discussion(create.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    let created = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            actor_user_id: "user_owner".into(),
            ..create
        })
        .await
        .unwrap();
    assert_eq!(created.discussion.author_user_id, "user_owner");
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: created.discussion.id.clone(),
        id: "reply_policy".into(),
        actor_user_id: "user_public".into(),
        client_reply_id: "client_reply_policy".into(),
        body_markdown: "Reply".into(),
        reply_to_reply_id: None,
        now_unix: 5,
    };
    assert_eq!(
        store
            .requests()
            .create_request_discussion_reply(reply.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            actor_user_id: "user_owner".into(),
            ..reply
        })
        .await
        .unwrap();

    for (transition, status, now_unix) in [
        (
            DiscussionTransition::Resolve,
            RequestDiscussionStatus::Resolved,
            6,
        ),
        (
            DiscussionTransition::Reopen,
            RequestDiscussionStatus::Open,
            7,
        ),
    ] {
        let command = TransitionRequestDiscussionCommand {
            request_id: "req_1".into(),
            discussion_id: created.discussion.id.clone(),
            actor_user_id: "user_public".into(),
            event_id: format!("policy_event_{now_unix}"),
            now_unix,
            transition,
        };
        assert_eq!(
            store
                .requests()
                .transition_request_discussion(command.clone())
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::NotFound
        );
        let discussion = store
            .requests()
            .transition_request_discussion(TransitionRequestDiscussionCommand {
                actor_user_id: "user_owner".into(),
                ..command
            })
            .await
            .unwrap();
        assert_eq!(discussion.status, status);
    }
    let events = store
        .requests()
        .request_events_by_request_id("req_1")
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.id.starts_with("policy_event_"))
            .count(),
        2
    );
}
