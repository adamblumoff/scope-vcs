use api::domain::{
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    projection::{
        AuthorVisibility, FileChange, LogicalCommit, ProjectionViewKey, SourceGraph,
        VisibilityEvent, project_graph,
    },
    repo_config::{
        ConfigVisibility, HistoryRewriteAction, HistoryRewriteRequest, RepoConfig,
        RepoConfigVisibilityRule,
    },
    reviewed_updates::{
        ReviewedConfigUpdateInput, ReviewedContentChange, ReviewedUpdateInput,
        apply_reviewed_config_to_repo, apply_reviewed_update_to_repo,
    },
    store::{RepoPublicationState, SourceBlob, StoredRepository, UserAccount},
};
use api::object_store::{MemoryObjectStore, put_source_blob};

fn blob(content: &str) -> SourceBlob {
    put_source_blob(&MemoryObjectStore::new(), "scope", content.as_bytes()).unwrap()
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn change(
    path_value: &str,
    visibility: Visibility,
    old_content: Option<SourceBlob>,
    new_content: Option<SourceBlob>,
) -> FileChange {
    FileChange {
        visibility,
        path: path(path_value),
        old_content,
        new_content,
    }
}

fn added(path_value: &str, visibility: Visibility, content: &str) -> FileChange {
    change(path_value, visibility, None, Some(blob(content)))
}

fn commit(id: &str, parent_id: Option<&str>, message: &str, change: FileChange) -> LogicalCommit {
    LogicalCommit {
        id: id.to_string(),
        parent_ids: parent_id.into_iter().map(str::to_string).collect(),
        author_id: "owner".to_string(),
        author_visibility: AuthorVisibility::Visible,
        message: message.to_string(),
        changes: vec![change],
    }
}

fn graph(commits: Vec<LogicalCommit>) -> SourceGraph {
    SourceGraph {
        repo_id: "scope".to_string(),
        commits,
    }
}

fn visibility_event(
    id: &str,
    after_commit_id: Option<&str>,
    source_commit_id: Option<&str>,
    path_value: &str,
    new_visibility: Visibility,
    current_content: SourceBlob,
) -> VisibilityEvent {
    VisibilityEvent {
        id: id.to_string(),
        after_commit_id: after_commit_id.map(str::to_string),
        source_commit_id: source_commit_id.map(str::to_string),
        author_id: "owner".to_string(),
        path: path(path_value),
        old_visibility: match new_visibility {
            Visibility::Public => Visibility::Private,
            Visibility::Private => Visibility::Public,
        },
        new_visibility,
        current_content: Some(current_content),
    }
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

fn published_repo_with_public_file(message: &str, path: &str, content: &str) -> StoredRepository {
    let mut repo = published_test_repo(Visibility::Public);
    repo.graph.commits.push(commit(
        "rv1",
        None,
        message,
        added(path, Visibility::Public, content),
    ));
    repo
}

fn config(
    default: Visibility,
    rule: Option<(&str, Visibility)>,
    rewrite_path: Option<&str>,
) -> RepoConfig {
    let mut config = RepoConfig::with_default_visibility(ConfigVisibility::from(default));
    config.visibility.rules = rule
        .into_iter()
        .map(|(path, visibility)| RepoConfigVisibilityRule {
            path: path.to_string(),
            visibility: ConfigVisibility::from(visibility),
        })
        .collect();
    config.history.rewrites = rewrite_path
        .into_iter()
        .map(|path| HistoryRewriteRequest {
            path: path.to_string(),
            action: HistoryRewriteAction::RedactPublicHistory,
        })
        .collect();
    config.validate().unwrap();
    config
}

fn project_repo(
    repo: &StoredRepository,
    view_key: ProjectionViewKey,
) -> api::domain::projection::Projection {
    project_graph(&repo.policy, &repo.graph, &repo.visibility_events, view_key)
}

fn reviewed_change(path_value: &str, content: Option<&str>) -> ReviewedContentChange {
    ReviewedContentChange {
        path: path(path_value),
        content: content.map(blob),
    }
}

fn apply_update(
    repo: &mut StoredRepository,
    message: &str,
    changes: Vec<ReviewedContentChange>,
    previous_config: Option<RepoConfig>,
    config: RepoConfig,
) {
    apply_reviewed_update_to_repo(
        repo,
        ReviewedUpdateInput {
            branch: "main".to_string(),
            author_id: "owner".to_string(),
            message: message.to_string(),
            git_snapshot: blob("snapshot v2"),
            changes,
            previous_config,
            config,
        },
    )
    .unwrap();
}

#[test]
fn config_only_update_changes_policy_without_content_commit() {
    let mut repo = published_test_repo(Visibility::Private);
    repo.graph.commits.push(commit(
        "rv1",
        None,
        "initial",
        added("/README.md", Visibility::Private, "hello"),
    ));

    let changed = apply_reviewed_config_to_repo(
        &mut repo,
        ReviewedConfigUpdateInput {
            author_id: "owner".to_string(),
            config: config(
                Visibility::Private,
                Some(("/README.md", Visibility::Public)),
                None,
            ),
        },
    )
    .unwrap();

    assert!(changed);
    assert_eq!(repo.graph.commits.len(), 1);
    assert_eq!(
        repo.policy.effective_visibility(&path("/README.md")),
        Visibility::Public
    );
    assert_eq!(repo.visibility_events.len(), 1);
    assert_eq!(repo.visibility_events[0].source_commit_id, None);
    assert_eq!(
        repo.repo_config.visibility_for_path(&path("/README.md")),
        Visibility::Public
    );
}

#[test]
fn public_projection_contains_only_visible_paths_from_mixed_commit() {
    let mut mixed = commit(
        "rv1",
        None,
        "mixed",
        added("/README.md", Visibility::Public, "hello"),
    );
    mixed.author_visibility = AuthorVisibility::Hidden;
    mixed
        .changes
        .push(added("/internal/model.rs", Visibility::Private, "secret"));
    let graph = graph(vec![mixed]);

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
    let graph = graph(vec![commit(
        "rv1",
        None,
        "public readme",
        added("/README.md", Visibility::Public, "public readme"),
    )]);

    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn destructive_rewrite_removes_old_public_history_for_changed_path() {
    let mut repo = published_repo_with_public_file("leaked", "/leaked.txt", "leaked secret");

    apply_update(
        &mut repo,
        "sanitize leaked file",
        vec![
            reviewed_change("/leaked.txt", Some("sanitized public content")),
            reviewed_change("/.scope/repo.json", Some("config v2")),
        ],
        None,
        config(
            Visibility::Private,
            Some(("/leaked.txt", Visibility::Public)),
            Some("/leaked.txt"),
        ),
    );

    let public_projection = project_repo(&repo, ProjectionViewKey::Public);
    let private_projection = project_repo(&repo, ProjectionViewKey::Private);

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
    let mut repo = published_repo_with_public_file(
        "old public readme history",
        "/README.md",
        "current public readme",
    );

    apply_update(
        &mut repo,
        "redact readme history",
        vec![reviewed_change("/.scope/repo.json", Some("config v2"))],
        None,
        config(
            Visibility::Private,
            Some(("/README.md", Visibility::Public)),
            Some("/README.md"),
        ),
    );

    let public_projection = project_repo(&repo, ProjectionViewKey::Public);

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "vis_1");
    assert_eq!(public_projection.commits[0].message, "Projection baseline");
    assert_eq!(public_projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn destructive_rewrite_to_private_leaves_no_public_boundary_commit() {
    let mut repo =
        published_repo_with_public_file("leaked public history", "/leaked.txt", "leaked secret");

    apply_update(
        &mut repo,
        "make leaked file private",
        vec![reviewed_change("/.scope/repo.json", Some("config v2"))],
        None,
        config(Visibility::Private, None, Some("/leaked.txt")),
    );

    let public_projection = project_repo(&repo, ProjectionViewKey::Public);

    assert!(public_projection.commits.is_empty());
    assert!(public_projection.visible_paths().is_empty());
}

#[test]
fn destructive_rewrite_delete_does_not_create_public_delete_commit() {
    let mut repo =
        published_repo_with_public_file("leaked public history", "/leaked.txt", "leaked secret");

    apply_update(
        &mut repo,
        "delete leaked file",
        vec![
            reviewed_change("/leaked.txt", None),
            reviewed_change("/.scope/repo.json", Some("config v2")),
        ],
        None,
        config(Visibility::Private, None, Some("/leaked.txt")),
    );

    let public_projection = project_repo(&repo, ProjectionViewKey::Public);

    assert!(public_projection.commits.is_empty());
    assert!(public_projection.visible_paths().is_empty());
}

#[test]
fn unchanged_history_rewrite_is_not_reapplied_on_later_push() {
    let config = config(Visibility::Public, None, Some("/leaked.txt"));
    let mut repo = published_repo_with_public_file(
        "existing public history",
        "/leaked.txt",
        "existing public content",
    );

    apply_update(
        &mut repo,
        "later config-only push",
        vec![reviewed_change("/.scope/repo.json", Some("same config"))],
        Some(config.clone()),
        config,
    );

    let public_projection = project_repo(&repo, ProjectionViewKey::Public);

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(public_projection.visible_paths(), vec!["/leaked.txt"]);
}

#[test]
fn private_to_public_source_change_starts_public_history_at_reveal() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(path("/notes.md")))
        .unwrap();
    let graph = graph(vec![
        commit(
            "rv1",
            None,
            "private draft",
            added("/notes.md", Visibility::Private, "private draft"),
        ),
        commit(
            "rv2",
            Some("rv1"),
            "public release",
            change(
                "/notes.md",
                Visibility::Public,
                Some(blob("private draft")),
                Some(blob("public release")),
            ),
        ),
    ]);

    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv2");
    assert_eq!(projection.commits[0].changes[0].path.as_str(), "/notes.md");
}

#[test]
fn private_to_public_visibility_event_adds_safe_projection_baseline() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(path("/notes.md")))
        .unwrap();
    let graph = graph(vec![commit(
        "rv1",
        None,
        "private draft",
        added("/notes.md", Visibility::Private, "private draft"),
    )]);
    let visibility_events = vec![visibility_event(
        "vis_1",
        Some("rv1"),
        None,
        "/notes.md",
        Visibility::Public,
        blob("private draft"),
    )];

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
    let graph = graph(vec![
        commit(
            "rv1",
            None,
            "public v1",
            added("/README.md", Visibility::Public, "public v1"),
        ),
        commit(
            "rv2",
            Some("rv1"),
            "private v2",
            change(
                "/README.md",
                Visibility::Private,
                Some(blob("public v1")),
                Some(blob("private v2")),
            ),
        ),
        commit(
            "rv3",
            Some("rv2"),
            "public v3",
            change(
                "/README.md",
                Visibility::Public,
                Some(blob("private v2")),
                Some(blob("public v3")),
            ),
        ),
    ]);
    let visibility_events = vec![
        visibility_event(
            "vis_1",
            Some("rv1"),
            Some("rv2"),
            "/README.md",
            Visibility::Private,
            blob("private v2"),
        ),
        visibility_event(
            "vis_2",
            None,
            Some("rv3"),
            "/README.md",
            Visibility::Public,
            blob("public v3"),
        ),
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
    let graph = graph(vec![commit(
        "rv1",
        None,
        "original public commit",
        added("/README.md", Visibility::Public, "public readme"),
    )]);
    let visibility_events = vec![
        visibility_event(
            "vis_1",
            Some("rv1"),
            None,
            "/README.md",
            Visibility::Private,
            blob("public readme"),
        ),
        visibility_event(
            "vis_2",
            Some("rv1"),
            None,
            "/README.md",
            Visibility::Public,
            blob("public readme"),
        ),
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
    let graph = graph(vec![commit(
        "rv1",
        None,
        "private",
        added("/internal/model.rs", Visibility::Private, "secret"),
    )]);

    let projection = project_graph(&fixture_policy(), &graph, &[], ProjectionViewKey::Private);

    assert_eq!(projection.visible_paths(), vec!["/internal/model.rs"]);
}
