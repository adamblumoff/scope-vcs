use scope_core::domain::{
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
use scope_core::object_store::{MemoryObjectStore, put_source_blob};

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

fn project_public(
    policy: &Policy,
    graph: &SourceGraph,
    events: &[VisibilityEvent],
) -> scope_core::domain::projection::Projection {
    project_graph(policy, graph, events, ProjectionViewKey::Public)
}

type EventSpec<'a> = (
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    Visibility,
    &'a str,
);

fn project_timeline(
    default_visibility: Visibility,
    path_value: &str,
    versions: &[(&str, Visibility, &str)],
    event_specs: &[EventSpec<'_>],
) -> scope_core::domain::projection::Projection {
    let mut policy = Policy::new(default_visibility);
    if default_visibility == Visibility::Private {
        policy
            .add_rule(VisibilityRule::public(path(path_value)))
            .unwrap();
    }
    let commits = versions
        .iter()
        .enumerate()
        .map(|(index, (id, visibility, content))| {
            let previous = index.checked_sub(1).map(|index| versions[index]);
            commit(
                id,
                previous.map(|(id, _, _)| id),
                content,
                change(
                    path_value,
                    *visibility,
                    previous.map(|(_, _, content)| blob(content)),
                    Some(blob(content)),
                ),
            )
        })
        .collect();
    let graph = graph(commits);
    let events = event_specs
        .iter()
        .map(|(id, after, source, visibility, content)| {
            visibility_event(id, *after, *source, path_value, *visibility, blob(content))
        })
        .collect::<Vec<_>>();
    project_public(&policy, &graph, &events)
}

fn published_test_repo(default_visibility: Visibility) -> StoredRepository {
    let owner = UserAccount {
        id: "owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = StoredRepository::new(&owner, "repo", default_visibility).unwrap();
    repo.record.publication_state = RepoPublicationState::Published;
    repo
}

fn published_repo_with_public_file(message: &str, path: &str, content: &str) -> StoredRepository {
    let mut repo = published_test_repo(Visibility::Public);
    let content = blob(content);
    repo.graph.commits.push(commit(
        "rv1",
        None,
        message,
        change(path, Visibility::Public, None, Some(content.clone())),
    ));
    repo.live_files.insert(self::path(path), content);
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
) -> scope_core::domain::projection::Projection {
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
    apply_update_with_head(
        repo,
        "2222222222222222222222222222222222222222",
        message,
        changes,
        previous_config,
        config,
    );
}

fn apply_update_with_head(
    repo: &mut StoredRepository,
    head_oid: &str,
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
            git_head: scope_core::domain::store::GitHead {
                head_oid: head_oid.to_string(),
                segment_sequence: 1,
                change_version: 1,
                manifest: blob("manifest v2"),
            },
            git_segment: scope_core::domain::store::GitSegment {
                sequence: 1,
                base_oid: None,
                head_oid: head_oid.to_string(),
                object: blob("segment v2"),
                manifest: blob("manifest segment v2"),
            },
            changes,
            previous_config,
            config,
        },
    )
    .unwrap();
}

#[test]
fn content_only_updates_keep_policy_and_commit_identity_in_sync() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = config(
        Visibility::Public,
        Some(("/secret.txt", Visibility::Private)),
        None,
    );
    repo.repo_config = config.clone();

    apply_update_with_head(
        &mut repo,
        "3333333333333333333333333333333333333333",
        "add secret",
        vec![reviewed_change("/secret.txt", Some("secret"))],
        Some(config.clone()),
        config.clone(),
    );
    apply_update_with_head(
        &mut repo,
        "4444444444444444444444444444444444444444",
        "update readme",
        vec![reviewed_change("/README.md", Some("updated"))],
        Some(config.clone()),
        config,
    );

    assert_eq!(
        repo.policy.effective_visibility(&path("/secret.txt")),
        Visibility::Private
    );
    assert_eq!(
        repo.graph.commits[repo.graph.commits.len() - 2].id,
        "rv_push_3333333333333333333333333333333333333333"
    );
    assert_eq!(
        repo.graph.commits.last().unwrap().id,
        "rv_push_4444444444444444444444444444444444444444"
    );
}

#[test]
fn content_only_update_preserves_existing_visibility_override() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    repo.policy
        .add_rule(VisibilityRule::private(path("/README.md")))
        .unwrap();
    let config = repo.repo_config.clone();

    apply_update(
        &mut repo,
        "update readme",
        vec![reviewed_change("/README.md", Some("updated"))],
        Some(config.clone()),
        config,
    );

    assert_eq!(
        repo.policy.effective_visibility(&path("/README.md")),
        Visibility::Private
    );
    assert_eq!(
        repo.graph.commits.last().unwrap().changes[0].visibility,
        Visibility::Private
    );
    assert!(repo.visibility_events.is_empty());
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
    repo.live_files.insert(path("/README.md"), blob("hello"));

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

    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(path("/internal")))
        .unwrap();
    let projection = project_graph(&policy, &graph, &[], ProjectionViewKey::Public);

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

    let projection = project_public(&policy, &graph, &[]);

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn destructive_rewrite_rebuilds_each_public_boundary_safely() {
    for (name, next_content, stays_public, expected_commit) in [
        (
            "changed",
            Some(Some("sanitized")),
            true,
            Some("rv_push_2222222222222222222222222222222222222222"),
        ),
        ("unchanged", None, true, Some("vis_1")),
        ("private", None, false, None),
        ("deleted", Some(None), false, None),
    ] {
        let path = "/leaked.txt";
        let mut repo = published_repo_with_public_file("leaked", path, "secret");
        let mut changes = vec![reviewed_change("/.scope/repo.json", Some("config v2"))];
        if let Some(content) = next_content {
            changes.insert(0, reviewed_change(path, content));
        }
        apply_update(
            &mut repo,
            name,
            changes,
            None,
            config(
                Visibility::Private,
                stays_public.then_some((path, Visibility::Public)),
                Some(path),
            ),
        );

        let projection = project_repo(&repo, ProjectionViewKey::Public);
        assert_eq!(
            projection
                .commits
                .first()
                .map(|commit| commit.logical_commit_id.as_str()),
            expected_commit,
            "{name}"
        );
        assert_eq!(
            projection.visible_paths(),
            if stays_public { vec![path] } else { vec![] }
        );
        assert!(
            projection
                .commits
                .iter()
                .all(|commit| commit.logical_commit_id != "rv1")
        );
    }
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
fn public_projection_handles_reveal_and_private_gap_timelines() {
    let cases = [
        (
            Visibility::Private,
            vec![
                ("rv1", Visibility::Private, "draft"),
                ("rv2", Visibility::Public, "release"),
            ],
            vec![],
            vec!["rv2"],
        ),
        (
            Visibility::Private,
            vec![("rv1", Visibility::Private, "draft")],
            vec![("vis_1", Some("rv1"), None, Visibility::Public, "draft")],
            vec!["vis_1"],
        ),
        (
            Visibility::Public,
            vec![
                ("rv1", Visibility::Public, "v1"),
                ("rv2", Visibility::Private, "v2"),
                ("rv3", Visibility::Public, "v3"),
            ],
            vec![
                ("vis_1", Some("rv1"), Some("rv2"), Visibility::Private, "v2"),
                ("vis_2", None, Some("rv3"), Visibility::Public, "v3"),
            ],
            vec!["rv1", "vis_1", "rv3"],
        ),
        (
            Visibility::Public,
            vec![("rv1", Visibility::Public, "readme")],
            vec![
                ("vis_1", Some("rv1"), None, Visibility::Private, "readme"),
                ("vis_2", Some("rv1"), None, Visibility::Public, "readme"),
            ],
            vec!["rv1", "vis_1", "vis_2"],
        ),
    ];
    for (default, versions, events, expected_ids) in cases {
        let projection = project_timeline(default, "/file", &versions, &events);
        assert_eq!(
            projection
                .commits
                .iter()
                .map(|commit| commit.logical_commit_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(projection.visible_paths(), vec!["/file"]);
    }
}
