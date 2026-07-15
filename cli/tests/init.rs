mod support;

use std::fs;
use support::*;

#[test]
fn init_warns_on_dirty_working_tree_and_continues_to_auth() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("README.md"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init with dirty working tree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(stderr.contains("start browser login"), "{stderr}");
}
