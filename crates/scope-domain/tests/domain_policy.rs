use scope_domain::policy::{Policy, PolicyError, ScopePath, Visibility, VisibilityRule};

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

#[test]
fn batched_rules_preserve_visibility_and_last_replacement() {
    let rules = [
        VisibilityRule::public(path("/docs")),
        VisibilityRule::private(path("/docs/secrets")),
        VisibilityRule::public(path("/src/lib.rs")),
        VisibilityRule::private(path("/src/lib.rs")),
        VisibilityRule::public(path("/src/main.rs")),
    ];
    let mut sequential = Policy::new(Visibility::Private);
    for rule in rules.clone() {
        sequential.add_rule(rule).unwrap();
    }
    let mut batch = Policy::new(Visibility::Private);
    batch.add_rules(rules).unwrap();
    assert_eq!(batch.rules(), sequential.rules());
    for file in [
        "/docs/guide.md",
        "/docs/secrets/key",
        "/src/lib.rs",
        "/src/main.rs",
        "/other",
    ] {
        assert_eq!(
            batch.effective_visibility(&path(file)),
            sequential.effective_visibility(&path(file))
        );
    }
    batch.remove_rule(&path("/docs/secrets"));
    assert_eq!(
        batch.effective_visibility(&path("/docs/secrets/key")),
        Visibility::Public
    );
}

#[test]
fn batch_rejects_private_ancestors_in_either_input_order_without_mutation() {
    for parent in ["/", "/docs", "/docs/private"] {
        let rules = [
            VisibilityRule::public(path("/docs/private/file")),
            VisibilityRule::private(path(parent)),
        ];
        for rules in [rules.clone(), [rules[1].clone(), rules[0].clone()]] {
            let mut policy = Policy::new(Visibility::Public);
            let error = policy.add_rules(rules).unwrap_err();
            assert_eq!(
                error,
                PolicyError::PublicIsland {
                    child: path("/docs/private/file"),
                    parent: path(parent),
                }
            );
            assert!(policy.rules().is_empty());
        }
    }
}

#[test]
fn ancestor_lookup_respects_segment_boundaries_and_lexical_siblings() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rules([
            VisibilityRule::private(path("/a")),
            VisibilityRule::public(path("/a-b")),
            VisibilityRule::public(path("/ab/child")),
        ])
        .unwrap();
    let before = policy.rules().to_vec();
    assert!(matches!(
        policy.add_rule(VisibilityRule::public(path("/a/child"))),
        Err(PolicyError::PublicIsland { .. })
    ));
    assert_eq!(policy.rules(), before);
}

#[test]
fn replacing_a_private_rule_with_public_keeps_existing_children_valid() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::private(path("/docs")))
        .unwrap();
    policy
        .add_rules([
            VisibilityRule::public(path("/docs")),
            VisibilityRule::public(path("/docs/guide.md")),
        ])
        .unwrap();
    assert_eq!(
        policy.effective_visibility(&path("/docs/guide.md")),
        Visibility::Public
    );
}
