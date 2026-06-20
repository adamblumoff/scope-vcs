use super::projection::Projection;
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
    let bytes = content.as_bytes();
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn build_virtual_git_projection(projection: &Projection) -> VirtualGitProjection {
    let mut tree = BTreeMap::new();
    for change in projection
        .commits
        .iter()
        .flat_map(|commit| commit.changes.iter())
    {
        let path = change.path.as_str().to_string();
        match &change.new_content {
            Some(content) => {
                tree.insert(
                    path.clone(),
                    VirtualGitBlob {
                        path,
                        oid: git_blob_oid(content),
                        content: content.clone(),
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

    VirtualGitProjection {
        principal_id: projection.principal_id.clone(),
        blobs,
        head_oid,
    }
}
