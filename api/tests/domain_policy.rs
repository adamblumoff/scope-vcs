use api::domain::policy::{Policy, PolicyError, ScopePath, Visibility, VisibilityRule};

#[test]
fn private_parent_hides_children() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
        ))
        .unwrap();

    let path = ScopePath::parse("/internal/model.rs").unwrap();

    assert!(!policy.can_read(&path, false));
    assert!(policy.can_read(&path, true));
}

#[test]
fn rejects_public_island_under_private_parent() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
        ))
        .unwrap();

    let error = policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/internal/readme.md").unwrap(),
        ))
        .unwrap_err();

    assert!(matches!(error, PolicyError::PublicIsland { .. }));
}
