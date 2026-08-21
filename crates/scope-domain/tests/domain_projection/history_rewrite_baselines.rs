use super::*;

#[test]
fn config_only_rewrite_coalesces_its_visibility_baseline() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    repo.policy
        .add_rule(VisibilityRule::private(path("/README.md")))
        .unwrap();
    repo.repo_config = config(
        Visibility::Public,
        Some(("/README.md", Visibility::Private)),
        None,
    );

    apply_reviewed_config_to_repo(
        &mut repo,
        ReviewedConfigUpdateInput {
            author_id: "owner".to_string(),
            config: config(Visibility::Public, None, Some("/README.md")),
        },
    )
    .unwrap();

    assert_eq!(repo.visibility_change_sets.len(), 1);
    assert_eq!(repo.visibility_change_sets[0].changes.len(), 1);
    assert_eq!(
        repo.visibility_change_sets[0].changes[0].path,
        path("/README.md")
    );
    assert_eq!(
        repo.visibility_change_sets[0].changes[0].old_visibility,
        Visibility::Private
    );
    assert_eq!(
        repo.visibility_change_sets[0].changes[0].new_visibility,
        Visibility::Public
    );
}

#[test]
fn push_rewrite_coalesces_its_visibility_baseline() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let previous_config = config(
        Visibility::Public,
        Some(("/README.md", Visibility::Private)),
        None,
    );
    repo.policy
        .add_rule(VisibilityRule::private(path("/README.md")))
        .unwrap();
    repo.repo_config = previous_config.clone();

    apply_update(
        &mut repo,
        "redact and reveal",
        vec![reviewed_change("/.scope/runs/test.yml", Some("name: Test"))],
        Some(previous_config),
        config(Visibility::Public, None, Some("/README.md")),
    );

    assert_eq!(repo.visibility_change_sets.len(), 1);
    assert_eq!(repo.visibility_change_sets[0].changes.len(), 1);
    assert_eq!(
        repo.visibility_change_sets[0].changes[0].path,
        path("/README.md")
    );
    assert!(repo.visibility_change_sets[0].source_update_id.is_some());
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
        (
            "unchanged",
            None,
            true,
            Some("rv_push_2222222222222222222222222222222222222222"),
        ),
        ("private", None, false, None),
        ("deleted", Some(None), false, None),
    ] {
        let path = "/leaked.txt";
        let mut repo = published_repo_with_public_file("leaked", path, "secret");
        let mut changes = vec![reviewed_change("/.scope/runs/test.yml", Some("name: Test"))];
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
