use super::*;

#[test]
fn duplicate_repo_error_names_repo_and_next_steps() {
    let message = duplicate_repo_error_message("scope-vcs");

    assert!(message.contains("Scope repository \"scope-vcs\" already exists"));
    assert!(message.contains("scope init --name <new-name>"));
    assert!(message.contains("scope push"));
}
