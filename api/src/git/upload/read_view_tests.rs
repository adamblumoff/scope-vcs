use super::*;
use crate::git::cache::RepositoryGitCache;
use scope_domain::requests::{RequestActorRole, RequestAudience};

fn base_repo(cache: &std::sync::Arc<RepositoryGitCache>, name: &str) -> GitRepoHandle {
    let path = cache.root().join(format!("{name}.git"));
    run_git(
        None,
        &["init", "--bare", path.to_str().unwrap()],
        "init base",
    )
    .unwrap();
    run_git(
        Some(&path),
        &["symbolic-ref", "HEAD", "refs/heads/main"],
        "set head",
    )
    .unwrap();
    let tree = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&path)
            .arg("mktree"),
        Some(b""),
    )
    .unwrap();
    let oid = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .arg("commit-tree")
            .arg(String::from_utf8(tree).unwrap().trim())
            .arg("-m")
            .arg(name),
        None,
    )
    .unwrap();
    run_git(
        Some(&path),
        &[
            "update-ref",
            "refs/heads/main",
            String::from_utf8(oid).unwrap().trim(),
        ],
        "set base",
    )
    .unwrap();
    cache.lease_derived(path).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_base_head_change_rebuilds_read_view_and_attaches_newly_available_ref() {
    let state = AppState::test_state();
    let source_dir = tempfile::tempdir().unwrap();
    let cache = RepositoryGitCache::new(source_dir.path().to_path_buf(), usize::MAX).unwrap();
    let primary = base_repo(&cache, "primary");
    let public_first = base_repo(&cache, "public-first");
    let public_second = base_repo(&cache, "public-second");
    let second_path = public_second.as_ref().to_path_buf();
    let head = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&second_path)
            .arg("rev-parse")
            .arg("refs/heads/main"),
        None,
    )
    .unwrap();
    let head = String::from_utf8(head).unwrap().trim().to_string();
    let requests = vec![Request {
        id: "request".into(),
        repo_id: "repo".into(),
        name: "topic".into(),
        author_user_id: "author".into(),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: head.clone(),
        head_oid: head.clone(),
        git_snapshot: None,
        title: "Topic".into(),
        description_markdown: String::new(),
        activity_version: 0,
        submitted_at_unix: Some(1),
        closed_at_unix: None,
        closed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: 1,
        updated_at_unix: 1,
    }];
    let incarnation = RepositoryIncarnation::new("repo", "incarnation").unwrap();
    let primary_path = primary.as_ref().to_path_buf();
    let first = git_read_view_repo(
        &state,
        &incarnation,
        primary,
        Some(public_first),
        &requests,
        &[],
    )
    .await
    .unwrap();
    let missing = git_process_output_with_timeout(
        Command::new("git")
            .arg("--git-dir")
            .arg(first.as_ref())
            .arg("rev-parse")
            .arg("--verify")
            .arg("refs/heads/topic"),
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(!missing.status.success());
    let second = git_read_view_repo(
        &state,
        &incarnation,
        cache.lease_derived(primary_path).unwrap(),
        Some(public_second),
        &requests,
        &[],
    )
    .await
    .unwrap();
    assert_ne!(first.as_ref(), second.as_ref());
    let attached = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(second.as_ref())
            .arg("rev-parse")
            .arg("refs/heads/topic"),
        None,
    )
    .unwrap();
    assert_eq!(String::from_utf8(attached).unwrap().trim(), head);
}
