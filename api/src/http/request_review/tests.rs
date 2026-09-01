use super::{
    MAX_IMPORTED_REQUEST_REVISIONS, MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES,
    RequestRevisionListWorkBudget, request_revision_commits,
};
use scope_domain::{
    account::UserAccount,
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    policy::{ScopePath, Visibility, VisibilityRule},
    repository::{Repository, access::RepositoryAccess},
    requests::RequestRevision,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn revision_listing_budget_caps_snapshot_count_bytes_and_commit_work() {
    let mut count_budget = RequestRevisionListWorkBudget::new();
    for _ in 0..MAX_IMPORTED_REQUEST_REVISIONS {
        assert!(count_budget.claim_revision(1).is_some());
        count_budget.record_inspected(1, 0);
    }
    assert_eq!(count_budget.claim_revision(1), None);

    let mut byte_budget = RequestRevisionListWorkBudget::new();
    assert!(
        byte_budget
            .claim_revision(MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES)
            .is_some()
    );
    assert_eq!(byte_budget.claim_revision(1), None);

    let mut commit_budget = RequestRevisionListWorkBudget::new();
    commit_budget.record_inspected(usize::MAX, 0);
    assert_eq!(commit_budget.claim_revision(1), None);

    let mut file_budget = RequestRevisionListWorkBudget::new();
    file_budget.record_inspected(0, usize::MAX);
    assert!(file_budget.claim_revision(1).is_some());
}

#[test]
fn revision_response_keeps_oversized_commit_identity_and_prioritizes_selection() {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"], None);
    let empty_tree = git(directory.path(), &["mktree"], Some(""));
    let base = git(
        directory.path(),
        &["commit-tree", &empty_tree, "-m", "base"],
        None,
    );
    let blob = git(
        directory.path(),
        &["hash-object", "-w", "--stdin"],
        Some("content\n"),
    );
    let mut tree_entries = String::new();
    for index in 0..10_001 {
        tree_entries.push_str(&format!("100644 blob {blob}\tfile-{index:05}.txt\n"));
    }
    let oversized_tree = git(directory.path(), &["mktree"], Some(&tree_entries));
    let oversized = git(
        directory.path(),
        &[
            "commit-tree",
            &oversized_tree,
            "-p",
            &base,
            "-m",
            "oversized",
        ],
        None,
    );
    tree_entries.push_str(&format!("100644 blob {blob}\tlast.txt\n"));
    let last_tree = git(directory.path(), &["mktree"], Some(&tree_entries));
    let last = git(
        directory.path(),
        &["commit-tree", &last_tree, "-p", &oversized, "-m", "last"],
        None,
    );
    let revision = RequestRevision {
        id: "revision-1".to_string(),
        request_id: "request-1".to_string(),
        position: 1,
        actor_user_id: "owner-1".to_string(),
        old_head_oid: base,
        new_head_oid: last.clone(),
        git_snapshot: SourceBlob {
            content_ref: ContentRef::blob_sha256("snapshot"),
            sha256: "snapshot".to_string(),
            git_oid: "snapshot".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
        created_at_unix: 1,
    };
    let owner = UserAccount {
        id: "owner-1".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let repo = Repository::new(&owner, "repo", Visibility::Public, "repoi_test").unwrap();
    let access = repo.access_for_user_id(&owner.id);

    let default = request_revision_commits(
        directory.path(),
        &repo,
        access,
        &revision,
        None,
        100,
        10_000,
    )
    .unwrap();
    assert_eq!(default.visible.len(), 2);
    assert_eq!(default.files_listed, 10_000);
    assert_eq!(default.visible[0].oid, oversized);
    assert_eq!(default.visible[0].change_count, 10_001);
    assert_eq!(default.visible[0].files.len(), 9_999);
    assert!(default.visible[0].files_truncated);
    assert_eq!(default.visible[1].oid, last);
    assert_eq!(default.visible[1].files.len(), 1);
    assert!(!default.visible[1].files_truncated);

    let selected = request_revision_commits(
        directory.path(),
        &repo,
        access,
        &revision,
        Some(&oversized),
        100,
        10_000,
    )
    .unwrap();
    assert_eq!(selected.visible.len(), 2);
    assert_eq!(selected.files_listed, 10_000);
    assert_eq!(selected.visible[0].oid, oversized);
    assert_eq!(selected.visible[0].files.len(), 10_000);
    assert!(selected.visible[0].files_truncated);
    assert_eq!(selected.visible[1].oid, last);
    assert!(selected.visible[1].files.is_empty());
    assert!(selected.visible[1].files_truncated);
}

#[test]
fn revision_response_keeps_changed_and_empty_identities_without_a_file_budget() {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"], None);
    let empty_tree = git(directory.path(), &["mktree"], Some(""));
    let base = git(
        directory.path(),
        &["commit-tree", &empty_tree, "-m", "base"],
        None,
    );
    let blob = git(
        directory.path(),
        &["hash-object", "-w", "--stdin"],
        Some("content\n"),
    );
    let changed_tree = git(
        directory.path(),
        &["mktree"],
        Some(&format!("100644 blob {blob}\tfile.txt\n")),
    );
    let changed = git(
        directory.path(),
        &["commit-tree", &changed_tree, "-p", &base, "-m", "changed"],
        None,
    );
    let empty = git(
        directory.path(),
        &["commit-tree", &changed_tree, "-p", &changed, "-m", "empty"],
        None,
    );
    let revision = RequestRevision {
        id: "revision-1".to_string(),
        request_id: "request-1".to_string(),
        position: 1,
        actor_user_id: "owner-1".to_string(),
        old_head_oid: base,
        new_head_oid: empty.clone(),
        git_snapshot: SourceBlob {
            content_ref: ContentRef::blob_sha256("snapshot"),
            sha256: "snapshot".to_string(),
            git_oid: "snapshot".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
        created_at_unix: 1,
    };
    let owner = UserAccount {
        id: "owner-1".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", Visibility::Public, "repoi_test").unwrap();

    let response = request_revision_commits(
        directory.path(),
        &repo,
        repo.access_for_user_id(&owner.id),
        &revision,
        None,
        100,
        0,
    )
    .unwrap();
    assert_eq!(response.visible.len(), 2);
    assert_eq!(response.visible[0].oid, changed);
    assert_eq!(response.visible[0].change_count, 1);
    assert!(response.visible[0].files_truncated);
    assert_eq!(response.visible[1].oid, empty);
    assert_eq!(response.visible[1].change_count, 0);
    assert!(!response.visible[1].files_truncated);

    repo.policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/file.txt").unwrap(),
        ))
        .unwrap();
    let public_response = request_revision_commits(
        directory.path(),
        &repo,
        RepositoryAccess::public(),
        &revision,
        None,
        100,
        0,
    )
    .unwrap();
    assert_eq!(public_response.visible.len(), 1);
    assert_eq!(public_response.visible[0].oid, empty);
}

fn git(repo: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Scope Test")
        .env("GIT_AUTHOR_EMAIL", "scope@example.test")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_NAME", "Scope Test")
        .env("GIT_COMMITTER_EMAIL", "scope@example.test")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
