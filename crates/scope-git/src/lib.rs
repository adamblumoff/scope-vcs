use scope_projection::Projection;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

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
    let mut blobs = projection
        .commits
        .iter()
        .flat_map(|commit| commit.changes.iter())
        .filter_map(|change| {
            change.new_content.as_ref().map(|content| VirtualGitBlob {
                path: change.path.as_str().to_string(),
                oid: git_blob_oid(content),
                content: content.clone(),
            })
        })
        .collect::<Vec<_>>();

    blobs.sort_by(|left, right| left.path.cmp(&right.path));
    blobs.dedup_by(|left, right| left.path == right.path && left.oid == right.oid);

    let head_oid = projection.commits.last().map(|commit| {
        let mut hasher = Sha1::new();
        let payload = format!(
            "projection:{}:{}:{}",
            projection.principal_id,
            commit.projected_id,
            blobs.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use scope_policy::{Policy, Principal, ScopePath, Visibility, VisibilityRule};
    use scope_projection::{
        AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, SourceGraph, project_graph,
    };

    #[test]
    fn projected_git_blobs_do_not_include_hidden_content() {
        let mut policy = Policy::new(Visibility::Public, "owner");
        policy
            .add_rule(VisibilityRule::private(
                ScopePath::parse("/internal").unwrap(),
                ["owner".to_string()],
            ))
            .unwrap();
        let graph = SourceGraph {
            repo_id: "scope".to_string(),
            commits: vec![LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Hidden,
                message: "mixed".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/README.md").unwrap(),
                        old_content: None,
                        new_content: Some("public".to_string()),
                    },
                    FileChange {
                        path: ScopePath::parse("/internal/secret.env").unwrap(),
                        old_content: None,
                        new_content: Some("SCOPE_TOKEN=secret".to_string()),
                    },
                ],
            }],
        };
        let projection = project_graph(&policy, &graph, &Principal::public());
        let git = build_virtual_git_projection(&projection);
        let serialized = serde_json::to_string(&git).unwrap();

        assert!(serialized.contains("/README.md"));
        assert!(!serialized.contains("secret.env"));
        assert!(!serialized.contains("SCOPE_TOKEN"));
    }
}
