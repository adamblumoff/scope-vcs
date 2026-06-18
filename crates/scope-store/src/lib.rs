use scope_policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule};
use scope_projection::{
    AuthorVisibility, FileChange, LogicalCommit, MixedCommitPolicy, SourceGraph,
};
use serde::{Deserialize, Serialize};

pub const POSTGRES_SCHEMA: &str = include_str!("schema.sql");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DemoRepository {
    pub policy: Policy,
    pub graph: SourceGraph,
}

impl DemoRepository {
    pub fn projection_principal(id: &str) -> Principal {
        match id {
            "public" => Principal::public(),
            "team-core" => Principal {
                id: "team-core".to_string(),
                kind: PrincipalKind::Team,
            },
            "agent-docs" => Principal {
                id: "agent-docs".to_string(),
                kind: PrincipalKind::Agent,
            },
            other => Principal {
                id: other.to_string(),
                kind: PrincipalKind::User,
            },
        }
    }
}

pub fn demo_repository() -> DemoRepository {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
            ["owner".to_string(), "team-core".to_string()],
        ))
        .unwrap();

    let graph = SourceGraph {
        repo_id: "scope-demo".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv_001".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "Initialize public surface".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some("# Scope Demo\n".to_string()),
                }],
            },
            LogicalCommit {
                id: "rv_002".to_string(),
                parent_ids: vec!["rv_001".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Hidden,
                message: "Add model and public API docs".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/docs/api.md").unwrap(),
                        old_content: None,
                        new_content: Some("Public API contract\n".to_string()),
                    },
                    FileChange {
                        path: ScopePath::parse("/internal/model.rs").unwrap(),
                        old_content: None,
                        new_content: Some("fn private_model() {}\n".to_string()),
                    },
                ],
            },
            LogicalCommit {
                id: "rv_003".to_string(),
                parent_ids: vec!["rv_002".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Hidden,
                message: "Tune internal model".to_string(),
                mixed_policy: MixedCommitPolicy::OmitFromPublic,
                changes: vec![
                    FileChange {
                        path: ScopePath::parse("/README.md").unwrap(),
                        old_content: Some("# Scope Demo\n".to_string()),
                        new_content: Some(
                            "# Scope Demo\n\nPublic text held for later.\n".to_string(),
                        ),
                    },
                    FileChange {
                        path: ScopePath::parse("/internal/model.rs").unwrap(),
                        old_content: Some("fn private_model() {}\n".to_string()),
                        new_content: Some("fn private_model_v2() {}\n".to_string()),
                    },
                ],
            },
        ],
    };

    DemoRepository { policy, graph }
}
