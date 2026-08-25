use super::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    requests::*,
};
use std::collections::BTreeMap;

#[test]
fn revision_records_snapshot_without_manufacturing_a_discussion() {
    let mut requests = BTreeMap::new();
    start_request(
        &mut requests,
        StartRequestInput {
            id: "request_change".to_string(),
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
            request_id: "request_change".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "head-1".to_string(),
            git_snapshot: source_blob("head-1"),
            now_unix: 11,
        },
    )
    .unwrap();
    let mutation = record_request_revision(
        &mut requests,
        &mut BTreeMap::new(),
        RecordRequestRevisionInput {
            request_id: "request_change".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head-1".to_string()),
            new_head_oid: "head-2".to_string(),
            git_snapshot: source_blob("head-2"),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 12,
        },
    )
    .unwrap();

    assert_eq!(mutation.orphan_objects, vec![source_blob("head-1")]);
    assert_eq!(mutation.revision.old_head_oid, "head-1");
    assert_eq!(mutation.revision.new_head_oid, "head-2");
    assert_eq!(mutation.revision.id, mutation.event.id);
}

#[test]
fn review_revision_is_newest_unless_an_existing_revision_is_pinned() {
    let revisions = vec![revision("revision-2", 2), revision("revision-1", 1)];

    assert_eq!(
        select_request_review_revision(&revisions, None)
            .unwrap()
            .unwrap()
            .id,
        "revision-2"
    );
    assert_eq!(
        select_request_review_revision(&revisions, Some("revision-1"))
            .unwrap()
            .unwrap()
            .id,
        "revision-1"
    );
    assert!(select_request_review_revision(&revisions, Some("missing")).is_err());
}

fn revision(id: &str, position: u64) -> RequestRevision {
    RequestRevision {
        id: id.to_string(),
        request_id: "request".to_string(),
        position,
        actor_user_id: "author".to_string(),
        old_head_oid: "old".to_string(),
        new_head_oid: "new".to_string(),
        git_snapshot: source_blob(id),
        created_at_unix: position,
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
