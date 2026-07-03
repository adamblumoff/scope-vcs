use api::domain::{
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, LogicalCommit, ProjectionViewKey, SourceGraph,
        VisibilityEvent, project_graph,
    },
};
use api::object_store::{MemoryObjectStore, put_source_blob};

fn blob(content: &str) -> api::domain::store::SourceBlob {
    put_source_blob(&MemoryObjectStore::new(), "scope", content.as_bytes()).unwrap()
}

fn fixture_policy() -> Policy {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
        ))
        .unwrap();
    policy
}

#[test]
fn public_projection_contains_only_visible_paths_from_mixed_commit() {
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Hidden,
            message: "mixed".to_string(),
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
        }],
    };

    let projection = project_graph(&fixture_policy(), &graph, &[], ProjectionViewKey::Public);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
    assert_eq!(projection.commits[0].message, "Projected public update");
    assert!(projection.commits[0].author.is_none());
}

#[test]
fn public_projection_omits_current_private_path_history() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/README.md").unwrap(),
        ))
        .unwrap();
    let public_blob = blob("public readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "public readme".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(public_blob),
            }],
        }],
    };

    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);

    assert!(projection.commits.is_empty());
    assert!(projection.visible_paths().is_empty());
}

#[test]
fn private_to_public_source_change_starts_public_history_at_reveal() {
    let mut policy = Policy::new(Visibility::Private);
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
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: None,
                    new_content: Some(private_blob.clone()),
                }],
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public release".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/notes.md").unwrap(),
                    old_content: Some(private_blob),
                    new_content: Some(blob("public release")),
                }],
            },
        ],
    };

    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/notes.md");
}

#[test]
fn private_to_public_visibility_event_adds_safe_projection_baseline() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/notes.md").unwrap(),
        ))
        .unwrap();
    let private_blob = blob("private draft");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "private draft".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/notes.md").unwrap(),
                old_content: None,
                new_content: Some(private_blob.clone()),
            }],
        }],
    };
    let visibility_events = vec![VisibilityEvent {
        id: "vis_1".to_string(),
        after_commit_id: Some("rv1".to_string()),
        source_commit_id: None,
        author_id: "owner".to_string(),
        path: ScopePath::parse("/notes.md").unwrap(),
        old_visibility: Visibility::Private,
        new_visibility: Visibility::Public,
        current_content: Some(private_blob),
    }];

    let projection = project_graph(
        &policy,
        &graph,
        &visibility_events,
        ProjectionViewKey::Public,
    );

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "vis_1");
    assert_eq!(projection.commits[0].message, "Projection baseline");
    assert_eq!(projection.visible_paths(), vec!["/notes.md"]);
}

#[test]
fn public_projection_restarts_after_private_gap() {
    let policy = Policy::new(Visibility::Public);
    let public_v1 = blob("public v1");
    let private_v2 = blob("private v2");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![
            LogicalCommit {
                id: "rv1".to_string(),
                parent_ids: vec![],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public v1".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: None,
                    new_content: Some(public_v1.clone()),
                }],
            },
            LogicalCommit {
                id: "rv2".to_string(),
                parent_ids: vec!["rv1".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "private v2".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Private,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: Some(public_v1),
                    new_content: Some(private_v2.clone()),
                }],
            },
            LogicalCommit {
                id: "rv3".to_string(),
                parent_ids: vec!["rv2".to_string()],
                author_id: "owner".to_string(),
                author_visibility: AuthorVisibility::Visible,
                message: "public v3".to_string(),
                changes: vec![FileChange {
                    visibility: Visibility::Public,
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_content: Some(private_v2),
                    new_content: Some(blob("public v3")),
                }],
            },
        ],
    };
    let path = ScopePath::parse("/README.md").unwrap();
    let visibility_events = vec![
        VisibilityEvent {
            id: "vis_1".to_string(),
            after_commit_id: None,
            source_commit_id: Some("rv2".to_string()),
            author_id: "owner".to_string(),
            path: path.clone(),
            old_visibility: Visibility::Public,
            new_visibility: Visibility::Private,
            current_content: Some(blob("private v2")),
        },
        VisibilityEvent {
            id: "vis_2".to_string(),
            after_commit_id: None,
            source_commit_id: Some("rv3".to_string()),
            author_id: "owner".to_string(),
            path,
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(blob("public v3")),
        },
    ];

    let projection = project_graph(
        &policy,
        &graph,
        &visibility_events,
        ProjectionViewKey::Public,
    );

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv3");
    assert_eq!(projection.commits[0].message, "public v3");
}

#[test]
fn pure_visibility_reveal_does_not_restore_prior_public_history() {
    let policy = Policy::new(Visibility::Public);
    let readme = blob("public readme");
    let graph = SourceGraph {
        repo_id: "scope".to_string(),
        commits: vec![LogicalCommit {
            id: "rv1".to_string(),
            parent_ids: vec![],
            author_id: "owner".to_string(),
            author_visibility: AuthorVisibility::Visible,
            message: "original public commit".to_string(),
            changes: vec![FileChange {
                visibility: Visibility::Public,
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(readme.clone()),
            }],
        }],
    };
    let path = ScopePath::parse("/README.md").unwrap();
    let visibility_events = vec![
        VisibilityEvent {
            id: "vis_1".to_string(),
            after_commit_id: Some("rv1".to_string()),
            source_commit_id: None,
            author_id: "owner".to_string(),
            path: path.clone(),
            old_visibility: Visibility::Public,
            new_visibility: Visibility::Private,
            current_content: Some(readme.clone()),
        },
        VisibilityEvent {
            id: "vis_2".to_string(),
            after_commit_id: Some("rv1".to_string()),
            source_commit_id: None,
            author_id: "owner".to_string(),
            path,
            old_visibility: Visibility::Private,
            new_visibility: Visibility::Public,
            current_content: Some(readme),
        },
    ];

    let projection = project_graph(
        &policy,
        &graph,
        &visibility_events,
        ProjectionViewKey::Public,
    );

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "vis_2");
    assert_eq!(projection.commits[0].message, "Projection baseline");
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
            changes: vec![FileChange {
                visibility: Visibility::Private,
                path: ScopePath::parse("/internal/model.rs").unwrap(),
                old_content: None,
                new_content: Some(blob("secret")),
            }],
        }],
    };

    let projection = project_graph(&fixture_policy(), &graph, &[], ProjectionViewKey::Private);

    assert_eq!(projection.visible_paths(), vec!["/internal/model.rs"]);
}
