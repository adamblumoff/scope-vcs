use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget, entities};
use scope_domain::repository::git::{GitPackSpan, GitSegmentRef, GitSegmentUploadState};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel};
use std::time::Duration;

async fn store_with_repository(repo_id: &str) -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_users (id, handle, email, email_verified)
             VALUES ('segment_user', 'segment-user', 'segment@scope.test', TRUE);
             INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, repo_config, policy
             ) VALUES (
                '{repo_id}', 'segment-user', 'repo', 'segment_user', 'Ready', 1,
                '{{\"kind\":\"scope.repo-config\",\"version\":1,\"visibility\":{{\"default\":\"private\",\"rules\":[]}}}}'::jsonb,
                '{{\"default_visibility\":\"Private\",\"rules\":[]}}'::jsonb
             )"
        ))
        .await
        .unwrap();
    store
}

fn segment() -> GitSegmentRef {
    segment_named("segment-1", 'a')
}

fn segment_named(segment_id: &str, digest: char) -> GitSegmentRef {
    GitSegmentRef {
        segment_id: segment_id.to_string(),
        sha256: digest.to_string().repeat(64),
        plaintext_bytes: 1_024,
        encoding_version: 2,
    }
}

#[tokio::test]
async fn upload_ledger_enforces_publication_and_recovery_states() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    let segment = segment();
    repositories
        .begin_git_segment_upload(
            "segment-user/repo",
            &segment.segment_id,
            "git/segments/v2/segment-user/repo/segment-1",
            segment.encoding_version,
            10,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&segment, 1_100, 11)
        .await
        .unwrap();
    assert!(
        repositories
            .touch_git_segment_upload(&segment.segment_id, 20)
            .await
            .unwrap()
    );

    assert!(
        repositories
            .load_stale_git_segment_uploads(19, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let stale = repositories
        .load_stale_git_segment_uploads(20, 10)
        .await
        .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].state, GitSegmentUploadState::Ready);

    let span = GitPackSpan {
        first_sequence: 1,
        last_sequence: 1,
        geometric_tier: 0,
        base_oid: None,
        head_oid: "a".repeat(40),
        segment: segment.clone(),
    };
    entities::git_pack_span::Model::from_domain("segment-user/repo", &span)
        .unwrap()
        .into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_published(&segment.segment_id, 12)
        .await
        .unwrap();
    assert!(
        !repositories
            .abandon_git_segment_upload(&segment.segment_id, 13)
            .await
            .unwrap()
    );
    assert!(
        repositories
            .mark_git_segment_upload_deleting(&segment.segment_id, 13)
            .await
            .is_err()
    );

    entities::git_pack_span::Entity::delete_many()
        .exec(store.db.as_ref())
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_deleting(&segment.segment_id, 13)
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_deleted(&segment.segment_id, 14)
        .await
        .unwrap();
    assert!(
        repositories
            .load_stale_git_segment_uploads(14, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn repository_git_write_lease_is_session_scoped() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    let first = repositories
        .acquire_git_write_lease("segment-user/repo")
        .await
        .unwrap();
    let waiting_store = repositories.clone();
    let mut waiting = tokio::spawn(async move {
        waiting_store
            .acquire_git_write_lease("segment-user/repo")
            .await
            .unwrap()
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiting)
            .await
            .is_err()
    );
    first.release().await;
    let second = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    second.release().await;
}

#[tokio::test]
async fn repository_deletion_keeps_segment_ledger_for_physical_cleanup() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    let segment = segment();
    repositories
        .begin_git_segment_upload(
            "segment-user/repo",
            &segment.segment_id,
            "git/segments/v2/segment-user/repo/segment-1",
            segment.encoding_version,
            10,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&segment, 1_100, 11)
        .await
        .unwrap();
    entities::git_pack_span::Model::from_domain(
        "segment-user/repo",
        &GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: "a".repeat(40),
            segment: segment.clone(),
        },
    )
    .unwrap()
    .into_active_model()
    .insert(store.db.as_ref())
    .await
    .unwrap();
    repositories
        .mark_git_segment_upload_published(&segment.segment_id, 12)
        .await
        .unwrap();

    repositories
        .delete_repo(
            "segment-user",
            "repo",
            "segment_user",
            20,
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

    let recoverable = repositories
        .load_stale_git_segment_uploads(20, 10)
        .await
        .unwrap();
    assert_eq!(recoverable.len(), 1);
    assert_eq!(recoverable[0].state, GitSegmentUploadState::Deleting);
}

#[tokio::test]
async fn trigger_and_run_pins_block_compaction_retirement() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    for (index, ref_kind) in ["push_trigger_source", "run_source"]
        .into_iter()
        .enumerate()
    {
        let segment = segment_named(&format!("segment-pin-{index}"), ['b', 'c'][index]);
        repositories
            .begin_git_segment_upload(
                "segment-user/repo",
                &segment.segment_id,
                &format!("git/segments/v2/segment-user/repo/{}", segment.segment_id),
                segment.encoding_version,
                10,
            )
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_ready(&segment, 1_100, 11)
            .await
            .unwrap();
        entities::git_pack_span::Model::from_domain(
            "segment-user/repo",
            &GitPackSpan {
                first_sequence: index as u64 + 1,
                last_sequence: index as u64 + 1,
                geometric_tier: 0,
                base_oid: (index > 0).then(|| "a".repeat(40)),
                head_oid: ['a', 'd'][index].to_string().repeat(40),
                segment: segment.clone(),
            },
        )
        .unwrap()
        .into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
        repositories
            .mark_git_segment_upload_published(&segment.segment_id, 12)
            .await
            .unwrap();
        insert_git_segment_references(
            store.db.as_ref(),
            ref_kind,
            &format!("pin-{index}"),
            [&segment],
        )
        .await
        .unwrap();

        entities::git_pack_span::Entity::delete_by_id((
            "segment-user/repo".to_string(),
            i64::try_from(index + 1).unwrap(),
        ))
        .exec(store.db.as_ref())
        .await
        .unwrap();
        assert!(
            !retire_git_segment(store.db.as_ref(), &segment.segment_id, 13)
                .await
                .unwrap()
        );
        let row = entities::git_segment_upload::Entity::find_by_id(&segment.segment_id)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_into_domain().unwrap().state,
            GitSegmentUploadState::Published
        );

        release_git_segment_references(store.db.as_ref(), ref_kind, &format!("pin-{index}"), 14)
            .await
            .unwrap();
        let row = entities::git_segment_upload::Entity::find_by_id(&segment.segment_id)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_into_domain().unwrap().state,
            GitSegmentUploadState::Deleting
        );
    }
}
