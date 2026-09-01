use crate::{config::data_dir, object_store_config};
use scope_git_storage::{
    ENCODING_VERSION, GitSegmentReservation, GitSegmentStore, StagedGitSegment,
};
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::{
    GitSegmentV1Cleanup, GitSegmentV2Backfill, GitSegmentV2BackfillRecord, LegacyGitSegment,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc, time::SystemTime};

pub async fn backfill_git_segments_v2_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let Some(backfill) = GitSegmentV2Backfill::begin(database_url).await? else {
        return Ok(0);
    };
    let encryption_key = object_store_config::encryption_key_from_env()?;
    let legacy_s3 = tokio::task::spawn_blocking(object_store_config::s3_from_env).await??;
    let legacy_store: Arc<dyn ObjectStore> = Arc::new(EncryptedObjectStore::new(
        Arc::new(legacy_s3),
        encryption_key,
    ));
    let local_root = maintenance_git_segment_root();
    let segment_store =
        object_store_config::git_segment_store_from_env(local_root.clone(), encryption_key)?;
    segment_store.cleanup_all_local().await?;
    let result = backfill_segments(&backfill, legacy_store, &segment_store).await;
    let cleanup = segment_store.cleanup_all_local().await;
    if let Err(error) = result {
        cleanup?;
        return Err(error);
    }
    cleanup?;
    result
}

pub async fn cleanup_git_segments_v1_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let cleanup = GitSegmentV1Cleanup::begin(database_url).await?;
    let encryption_key = object_store_config::encryption_key_from_env()?;
    let legacy_s3 = tokio::task::spawn_blocking(object_store_config::s3_from_env).await??;
    let legacy_store = EncryptedObjectStore::new(Arc::new(legacy_s3), encryption_key);
    let objects = cleanup.legacy_objects().await?;
    for object in &objects {
        legacy_store.delete(&format!("objects/git-segments/{}", object.sha256))?;
        cleanup.remove_record(object).await?;
    }
    Ok(objects.len())
}

async fn backfill_segments(
    backfill: &GitSegmentV2Backfill,
    legacy_store: Arc<dyn ObjectStore>,
    segment_store: &GitSegmentStore,
) -> anyhow::Result<usize> {
    let segments = backfill.legacy_segments().await?;
    for legacy in &segments {
        let expected = expected_record(legacy);
        if let Some(prepared) = &legacy.prepared {
            require_expected_record(legacy, prepared, &expected)?;
            segment_store
                .restore_to(
                    &legacy.repository_id,
                    &prepared_segment_ref(prepared),
                    tokio::io::sink(),
                )
                .await?;
            continue;
        }

        let staged =
            rewrite_legacy_segment(legacy, Arc::clone(&legacy_store), segment_store, &expected)
                .await?;
        let prepared = GitSegmentV2BackfillRecord {
            segment_id: staged.segment.segment_id.clone(),
            object_key: staged.object_key.clone(),
            sha256: staged.segment.sha256.clone(),
            plaintext_bytes: staged.segment.plaintext_bytes,
            encrypted_bytes: staged.encrypted_bytes,
            encoding_version: staged.segment.encoding_version,
            completed_at_unix: unix_now()?,
        };
        if let Err(error) = require_expected_record(legacy, &prepared, &expected) {
            let _ = segment_store
                .cleanup_remote_bounded(&staged.object_key)
                .await;
            let _ = segment_store.delete_local(&staged).await;
            return Err(error);
        }
        backfill.record(legacy, &prepared).await?;
        segment_store.delete_local(&staged).await?;
    }
    Ok(segments.len())
}

async fn rewrite_legacy_segment(
    legacy: &LegacyGitSegment,
    legacy_store: Arc<dyn ObjectStore>,
    segment_store: &GitSegmentStore,
    expected: &GitSegmentV2BackfillRecord,
) -> anyhow::Result<StagedGitSegment> {
    let legacy_key = format!("objects/git-segments/{}", legacy.sha256);
    let max_bytes = usize::try_from(legacy.size_bytes)?;
    let bytes =
        tokio::task::spawn_blocking(move || legacy_store.get_bounded(&legacy_key, max_bytes))
            .await??;
    verify_legacy_bytes(legacy, &bytes)?;
    segment_store
        .ingest_reserved_blocking_reader(
            &legacy.repository_id,
            GitSegmentReservation {
                segment_id: expected.segment_id.clone(),
                object_key: expected.object_key.clone(),
            },
            Cursor::new(bytes),
            legacy.size_bytes,
        )
        .await
        .map_err(anyhow::Error::from)
}

fn expected_record(legacy: &LegacyGitSegment) -> GitSegmentV2BackfillRecord {
    let segment_id = deterministic_segment_id(legacy);
    GitSegmentV2BackfillRecord {
        object_key: scope_git_storage::object_key(&legacy.repository_id, &segment_id),
        segment_id,
        sha256: legacy.sha256.clone(),
        plaintext_bytes: legacy.size_bytes,
        encrypted_bytes: 0,
        encoding_version: ENCODING_VERSION,
        completed_at_unix: 0,
    }
}

fn require_expected_record(
    legacy: &LegacyGitSegment,
    prepared: &GitSegmentV2BackfillRecord,
    expected: &GitSegmentV2BackfillRecord,
) -> anyhow::Result<()> {
    if prepared.segment_id != expected.segment_id
        || prepared.object_key != expected.object_key
        || prepared.sha256 != expected.sha256
        || prepared.plaintext_bytes != expected.plaintext_bytes
        || prepared.encoding_version != expected.encoding_version
        || prepared.encrypted_bytes == 0
    {
        anyhow::bail!(
            "Git segment backfill for {} sequence {} does not match its legacy source",
            legacy.repository_id,
            legacy.first_sequence
        );
    }
    Ok(())
}

fn prepared_segment_ref(
    prepared: &GitSegmentV2BackfillRecord,
) -> scope_domain::repository::git::GitSegmentRef {
    scope_domain::repository::git::GitSegmentRef {
        segment_id: prepared.segment_id.clone(),
        sha256: prepared.sha256.clone(),
        plaintext_bytes: prepared.plaintext_bytes,
        encoding_version: prepared.encoding_version,
    }
}

fn verify_legacy_bytes(legacy: &LegacyGitSegment, bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() as u64 != legacy.size_bytes {
        anyhow::bail!(
            "legacy Git segment for {} sequence {} has size {}, expected {}",
            legacy.repository_id,
            legacy.first_sequence,
            bytes.len(),
            legacy.size_bytes
        );
    }
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != legacy.sha256 {
        anyhow::bail!(
            "legacy Git segment for {} sequence {} failed sha256 verification",
            legacy.repository_id,
            legacy.first_sequence
        );
    }
    Ok(())
}

fn deterministic_segment_id(legacy: &LegacyGitSegment) -> String {
    let mut digest = Sha256::new();
    for bytes in [
        b"scope-git-segment-v2".as_slice(),
        legacy.repository_id.as_bytes(),
        &legacy.first_sequence.to_be_bytes(),
        &legacy.last_sequence.to_be_bytes(),
        legacy.sha256.as_bytes(),
    ] {
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    hex::encode(&digest.finalize()[..16])
}

fn unix_now() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs())
}

fn maintenance_git_segment_root() -> PathBuf {
    data_dir(&PathBuf::from("/tmp/scope-maintenance")).join("git-segment-v2-backfill")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_git_storage::{GitSegmentStoreConfig, MemoryMultipartStore, SegmentEncryptionKey};
    use scope_object_store::MemoryObjectStore;
    use tempfile::tempdir;

    #[tokio::test]
    async fn rewrites_legacy_encrypted_object_into_v2_segment_envelope() {
        let bytes = b"PACK one-time migration".to_vec();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let legacy = LegacyGitSegment {
            repository_id: "owner/repo".to_string(),
            first_sequence: 1,
            last_sequence: 1,
            legacy_object_key: format!(r#"{{"GitSegmentSha256":"{sha256}"}}"#),
            sha256: sha256.clone(),
            size_bytes: bytes.len() as u64,
            prepared: None,
        };
        let raw_legacy = Arc::new(MemoryObjectStore::new());
        let legacy_store: Arc<dyn ObjectStore> =
            Arc::new(EncryptedObjectStore::new(raw_legacy.clone(), [7_u8; 32]));
        legacy_store
            .put(&format!("objects/git-segments/{sha256}"), &bytes)
            .unwrap();

        let backend = Arc::new(MemoryMultipartStore::default());
        let directory = tempdir().unwrap();
        let segment_store = GitSegmentStore::new(
            backend.clone(),
            SegmentEncryptionKey::new("test", [7_u8; 32]).unwrap(),
            GitSegmentStoreConfig::new(directory.path()),
        )
        .unwrap();
        let expected = expected_record(&legacy);
        let staged = rewrite_legacy_segment(
            &legacy,
            Arc::clone(&legacy_store),
            &segment_store,
            &expected,
        )
        .await
        .unwrap();
        assert_eq!(staged.segment.sha256, sha256);
        assert_eq!(staged.segment.plaintext_bytes, bytes.len() as u64);
        assert_ne!(backend.object(&staged.object_key).unwrap(), bytes);

        let mut restored = Vec::new();
        segment_store
            .restore_to(&legacy.repository_id, &staged.segment, &mut restored)
            .await
            .unwrap();
        assert_eq!(restored, bytes);
    }

    #[test]
    fn deterministic_ids_distinguish_spans_with_the_same_content() {
        let base = LegacyGitSegment {
            repository_id: "owner/repo".to_string(),
            first_sequence: 1,
            last_sequence: 1,
            legacy_object_key: "legacy".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1,
            prepared: None,
        };
        let mut later = base.clone();
        later.first_sequence = 2;
        later.last_sequence = 2;
        assert_ne!(
            deterministic_segment_id(&base),
            deterministic_segment_id(&later)
        );
    }
}
