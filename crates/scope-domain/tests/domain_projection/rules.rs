use super::*;

#[test]
fn content_push_requires_rules_in_the_resulting_tree() {
    let repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_tree(),
        git_head: repo.git_head.clone(),
    };

    let deletion = accept_content_push(
        state,
        reviewed_update(
            "3333333333333333333333333333333333333333",
            "delete rules",
            vec![reviewed_change("/.scope/RULES.md", None)],
            Some(config.clone()),
            config.clone(),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(deletion, ReviewedUpdateError::BadRequest(message) if message.contains("RULES.md"))
    );

    let missing_state = ContentPushState {
        change_version: 1,
        policy: Policy::new(Visibility::Public),
        repo_config: config.clone(),
        live_files: Default::default(),
        git_head: None,
    };
    let missing = accept_content_push(
        missing_state.clone(),
        reviewed_update(
            "4444444444444444444444444444444444444444",
            "missing rules",
            vec![reviewed_change("/README.md", Some("hello"))],
            Some(config.clone()),
            config.clone(),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(missing, ReviewedUpdateError::BadRequest(message) if message.contains("RULES.md"))
    );

    accept_content_push(
        missing_state,
        reviewed_update(
            "5555555555555555555555555555555555555555",
            "add rules",
            vec![reviewed_change("/.scope/RULES.md", Some(""))],
            Some(config.clone()),
            config,
        ),
    )
    .unwrap();
}

#[test]
fn request_merge_accepts_unchanged_tree_without_weakening_push_rules() {
    let repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_tree(),
        git_head: repo.git_head.clone(),
    };
    let update = reviewed_update(
        "3333333333333333333333333333333333333333",
        "merge request",
        Vec::new(),
        Some(config.clone()),
        config,
    );

    assert!(accept_content_push(state.clone(), update.clone()).is_err());
    let accepted = accept_request_merge(
        state,
        update,
        RequestMergeOrigin::Private {
            request_id: "request-1".to_string(),
            request_head_oid: "2222222222222222222222222222222222222222".to_string(),
        },
    )
    .unwrap();
    assert_eq!(accepted.change_version, 2);
    assert_eq!(accepted.git_head.change_version, 2);
    assert_eq!(
        accepted.logical_commit.id,
        "rv_merge_3333333333333333333333333333333333333333"
    );
    assert_eq!(
        accepted.logical_commit.origin,
        LogicalCommitOrigin::PrivateRequestMerge {
            request_id: "request-1".to_string(),
            request_head_oid: "2222222222222222222222222222222222222222".to_string(),
        }
    );
    assert!(accepted.logical_commit.changes.is_empty());
}

#[test]
fn public_projection_always_includes_canonical_rules_changes() {
    let graph = graph(vec![commit(
        "rv1",
        None,
        "add rules",
        added("/.scope/RULES.md", Visibility::Public, ""),
    )]);

    let projection = project_graph(&graph, &[], ProjectionViewKey::Public);

    assert_eq!(projection.visible_paths(), vec!["/.scope/RULES.md"]);
}
