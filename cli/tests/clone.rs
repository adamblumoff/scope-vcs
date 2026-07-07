use scope_cli::clone::{
    RepoSpec, default_clone_dir, parse_repo_spec, permissioned_git_remote_path,
};
use std::path::PathBuf;

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

#[test]
fn permissioned_git_remote_path_uses_explicit_git_mode() {
    assert_eq!(
        permissioned_git_remote_path("adam", "scope-vcs"),
        "/git/permissioned/adam/scope-vcs"
    );
}

#[test]
fn default_clone_dir_strips_dot_git_suffix_like_git_clone() {
    assert_eq!(default_clone_dir("scope-vcs"), PathBuf::from("scope-vcs"));
    assert_eq!(
        default_clone_dir("scope-vcs.git"),
        PathBuf::from("scope-vcs")
    );
    assert_eq!(default_clone_dir(".git"), PathBuf::from(".git"));
}
