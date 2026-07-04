use api::domain::{
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, LogicalCommit, ProjectionViewKey, SourceGraph,
        VisibilityEvent, project_graph,
    },
    repo_config::RepoConfig,
    staged_updates::{ReviewedUpdateInput, StagedContentChange, apply_reviewed_update_to_repo},
    store::{RepoPublicationState, StoredRepository, UserAccount},
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

fn test_owner() -> UserAccount {
    UserAccount {
        id: "owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    }
}

fn published_test_repo(default_visibility: Visibility) -> StoredRepository {
    let mut repo = StoredRepository::new(&test_owner(), "repo", default_visibility).unwrap();
    repo.record.publication_state = RepoPublicationState::Published;
    repo
}

fn parse_config(json: &[u8]) -> RepoConfig {
    RepoConfig::parse_json(json).unwrap()
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
fn public_projection_keeps_public_history_when_policy_later_marks_path_private() {
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

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn destructive_rewrite_removes_old_public_history_for_changed_path() {
    let mut repo = published_test_repo(Visibility::Public);
    let leaked = blob("leaked secret");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: "leaked".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/leaked.txt").unwrap(),
            old_content: None,
            new_content: Some(leaked.clone()),
        }],
    });

    apply_reviewed_update_to_repo(
        &mut repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: "sanitize leaked file".to_string(),
            git_snapshot: blob("snapshot v2"),
            changes: vec![
                StagedContentChange {
                    path: ScopePath::parse("/leaked.txt").unwrap(),
                    content: Some(blob("sanitized public content")),
                },
                StagedContentChange {
                    path: ScopePath::parse("/.scope/repo.json").unwrap(),
                    content: Some(blob("config v2")),
                },
            ],
            previous_config: None,
            config: parse_config(
                br#"{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {
                        "default": "private",
                        "rules": [
                            { "path": "/leaked.txt", "visibility": "public" }
                        ]
                    },
                    "history": {
                        "rewrites": [
                            {
                                "path": "/leaked.txt",
                                "action": "redact-public-history"
                            }
                        ]
                    }
                }"#,
            ),
        },
    )
    .unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );
    let private_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Private,
    );

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "rv_push_2");
    assert_eq!(
        public_projection.commits[0].changes[0].path.as_str(),
        "/leaked.txt"
    );
    assert_eq!(public_projection.visible_paths(), vec!["/leaked.txt"]);
    assert!(
        public_projection
            .commits
            .iter()
            .all(|commit| { commit.logical_commit_id != "rv1" })
    );
    assert_eq!(private_projection.commits.len(), 2);
}

#[test]
fn destructive_rewrite_replaces_unchanged_public_history_with_baseline() {
    let mut repo = published_test_repo(Visibility::Public);
    let readme = blob("current public readme");
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: "old public readme history".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/README.md").unwrap(),
            old_content: None,
            new_content: Some(readme),
        }],
    });

    apply_reviewed_update_to_repo(
        &mut repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: "redact readme history".to_string(),
            git_snapshot: blob("snapshot v2"),
            changes: vec![StagedContentChange {
                path: ScopePath::parse("/.scope/repo.json").unwrap(),
                content: Some(blob("config v2")),
            }],
            previous_config: None,
            config: parse_config(
                br#"{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {
                        "default": "private",
                        "rules": [
                            { "path": "/README.md", "visibility": "public" }
                        ]
                    },
                    "history": {
                        "rewrites": [
                            {
                                "path": "/README.md",
                                "action": "redact-public-history"
                            }
                        ]
                    }
                }"#,
            ),
        },
    )
    .unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "vis_1");
    assert_eq!(public_projection.commits[0].message, "Projection baseline");
    assert_eq!(public_projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn destructive_rewrite_to_private_leaves_no_public_boundary_commit() {
    let mut repo = published_test_repo(Visibility::Public);
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: "leaked public history".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/leaked.txt").unwrap(),
            old_content: None,
            new_content: Some(blob("leaked secret")),
        }],
    });

    apply_reviewed_update_to_repo(
        &mut repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: "make leaked file private".to_string(),
            git_snapshot: blob("snapshot v2"),
            changes: vec![StagedContentChange {
                path: ScopePath::parse("/.scope/repo.json").unwrap(),
                content: Some(blob("config v2")),
            }],
            previous_config: None,
            config: parse_config(
                br#"{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {
                        "default": "private",
                        "rules": []
                    },
                    "history": {
                        "rewrites": [
                            {
                                "path": "/leaked.txt",
                                "action": "redact-public-history"
                            }
                        ]
                    }
                }"#,
            ),
        },
    )
    .unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );

    assert!(public_projection.commits.is_empty());
    assert!(public_projection.visible_paths().is_empty());
}

#[test]
fn destructive_rewrite_delete_does_not_create_public_delete_commit() {
    let mut repo = published_test_repo(Visibility::Public);
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: "leaked public history".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/leaked.txt").unwrap(),
            old_content: None,
            new_content: Some(blob("leaked secret")),
        }],
    });

    apply_reviewed_update_to_repo(
        &mut repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: "delete leaked file".to_string(),
            git_snapshot: blob("snapshot v2"),
            changes: vec![
                StagedContentChange {
                    path: ScopePath::parse("/leaked.txt").unwrap(),
                    content: None,
                },
                StagedContentChange {
                    path: ScopePath::parse("/.scope/repo.json").unwrap(),
                    content: Some(blob("config v2")),
                },
            ],
            previous_config: None,
            config: parse_config(
                br#"{
                    "kind": "scope.repo-config",
                    "version": 1,
                    "visibility": {
                        "default": "private",
                        "rules": []
                    },
                    "history": {
                        "rewrites": [
                            {
                                "path": "/leaked.txt",
                                "action": "redact-public-history"
                            }
                        ]
                    }
                }"#,
            ),
        },
    )
    .unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );

    assert!(public_projection.commits.is_empty());
    assert!(public_projection.visible_paths().is_empty());
}

#[test]
fn unchanged_history_rewrite_is_not_reapplied_on_later_push() {
    let config = parse_config(
        br#"{
            "kind": "scope.repo-config",
            "version": 1,
            "visibility": {
                "default": "public",
                "rules": []
            },
            "history": {
                "rewrites": [
                    {
                        "path": "/leaked.txt",
                        "action": "redact-public-history"
                    }
                ]
            }
        }"#,
    );
    let mut repo = published_test_repo(Visibility::Public);
    repo.graph.commits.push(LogicalCommit {
        id: "rv1".to_string(),
        parent_ids: Vec::new(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: "existing public history".to_string(),
        changes: vec![FileChange {
            visibility: Visibility::Public,
            path: ScopePath::parse("/leaked.txt").unwrap(),
            old_content: None,
            new_content: Some(blob("existing public content")),
        }],
    });

    apply_reviewed_update_to_repo(
        &mut repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: "later config-only push".to_string(),
            git_snapshot: blob("snapshot v2"),
            changes: vec![StagedContentChange {
                path: ScopePath::parse("/.scope/repo.json").unwrap(),
                content: Some(blob("same config")),
            }],
            previous_config: Some(config.clone()),
            config,
        },
    )
    .unwrap();

    let public_projection = project_graph(
        &repo.policy,
        &repo.graph,
        &repo.visibility_events,
        ProjectionViewKey::Public,
    );

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(public_projection.visible_paths(), vec!["/leaked.txt"]);
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
            after_commit_id: Some("rv1".to_string()),
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

    assert_eq!(projection.commits.len(), 3);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.commits[1].logical_commit_id, "vis_1");
    assert_eq!(
        projection.commits[1].message,
        "Projection visibility boundary"
    );
    assert_eq!(projection.commits[2].logical_commit_id, "rv3");
    assert_eq!(projection.commits[2].message, "public v3");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn pure_visibility_toggle_keeps_public_history_and_restores_current_content() {
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

    assert_eq!(projection.commits.len(), 3);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.commits[1].logical_commit_id, "vis_1");
    assert_eq!(
        projection.commits[1].message,
        "Projection visibility boundary"
    );
    assert_eq!(projection.commits[2].logical_commit_id, "vis_2");
    assert_eq!(projection.commits[2].message, "Projection baseline");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
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
