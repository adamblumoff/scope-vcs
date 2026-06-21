use super::{projection::Projection, store::SourceBlob};
use crate::{error::ApiError, object_store::ObjectStore};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualGitBlob {
    pub path: String,
    pub oid: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualGitProjection {
    pub principal_id: String,
    pub blobs: Vec<VirtualGitBlob>,
    pub head_oid: Option<String>,
}

pub fn git_blob_oid(content: &str) -> String {
    git_blob_oid_bytes(content.as_bytes())
}

pub fn git_blob_oid_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn build_virtual_git_projection(
    store: &dyn ObjectStore,
    projection: &Projection,
) -> Result<VirtualGitProjection, ApiError> {
    let mut tree = BTreeMap::new();
    for change in projection
        .commits
        .iter()
        .flat_map(|commit| commit.changes.iter())
    {
        let path = change.path.as_str().to_string();
        match &change.new_content {
            Some(blob) => {
                let content = projection_blob_text(store, blob)?;
                tree.insert(
                    path.clone(),
                    VirtualGitBlob {
                        path,
                        oid: blob.git_oid.clone(),
                        content,
                    },
                );
            }
            None => {
                tree.remove(&path);
            }
        }
    }

    let blobs = tree.into_values().collect::<Vec<_>>();

    let head_oid = projection.commits.last().map(|commit| {
        let mut hasher = Sha1::new();
        let tree_payload = blobs
            .iter()
            .map(|blob| format!("100644 blob {}\t{}", blob.oid, blob.path))
            .collect::<Vec<_>>()
            .join("\n");
        let payload = format!(
            "projection:{}\nhead:{}\ntree:\n{}\n",
            projection.principal_id, commit.projected_id, tree_payload
        );
        hasher.update(format!("commit {}\0", payload.len()).as_bytes());
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    });

    Ok(VirtualGitProjection {
        principal_id: projection.principal_id.clone(),
        blobs,
        head_oid,
    })
}

pub(crate) fn projection_blob_text(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
) -> Result<String, ApiError> {
    crate::object_store::source_blob_text(store, blob)
}
