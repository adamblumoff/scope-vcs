use super::{
    local::{maybe_request_branch_audience, store_request_metadata_fields},
    new_client_discussion_id,
    text::terminal_text,
};
use crate::{git_repo::GitRepo, test_support::TestDir};
use scope_core::domain::requests::RequestAudience;

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn request_metadata_stores_the_request_audience() {
    let dir = TestDir::git_repo("request-audience", "request");
    let git_repo = GitRepo {
        root: dir.path.clone(),
    };
    store_request_metadata_fields(&git_repo, "request", "req_1", RequestAudience::Public).unwrap();

    assert_eq!(
        maybe_request_branch_audience(&git_repo).unwrap(),
        Some(RequestAudience::Public)
    );
}

#[test]
fn client_discussion_ids_are_opaque_and_unique() {
    let first = new_client_discussion_id().unwrap();
    let second = new_client_discussion_id().unwrap();

    assert!(first.starts_with("client_discussion_"));
    assert!(second.starts_with("client_discussion_"));
    assert_ne!(first, second);
}
