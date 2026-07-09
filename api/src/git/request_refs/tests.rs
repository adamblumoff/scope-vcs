use super::*;

#[test]
fn request_ref_store_head_must_match_advertised_old_head() {
    ensure_request_ref_store_head_matches_push(None, None).unwrap();
    ensure_request_ref_store_head_matches_push(Some("a"), Some("a")).unwrap();

    let create_error = ensure_request_ref_store_head_matches_push(Some("a"), None).unwrap_err();
    assert!(create_error.message.contains("fetch and retry"));

    let update_error =
        ensure_request_ref_store_head_matches_push(Some("b"), Some("a")).unwrap_err();
    assert!(update_error.message.contains("fetch and retry"));

    let missing_error = ensure_request_ref_store_head_matches_push(None, Some("a")).unwrap_err();
    assert!(missing_error.message.contains("fetch and retry"));
}

#[test]
fn git_lock_with_current_owner_is_not_stale() {
    let path = temp_lock_path("current-owner");
    fs::write(
        &path,
        format!(
            "pid={}\ncreated_at_unix={}",
            std::process::id(),
            unix_now().unwrap()
        ),
    )
    .unwrap();

    assert!(!git_lock_is_stale(&path).unwrap());
    let _ = fs::remove_file(path);
}

#[test]
fn git_lock_with_old_timestamp_is_stale() {
    let path = temp_lock_path("old-timestamp");
    fs::write(
        &path,
        format!("pid={}\ncreated_at_unix=1", std::process::id()),
    )
    .unwrap();

    assert!(git_lock_is_stale(&path).unwrap());
    let _ = fs::remove_file(path);
}

#[test]
fn request_ref_head_must_be_commit_object_not_annotated_tag() {
    let repo = temp_repo_path("tag-object");
    run_git(
        None,
        &["init", repo.to_string_lossy().as_ref()],
        "init test repo",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
        "create test commit",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.com",
            "tag",
            "-a",
            "request-tag",
            "-m",
            "tag",
        ],
        "create annotated tag",
    )
    .unwrap();
    let head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let tag = git_stdout(&repo, &["rev-parse", "request-tag^{tag}"]);

    ensure_request_ref_oid_is_commit(&repo, head.trim()).unwrap();
    let error = ensure_request_ref_oid_is_commit(&repo, tag.trim()).unwrap_err();
    assert!(error.message.contains("must point at commits"));
    let _ = fs::remove_dir_all(repo);
}

fn git_stdout(repo: &FsPath, args: &[&str]) -> String {
    let output = run_git_output(Some(repo), args, "read test git stdout").unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

fn temp_lock_path(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-request-ref-{label}-{}-{}.lock",
        std::process::id(),
        unix_now().unwrap()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn temp_repo_path(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-request-ref-{label}-{}-{}",
        std::process::id(),
        unix_now().unwrap()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}
