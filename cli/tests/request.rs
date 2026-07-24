mod support;

use support::*;

#[test]
fn request_start_requires_a_name_before_login() {
    let dir = TempDir::new("start-name");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "start"],
        "the following required arguments were not provided",
    );
}

#[test]
fn obsolete_request_transport_commands_are_removed() {
    let dir = TempDir::new("removed-request-transport");
    create_repo_with_head(dir.path());

    for command in ["delete", "join", "pull", "share", "sync-main"] {
        scope_failure(
            dir.path(),
            ["request", command],
            &format!("unrecognized subcommand '{command}'"),
        );
    }
}

#[test]
fn request_comment_is_replaced_by_discuss() {
    let dir = TempDir::new("removed-request-comment");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "comment"],
        "unrecognized subcommand 'comment'",
    );
}

#[test]
fn request_discuss_requires_a_body_before_login() {
    let dir = TempDir::new("discuss-body");
    create_repo_with_head(dir.path());

    scope_failure(
        dir.path(),
        ["request", "discuss"],
        "the following required arguments were not provided",
    );
}
