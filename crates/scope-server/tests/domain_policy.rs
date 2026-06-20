use scope_server::domain::policy::{
    Policy, PolicyError, Principal, PrincipalKind, ScopePath, Visibility, VisibilityRule,
};

#[test]
fn private_parent_hides_children() {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
            ["user_collaborator".to_string()],
        ))
        .unwrap();

    let public = Principal::public();
    let collaborator = Principal {
        id: "user_collaborator".to_string(),
        kind: PrincipalKind::User,
    };
    let path = ScopePath::parse("/internal/model.rs").unwrap();

    assert!(!policy.can_read(&public, &path));
    assert!(policy.can_read(&collaborator, &path));
}

#[test]
fn rejects_public_island_under_private_parent() {
    let mut policy = Policy::new(Visibility::Public, "owner");
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").unwrap(),
            ["user_collaborator".to_string()],
        ))
        .unwrap();

    let error = policy
        .add_rule(VisibilityRule::public(
            ScopePath::parse("/internal/readme.md").unwrap(),
        ))
        .unwrap_err();

    assert!(matches!(error, PolicyError::PublicIsland { .. }));
}
