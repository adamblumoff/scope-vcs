use super::{
    local::{maybe_request_branch_base_audience, store_request_metadata_fields},
    text::terminal_text,
};
use crate::{git_repo::GitRepo, test_support::TestDir};
use scope_core::domain::requests::RequestBaseAudience;

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn request_metadata_stores_the_request_base_audience() {
    let dir = TestDir::git_repo("request-audience", "request");
    let git_repo = GitRepo {
        root: dir.path.clone(),
    };
    store_request_metadata_fields(
        &git_repo,
        "request",
        "req_1",
        "refs/scope/requests/req_1",
        RequestBaseAudience::Public,
    )
    .unwrap();

    assert_eq!(
        maybe_request_branch_base_audience(&git_repo).unwrap(),
        Some(RequestBaseAudience::Public)
    );
}
