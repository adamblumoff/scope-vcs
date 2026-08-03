use super::{ensure_public_request_paths_allowed, new_client_discussion_id, text::terminal_text};
use crate::{error::CliError, git_repo::GitRepo, test_support::TestDir};
use std::fs;

#[test]
fn terminal_text_replaces_control_characters() {
    assert_eq!(terminal_text("ok\u{1b}[31m\nnext\u{7}"), "ok [31m next ");
}

#[test]
fn client_discussion_ids_are_opaque_and_unique() {
    let first = new_client_discussion_id().unwrap();
    let second = new_client_discussion_id().unwrap();

    assert!(first.starts_with("client_discussion_"));
    assert!(second.starts_with("client_discussion_"));
    assert_ne!(first, second);
}

#[test]
fn public_request_preflight_names_protected_committed_paths() {
    let dir = TestDir::git_repo("request-protected-path", "main");
    dir.run_git(["config", "user.email", "scope@example.test"]);
    dir.run_git(["config", "user.name", "Scope Test"]);
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    dir.run_git(["add", "README.md"]);
    dir.run_git(["commit", "-m", "base"]);
    let base_oid = git_oid(&dir);
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/RULES.md"), "protected\n").unwrap();
    dir.run_git(["add", ".scope/RULES.md"]);
    dir.run_git(["commit", "-m", "change rules"]);
    let head_oid = git_oid(&dir);
    let detail = request_detail(&base_oid, &head_oid);
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };

    let error =
        ensure_public_request_paths_allowed(&repo, &detail, &base_oid, &head_oid).unwrap_err();
    let structured = error.downcast_ref::<CliError>().unwrap();

    assert_eq!(
        structured.response().code,
        scope_api_contract::ErrorCode::ProtectedPath
    );
    assert_eq!(structured.response().fields.paths, [".scope/RULES.md"]);
    assert!(error.to_string().contains(".scope/RULES.md"));
    assert_eq!(crate::error::exit_code(&error), 4);
}

#[test]
fn public_request_preflight_excludes_protected_paths_in_current_main() {
    let dir = TestDir::git_repo("request-current-public-main", "main");
    dir.run_git(["config", "user.email", "scope@example.test"]);
    dir.run_git(["config", "user.name", "Scope Test"]);
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    dir.run_git(["add", "README.md"]);
    dir.run_git(["commit", "-m", "base"]);
    let original_base_oid = git_oid(&dir);
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/RULES.md"), "maintainer change\n").unwrap();
    dir.run_git(["add", ".scope/RULES.md"]);
    dir.run_git(["commit", "-m", "advance public main"]);
    let current_main_oid = git_oid(&dir);
    fs::write(dir.path().join("README.md"), "request change\n").unwrap();
    dir.run_git(["add", "README.md"]);
    dir.run_git(["commit", "-m", "request change"]);
    let head_oid = git_oid(&dir);
    let detail = request_detail(&original_base_oid, &head_oid);
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };

    ensure_public_request_paths_allowed(&repo, &detail, &current_main_oid, &head_oid).unwrap();
}

fn git_oid(dir: &TestDir) -> String {
    String::from_utf8(dir.run_git(["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn request_detail(base_oid: &str, head_oid: &str) -> crate::api::RequestDetailResponse {
    serde_json::from_value(serde_json::json!({
        "request": {
            "id":"req_one","name":"change","title":"Change",
            "description_markdown":"","author_user_id":"scope_usr_author",
            "author_role":"Public","audience":"Public",
            "base_main_oid":base_oid,"head_oid":head_oid,"state":"Draft",
            "activity_version":1,"submitted_at_unix":null,"closed_at_unix":null,
            "closed_by_user_id":null,"merged_at_unix":null,"merged_by_user_id":null,
            "merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,
            "updated_at_unix":2,"invitees":[],
            "permissions":{"can_view_activity":false,"can_open_discussion":false,
                "can_reply_to_discussion":false,"can_edit_identity":false,
                "can_pull_branch":false,"can_push_branch":true,"can_submit":false,
                "can_manage_invitees":false,"can_leave_request":false,
                "can_close":false,"can_merge":false},
            "mergeability":{"status":"Draft","current_main_oid":base_oid,
                "request_head_oid":head_oid,"reason":null}
        }
    }))
    .unwrap()
}
