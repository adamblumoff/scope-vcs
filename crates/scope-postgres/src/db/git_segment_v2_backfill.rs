use super::ExclusiveWriterFence;
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

const MIGRATION_NAME: &str = "m0033_git_segment_streaming_v2";

pub(crate) const CREATE_BACKFILL_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS scope_git_segment_v2_backfill (
    repo_id text NOT NULL,
    first_sequence bigint NOT NULL,
    last_sequence bigint NOT NULL,
    legacy_object_key text NOT NULL,
    legacy_sha256 text NOT NULL,
    legacy_size_bytes bigint NOT NULL,
    segment_id text NOT NULL UNIQUE,
    object_key text NOT NULL UNIQUE,
    sha256 text NOT NULL,
    plaintext_bytes bigint NOT NULL,
    encrypted_bytes bigint NOT NULL,
    encoding_version integer NOT NULL,
    completed_at_unix bigint NOT NULL,
    PRIMARY KEY (repo_id, first_sequence, last_sequence, legacy_object_key),
    CONSTRAINT scope_git_segment_v2_backfill_values CHECK (
        first_sequence > 0 AND
        last_sequence >= first_sequence AND
        legacy_size_bytes >= 0 AND
        length(legacy_sha256) = 64 AND
        legacy_sha256 ~ '^[0-9a-f]+$' AND
        legacy_object_key::jsonb =
            jsonb_build_object('GitSegmentSha256', legacy_sha256) AND
        length(segment_id) = 32 AND
        segment_id ~ '^[0-9a-f]+$' AND
        object_key LIKE 'git/segments/v2/%/' || segment_id AND
        length(sha256) = 64 AND
        sha256 ~ '^[0-9a-f]+$' AND
        sha256 = legacy_sha256 AND
        plaintext_bytes = legacy_size_bytes AND
        encrypted_bytes > 0 AND
        encoding_version = 2 AND
        completed_at_unix > 0
    )
);
ALTER TABLE scope_git_segment_v2_backfill
    DROP CONSTRAINT IF EXISTS scope_git_segment_v2_backfill_pkey,
    ADD CONSTRAINT scope_git_segment_v2_backfill_pkey
        PRIMARY KEY (repo_id, first_sequence, last_sequence, legacy_object_key);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyGitSegment {
    pub repository_id: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub legacy_object_key: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub prepared: Option<GitSegmentV2BackfillRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSegmentV2BackfillRecord {
    pub segment_id: String,
    pub object_key: String,
    pub sha256: String,
    pub plaintext_bytes: u64,
    pub encrypted_bytes: u64,
    pub encoding_version: u32,
    pub completed_at_unix: u64,
}

pub struct GitSegmentV2Backfill {
    db: DatabaseConnection,
    _fence: ExclusiveWriterFence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyGitSegmentObject {
    pub object_key: String,
    pub sha256: String,
}

pub struct GitSegmentV1Cleanup {
    db: DatabaseConnection,
}

impl GitSegmentV1Cleanup {
    pub async fn begin(database_url: String) -> anyhow::Result<Self> {
        let db = Database::connect(database_url).await?;
        let migrated = scalar_i64(
            &db,
            "SELECT count(*) AS value FROM seaql_migrations WHERE version = 'm0033_git_segment_streaming_v2'",
        )
        .await?;
        if migrated != 1 {
            anyhow::bail!("refusing legacy Git segment cleanup before m0033 commits");
        }
        Ok(Self { db })
    }

    pub async fn legacy_objects(&self) -> anyhow::Result<Vec<LegacyGitSegmentObject>> {
        let referenced = scalar_i64(
            &self.db,
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitSegmentSha256'",
        )
        .await?;
        if referenced != 0 {
            anyhow::bail!("refusing to delete referenced legacy Git segment objects");
        }
        self.db
            .query_all(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"
                SELECT object_key, sha256
                FROM scope_orphan_object_jobs
                WHERE object_key::jsonb =
                      jsonb_build_object('GitSegmentSha256', sha256)
                ORDER BY object_key
                "#
                .to_string(),
            ))
            .await?
            .into_iter()
            .map(|row| {
                Ok(LegacyGitSegmentObject {
                    object_key: required(&row, "object_key")?,
                    sha256: required(&row, "sha256")?,
                })
            })
            .collect()
    }

    pub async fn remove_record(&self, object: &LegacyGitSegmentObject) -> anyhow::Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_orphan_object_jobs
                 WHERE object_key = $1 AND sha256 = $2
                   AND object_key::jsonb = jsonb_build_object('GitSegmentSha256', sha256)",
                [
                    object.object_key.clone().into(),
                    object.sha256.clone().into(),
                ],
            ))
            .await?;
        Ok(())
    }
}

impl GitSegmentV2Backfill {
    pub async fn begin(database_url: String) -> anyhow::Result<Option<Self>> {
        let fence = ExclusiveWriterFence::acquire(&database_url).await?;
        let db = Database::connect(database_url).await?;
        let plan = crate::migrations::plan(&db).await?;
        if !plan
            .pending
            .iter()
            .any(|migration| migration.name == MIGRATION_NAME)
        {
            return Ok(None);
        }
        db.execute_unprepared(CREATE_BACKFILL_TABLE).await?;
        Ok(Some(Self { db, _fence: fence }))
    }

    pub async fn legacy_segments(&self) -> anyhow::Result<Vec<LegacyGitSegment>> {
        let conflicts = scalar_i64(
            &self.db,
            r#"
            WITH legacy_sources AS (
                SELECT repo_id, first_sequence, last_sequence,
                       (object_key::jsonb)::text AS legacy_object_key,
                       sha256 AS legacy_sha256, size_bytes AS legacy_size_bytes,
                       geometric_tier, base_oid, head_oid
                FROM scope_git_segments
                UNION ALL
                SELECT runs.source->>'repository_id',
                       (span->>'first_sequence')::bigint,
                       (span->>'last_sequence')::bigint,
                       (span#>'{object,content_ref}')::text,
                       span#>>'{object,sha256}',
                       (span#>>'{object,size_bytes}')::bigint,
                       (span->>'geometric_tier')::integer,
                       span->>'base_oid', span->>'head_oid'
                FROM scope_runs runs
                CROSS JOIN LATERAL jsonb_array_elements(runs.source->'pack_spans') span
                WHERE runs.source->>'kind' = 'accepted-git-head'
                UNION ALL
                SELECT jobs.repo_id,
                       (span->>'first_sequence')::bigint,
                       (span->>'last_sequence')::bigint,
                       (span#>'{object,content_ref}')::text,
                       span#>>'{object,sha256}',
                       (span#>>'{object,size_bytes}')::bigint,
                       (span->>'geometric_tier')::integer,
                       span->>'base_oid', span->>'head_oid'
                FROM scope_outbox_jobs jobs
                CROSS JOIN LATERAL jsonb_array_elements(jobs.payload->'pack_spans') span
                WHERE jobs.kind = 'push_main_trigger_evaluation'
                  AND jobs.completed_at_unix IS NULL
            )
            SELECT count(*) AS value
            FROM (
                SELECT 1
                FROM legacy_sources spans
                WHERE spans.repo_id IS NULL
                   OR length(btrim(spans.repo_id)) = 0
                   OR spans.first_sequence IS NULL
                   OR spans.first_sequence <= 0
                   OR spans.last_sequence IS NULL
                   OR spans.last_sequence < spans.first_sequence
                   OR spans.geometric_tier IS NULL
                   OR spans.geometric_tier NOT BETWEEN 0 AND 62
                   OR spans.last_sequence - spans.first_sequence + 1 <>
                      power(2::numeric, spans.geometric_tier)
                   OR spans.legacy_sha256 IS NULL
                   OR length(spans.legacy_sha256) <> 64
                   OR spans.legacy_sha256 !~ '^[0-9a-f]+$'
                   OR spans.legacy_size_bytes IS NULL
                   OR spans.legacy_size_bytes < 0
                   OR spans.legacy_object_key IS NULL
                   OR spans.legacy_object_key::jsonb <>
                      jsonb_build_object('GitSegmentSha256', spans.legacy_sha256)
                   OR spans.head_oid IS NULL
                   OR length(btrim(spans.head_oid)) = 0
                UNION ALL
                SELECT 1
                FROM legacy_sources
                GROUP BY repo_id, first_sequence, last_sequence, legacy_object_key
                HAVING count(DISTINCT jsonb_build_object(
                    'sha256', legacy_sha256,
                    'size_bytes', legacy_size_bytes,
                    'geometric_tier', geometric_tier,
                    'base_oid', base_oid,
                    'head_oid', head_oid
                )) > 1
            ) conflicts
            "#,
        )
        .await
        .map_err(|error| anyhow::anyhow!("legacy Git segment metadata is malformed: {error}"))?;
        if conflicts != 0 {
            anyhow::bail!("legacy Git segment metadata is invalid or conflicting");
        }

        let rows = self
            .db
            .query_all(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"
                WITH legacy_sources AS (
                    SELECT repo_id, first_sequence, last_sequence,
                           (object_key::jsonb)::text AS legacy_object_key,
                           sha256 AS legacy_sha256, size_bytes AS legacy_size_bytes,
                           geometric_tier, base_oid, head_oid
                    FROM scope_git_segments
                    UNION ALL
                    SELECT runs.source->>'repository_id',
                           (span->>'first_sequence')::bigint,
                           (span->>'last_sequence')::bigint,
                           (span#>'{object,content_ref}')::text,
                           span#>>'{object,sha256}',
                           (span#>>'{object,size_bytes}')::bigint,
                           (span->>'geometric_tier')::integer,
                           span->>'base_oid', span->>'head_oid'
                    FROM scope_runs runs
                    CROSS JOIN LATERAL jsonb_array_elements(runs.source->'pack_spans') span
                    WHERE runs.source->>'kind' = 'accepted-git-head'
                    UNION ALL
                    SELECT jobs.repo_id,
                           (span->>'first_sequence')::bigint,
                           (span->>'last_sequence')::bigint,
                           (span#>'{object,content_ref}')::text,
                           span#>>'{object,sha256}',
                           (span#>>'{object,size_bytes}')::bigint,
                           (span->>'geometric_tier')::integer,
                           span->>'base_oid', span->>'head_oid'
                    FROM scope_outbox_jobs jobs
                    CROSS JOIN LATERAL jsonb_array_elements(jobs.payload->'pack_spans') span
                    WHERE jobs.kind = 'push_main_trigger_evaluation'
                      AND jobs.completed_at_unix IS NULL
                ), legacy_segments AS (
                    SELECT DISTINCT repo_id, first_sequence, last_sequence,
                           legacy_object_key, legacy_sha256, legacy_size_bytes,
                           geometric_tier, base_oid, head_oid
                    FROM legacy_sources
                )
                SELECT spans.repo_id, spans.first_sequence, spans.last_sequence,
                       spans.legacy_object_key AS object_key,
                       spans.legacy_sha256 AS sha256,
                       spans.legacy_size_bytes AS size_bytes,
                       prepared.segment_id, prepared.object_key AS prepared_object_key,
                       prepared.sha256 AS prepared_sha256,
                       prepared.plaintext_bytes, prepared.encrypted_bytes,
                       prepared.encoding_version, prepared.completed_at_unix
                FROM legacy_segments spans
                LEFT JOIN scope_git_segment_v2_backfill prepared
                  ON prepared.repo_id = spans.repo_id
                 AND prepared.first_sequence = spans.first_sequence
                 AND prepared.last_sequence = spans.last_sequence
                 AND prepared.legacy_object_key = spans.legacy_object_key
                 AND prepared.legacy_sha256 = spans.legacy_sha256
                 AND prepared.legacy_size_bytes = spans.legacy_size_bytes
                WHERE spans.legacy_object_key::jsonb =
                      jsonb_build_object('GitSegmentSha256', spans.legacy_sha256)
                ORDER BY spans.repo_id, spans.first_sequence, spans.last_sequence,
                         spans.legacy_object_key
                "#
                .to_string(),
            ))
            .await?;
        let mut segments = Vec::with_capacity(rows.len());
        for row in rows {
            let prepared = row
                .try_get::<Option<String>>("", "segment_id")?
                .map(|segment_id| {
                    Ok::<_, anyhow::Error>(GitSegmentV2BackfillRecord {
                        segment_id,
                        object_key: required(&row, "prepared_object_key")?,
                        sha256: required(&row, "prepared_sha256")?,
                        plaintext_bytes: non_negative(&row, "plaintext_bytes")?,
                        encrypted_bytes: non_negative(&row, "encrypted_bytes")?,
                        encoding_version: positive_u32(&row, "encoding_version")?,
                        completed_at_unix: non_negative(&row, "completed_at_unix")?,
                    })
                })
                .transpose()?;
            segments.push(LegacyGitSegment {
                repository_id: required(&row, "repo_id")?,
                first_sequence: positive(&row, "first_sequence")?,
                last_sequence: positive(&row, "last_sequence")?,
                legacy_object_key: required(&row, "object_key")?,
                sha256: required(&row, "sha256")?,
                size_bytes: non_negative(&row, "size_bytes")?,
                prepared,
            });
        }

        Ok(segments)
    }

    pub async fn record(
        &self,
        legacy: &LegacyGitSegment,
        prepared: &GitSegmentV2BackfillRecord,
    ) -> anyhow::Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                INSERT INTO scope_git_segment_v2_backfill (
                    repo_id, first_sequence, last_sequence,
                    legacy_object_key, legacy_sha256, legacy_size_bytes,
                    segment_id, object_key, sha256, plaintext_bytes,
                    encrypted_bytes, encoding_version, completed_at_unix
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                ON CONFLICT (repo_id, first_sequence, last_sequence, legacy_object_key)
                DO NOTHING
                "#,
                [
                    legacy.repository_id.clone().into(),
                    i64::try_from(legacy.first_sequence)?.into(),
                    i64::try_from(legacy.last_sequence)?.into(),
                    legacy.legacy_object_key.clone().into(),
                    legacy.sha256.clone().into(),
                    i64::try_from(legacy.size_bytes)?.into(),
                    prepared.segment_id.clone().into(),
                    prepared.object_key.clone().into(),
                    prepared.sha256.clone().into(),
                    i64::try_from(prepared.plaintext_bytes)?.into(),
                    i64::try_from(prepared.encrypted_bytes)?.into(),
                    i32::try_from(prepared.encoding_version)?.into(),
                    i64::try_from(prepared.completed_at_unix)?.into(),
                ],
            ))
            .await?;

        let matched = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                SELECT count(*) AS value
                FROM scope_git_segment_v2_backfill
                WHERE repo_id = $1 AND first_sequence = $2 AND last_sequence = $3
                  AND legacy_object_key = $4 AND legacy_sha256 = $5
                  AND legacy_size_bytes = $6 AND segment_id = $7
                  AND object_key = $8 AND sha256 = $9 AND plaintext_bytes = $10
                  AND encrypted_bytes = $11 AND encoding_version = $12
                  AND completed_at_unix = $13
                "#,
                [
                    legacy.repository_id.clone().into(),
                    i64::try_from(legacy.first_sequence)?.into(),
                    i64::try_from(legacy.last_sequence)?.into(),
                    legacy.legacy_object_key.clone().into(),
                    legacy.sha256.clone().into(),
                    i64::try_from(legacy.size_bytes)?.into(),
                    prepared.segment_id.clone().into(),
                    prepared.object_key.clone().into(),
                    prepared.sha256.clone().into(),
                    i64::try_from(prepared.plaintext_bytes)?.into(),
                    i64::try_from(prepared.encrypted_bytes)?.into(),
                    i32::try_from(prepared.encoding_version)?.into(),
                    i64::try_from(prepared.completed_at_unix)?.into(),
                ],
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("Git segment backfill record disappeared"))?
            .try_get::<i64>("", "value")?;
        if matched != 1 {
            anyhow::bail!("Git segment backfill record conflicts with an earlier conversion");
        }
        Ok(())
    }
}

fn required(row: &sea_orm::QueryResult, column: &str) -> anyhow::Result<String> {
    Ok(row.try_get::<String>("", column)?)
}

fn positive(row: &sea_orm::QueryResult, column: &str) -> anyhow::Result<u64> {
    let value = row.try_get::<i64>("", column)?;
    if value <= 0 {
        anyhow::bail!("{column} must be positive");
    }
    Ok(u64::try_from(value)?)
}

fn non_negative(row: &sea_orm::QueryResult, column: &str) -> anyhow::Result<u64> {
    Ok(u64::try_from(row.try_get::<i64>("", column)?)?)
}

fn positive_u32(row: &sea_orm::QueryResult, column: &str) -> anyhow::Result<u32> {
    let value = row.try_get::<i32>("", column)?;
    if value <= 0 {
        anyhow::bail!("{column} must be positive");
    }
    Ok(u32::try_from(value)?)
}

async fn scalar_i64(db: &DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
    Ok(db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_string(),
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("database query returned no row"))?
        .try_get::<i64>("", "value")?)
}
