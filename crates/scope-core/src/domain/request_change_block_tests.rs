use super::{
    requests::*,
    store::{DEFAULT_GIT_FILE_MODE, SourceBlob},
};
use std::collections::BTreeMap;

#[test]
fn submission_creates_a_dormant_thread_that_opens_on_first_reply() {
    let (mut requests, mutation) = submitted_fixture();

    assert_eq!(mutation.change_block.old_head_oid, "base");
    assert_eq!(mutation.change_block.new_head_oid, "head");
    assert_eq!(mutation.change_block.git_snapshot, source_blob("head"));
    assert_eq!(mutation.discussion.opened_position, mutation.event.position);
    assert_eq!(mutation.discussion.status, RequestDiscussionStatus::Dormant);
    assert!(matches!(
        mutation.discussion.subject,
        RequestDiscussionSubject::ChangeBlock { ref change_block_id }
            if change_block_id == &mutation.change_block.id
    ));

    let discussion_id = mutation.discussion.id.clone();
    let mut discussions = BTreeMap::from([(discussion_id.clone(), mutation.discussion)]);
    let reply = create_request_discussion_reply(
        &mut requests,
        &mut discussions,
        &mut BTreeMap::new(),
        CreateRequestDiscussionReplyInput {
            request_id: "req_change".to_string(),
            discussion_id,
            id: "reply_change_block".to_string(),
            actor_user_id: "maintainer".to_string(),
            actor_can_participate: true,
            client_reply_id: "client_change_block".to_string(),
            body_markdown: "Can we cover the retry path?".to_string(),
            reply_to_reply_id: None,
            now_unix: 13,
        },
    )
    .unwrap();

    assert_eq!(reply.discussion.status, RequestDiscussionStatus::Open);
    assert_eq!(
        reply.discussion.last_activity_position,
        reply.reply.position
    );
}

fn submitted_fixture() -> (BTreeMap<String, Request>, SubmitRequestMutation) {
    let mut requests = BTreeMap::new();
    start_request(
        &mut requests,
        StartRequestInput {
            id: "req_change".to_string(),
            repo_id: "owner/repo".to_string(),
            name: "change".to_string(),
            author_user_id: "author".to_string(),
            title: Some("Change".to_string()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: "base".to_string(),
            event_id: "event_started".to_string(),
            now_unix: 10,
        },
    )
    .unwrap();
    record_working_request_upload(
        &mut requests,
        RecordWorkingRequestUploadInput {
            request_id: "req_change".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "head".to_string(),
            git_snapshot: source_blob("head"),
            now_unix: 11,
        },
    )
    .unwrap();
    let mutation = submit_request(
        &mut requests,
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
        SubmitRequestInput {
            request_id: "req_change".to_string(),
            actor_user_id: "author".to_string(),
            expected_head_oid: "head".to_string(),
            stake_credits: 0,
            stake_ledger_entry_id: None,
            event_id: "event_submitted".to_string(),
            now_unix: 12,
        },
    )
    .unwrap();
    (requests, mutation)
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
