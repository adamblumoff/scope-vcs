use super::CacheStore;
use crate::error::PostgresError;
use scope_cache_domain::{
    CacheDigest, CacheDomainError, CacheObject, CachePolicy, CacheReference, CommitUploadDecision,
    PrepareUpload, PrepareUploadDecision, RepositoryId, UploadLease, UploadLeaseId,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, QueryResult, Statement, TransactionTrait,
};

mod retention;
#[cfg(test)]
mod tests;
mod types;

use retention::{active_repository_bytes, expire_repository_references, make_repository_room};
pub use types::{
    CacheCommitResult, CacheObjectRecord, CachePrepareResult, CacheRestoreKind, CacheRestoreRecord,
    CacheUploadRecord, CacheUploadState, PendingCacheDeletion,
};

#[allow(clippy::too_many_arguments)]
impl CacheStore {
    pub async fn restore(
        &self,
        repository_id: &str,
        identity_digest: &str,
        compatibility_group_digest: &str,
        now_unix: u64,
    ) -> Result<Option<CacheRestoreRecord>, PostgresError> {
        let now = to_i64(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        lock_repository(&tx, repository_id).await?;
        let row = tx
            .query_one(statement(
                "SELECT o.repository_id, o.checksum_sha256, o.storage_backend, o.object_key,
                        o.size_bytes, o.created_at_unix,
                        r.last_accessed_at_unix AS reference_accessed_at_unix,
                        r.expires_at_unix AS reference_expires_at_unix,
                        r.identity_digest AS reference_identity_digest,
                        r.compatibility_group_digest
                 FROM scope_cache_references r
                 JOIN scope_cache_objects o USING (repository_id, checksum_sha256)
                 WHERE r.repository_id = $1
                   AND r.compatibility_group_digest = $3
                   AND r.expires_at_unix > $4
                 ORDER BY (r.identity_digest = $2) DESC, r.created_at_unix DESC,
                          r.identity_digest
                 LIMIT 1
                 FOR UPDATE OF r, o",
                vec![
                    repository_id.into(),
                    identity_digest.into(),
                    compatibility_group_digest.into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let Some(row) = row else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let object = decode_object(&row)?;
        let restored_identity: String = row
            .try_get("", "reference_identity_digest")
            .map_err(PostgresError::internal)?;
        let current = ReferenceRow {
            checksum_sha256: object.checksum_sha256.clone(),
            compatibility_group_digest: row
                .try_get("", "compatibility_group_digest")
                .map_err(PostgresError::internal)?,
            last_accessed_at_unix: from_i64(
                row.try_get("", "reference_accessed_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
            expires_at_unix: from_i64(
                row.try_get("", "reference_expires_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
        };
        let reference = scope_cache_domain::access_reference(
            CachePolicy,
            &domain_reference(repository_id, &restored_identity, &current)?,
            now_unix,
        )?;
        tx.execute(statement(
            "UPDATE scope_cache_references
             SET last_accessed_at_unix = $3, expires_at_unix = $4
             WHERE repository_id = $1 AND identity_digest = $2",
            vec![
                repository_id.into(),
                restored_identity.clone().into(),
                now.into(),
                to_i64(reference.expires_at_unix())?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(statement(
            "UPDATE scope_cache_objects SET last_accessed_at_unix = $3
             WHERE repository_id = $1 AND checksum_sha256 = $2",
            vec![
                repository_id.into(),
                object.checksum_sha256.clone().into(),
                now.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(CacheRestoreRecord {
            source: if restored_identity == identity_digest {
                CacheRestoreKind::Exact
            } else {
                CacheRestoreKind::Compatible
            },
            object,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_upload(
        &self,
        repository_id: &str,
        identity_digest: &str,
        compatibility_group_digest: &str,
        checksum_sha256: &str,
        size_bytes: u64,
        storage_backend: &str,
        upload_id: &str,
        now_unix: u64,
    ) -> Result<CachePrepareResult, PostgresError> {
        let now = to_i64(now_unix)?;
        let size = to_i64(size_bytes)?;
        let requested_object = CacheObject::new(
            RepositoryId::parse(repository_id.to_string())?,
            CacheDigest::parse(checksum_sha256.to_string())?,
            size_bytes,
            now_unix,
            CachePolicy,
        )?;
        let identity = CacheDigest::parse(identity_digest.to_string())?;
        let lease_id = UploadLeaseId::parse(upload_id.to_string())?;
        let object_key = cache_object_key(repository_id, checksum_sha256);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        lock_repository(&tx, repository_id).await?;
        expire_repository_references(&tx, repository_id, now).await?;

        let current = current_reference(&tx, repository_id, identity_digest).await?;
        if let Some(current) = current {
            if current.compatibility_group_digest != compatibility_group_digest {
                return Err(PostgresError::conflict(
                    "cache exact identity belongs to a different compatibility group",
                ));
            }
            let object = stored_object(&tx, repository_id, &current.checksum_sha256)
                .await?
                .ok_or_else(|| PostgresError::internal_message("cache reference object missing"))?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CachePrepareResult::UseObject {
                object,
                expires_at_unix: current.expires_at_unix,
            });
        }
        if let Some(object) = stored_object(&tx, repository_id, checksum_sha256).await? {
            if deletion_is_claimed(&tx, repository_id, checksum_sha256).await? {
                return Err(PostgresError::conflict(
                    "cache object deletion is already in progress",
                ));
            }
            if object.storage_backend != storage_backend || object.size_bytes != size_bytes {
                return Err(PostgresError::conflict(
                    "cache object digest is already committed with different metadata",
                ));
            }
            let PrepareUploadDecision::UseObject { reference } =
                scope_cache_domain::prepare_upload(
                    CachePolicy,
                    PrepareUpload {
                        identity_digest: identity,
                        compatibility_group_digest: CacheDigest::parse(
                            compatibility_group_digest.to_string(),
                        )?,
                        object: &domain_object(&object)?,
                        object_already_stored: true,
                        current_reference: None,
                        repository_storage_bytes: 0,
                        lease_id,
                        now_unix,
                    },
                )?
            else {
                return Err(PostgresError::internal_message(
                    "stored cache object unexpectedly required an upload",
                ));
            };
            insert_reference(
                &tx,
                repository_id,
                identity_digest,
                compatibility_group_digest,
                checksum_sha256,
                now,
                to_i64(reference.expires_at_unix())?,
            )
            .await?;
            tx.execute(statement(
                "DELETE FROM scope_cache_deletion_queue
                 WHERE repository_id = $1 AND checksum_sha256 = $2",
                vec![repository_id.into(), checksum_sha256.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CachePrepareResult::UseObject {
                object,
                expires_at_unix: reference.expires_at_unix(),
            });
        }

        make_repository_room(
            &tx,
            repository_id,
            identity_digest,
            size,
            to_i64(CachePolicy.max_repository_bytes())?,
            now_unix,
        )
        .await?;
        let repository_storage_bytes =
            from_i64(active_repository_bytes(&tx, repository_id).await?)?;
        let PrepareUploadDecision::Upload { lease } = scope_cache_domain::prepare_upload(
            CachePolicy,
            PrepareUpload {
                identity_digest: identity,
                compatibility_group_digest: CacheDigest::parse(
                    compatibility_group_digest.to_string(),
                )?,
                object: &requested_object,
                object_already_stored: false,
                current_reference: None,
                repository_storage_bytes,
                lease_id,
                now_unix,
            },
        )?
        else {
            return Err(PostgresError::internal_message(
                "missing cache object unexpectedly bypassed upload",
            ));
        };
        let inserted = tx
            .execute(statement(
                "INSERT INTO scope_cache_uploads (
                    upload_id, repository_id, identity_digest, compatibility_group_digest,
                    checksum_sha256,
                    storage_backend, object_key, size_bytes,
                    state, created_at_unix, expires_at_unix
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10)
                 ON CONFLICT DO NOTHING",
                vec![
                    upload_id.into(),
                    repository_id.into(),
                    identity_digest.into(),
                    compatibility_group_digest.into(),
                    checksum_sha256.into(),
                    storage_backend.into(),
                    object_key.clone().into(),
                    size.into(),
                    now.into(),
                    to_i64(lease.expires_at_unix())?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        if inserted.rows_affected() != 1 {
            return Err(PostgresError::conflict(
                "an active cache upload already exists for this logical identity",
            ));
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(CachePrepareResult::Upload(CacheUploadRecord {
            upload_id: upload_id.to_string(),
            repository_id: repository_id.to_string(),
            identity_digest: identity_digest.to_string(),
            compatibility_group_digest: compatibility_group_digest.to_string(),
            checksum_sha256: checksum_sha256.to_string(),
            storage_backend: storage_backend.to_string(),
            object_key,
            size_bytes,
            state: CacheUploadState::Active,
            created_at_unix: now_unix,
            expires_at_unix: lease.expires_at_unix(),
        }))
    }

    pub async fn upload(&self, upload_id: &str) -> Result<CacheUploadRecord, PostgresError> {
        let row = self
            .db
            .query_one(statement(
                "SELECT upload_id, repository_id, identity_digest, compatibility_group_digest,
                        checksum_sha256,
                        storage_backend, object_key, size_bytes,
                        state, created_at_unix, expires_at_unix
                 FROM scope_cache_uploads
                 WHERE upload_id = $1 AND state IN ('active', 'committed')",
                vec![upload_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("cache upload lease not found"))?;
        decode_upload(&row)
    }

    pub async fn commit_upload(
        &self,
        upload_id: &str,
        now_unix: u64,
    ) -> Result<CacheCommitResult, PostgresError> {
        let now = to_i64(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let row = tx
            .query_one(statement(
                "SELECT upload_id, repository_id, identity_digest, compatibility_group_digest,
                        checksum_sha256,
                        storage_backend, object_key, size_bytes,
                        state, created_at_unix, expires_at_unix
                 FROM scope_cache_uploads
                 WHERE upload_id = $1 AND state IN ('active', 'committed') FOR UPDATE",
                vec![upload_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("cache upload lease not found"))?;
        let upload = decode_upload(&row)?;
        lock_repository(&tx, &upload.repository_id).await?;
        let current =
            current_reference(&tx, &upload.repository_id, &upload.identity_digest).await?;
        let current_domain = current
            .as_ref()
            .map(|reference| {
                domain_reference(&upload.repository_id, &upload.identity_digest, reference)
            })
            .transpose()?;
        if upload.state == CacheUploadState::Committed {
            let Some(current) = current.as_ref() else {
                return Err(PostgresError::conflict(
                    "committed cache upload is no longer the current reference",
                ));
            };
            if current.checksum_sha256 != upload.checksum_sha256 {
                return Err(PostgresError::conflict(
                    "committed cache upload is no longer the current reference",
                ));
            }
            let object = stored_object(&tx, &upload.repository_id, &upload.checksum_sha256)
                .await?
                .ok_or_else(|| PostgresError::internal_message("cache reference object missing"))?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CacheCommitResult::AlreadyCommitted {
                object,
                expires_at_unix: current.expires_at_unix,
            });
        }
        let uploaded_object = CacheObject::new(
            RepositoryId::parse(upload.repository_id.clone())?,
            CacheDigest::parse(upload.checksum_sha256.clone())?,
            upload.size_bytes,
            upload.created_at_unix,
            CachePolicy,
        )?;
        let decision = match scope_cache_domain::commit_upload(
            CachePolicy,
            &domain_upload(&upload)?,
            &uploaded_object,
            current_domain.as_ref(),
            now_unix,
        ) {
            Ok(decision) => decision,
            Err(CacheDomainError::StaleUploadLease) => {
                tx.execute(statement(
                    "UPDATE scope_cache_uploads SET state = 'deleting' WHERE upload_id = $1",
                    vec![upload_id.into()],
                ))
                .await
                .map_err(PostgresError::internal)?;
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(CacheCommitResult::Stale {
                    orphaned_object_key: upload.object_key,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let reference = match decision {
            CommitUploadDecision::AlreadyCommitted { reference } => {
                let object = stored_object(&tx, &upload.repository_id, &upload.checksum_sha256)
                    .await?
                    .ok_or_else(|| {
                        PostgresError::internal_message("cache reference object missing")
                    })?;
                tx.execute(statement(
                    "UPDATE scope_cache_uploads SET state = 'committed' WHERE upload_id = $1",
                    vec![upload_id.into()],
                ))
                .await
                .map_err(PostgresError::internal)?;
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(CacheCommitResult::AlreadyCommitted {
                    object,
                    expires_at_unix: reference.expires_at_unix(),
                });
            }
            CommitUploadDecision::Committed { reference } => reference,
        };

        tx.execute(statement(
            "INSERT INTO scope_cache_objects (
                repository_id, checksum_sha256, storage_backend, object_key,
                size_bytes, created_at_unix, last_accessed_at_unix
             ) VALUES ($1,$2,$3,$4,$5,$6,$6)
             ON CONFLICT (repository_id, checksum_sha256) DO NOTHING",
            vec![
                upload.repository_id.clone().into(),
                upload.checksum_sha256.clone().into(),
                upload.storage_backend.clone().into(),
                upload.object_key.clone().into(),
                to_i64(upload.size_bytes)?.into(),
                to_i64(uploaded_object.created_at_unix())?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        let object = stored_object(&tx, &upload.repository_id, &upload.checksum_sha256)
            .await?
            .ok_or_else(|| PostgresError::internal_message("committed cache object missing"))?;
        if object.object_key != upload.object_key
            || object.storage_backend != upload.storage_backend
            || object.size_bytes != upload.size_bytes
        {
            return Err(PostgresError::conflict(
                "cache object digest is already committed with different metadata",
            ));
        }
        insert_reference(
            &tx,
            &upload.repository_id,
            &upload.identity_digest,
            &upload.compatibility_group_digest,
            &upload.checksum_sha256,
            now,
            to_i64(reference.expires_at_unix())?,
        )
        .await?;
        tx.execute(statement(
            "UPDATE scope_cache_uploads SET state = 'committed' WHERE upload_id = $1",
            vec![upload_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(statement(
            "DELETE FROM scope_cache_deletion_queue
             WHERE repository_id = $1 AND checksum_sha256 = $2",
            vec![
                upload.repository_id.clone().into(),
                upload.checksum_sha256.clone().into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(CacheCommitResult::Committed {
            object,
            expires_at_unix: reference.expires_at_unix(),
        })
    }
}

#[derive(Clone, Debug)]
struct ReferenceRow {
    checksum_sha256: String,
    compatibility_group_digest: String,
    last_accessed_at_unix: u64,
    expires_at_unix: u64,
}

async fn lock_repository(
    tx: &DatabaseTransaction,
    repository_id: &str,
) -> Result<(), PostgresError> {
    tx.execute(statement(
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:cache:' || $1, 0))",
        vec![repository_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn current_reference(
    tx: &DatabaseTransaction,
    repository_id: &str,
    identity_digest: &str,
) -> Result<Option<ReferenceRow>, PostgresError> {
    tx.query_one(statement(
        "SELECT checksum_sha256, compatibility_group_digest,
                last_accessed_at_unix, expires_at_unix
         FROM scope_cache_references
         WHERE repository_id = $1 AND identity_digest = $2 FOR UPDATE",
        vec![repository_id.into(), identity_digest.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .map(|row| {
        Ok(ReferenceRow {
            checksum_sha256: row
                .try_get("", "checksum_sha256")
                .map_err(PostgresError::internal)?,
            compatibility_group_digest: row
                .try_get("", "compatibility_group_digest")
                .map_err(PostgresError::internal)?,
            last_accessed_at_unix: from_i64(
                row.try_get("", "last_accessed_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
            expires_at_unix: from_i64(
                row.try_get("", "expires_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
        })
    })
    .transpose()
}

async fn stored_object<C: ConnectionTrait>(
    db: &C,
    repository_id: &str,
    checksum_sha256: &str,
) -> Result<Option<CacheObjectRecord>, PostgresError> {
    db.query_one(statement(
        "SELECT repository_id, checksum_sha256, storage_backend, object_key, size_bytes,
                created_at_unix
         FROM scope_cache_objects
         WHERE repository_id = $1 AND checksum_sha256 = $2",
        vec![repository_id.into(), checksum_sha256.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .map(|row| decode_object(&row))
    .transpose()
}

async fn deletion_is_claimed(
    tx: &DatabaseTransaction,
    repository_id: &str,
    checksum_sha256: &str,
) -> Result<bool, PostgresError> {
    let row = tx
        .query_one(statement(
            "SELECT attempts FROM scope_cache_deletion_queue
             WHERE repository_id = $1 AND checksum_sha256 = $2 FOR UPDATE",
            vec![repository_id.into(), checksum_sha256.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    row.map(|row| {
        row.try_get::<i32>("", "attempts")
            .map(|attempts| attempts > 0)
            .map_err(PostgresError::internal)
    })
    .transpose()
    .map(|claimed| claimed.unwrap_or(false))
}

#[allow(clippy::too_many_arguments)]
async fn insert_reference(
    tx: &DatabaseTransaction,
    repository_id: &str,
    identity_digest: &str,
    compatibility_group_digest: &str,
    checksum_sha256: &str,
    now: i64,
    expires: i64,
) -> Result<(), PostgresError> {
    tx.execute(statement(
        "INSERT INTO scope_cache_references (
            repository_id, identity_digest, compatibility_group_digest, checksum_sha256,
            created_at_unix, expires_at_unix, last_accessed_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6,$5)
         ON CONFLICT (repository_id, identity_digest) DO NOTHING",
        vec![
            repository_id.into(),
            identity_digest.into(),
            compatibility_group_digest.into(),
            checksum_sha256.into(),
            now.into(),
            expires.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn cache_object_key(repository_id: &str, checksum_sha256: &str) -> String {
    format!("repos/{repository_id}/objects/sha256/{checksum_sha256}")
}

fn statement(sql: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}

fn decode_object(row: &QueryResult) -> Result<CacheObjectRecord, PostgresError> {
    Ok(CacheObjectRecord {
        repository_id: row
            .try_get("", "repository_id")
            .map_err(PostgresError::internal)?,
        checksum_sha256: row
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?,
        storage_backend: row
            .try_get("", "storage_backend")
            .map_err(PostgresError::internal)?,
        object_key: row
            .try_get("", "object_key")
            .map_err(PostgresError::internal)?,
        size_bytes: from_i64(
            row.try_get("", "size_bytes")
                .map_err(PostgresError::internal)?,
        )?,
        created_at_unix: from_i64(
            row.try_get("", "created_at_unix")
                .map_err(PostgresError::internal)?,
        )?,
    })
}

fn decode_upload(row: &QueryResult) -> Result<CacheUploadRecord, PostgresError> {
    Ok(CacheUploadRecord {
        upload_id: row
            .try_get("", "upload_id")
            .map_err(PostgresError::internal)?,
        repository_id: row
            .try_get("", "repository_id")
            .map_err(PostgresError::internal)?,
        identity_digest: row
            .try_get("", "identity_digest")
            .map_err(PostgresError::internal)?,
        compatibility_group_digest: row
            .try_get("", "compatibility_group_digest")
            .map_err(PostgresError::internal)?,
        checksum_sha256: row
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?,
        storage_backend: row
            .try_get("", "storage_backend")
            .map_err(PostgresError::internal)?,
        object_key: row
            .try_get("", "object_key")
            .map_err(PostgresError::internal)?,
        size_bytes: from_i64(
            row.try_get("", "size_bytes")
                .map_err(PostgresError::internal)?,
        )?,
        state: match row
            .try_get::<String>("", "state")
            .map_err(PostgresError::internal)?
            .as_str()
        {
            "active" => CacheUploadState::Active,
            "deleting" => CacheUploadState::Deleting,
            "committed" => CacheUploadState::Committed,
            value => {
                return Err(PostgresError::internal_message(format!(
                    "unknown cache upload state {value}"
                )));
            }
        },
        created_at_unix: from_i64(
            row.try_get("", "created_at_unix")
                .map_err(PostgresError::internal)?,
        )?,
        expires_at_unix: from_i64(
            row.try_get("", "expires_at_unix")
                .map_err(PostgresError::internal)?,
        )?,
    })
}

fn to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(PostgresError::internal)
}

fn from_i64(value: i64) -> Result<u64, PostgresError> {
    u64::try_from(value).map_err(PostgresError::internal)
}

fn domain_reference_from_query(
    repository_id: &str,
    identity_digest: &str,
    row: &QueryResult,
) -> Result<CacheReference, PostgresError> {
    domain_reference(
        repository_id,
        identity_digest,
        &ReferenceRow {
            checksum_sha256: row
                .try_get("", "checksum_sha256")
                .map_err(PostgresError::internal)?,
            compatibility_group_digest: row
                .try_get("", "compatibility_group_digest")
                .map_err(PostgresError::internal)?,
            last_accessed_at_unix: from_i64(
                row.try_get("", "last_accessed_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
            expires_at_unix: from_i64(
                row.try_get("", "expires_at_unix")
                    .map_err(PostgresError::internal)?,
            )?,
        },
    )
}

fn domain_object(record: &CacheObjectRecord) -> Result<CacheObject, PostgresError> {
    CacheObject::new(
        RepositoryId::parse(record.repository_id.clone())?,
        CacheDigest::parse(record.checksum_sha256.clone())?,
        record.size_bytes,
        record.created_at_unix,
        CachePolicy,
    )
    .map_err(PostgresError::from)
}

fn domain_reference(
    repository_id: &str,
    identity_digest: &str,
    row: &ReferenceRow,
) -> Result<CacheReference, PostgresError> {
    CacheReference::restore(
        RepositoryId::parse(repository_id.to_string())?,
        CacheDigest::parse(identity_digest.to_string())?,
        CacheDigest::parse(row.compatibility_group_digest.clone())?,
        CacheDigest::parse(row.checksum_sha256.clone())?,
        row.last_accessed_at_unix,
        row.expires_at_unix,
        CachePolicy,
    )
    .map_err(PostgresError::from)
}

fn domain_upload(record: &CacheUploadRecord) -> Result<UploadLease, PostgresError> {
    UploadLease::restore(
        UploadLeaseId::parse(record.upload_id.clone())?,
        RepositoryId::parse(record.repository_id.clone())?,
        CacheDigest::parse(record.identity_digest.clone())?,
        CacheDigest::parse(record.compatibility_group_digest.clone())?,
        CacheDigest::parse(record.checksum_sha256.clone())?,
        record.size_bytes,
        record.created_at_unix,
        record.expires_at_unix,
        CachePolicy,
    )
    .map_err(PostgresError::from)
}
