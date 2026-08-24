use super::*;
use scope_cache_domain::{DeletionCandidate, EvictionDecision};

impl CacheStore {
    pub async fn expire_references(&self, now_unix: u64, limit: u64) -> Result<u64, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let rows = tx
            .query_all(statement(
                "SELECT repository_id, identity_digest, compatibility_group_digest,
                        checksum_sha256,
                        last_accessed_at_unix, expires_at_unix
                 FROM scope_cache_references
                 WHERE expires_at_unix <= $1
                 ORDER BY expires_at_unix
                 FOR UPDATE SKIP LOCKED LIMIT $2",
                vec![to_i64(now_unix)?.into(), to_i64(limit)?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        for row in &rows {
            let repository_id: String = row
                .try_get("", "repository_id")
                .map_err(PostgresError::internal)?;
            let identity_digest: String = row
                .try_get("", "identity_digest")
                .map_err(PostgresError::internal)?;
            let checksum: String = row
                .try_get("", "checksum_sha256")
                .map_err(PostgresError::internal)?;
            let reference = domain_reference_from_query(&repository_id, &identity_digest, row)?;
            let EvictionDecision::RemoveReference { deletion, .. } =
                scope_cache_domain::decide_reference_eviction(
                    CachePolicy,
                    &reference,
                    0,
                    now_unix,
                )?
            else {
                return Err(PostgresError::internal_message(
                    "expired cache reference was unexpectedly retained",
                ));
            };
            tx.execute(statement(
                "DELETE FROM scope_cache_references
                 WHERE repository_id = $1 AND identity_digest = $2",
                vec![repository_id.clone().into(), identity_digest.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
            queue_if_unreferenced(
                &tx,
                &repository_id,
                &checksum,
                to_i64(deletion.eligible_after_unix())?,
            )
            .await?;
        }
        let count = rows.len() as u64;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(count)
    }

    pub async fn expire_uploads(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<CacheUploadRecord>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let rows = tx
            .query_all(statement(
                "WITH due AS (
                    SELECT upload_id FROM scope_cache_uploads
                    WHERE state = 'active' AND expires_at_unix <= $1
                    ORDER BY expires_at_unix
                    FOR UPDATE SKIP LOCKED LIMIT $2
                 )
                 UPDATE scope_cache_uploads u SET state = 'deleting'
                 FROM due WHERE u.upload_id = due.upload_id
                 RETURNING u.upload_id, u.repository_id, u.identity_digest,
                    u.compatibility_group_digest, u.checksum_sha256,
                    u.storage_backend, u.object_key, u.size_bytes,
                    u.state, u.created_at_unix, u.expires_at_unix",
                vec![to_i64(now_unix)?.into(), to_i64(limit)?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let uploads = rows
            .iter()
            .map(decode_upload)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(uploads)
    }

    pub async fn complete_upload_cleanup(&self, upload_id: &str) -> Result<(), PostgresError> {
        self.db
            .execute(statement(
                "DELETE FROM scope_cache_uploads
                 WHERE upload_id = $1 AND state = 'deleting'",
                vec![upload_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn retry_upload_cleanup(&self, upload_id: &str) -> Result<(), PostgresError> {
        self.db
            .execute(statement(
                "UPDATE scope_cache_uploads SET state = 'active'
                 WHERE upload_id = $1 AND state = 'deleting'",
                vec![upload_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn expire_committed_uploads(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<u64, PostgresError> {
        let result = self
            .db
            .execute(statement(
                "DELETE FROM scope_cache_uploads
                 WHERE upload_id IN (
                    SELECT upload_id FROM scope_cache_uploads
                    WHERE state = 'committed' AND expires_at_unix <= $1
                    ORDER BY expires_at_unix LIMIT $2
                 )",
                vec![to_i64(now_unix)?.into(), to_i64(limit)?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected())
    }

    pub async fn claim_deletions(
        &self,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<PendingCacheDeletion>, PostgresError> {
        let candidates = self
            .db
            .query_all(statement(
                "SELECT repository_id, checksum_sha256
                 FROM scope_cache_deletion_queue
                 WHERE not_before_unix <= $1
                 ORDER BY not_before_unix, repository_id, checksum_sha256
                 LIMIT $2",
                vec![to_i64(now_unix)?.into(), to_i64(limit)?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let mut deletions = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let repository_id: String = candidate
                .try_get("", "repository_id")
                .map_err(PostgresError::internal)?;
            let checksum_sha256: String = candidate
                .try_get("", "checksum_sha256")
                .map_err(PostgresError::internal)?;
            let tx = self.db.begin().await.map_err(PostgresError::internal)?;
            lock_repository(&tx, &repository_id).await?;
            let row = tx
                .query_one(statement(
                    "WITH due AS (
                        SELECT not_before_unix
                        FROM scope_cache_deletion_queue
                        WHERE repository_id = $1 AND checksum_sha256 = $2
                          AND not_before_unix <= $3
                        FOR UPDATE
                     )
                     UPDATE scope_cache_deletion_queue q
                     SET attempts = q.attempts + 1, not_before_unix = $4
                     FROM due, scope_cache_objects o
                     WHERE q.repository_id = $1 AND q.checksum_sha256 = $2
                       AND o.repository_id = q.repository_id
                       AND o.checksum_sha256 = q.checksum_sha256
                       AND NOT EXISTS (
                        SELECT 1 FROM scope_cache_references r
                        WHERE r.repository_id = q.repository_id
                          AND r.checksum_sha256 = q.checksum_sha256
                       )
                     RETURNING q.repository_id, q.checksum_sha256, o.object_key,
                        q.attempts, due.not_before_unix AS eligible_after_unix",
                    vec![
                        repository_id.into(),
                        checksum_sha256.into(),
                        to_i64(now_unix)?.into(),
                        to_i64(retry_at_unix)?.into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            let Some(row) = row else {
                tx.commit().await.map_err(PostgresError::internal)?;
                continue;
            };
            let deletion = decode_deletion(row)?;
            let candidate = DeletionCandidate::restore(
                RepositoryId::parse(deletion.repository_id.clone())?,
                CacheDigest::parse(deletion.checksum_sha256.clone())?,
                deletion.eligible_after_unix,
            );
            if !matches!(
                scope_cache_domain::decide_object_deletion(&candidate, 0, now_unix),
                EvictionDecision::DeleteObject { .. }
            ) {
                return Err(PostgresError::internal_message(
                    "due unreferenced cache object was unexpectedly retained",
                ));
            }
            tx.commit().await.map_err(PostgresError::internal)?;
            deletions.push(deletion);
        }
        Ok(deletions)
    }

    pub async fn complete_deletion(
        &self,
        repository_id: &str,
        checksum_sha256: &str,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        lock_repository(&tx, repository_id).await?;
        let result = tx
            .execute(statement(
                "DELETE FROM scope_cache_objects o
                 WHERE o.repository_id = $1 AND o.checksum_sha256 = $2
                   AND EXISTS (
                    SELECT 1 FROM scope_cache_deletion_queue q
                    WHERE q.repository_id = o.repository_id
                      AND q.checksum_sha256 = o.checksum_sha256
                      AND q.attempts > 0
                   )
                   AND NOT EXISTS (
                    SELECT 1 FROM scope_cache_references r
                    WHERE r.repository_id = o.repository_id
                      AND r.checksum_sha256 = o.checksum_sha256
                   )",
                vec![repository_id.into(), checksum_sha256.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn fail_deletion(
        &self,
        deletion: &PendingCacheDeletion,
        retry_at_unix: u64,
        error: &str,
    ) -> Result<(), PostgresError> {
        let message = error.chars().take(8192).collect::<String>();
        self.db
            .execute(statement(
                "UPDATE scope_cache_deletion_queue
                 SET not_before_unix = $3, last_error = $4
                 WHERE repository_id = $1 AND checksum_sha256 = $2",
                vec![
                    deletion.repository_id.clone().into(),
                    deletion.checksum_sha256.clone().into(),
                    to_i64(retry_at_unix)?.into(),
                    message.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub(super) async fn expire_repository_references(
    tx: &DatabaseTransaction,
    repository_id: &str,
    now: i64,
) -> Result<(), PostgresError> {
    let rows = tx
        .query_all(statement(
            "SELECT identity_digest, compatibility_group_digest, checksum_sha256,
                    last_accessed_at_unix, expires_at_unix
             FROM scope_cache_references
             WHERE repository_id = $1 AND expires_at_unix <= $2
             FOR UPDATE",
            vec![repository_id.into(), now.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    for row in rows {
        let identity: String = row
            .try_get("", "identity_digest")
            .map_err(PostgresError::internal)?;
        let checksum: String = row
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?;
        let reference = domain_reference_from_query(repository_id, &identity, &row)?;
        let EvictionDecision::RemoveReference { deletion, .. } =
            scope_cache_domain::decide_reference_eviction(
                CachePolicy,
                &reference,
                0,
                from_i64(now)?,
            )?
        else {
            return Err(PostgresError::internal_message(
                "expired cache reference was unexpectedly retained",
            ));
        };
        tx.execute(statement(
            "DELETE FROM scope_cache_references
             WHERE repository_id = $1 AND identity_digest = $2",
            vec![repository_id.into(), identity.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        queue_if_unreferenced(
            tx,
            repository_id,
            &checksum,
            to_i64(deletion.eligible_after_unix())?,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn make_repository_room(
    tx: &DatabaseTransaction,
    repository_id: &str,
    protected_identity: &str,
    additional_bytes: i64,
    budget_bytes: i64,
    now_unix: u64,
) -> Result<(), PostgresError> {
    loop {
        let usage = active_repository_bytes(tx, repository_id).await?;
        if usage.saturating_add(additional_bytes) <= budget_bytes {
            return Ok(());
        }
        let victim = tx
            .query_one(statement(
                "SELECT identity_digest, compatibility_group_digest, checksum_sha256,
                        last_accessed_at_unix, expires_at_unix
                 FROM scope_cache_references
                 WHERE repository_id = $1 AND identity_digest <> $2
                 ORDER BY last_accessed_at_unix, identity_digest
                 FOR UPDATE LIMIT 1",
                vec![repository_id.into(), protected_identity.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::resource_exhausted(
                    "repository cache budget cannot fit the requested object",
                )
            })?;
        let identity: String = victim
            .try_get("", "identity_digest")
            .map_err(PostgresError::internal)?;
        let checksum: String = victim
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?;
        let reference = domain_reference_from_query(repository_id, &identity, &victim)?;
        let storage_with_upload = from_i64(usage.saturating_add(additional_bytes))?;
        let EvictionDecision::RemoveReference { deletion, .. } =
            scope_cache_domain::decide_reference_eviction(
                CachePolicy,
                &reference,
                storage_with_upload,
                now_unix,
            )?
        else {
            return Err(PostgresError::internal_message(
                "over-budget cache reference was unexpectedly retained",
            ));
        };
        tx.execute(statement(
            "DELETE FROM scope_cache_references
             WHERE repository_id = $1 AND identity_digest = $2",
            vec![repository_id.into(), identity.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        queue_if_unreferenced(
            tx,
            repository_id,
            &checksum,
            to_i64(deletion.eligible_after_unix())?,
        )
        .await?;
    }
}

pub(super) async fn active_repository_bytes(
    tx: &DatabaseTransaction,
    repository_id: &str,
) -> Result<i64, PostgresError> {
    let row = tx
        .query_one(statement(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint AS bytes FROM (
                SELECT o.size_bytes
                FROM scope_cache_objects o
                WHERE o.repository_id = $1 AND EXISTS (
                    SELECT 1 FROM scope_cache_references r
                    WHERE r.repository_id = o.repository_id
                      AND r.checksum_sha256 = o.checksum_sha256
                )
                UNION ALL
                SELECT u.size_bytes
                FROM scope_cache_uploads u
                WHERE u.repository_id = $1 AND u.state IN ('active', 'deleting')
             ) active_cache_bytes",
            vec![repository_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("cache usage query returned no row"))?;
    row.try_get("", "bytes").map_err(PostgresError::internal)
}

pub(super) async fn queue_if_unreferenced(
    tx: &DatabaseTransaction,
    repository_id: &str,
    checksum_sha256: &str,
    not_before: i64,
) -> Result<(), PostgresError> {
    tx.execute(statement(
        "INSERT INTO scope_cache_deletion_queue (
            repository_id, checksum_sha256, not_before_unix, attempts, last_error
         )
         SELECT $1, $2, $3, 0, NULL
         WHERE EXISTS (
            SELECT 1 FROM scope_cache_objects
            WHERE repository_id = $1 AND checksum_sha256 = $2
         ) AND NOT EXISTS (
            SELECT 1 FROM scope_cache_references
            WHERE repository_id = $1 AND checksum_sha256 = $2
         )
         ON CONFLICT (repository_id, checksum_sha256) DO UPDATE SET
            not_before_unix = GREATEST(
                scope_cache_deletion_queue.not_before_unix,
                EXCLUDED.not_before_unix
            ),
            last_error = NULL",
        vec![
            repository_id.into(),
            checksum_sha256.into(),
            not_before.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn decode_deletion(row: QueryResult) -> Result<PendingCacheDeletion, PostgresError> {
    Ok(PendingCacheDeletion {
        repository_id: row
            .try_get("", "repository_id")
            .map_err(PostgresError::internal)?,
        checksum_sha256: row
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?,
        object_key: row
            .try_get("", "object_key")
            .map_err(PostgresError::internal)?,
        attempts: u32::try_from(
            row.try_get::<i32>("", "attempts")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
        eligible_after_unix: from_i64(
            row.try_get("", "eligible_after_unix")
                .map_err(PostgresError::internal)?,
        )?,
    })
}
