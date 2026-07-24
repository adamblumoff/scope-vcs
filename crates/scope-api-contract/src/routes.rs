pub const ACCOUNT_SESSION: &str = "/v1/session";
pub const HEALTH: &str = "/healthz";
pub const READINESS: &str = "/readyz";
pub const ADMIN_CLEANUP: &str = "/v1/admin/cleanup";
pub const ADMIN_CLEANUP_DRAIN: &str = "/v1/admin/cleanup/drain";
pub const ADMIN_METADATA_RESET: &str = "/v1/admin/metadata/reset";
pub const CLI_BROWSER_LOGIN: &str = "/v1/cli/browser-login";
pub const CLI_BROWSER_LOGIN_COMPLETE: &str = "/v1/cli/browser-login/{request_id}/complete";
pub const CLI_BROWSER_LOGIN_EXCHANGE: &str = "/v1/cli/browser-login/{request_id}/exchange";
pub const CLI_DEVICE_LOGIN: &str = "/v1/cli/device-login";
pub const CLI_DEVICE_LOGIN_COMPLETE: &str = "/v1/cli/device-login/{user_code}/complete";
pub const CLI_DEVICE_LOGIN_POLL: &str = "/v1/cli/device-login/{device_code}/poll";
pub const CLI_EXCHANGE_GRANTS: &str = "/v1/cli/exchange-grants";
pub const CLI_EXCHANGE_GRANTS_EXCHANGE: &str = "/v1/cli/exchange-grants/exchange";
pub const CLI_SESSION: &str = "/v1/cli/session";
pub const CLI_SESSIONS: &str = "/v1/cli/sessions";
pub const CLI_SESSION_BY_ID: &str = "/v1/cli/sessions/{session_id}";
pub const REPOS: &str = "/v1/repos";
pub const REPO: &str = "/v1/repos/{owner}/{repo}";
pub const REPO_CONFIG: &str = "/v1/repos/{owner}/{repo}/config";
pub const REPO_PUSH_INTENTS: &str = "/v1/repos/{owner}/{repo}/push-intents";
pub const REPO_REQUESTS: &str = "/v1/repos/{owner}/{repo}/requests";
pub const REPO_REQUEST: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}";
pub const REPO_REQUEST_READY: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/ready";
pub const REPO_REQUEST_WORKING: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/working";
pub const REPO_REQUEST_HOLD: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/hold";
pub const REPO_REQUEST_REQUEST_CHANGES: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/request-changes";
pub const REPO_REQUEST_ASSESSMENT: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/assessment";
pub const REPO_REQUEST_MERGE: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/merge";
pub const REPO_SESSION: &str = "/v1/repos/{owner}/{repo}/session";
pub const REPO_FILES: &str = "/v1/repos/{owner}/{repo}/files";
pub const REPO_FILE_CONTENT: &str = "/v1/repos/{owner}/{repo}/files/content";
pub const REPO_REQUEST_CHANGE_BLOCK_FILES: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{block_id}";
pub const REPO_REQUEST_CHANGE_BLOCK_FILE_DIFF: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{block_id}/file-diff";
pub const REPO_REQUEST_DESCRIPTION: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/description";
pub const REPO_REQUEST_DISCUSSIONS: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/timeline";
pub const REPO_REQUEST_DISCUSSION_CHANGES: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/timeline/changes";
pub const REPO_REQUEST_DISCUSSION_REPLIES: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/replies";
pub const REPO_REQUEST_DISCUSSION_RESOLVE: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/resolve";
pub const REPO_REQUEST_DISCUSSION_REOPEN: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/reopen";
pub const REPO_REQUEST_DISCUSSION_REOPEN_AND_REPLY: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/reopen-and-reply";
pub const REPO_REQUEST_DISCUSSION_READ: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/threads/{discussion_id}/read";
pub const REPO_REQUEST_ACTIVITY: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/activity";
pub const REPO_EVENTS: &str = "/v1/repos/{owner}/{repo}/events";
pub const REPO_COMMITS: &str = "/v1/repos/{owner}/{repo}/commits";
pub const REPO_COMMIT: &str = "/v1/repos/{owner}/{repo}/commits/{commit_id}";
pub const REPO_COMMIT_FILE_DIFF: &str = "/v1/repos/{owner}/{repo}/commits/{commit_id}/file-diff";
pub const REPO_MEMBERS: &str = "/v1/repos/{owner}/{repo}/members";
pub const REPO_INVITES: &str = "/v1/repos/{owner}/{repo}/invites";
pub const REPO_INVITE: &str = "/v1/repos/{owner}/{repo}/invites/{invite_id}";
pub const REPO_MEMBER: &str = "/v1/repos/{owner}/{repo}/members/{member_user_id}";
pub const REPOSITORY_INVITE: &str = "/v1/repository-invites/{token}";
pub const REPOSITORY_INVITE_ACCEPT: &str = "/v1/repository-invites/{token}/accept";
pub const REPO_PROJECTION_PREVIEW: &str = "/v1/repos/{owner}/{repo}/projection-preview";
pub const GIT_REPO: &str = "/git/{mode}/{org}/{repo}";
pub const GIT_INFO_REFS: &str = "/git/{mode}/{org}/{repo}/info/refs";
pub const GIT_RECEIVE_PACK: &str = "/git/{mode}/{org}/{repo}/git-receive-pack";
pub const GIT_UPLOAD_PACK: &str = "/git/{mode}/{org}/{repo}/git-upload-pack";
pub const DEV_BENCH_CLI_SESSION: &str = "/v1/dev/bench/cli-session";

pub fn cli_browser_login_exchange(request_id: &str) -> String {
    format!(
        "/v1/cli/browser-login/{}/exchange",
        path_segment(request_id)
    )
}

pub fn cli_device_login_poll(device_code: &str) -> String {
    format!("/v1/cli/device-login/{}/poll", path_segment(device_code))
}

pub fn repo(owner: &str, repo: &str) -> String {
    format!("/v1/repos/{}/{}", path_segment(owner), path_segment(repo))
}

pub fn repo_config(owner: &str, repo: &str) -> String {
    format!("{}/config", self::repo(owner, repo))
}

pub fn repo_push_intents(owner: &str, repo: &str) -> String {
    format!("{}/push-intents", self::repo(owner, repo))
}

pub fn repo_requests(owner: &str, repo: &str) -> String {
    format!("{}/requests", self::repo(owner, repo))
}

pub fn repo_request(owner: &str, repo: &str, request_id: &str) -> String {
    format!(
        "{}/{}",
        repo_requests(owner, repo),
        path_segment(request_id)
    )
}

pub fn repo_request_ready(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/ready", repo_request(owner, repo, request_id))
}

pub fn repo_request_working(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/working", repo_request(owner, repo, request_id))
}

pub fn repo_request_hold(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/hold", repo_request(owner, repo, request_id))
}

pub fn repo_request_request_changes(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/request-changes", repo_request(owner, repo, request_id))
}

pub fn repo_request_assessment(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/assessment", repo_request(owner, repo, request_id))
}

pub fn repo_request_merge(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/merge", repo_request(owner, repo, request_id))
}

pub fn repo_request_action(owner: &str, repo: &str, request_id: &str, action: &str) -> String {
    format!(
        "{}/{}",
        repo_request(owner, repo, request_id),
        path_segment(action)
    )
}

pub fn git_repo(mode: &str, owner: &str, repo: &str) -> String {
    format!(
        "/git/{}/{}/{}",
        path_segment(mode),
        path_segment(owner),
        path_segment(repo)
    )
}

pub fn path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_routes_encode_each_path_segment() {
        let routes = [
            (
                repo_request("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231",
            ),
            (
                repo_request_ready("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/ready",
            ),
            (
                repo_request_working("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/working",
            ),
            (
                repo_request_hold("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/hold",
            ),
            (
                repo_request_request_changes("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/request-changes",
            ),
            (
                repo_request_assessment("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/assessment",
            ),
            (
                repo_request_merge("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/merge",
            ),
            (
                cli_device_login_poll("code/with space"),
                "/v1/cli/device-login/code%2Fwith%20space/poll",
            ),
            (
                git_repo("permissioned", "an owner", "r/name"),
                "/git/permissioned/an%20owner/r%2Fname",
            ),
        ];
        for (actual, expected) in routes {
            assert_eq!(actual, expected);
        }
    }
}
