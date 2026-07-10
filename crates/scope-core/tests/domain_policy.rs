use scope_core::domain::policy::{Policy, PolicyError, ScopePath, Visibility, VisibilityRule};

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn policy_with_private_internal() -> Policy {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(path("/internal")))
        .unwrap();
    policy
}

#[test]
fn private_parent_hides_children() {
    let policy = policy_with_private_internal();
    let path = path("/internal/model.rs");

    assert!(!policy.can_read(&path, false));
    assert!(policy.can_read(&path, true));
}

#[test]
fn rejects_public_island_under_private_parent() {
    let mut policy = policy_with_private_internal();

    let error = policy
        .add_rule(VisibilityRule::public(path("/internal/readme.md")))
        .unwrap_err();

    assert!(matches!(error, PolicyError::PublicIsland { .. }));
}
