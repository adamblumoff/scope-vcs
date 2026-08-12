use super::RunStore;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[derive(Clone, Debug)]
pub struct RunCacheObject {
    pub object_key: String,
    pub checksum_sha256: String,
    pub size_bytes: u64,
    pub generation: u64,
}

impl RunStore {
    pub async fn ready_cache_object(
        &self,
        digest: &str,
    ) -> Result<Option<RunCacheObject>, PostgresError> {
        validate_digest(digest)?;
        let row = self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "SELECT object_key, checksum_sha256, size_bytes, generation FROM scope_run_cache_objects WHERE identity_digest = $1 AND ready = TRUE",
            [digest.into()])).await.map_err(PostgresError::internal)?;
        row.map(decode).transpose()
    }

    pub async fn begin_cache_upload(
        &self,
        digest: &str,
        now_unix: u64,
    ) -> Result<RunCacheObject, PostgresError> {
        validate_digest(digest)?;
        let now = i64::try_from(now_unix).map_err(PostgresError::internal)?;
        let key_prefix = format!("run-caches/v1/{digest}");
        let row = self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_run_cache_objects (identity_digest, object_key, checksum_sha256, size_bytes, generation, ready, updated_at_unix)
             VALUES ($1, $2 || '/1.tar.zst', repeat('0', 64), 0, 1, FALSE, $3)
             ON CONFLICT (identity_digest) DO UPDATE SET
               generation = scope_run_cache_objects.generation + 1,
               object_key = $2 || '/' || (scope_run_cache_objects.generation + 1)::text || '.tar.zst',
               checksum_sha256 = repeat('0', 64), size_bytes = 0, ready = FALSE, updated_at_unix = $3
             RETURNING object_key, checksum_sha256, size_bytes, generation",
            [digest.into(), key_prefix.into(), now.into()])).await.map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("cache upload session is missing"))?;
        decode(row)
    }

    pub async fn commit_cache_upload(
        &self,
        digest: &str,
        generation: u64,
        checksum: &str,
        size_bytes: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        validate_digest(digest)?;
        validate_digest(checksum)?;
        if size_bytes > 10 * 1024 * 1024 * 1024 {
            return Err(PostgresError::invalid_input("cache object exceeds 10 GiB"));
        }
        let values = [
            digest.into(),
            i64::try_from(generation)
                .map_err(PostgresError::internal)?
                .into(),
            checksum.into(),
            i64::try_from(size_bytes)
                .map_err(PostgresError::internal)?
                .into(),
            i64::try_from(now_unix)
                .map_err(PostgresError::internal)?
                .into(),
        ];
        let result = self.db.execute(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "UPDATE scope_run_cache_objects SET checksum_sha256 = $3, size_bytes = $4, ready = TRUE, updated_at_unix = $5 WHERE identity_digest = $1 AND generation = $2 AND ready = FALSE", values))
            .await.map_err(PostgresError::internal)?;
        if result.rows_affected() != 1 {
            return Err(PostgresError::conflict("cache upload session is stale"));
        }
        Ok(())
    }
}

fn validate_digest(value: &str) -> Result<(), PostgresError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PostgresError::invalid_input(
            "cache digest must be 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn decode(row: sea_orm::QueryResult) -> Result<RunCacheObject, PostgresError> {
    Ok(RunCacheObject {
        object_key: row
            .try_get("", "object_key")
            .map_err(PostgresError::internal)?,
        checksum_sha256: row
            .try_get("", "checksum_sha256")
            .map_err(PostgresError::internal)?,
        size_bytes: u64::try_from(
            row.try_get::<i64>("", "size_bytes")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
        generation: u64::try_from(
            row.try_get::<i64>("", "generation")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
    })
}
