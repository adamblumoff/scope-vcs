use api::domain::{
    policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, SourceGraph, project_graph,
    },
};
use api::object_store::{MemoryObjectStore, put_source_blob};

fn blob(content: &str) -> api::domain::store::SourceBlob {
    put_source_blob(&MemoryObjectStore::new(), "scope", content.as_bytes()).unwrap()
}

fn fixture_policy() -> Policy {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
            ["owner".to_string(), "user_collaborator".to_string()],
        ))
        .unwrap();
    policy
}

#[test]
fn synthetic_commit_contains_only_visible_paths() {
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
                    new_content: Some(blob("hello")),
                },
                FileChange {
                    path: ScopePath::parse("/internal/model.rs").unwrap(),
                    old_content: None,
                    new_content: Some(blob("secret")),
                },
            ],
        }],
    };

    let projection = project_graph(&fixture_policy(), &graph, &Principal::public());

    assert_eq!(projection.commits.len(), 1);
    assert!(projection.commits[0].synthetic);
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
    assert!(projection.commits[0].author.is_none());
}

#[test]
fn omitted_mixed_commit_hides_public_changes_too() {
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "mixed".to_string(),
            mixed_policy: MixedCommitPolicy::OmitFromPublic,
            changes: vec![
                FileChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("hello")),
                },
                FileChange {
                    path: ScopePath::parse("/internal/model.rs").unwrap(),
                    old_content: None,
                    new_content: Some(blob("secret")),
                },
            ],
        }],
    };

    let projection = project_graph(&fixture_policy(), &graph, &Principal::public());

    assert!(projection.commits.is_empty());
}

#[test]
fn authorized_collaborator_sees_private_paths() {
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "private".to_string(),
            mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
            changes: vec![FileChange {
                path: ScopePath::parse("/internal/model.rs").unwrap(),
                old_content: None,
                new_content: Some(blob("secret")),
            }],
        }],
    };

    let collaborator = Principal {
        id: "user_collaborator".to_string(),
        kind: PrincipalKind::User,
    };
    let projection = project_graph(&fixture_policy(), &graph, &collaborator);

    assert_eq!(projection.visible_paths(), vec!["/internal/model.rs"]);
}
