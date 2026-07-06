use scope_cli::{
    clone::{RepoSpec, default_clone_dir, parse_repo_spec},
    git_credentials::{credential_home_dir, git_clone_plan},
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
fn git_clone_plan_stores_scope_credentials_by_host_and_path() {
    let home = PathBuf::from("/home/adam");
    let plan = git_clone_plan(
        "https://old-user@scope.example/git/adam/scope-vcs",
        "scope_git_secret",
        Some(Path::new("local-dir")),
        &home,
    )
    .unwrap();

    assert_eq!(
        plan.credential_remote_url,
        "https://scope@scope.example/git/adam/scope-vcs"
    );
    assert_eq!(
        plan.credential_fields,
        vec![
            "protocol=https",
            "host=scope.example",
            "path=git/adam/scope-vcs",
            "username=scope",
            "password=scope_git_secret",
            "",
        ]
    );
    assert_eq!(
        plan.credential_store_path,
        PathBuf::from("/home/adam/.config/scope/git-credentials")
    );
    assert_eq!(
        plan.helper_config_key,
        "credential.https://scope.example.helper"
    );
    assert_eq!(
        plan.helper_config_value,
        "store --file \"/home/adam/.config/scope/git-credentials\""
    );
    assert_eq!(
        plan.use_http_path_config_key,
        "credential.https://scope.example.useHttpPath"
    );
    assert_eq!(
        plan.clone_args,
        vec![
            "clone",
            "-c",
            "http.proactiveAuth=basic",
            "https://scope@scope.example/git/adam/scope-vcs",
            "local-dir",
        ]
    );
}

#[test]
fn git_clone_plan_preserves_localhost_ports() {
    let plan = git_clone_plan(
        "http://localhost:8080/git/local/scope-vcs",
        "scope_git_secret",
        None,
        Path::new("C:/Users/Adam"),
    )
    .unwrap();

    assert_eq!(
        plan.helper_config_key,
        "credential.http://localhost:8080.helper"
    );
    assert_eq!(
        plan.use_http_path_config_key,
        "credential.http://localhost:8080.useHttpPath"
    );
    assert_eq!(
        plan.helper_config_value,
        "store --file \"C:/Users/Adam/.config/scope/git-credentials\""
    );
    assert_eq!(plan.credential_fields[1], "host=localhost:8080");
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

#[test]
fn git_clone_plan_quotes_space_containing_credential_store_paths() {
    let plan = git_clone_plan(
        "https://scope.example/git/adam/scope-vcs",
        "scope_git_secret",
        None,
        Path::new("C:/Users/Adam Smith"),
    )
    .unwrap();

    assert_eq!(
        plan.helper_config_value,
        "store --file \"C:/Users/Adam Smith/.config/scope/git-credentials\""
    );
}

#[test]
#[cfg(not(windows))]
fn credential_home_dir_prefers_home_on_non_windows() {
    assert_eq!(
        credential_home_dir(
            Some(OsString::from("/home/scope")),
            Some(OsString::from("C:/Users/Scope")),
        ),
        Some(PathBuf::from("/home/scope"))
    );
}

#[test]
#[cfg(windows)]
fn credential_home_dir_prefers_userprofile_on_windows() {
    assert_eq!(
        credential_home_dir(
            Some(OsString::from("C:/msys/home/scope")),
            Some(OsString::from("C:/Users/Scope")),
        ),
        Some(PathBuf::from("C:/Users/Scope"))
    );
}
