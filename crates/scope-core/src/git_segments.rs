use crate::{domain::store::SourceBlob, error::ApiError};
use serde::{Deserialize, Serialize};

pub const GIT_SEGMENT_MANIFEST_VERSION: u8 = 1;
pub const GIT_BLOB_REFERENCE_PREFIX: &str = "git-blobs/";
pub const GIT_MANIFEST_OBJECT_PREFIX: &str = "objects/git-manifests/";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitSegmentManifest {
    pub version: u8,
    pub head_oid: String,
    pub previous: Option<SourceBlob>,
    pub segment: SourceBlob,
}

impl GitSegmentManifest {
    pub fn new(head_oid: String, previous: Option<SourceBlob>, segment: SourceBlob) -> Self {
        Self {
            version: GIT_SEGMENT_MANIFEST_VERSION,
            head_oid,
            previous,
            segment,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, ApiError> {
        serde_json::to_vec(self).map_err(ApiError::internal)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ApiError> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(ApiError::internal)?;
        if manifest.version != GIT_SEGMENT_MANIFEST_VERSION {
            return Err(ApiError::internal_message(format!(
                "unsupported Git segment manifest version {}",
                manifest.version
            )));
        }
        Ok(manifest)
    }
}

pub fn is_git_segment_manifest(snapshot: &SourceBlob) -> bool {
    snapshot.object_key.starts_with(GIT_MANIFEST_OBJECT_PREFIX)
}

pub fn git_blob_reference(
    manifest: &SourceBlob,
    oid: String,
    mode: String,
    size_bytes: u64,
) -> Result<SourceBlob, ApiError> {
    let manifest_id = manifest
        .object_key
        .strip_prefix(GIT_MANIFEST_OBJECT_PREFIX)
        .ok_or_else(|| ApiError::internal_message("Git blob reference requires a manifest"))?;
    Ok(SourceBlob {
        object_key: format!(
            "{GIT_BLOB_REFERENCE_PREFIX}{manifest_id}/{}",
            manifest.sha256
        ),
        sha256: oid.clone(),
        git_oid: oid,
        git_file_mode: mode,
        size_bytes,
    })
}

pub fn repoint_git_blob_reference(
    content: &mut SourceBlob,
    manifest: &SourceBlob,
) -> Result<bool, ApiError> {
    if !content.object_key.starts_with(GIT_BLOB_REFERENCE_PREFIX) {
        return Ok(false);
    }
    let replacement = git_blob_reference(
        manifest,
        content.git_oid.clone(),
        content.git_file_mode.clone(),
        content.size_bytes,
    )?;
    content.object_key = replacement.object_key;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::store::DEFAULT_GIT_FILE_MODE;

    fn manifest(id: &str, sha256: &str) -> SourceBlob {
        SourceBlob {
            object_key: format!("{GIT_MANIFEST_OBJECT_PREFIX}{id}"),
            sha256: sha256.to_string(),
            git_oid: "head".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 10,
        }
    }

    #[test]
    fn compaction_repoints_git_blob_locator_without_changing_blob_identity() {
        let mut content = git_blob_reference(
            &manifest("old", "old-sha"),
            "blob-oid".to_string(),
            DEFAULT_GIT_FILE_MODE.to_string(),
            42,
        )
        .unwrap();

        assert!(repoint_git_blob_reference(&mut content, &manifest("new", "new-sha")).unwrap());
        assert_eq!(content.object_key, "git-blobs/new/new-sha");
        assert_eq!(content.git_oid, "blob-oid");
        assert_eq!(content.size_bytes, 42);
    }
}
