use api::domain::{
    policy::{Policy, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, FileVisibilityChange, LogicalCommit, MixedCommitPolicy,
        SourceGraph, project_graph,
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
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/internal/model.rs").unwrap(),
                    old_content: None,
                    new_content: Some(blob("secret")),
                },
            ],
            visibility_changes: Vec::new(),
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
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("hello")),
                },
                FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/internal/model.rs").unwrap(),
                    old_content: None,
                    new_content: Some(blob("secret")),
                },
            ],
            visibility_changes: Vec::new(),
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
                visibility: Visibility::Private,
                path: ScopePath::parse("/internal/model.rs").unwrap(),
                old_content: None,
                new_content: Some(blob("secret")),
            }],
            visibility_changes: Vec::new(),
        }],
    };

    let collaborator = Principal {
        id: "user_collaborator".to_string(),
        kind: PrincipalKind::User,
    };
    let projection = project_graph(&fixture_policy(), &graph, &collaborator);

    assert_eq!(projection.visible_paths(), vec!["/internal/model.rs"]);
}

#[test]
fn public_projection_starts_at_private_to_public_transition() {
    let mut policy = Policy::new(Visibility::Private, "owner");
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private draft".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("private draft")),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public release".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: Some(blob("private draft")),
                    new_content: Some(blob("public release")),
                }],
                visibility_changes: Vec::new(),
            },
        ],
    };

    let projection = project_graph(&policy, &graph, &Principal::public());

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/notes.md");
}

#[test]
fn non_owner_projection_starts_at_private_to_public_transition() {
    let mut policy = Policy::new(Visibility::Private, "owner");
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private draft".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("private draft")),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public release".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: Some(blob("private draft")),
                    new_content: Some(blob("public release")),
                }],
                visibility_changes: Vec::new(),
            },
        ],
    };
    let member = Principal {
        id: "user_member".to_string(),
        kind: PrincipalKind::User,
    };

    let projection = project_graph(&policy, &graph, &member);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/notes.md");
}

#[test]
fn non_owner_projection_applies_private_to_public_visibility_transition() {
    let mut policy = Policy::new(Visibility::Private, "owner");
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let private_blob = blob("private draft");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private draft".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(private_blob.clone()),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "make public".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_visibility: Visibility::Private,
                    new_visibility: Visibility::Public,
                    current_content: Some(private_blob),
                }],
            },
        ],
    };
    let member = Principal {
        id: "user_member".to_string(),
        kind: PrincipalKind::User,
    };

    let projection = project_graph(&policy, &graph, &member);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/notes.md");
    assert!(projection.commits[0].changes[0].new_content.is_some());
}

#[test]
fn public_projection_replays_visibility_toggle_ending_private() {
    let policy = Policy::new(Visibility::Private, "owner");
    let public_blob = blob("public interval");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private draft".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(blob("private draft")),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "make public".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_visibility: Visibility::Private,
                    new_visibility: Visibility::Public,
                    current_content: Some(public_blob.clone()),
                }],
            },
            LogicalCommit {
                id: "rv3".to_string(),
                parent_ids: vec!["rv2".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "make private".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(public_blob),
                }],
            },
        ],
    };

    let projection = project_graph(&policy, &graph, &Principal::public());

    assert_eq!(projection.commits.len(), 2);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert!(projection.commits[0].changes[0].new_content.is_some());
    assert_eq!(projection.commits[1].logical_commit_id, "rv3");
    assert!(projection.commits[1].changes[0].new_content.is_none());
}

#[test]
fn public_projection_replays_visibility_toggle_ending_public() {
    let policy = Policy::new(Visibility::Public, "owner");
    let public_blob = blob("public readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_blob.clone()),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "make private".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(public_blob.clone()),
                }],
            },
            LogicalCommit {
                id: "rv3".to_string(),
                parent_ids: vec!["rv2".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "make public".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility: Visibility::Private,
                    new_visibility: Visibility::Public,
                    current_content: Some(public_blob),
                }],
            },
        ],
    };

    let projection = project_graph(&policy, &graph, &Principal::public());

    assert_eq!(projection.commits.len(), 3);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.commits[1].logical_commit_id, "rv2");
    assert!(projection.commits[1].changes[0].new_content.is_none());
    assert_eq!(projection.commits[2].logical_commit_id, "rv3");
    assert!(projection.commits[2].changes[0].new_content.is_some());
}

#[test]
fn public_projection_keeps_public_history_before_public_to_private_transition() {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
            ["owner".to_string()],
        ))
        .unwrap();
    let public_blob = blob("public readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_blob.clone()),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: Some(public_blob),
                    new_content: Some(blob("private readme")),
                }],
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(blob("private readme")),
                }],
            },
        ],
    };

    let projection = project_graph(&policy, &graph, &Principal::public());

    assert_eq!(projection.commits.len(), 2);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/README.md");
    assert!(projection.commits[1].changes[0].new_content.is_none());
}

#[test]
fn non_owner_projection_applies_public_to_private_transition() {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
            ["owner".to_string()],
        ))
        .unwrap();
    let public_blob = blob("public readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_blob.clone()),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: Vec::new(),
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(public_blob),
                }],
            },
        ],
    };
    let member = Principal {
        id: "user_member".to_string(),
        kind: PrincipalKind::User,
    };

    let projection = project_graph(&policy, &graph, &member);

    assert_eq!(projection.commits.len(), 2);
    assert!(projection.commits[1].changes[0].new_content.is_none());
}

#[test]
fn authorized_reader_keeps_private_content_after_public_to_private_transition() {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
            ["owner".to_string(), "user_member".to_string()],
        ))
        .unwrap();
    let public_blob = blob("public readme");
    let private_blob = blob("private readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_blob.clone()),
                }],
                visibility_changes: Vec::new(),
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private readme".to_string(),
                mixed_policy: MixedCommitPolicy::SyntheticPublicCommit,
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: Some(public_blob),
                    new_content: Some(private_blob),
                }],
                visibility_changes: vec![FileVisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(blob("private readme")),
                }],
            },
        ],
    };
    let member = Principal {
        id: "user_member".to_string(),
        kind: PrincipalKind::User,
    };

    let projection = project_graph(&policy, &graph, &member);

    assert_eq!(projection.commits.len(), 2);
    assert_eq!(projection.commits[1].changes.len(), 1);
    assert!(projection.commits[1].changes[0].new_content.is_some());
}
