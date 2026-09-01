use super::git_segment_streaming_v2_support::*;
use super::isolated_database;
use crate::{
    db::git_segment_v2_backfill::{CREATE_BACKFILL_TABLE, GitSegmentV2Backfill},
    migrations,
};
use scope_domain::runs::source::RunSource;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn v2_cutover_refuses_segments_without_completed_object_backfill() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();

    let error = migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires every legacy Git segment to be backfilled")
    );
    assert!(!table_exists(db.as_ref(), "scope_git_segment_uploads").await);
    assert!(column_exists(db.as_ref(), "scope_git_segments", "object_key").await);
}

#[tokio::test]
async fn v2_cutover_preserves_segments_and_pinned_sources_from_completed_backfill() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();
    insert_completed_backfill(db.as_ref()).await;

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let upload = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "SELECT repo_id, state, sha256, plaintext_bytes, encrypted_bytes, encoding_version
                 FROM scope_git_segment_uploads WHERE segment_id = '{SEGMENT_ID}'"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        upload.try_get::<String>("", "repo_id").unwrap(),
        REPOSITORY_ID
    );
    assert_eq!(upload.try_get::<String>("", "state").unwrap(), "published");
    assert_eq!(
        upload.try_get::<String>("", "sha256").unwrap(),
        LEGACY_SHA256
    );
    assert_eq!(upload.try_get::<i64>("", "plaintext_bytes").unwrap(), 3);
    assert_eq!(upload.try_get::<i64>("", "encrypted_bytes").unwrap(), 99);
    assert_eq!(upload.try_get::<i32>("", "encoding_version").unwrap(), 2);

    let run_source = json_column(
        db.as_ref(),
        "SELECT source AS value FROM scope_runs WHERE id = 'pinned-run'",
    )
    .await;
    let source: RunSource = serde_json::from_value(run_source.clone()).unwrap();
    assert_eq!(
        source.logical_git_head().unwrap().2[0].segment.segment_id,
        SEGMENT_ID
    );
    assert!(run_source["pack_spans"][0].get("object").is_none());

    let push_payload = json_column(
        db.as_ref(),
        "SELECT payload AS value FROM scope_outbox_jobs WHERE id = 'pending-push'",
    )
    .await;
    assert_eq!(
        push_payload["pack_spans"][0]["segment"]["segment_id"],
        SEGMENT_ID
    );
    assert!(push_payload["pack_spans"][0].get("object").is_none());

    let references = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT ref_kind, ref_id FROM scope_git_segment_references ORDER BY ref_kind, ref_id"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "ref_kind").unwrap(),
                row.try_get::<String>("", "ref_id").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        references,
        [
            (
                "push_trigger_source".to_string(),
                format!("{REPOSITORY_ID}:1")
            ),
            ("run_source".to_string(), "pinned-run".to_string()),
        ]
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitSegmentSha256'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitManifestSha256'"
        )
        .await,
        1
    );
    assert!(!table_exists(db.as_ref(), "scope_git_segment_v2_backfill").await);
    let cleanup = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "SELECT generation, sha256, git_oid, size_bytes, completed_at_unix
                 FROM scope_orphan_object_jobs
                 WHERE object_key = jsonb_build_object(
                     'GitSegmentSha256', '{LEGACY_SHA256}'
                 )::text"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cleanup.try_get::<String>("", "generation").unwrap(),
        "m0033_git_segment_streaming_v2"
    );
    assert_eq!(
        cleanup.try_get::<String>("", "sha256").unwrap(),
        LEGACY_SHA256
    );
    assert_eq!(
        cleanup.try_get::<String>("", "git_oid").unwrap(),
        "b".repeat(40)
    );
    assert_eq!(cleanup.try_get::<i64>("", "size_bytes").unwrap(), 3);
    assert!(
        cleanup
            .try_get::<Option<i64>>("", "completed_at_unix")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn v2_backfill_enumerates_live_and_retained_segments_with_the_same_start() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_compacted_legacy_git_state(db.as_ref()).await;
    drop(db);

    let backfill = GitSegmentV2Backfill::begin(target.schema_database_url())
        .await
        .unwrap()
        .unwrap();
    let segments = backfill.legacy_segments().await.unwrap();
    assert_eq!(
        segments
            .iter()
            .map(|segment| {
                (
                    segment.first_sequence,
                    segment.last_sequence,
                    segment.sha256.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        [(1, 1, LEGACY_SHA256), (1, 2, LIVE_SHA256)]
    );
}

#[tokio::test]
async fn v2_backfill_deduplicates_compact_live_and_jsonb_retained_keys() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    drop(db);

    let backfill = GitSegmentV2Backfill::begin(target.schema_database_url())
        .await
        .unwrap()
        .unwrap();
    let segments = backfill.legacy_segments().await.unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].sha256, LEGACY_SHA256);
    assert_eq!(
        segments[0].legacy_object_key,
        format!(r#"{{"GitSegmentSha256": "{LEGACY_SHA256}"}}"#)
    );
}

#[tokio::test]
async fn v2_backfill_rejects_invalid_retained_object_references() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_compacted_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(&format!(
        "UPDATE scope_runs
         SET source = jsonb_set(
             source,
             '{{pack_spans,0,object,content_ref}}',
             jsonb_build_object('GitManifestSha256', '{LEGACY_SHA256}')
         )
         WHERE id = 'pinned-run'"
    ))
    .await
    .unwrap();
    drop(db);

    let backfill = GitSegmentV2Backfill::begin(target.schema_database_url())
        .await
        .unwrap()
        .unwrap();
    let error = backfill.legacy_segments().await.unwrap_err();
    assert!(error.to_string().contains("invalid or conflicting"));
}

#[tokio::test]
async fn v2_backfill_reports_malformed_retained_span_shapes() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_compacted_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(
        r#"UPDATE scope_runs
           SET source = jsonb_set(
               source, '{pack_spans,0,first_sequence}', '"not-a-sequence"'::jsonb
           )
           WHERE id = 'pinned-run'"#,
    )
    .await
    .unwrap();
    drop(db);

    let backfill = GitSegmentV2Backfill::begin(target.schema_database_url())
        .await
        .unwrap()
        .unwrap();
    let error = backfill.legacy_segments().await.unwrap_err();
    assert!(error.to_string().contains("metadata is malformed"));
}

#[tokio::test]
async fn v2_cutover_distinguishes_live_and_retained_segment_incarnations() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_compacted_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();
    insert_compacted_backfills(db.as_ref()).await;

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let uploads = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT segment_id, state FROM scope_git_segment_uploads ORDER BY segment_id"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "segment_id").unwrap(),
                row.try_get::<String>("", "state").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        uploads,
        [
            (SEGMENT_ID.to_string(), "retained".to_string()),
            (LIVE_SEGMENT_ID.to_string(), "published".to_string()),
        ]
    );

    let live_segment = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT first_sequence, last_sequence, segment_id FROM scope_git_segments".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        live_segment.try_get::<i64>("", "first_sequence").unwrap(),
        1
    );
    assert_eq!(live_segment.try_get::<i64>("", "last_sequence").unwrap(), 2);
    assert_eq!(
        live_segment.try_get::<String>("", "segment_id").unwrap(),
        LIVE_SEGMENT_ID
    );

    let run_source: RunSource = serde_json::from_value(
        json_column(
            db.as_ref(),
            "SELECT source AS value FROM scope_runs WHERE id = 'pinned-run'",
        )
        .await,
    )
    .unwrap();
    assert_eq!(
        run_source.logical_git_head().unwrap().2[0]
            .segment
            .segment_id,
        SEGMENT_ID
    );
    let push_payload = json_column(
        db.as_ref(),
        "SELECT payload AS value FROM scope_outbox_jobs WHERE id = 'pending-push'",
    )
    .await;
    assert_eq!(
        push_payload["pack_spans"][0]["segment"]["segment_id"],
        SEGMENT_ID
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_git_segment_references
             WHERE segment_id = '11111111111111111111111111111111'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar_i64(
            db.as_ref(),
            "SELECT count(*) AS value FROM scope_orphan_object_jobs
             WHERE generation = 'm0033_git_segment_streaming_v2'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn v2_cutover_rejects_conflicting_retained_metadata() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_compacted_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();
    insert_compacted_backfills(db.as_ref()).await;
    db.execute_unprepared(
        "UPDATE scope_outbox_jobs
         SET payload = jsonb_set(
             payload, '{pack_spans,0,head_oid}', to_jsonb(repeat('c', 40))
         )
         WHERE id = 'pending-push'",
    )
    .await
    .unwrap();

    let error = migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("conflicting metadata"));
}

#[tokio::test]
async fn v2_cutover_rejects_stale_exact_identity_backfills() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(32))
        .await
        .unwrap();
    insert_legacy_git_state(db.as_ref()).await;
    db.execute_unprepared(CREATE_BACKFILL_TABLE).await.unwrap();
    insert_completed_backfill(db.as_ref()).await;
    db.execute_unprepared(
        "INSERT INTO scope_git_segment_v2_backfill (
            repo_id, first_sequence, last_sequence,
            legacy_object_key, legacy_sha256, legacy_size_bytes,
            segment_id, object_key, sha256, plaintext_bytes,
            encrypted_bytes, encoding_version, completed_at_unix
         ) VALUES (
            'cutover-user/repo', 2, 2,
            jsonb_build_object('GitSegmentSha256', repeat('c', 64))::text,
            repeat('c', 64), 7, repeat('3', 32),
            'git/segments/v2/repository-hash/33333333333333333333333333333333',
            repeat('c', 64), 7, 100, 2, 10
         )",
    )
    .await
    .unwrap();

    let error = migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stale Git segment backfill"));
}

#[tokio::test]
async fn v2_cutover_replaces_generic_object_columns_with_segment_identity() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'scope_git_segments'
             ORDER BY column_name"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "column_name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "base_oid",
            "first_sequence",
            "geometric_tier",
            "head_oid",
            "last_sequence",
            "repo_id",
            "segment_id",
        ]
    );
}
