use api::domain::{
    git_projection::build_virtual_git_projection,
    policy::{Policy, Principal, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, ProjectedChange,
        ProjectedCommit, Projection, SourceGraph, project_graph,
    },
};
use api::object_store::{MemoryObjectStore, put_source_blob};

fn blob(store: &MemoryObjectStore, content: &str) -> api::domain::store::SourceBlob {
    put_source_blob(store, "scope", content.as_bytes()).unwrap()
}

#[test]
fn projected_git_blobs_do_not_include_hidden_content() {
    let store = MemoryObjectStore::new();
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
                    new_content: Some(blob(&store, "public")),
                },
                FileChange {
                    path: ScopePath::parse("/internal/secret.env").unwrap(),
                    old_content: None,
                    new_content: Some(blob(&store, "SCOPE_TOKEN=secret")),
                },
            ],
        }],
    };
    let projection = project_graph(&policy, &graph, &Principal::public());
    let git = build_virtual_git_projection(&store, &projection).unwrap();
    let serialized = serde_json::to_string(&git).unwrap();

    assert!(serialized.contains("/README.md"));
    assert!(!serialized.contains("secret.env"));
    assert!(!serialized.contains("SCOPE_TOKEN"));
}

#[test]
fn projected_git_blobs_are_final_visible_tree() {
    let store = MemoryObjectStore::new();
    let policy = Policy::new(Visibility::Public, "owner");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "initial".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/README.md").unwrap(),
                        old_content: None,
                        new_content: Some(blob(&store, "old")),
                    },
                    FileChange {
                        path: ScopePath::parse("/deleted.txt").unwrap(),
                        old_content: None,
                        new_content: Some(blob(&store, "remove me")),
                    },
                ],
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "update".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/README.md").unwrap(),
                        old_content: Some(blob(&store, "old")),
                        new_content: Some(blob(&store, "new")),
                    },
                    FileChange {
                        path: ScopePath::parse("/deleted.txt").unwrap(),
                        old_content: Some(blob(&store, "remove me")),
                        new_content: None,
                    },
                ],
            },
        ],
    };
    let projection = project_graph(&policy, &graph, &Principal::public());
    let git = build_virtual_git_projection(&store, &projection).unwrap();

    assert_eq!(git.blobs.len(), 1);
    assert_eq!(git.blobs[0].path, "/README.md");
    assert_eq!(git.blobs[0].content, "new");
}

#[test]
fn head_oid_changes_when_tree_content_changes_with_same_blob_count() {
    let store = MemoryObjectStore::new();
    let projection = |content: &str| Projection {
        repo_id: "scope".to_string(),
        principal_id: "public".to_string(),
        commits: vec![ProjectedCommit {
            projected_id: "pv_public_rv1_1".to_string(),
            logical_commit_id: "rv1".to_string(),
            parent_projected_id: None,
            author: Some("owner".to_string()),
            message: "commit".to_string(),
            synthetic: false,
            changes: vec![ProjectedChange {
                path: ScopePath::parse("/README.md").unwrap(),
                new_content: Some(blob(&store, content)),
            }],
        }],
    };

    let left = build_virtual_git_projection(&store, &projection("left")).unwrap();
    let right = build_virtual_git_projection(&store, &projection("right")).unwrap();

    assert_ne!(left.head_oid, right.head_oid);
}
