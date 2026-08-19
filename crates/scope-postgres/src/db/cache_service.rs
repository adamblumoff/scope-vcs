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

use retention::{
    active_repository_bytes, expire_repository_references, make_repository_room,
    queue_if_unreferenced,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheObjectRecord {
    pub repository_id: String,
    pub checksum_sha256: String,
    pub storage_backend: String,
    pub object_key: String,
    pub size_bytes: u64,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheUploadRecord {
    pub upload_id: String,
    pub repository_id: String,
    pub identity_digest: String,
    pub checksum_sha256: String,
    pub storage_backend: String,
    pub object_key: String,
    pub size_bytes: u64,
    pub expected_reference_version: Option<u64>,
    pub state: CacheUploadState,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheUploadState {
    Active,
    Deleting,
    Committed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachePrepareResult {
    UseObject {
        object: CacheObjectRecord,
        reference_version: u64,
        expires_at_unix: u64,
    },
    Upload(CacheUploadRecord),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheCommitResult {
    Committed {
        object: CacheObjectRecord,
        reference_version: u64,
        expires_at_unix: u64,
    },
    AlreadyCommitted {
        object: CacheObjectRecord,
        reference_version: u64,
        expires_at_unix: u64,
    },
    Stale {
        orphaned_object_key: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCacheDeletion {
    pub repository_id: String,
    pub checksum_sha256: String,
    pub object_key: String,
    pub attempts: u32,
    pub eligible_after_unix: u64,
}

#[allow(clippy::too_many_arguments)]
impl CacheStore {
    pub async fn restore(
        &self,
        repository_id: &str,
        identity_digest: &str,
        now_unix: u64,
    ) -> Result<Option<CacheObjectRecord>, PostgresError> {
        let now = to_i64(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        lock_repository(&tx, repository_id).await?;
        let row = tx
            .query_one(statement(
                "SELECT o.repository_id, o.checksum_sha256, o.storage_backend, o.object_key,
                        o.size_bytes, o.created_at_unix,
                        r.version AS reference_version,
                        r.last_accessed_at_unix AS reference_accessed_at_unix,
                        r.expires_at_unix AS reference_expires_at_unix
                 FROM scope_cache_references r
                 JOIN scope_cache_objects o USING (repository_id, checksum_sha256)
                 WHERE r.repository_id = $1 AND r.identity_digest = $2
                   AND r.expires_at_unix > $3
                 FOR UPDATE OF r, o",
                vec![repository_id.into(), identity_digest.into(), now.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let Some(row) = row else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let object = decode_object(&row)?;
        let current = ReferenceRow {
            checksum_sha256: object.checksum_sha256.clone(),
            version: from_i64(
                row.try_get("", "reference_version")
                    .map_err(PostgresError::internal)?,
            )?,
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
            &domain_reference(repository_id, identity_digest, &current)?,
            now_unix,
        )?;
        tx.execute(statement(
            "UPDATE scope_cache_references
             SET last_accessed_at_unix = $3, expires_at_unix = $4
             WHERE repository_id = $1 AND identity_digest = $2",
            vec![
                repository_id.into(),
                identity_digest.into(),
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
        Ok(Some(object))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_upload(
        &self,
        repository_id: &str,
        identity_digest: &str,
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
        let current_domain = current
            .as_ref()
            .map(|reference| domain_reference(repository_id, identity_digest, reference))
            .transpose()?;
        if let Some(object) = stored_object(&tx, repository_id, checksum_sha256).await? {
            if object.storage_backend != storage_backend || object.size_bytes != size_bytes {
                return Err(PostgresError::conflict(
                    "cache object digest is already committed with different metadata",
                ));
            }
            let PrepareUploadDecision::UseObject {
                reference,
                superseded,
                ..
            } = scope_cache_domain::prepare_upload(
                CachePolicy,
                PrepareUpload {
                    identity_digest: identity,
                    object: &domain_object(&object)?,
                    object_already_stored: true,
                    current_reference: current_domain.as_ref(),
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
            replace_reference(
                &tx,
                repository_id,
                identity_digest,
                checksum_sha256,
                reference.version(),
                now,
                to_i64(reference.expires_at_unix())?,
            )
            .await?;
            if let Some(candidate) = superseded {
                queue_if_unreferenced(
                    &tx,
                    candidate.repository_id().as_str(),
                    candidate.object_digest().as_str(),
                    to_i64(candidate.eligible_after_unix())?,
                )
                .await?;
            }
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
                reference_version: reference.version(),
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
                object: &requested_object,
                object_already_stored: false,
                current_reference: current_domain.as_ref(),
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
                    upload_id, repository_id, identity_digest, checksum_sha256,
                    storage_backend, object_key, size_bytes,
                    expected_reference_version, state, created_at_unix, expires_at_unix
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10)
                 ON CONFLICT DO NOTHING",
                vec![
                    upload_id.into(),
                    repository_id.into(),
                    identity_digest.into(),
                    checksum_sha256.into(),
                    storage_backend.into(),
                    object_key.clone().into(),
                    size.into(),
                    lease
                        .expected_reference_version()
                        .and_then(|version| i64::try_from(version).ok())
                        .into(),
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
            checksum_sha256: checksum_sha256.to_string(),
            storage_backend: storage_backend.to_string(),
            object_key,
            size_bytes,
            expected_reference_version: lease.expected_reference_version(),
            state: CacheUploadState::Active,
            created_at_unix: now_unix,
            expires_at_unix: lease.expires_at_unix(),
        }))
    }

    pub async fn upload(&self, upload_id: &str) -> Result<CacheUploadRecord, PostgresError> {
        let row = self
            .db
            .query_one(statement(
                "SELECT upload_id, repository_id, identity_digest, checksum_sha256,
                        storage_backend, object_key, size_bytes,
                        expected_reference_version, state, created_at_unix, expires_at_unix
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
                "SELECT upload_id, repository_id, identity_digest, checksum_sha256,
                        storage_backend, object_key, size_bytes,
                        expected_reference_version, state, created_at_unix, expires_at_unix
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
                reference_version: current.version,
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
        let (reference, superseded) = match decision {
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
                    reference_version: reference.version(),
                    expires_at_unix: reference.expires_at_unix(),
                });
            }
            CommitUploadDecision::Committed {
                reference,
                superseded,
            } => (reference, superseded),
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
        replace_reference(
            &tx,
            &upload.repository_id,
            &upload.identity_digest,
            &upload.checksum_sha256,
            reference.version(),
            now,
            to_i64(reference.expires_at_unix())?,
        )
        .await?;
        if let Some(candidate) = superseded {
            queue_if_unreferenced(
                &tx,
                candidate.repository_id().as_str(),
                candidate.object_digest().as_str(),
                to_i64(candidate.eligible_after_unix())?,
            )
            .await?;
        }
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
            reference_version: reference.version(),
            expires_at_unix: reference.expires_at_unix(),
        })
    }
}

#[derive(Clone, Debug)]
struct ReferenceRow {
    checksum_sha256: String,
    version: u64,
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
        "SELECT checksum_sha256, version, last_accessed_at_unix, expires_at_unix
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
            version: from_i64(
                row.try_get("", "version")
                    .map_err(PostgresError::internal)?,
            )?,
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

#[allow(clippy::too_many_arguments)]
async fn replace_reference(
    tx: &DatabaseTransaction,
    repository_id: &str,
    identity_digest: &str,
    checksum_sha256: &str,
    version: u64,
    now: i64,
    expires: i64,
) -> Result<(), PostgresError> {
    tx.execute(statement(
        "INSERT INTO scope_cache_references (
            repository_id, identity_digest, checksum_sha256, version,
            expires_at_unix, last_accessed_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (repository_id, identity_digest) DO UPDATE SET
            checksum_sha256 = EXCLUDED.checksum_sha256,
            version = EXCLUDED.version,
            expires_at_unix = EXCLUDED.expires_at_unix,
            last_accessed_at_unix = EXCLUDED.last_accessed_at_unix",
        vec![
            repository_id.into(),
            identity_digest.into(),
            checksum_sha256.into(),
            to_i64(version)?.into(),
            expires.into(),
            now.into(),
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
        expected_reference_version: row
            .try_get::<Option<i64>>("", "expected_reference_version")
            .map_err(PostgresError::internal)?
            .map(from_i64)
            .transpose()?,
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
            version: from_i64(
                row.try_get("", "version")
                    .map_err(PostgresError::internal)?,
            )?,
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
        CacheDigest::parse(row.checksum_sha256.clone())?,
        row.version,
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
        CacheDigest::parse(record.checksum_sha256.clone())?,
        record.size_bytes,
        record.expected_reference_version,
        record.created_at_unix,
        record.expires_at_unix,
        CachePolicy,
    )
    .map_err(PostgresError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
    use scope_domain::{
        policy::Visibility,
        store::{RepoLifecycleState, StoredRepository, UserAccount},
    };

    #[test]
    fn object_keys_are_repository_scoped_and_content_addressed() {
        assert_eq!(
            cache_object_key("repo-1", &"a".repeat(64)),
            format!("repos/repo-1/objects/sha256/{}", "a".repeat(64))
        );
    }

    #[tokio::test]
    async fn cache_store_deduplicates_swaps_and_collects_objects() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repository_id = seed_repository(&store);
        let caches = store.caches();
        let now = 1_700_000_000_u64;
        let identity = "1".repeat(64);
        let first_digest = "a".repeat(64);
        let second_digest = "b".repeat(64);

        let CachePrepareResult::Upload(first_upload) = caches
            .prepare_upload(
                &repository_id,
                &identity,
                &first_digest,
                100,
                "test-local",
                "upload-1",
                now,
            )
            .await
            .unwrap()
        else {
            panic!("first content must require an upload");
        };
        assert_eq!(first_upload.expected_reference_version, None);
        assert!(matches!(
            caches.commit_upload("upload-1", now + 1).await.unwrap(),
            CacheCommitResult::Committed {
                reference_version: 1,
                ..
            }
        ));
        assert!(matches!(
            caches.commit_upload("upload-1", now + 1).await.unwrap(),
            CacheCommitResult::AlreadyCommitted {
                reference_version: 1,
                ..
            }
        ));
        assert_eq!(
            caches
                .restore(&repository_id, &identity, now + 2)
                .await
                .unwrap()
                .unwrap()
                .checksum_sha256,
            first_digest
        );

        assert!(matches!(
            caches
                .prepare_upload(
                    &repository_id,
                    &identity,
                    &first_digest,
                    100,
                    "test-local",
                    "unused-upload",
                    now + 3,
                )
                .await
                .unwrap(),
            CachePrepareResult::UseObject {
                reference_version: 2,
                ..
            }
        ));

        let CachePrepareResult::Upload(second_upload) = caches
            .prepare_upload(
                &repository_id,
                &identity,
                &second_digest,
                200,
                "test-local",
                "upload-2",
                now + 4,
            )
            .await
            .unwrap()
        else {
            panic!("changed content must require an upload");
        };
        assert_eq!(second_upload.expected_reference_version, Some(2));
        assert!(matches!(
            caches.commit_upload("upload-2", now + 5).await.unwrap(),
            CacheCommitResult::Committed {
                reference_version: 3,
                ..
            }
        ));

        let due = caches
            .claim_deletions(now + 3_605, now + 4_000, 10)
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].checksum_sha256, first_digest);
        assert!(
            caches
                .complete_deletion(&repository_id, &due[0].checksum_sha256)
                .await
                .unwrap()
        );
        assert_eq!(
            caches
                .restore(&repository_id, &identity, now + 6)
                .await
                .unwrap()
                .unwrap()
                .checksum_sha256,
            second_digest
        );

        let expired_identity = "2".repeat(64);
        let expired_digest = "c".repeat(64);
        caches
            .prepare_upload(
                &repository_id,
                &expired_identity,
                &expired_digest,
                300,
                "test-local",
                "expired-upload",
                now + 10,
            )
            .await
            .unwrap();
        let cleanup_now = now + 10 + CachePolicy.upload_lease_seconds();
        let expired = caches.expire_uploads(cleanup_now, 10).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].upload_id, "expired-upload");
        assert!(
            caches
                .expire_uploads(cleanup_now, 10)
                .await
                .unwrap()
                .is_empty()
        );
        caches.retry_upload_cleanup("expired-upload").await.unwrap();
        assert_eq!(
            caches.expire_uploads(cleanup_now, 10).await.unwrap().len(),
            1
        );
        caches
            .complete_upload_cleanup("expired-upload")
            .await
            .unwrap();
        assert!(
            caches
                .expire_uploads(cleanup_now, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    fn seed_repository(store: &MetadataStore) -> String {
        let owner = UserAccount {
            id: "user_cache_owner".to_string(),
            handle: "cache-owner".to_string(),
            email: "cache-owner@example.com".to_string(),
            email_verified: true,
        };
        let mut repository = StoredRepository::new(&owner, "cache-repo", Visibility::Private)
            .expect("test repository is valid");
        repository.record.lifecycle_state = RepoLifecycleState::Ready;
        let repository_id = repository.record.id.clone();
        let mut catalog = CatalogFixture::default();
        catalog.users.insert(owner.id.clone(), owner);
        catalog
            .repositories
            .insert(repository_id.clone(), repository);
        store.admin().seed_catalog_for_tests(catalog).unwrap();
        repository_id
    }
}
