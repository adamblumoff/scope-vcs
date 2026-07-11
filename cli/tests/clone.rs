use scope_cli::clone::{RepoSpec, parse_repo_spec};

#[test]
fn parse_repo_spec_accepts_owner_and_repo() {
    assert_eq!(
        parse_repo_spec(" adam/scope-vcs ").unwrap(),
        RepoSpec {
            owner: "adam".to_string(),
            repo: "scope-vcs".to_string(),
        }
    );
}

#[test]
fn parse_repo_spec_rejects_urls_and_partial_specs() {
    for repository in [
        "",
        "adam",
        "adam/",
        "/scope-vcs",
        "adam/scope-vcs/extra",
        "https://scopevcs.com/git/adam/scope-vcs",
    ] {
        assert!(parse_repo_spec(repository).is_err(), "{repository}");
    }
}
