pub const ACCOUNT_SESSION: &str = "/v1/session";
pub const HEALTH: &str = "/healthz";
pub const READINESS: &str = "/readyz";
pub const ADMIN_CLEANUP: &str = "/v1/admin/cleanup";
pub const ADMIN_CLEANUP_DRAIN: &str = "/v1/admin/cleanup/drain";
pub const ADMIN_RUNNER_CUTOVER: &str = "/v1/admin/runner-cutover";
pub const ADMIN_RUNNER_CUTOVER_ADVANCE: &str = "/v1/admin/runner-cutover/advance";
pub const ADMIN_RUNNER_CUTOVER_CANARY: &str = "/v1/admin/runner-cutover/canary";
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
pub const RUNNERS: &str = "/v1/runners";
pub const RUNNER: &str = "/v1/runners/{runner_id}";
pub const RUNNER_UPGRADE: &str = "/v1/runners/{runner_id}/upgrade";
pub const RUNNER_REPOSITORY: &str = "/v1/runners/{runner_id}/repos/{owner}/{repo}";
pub const RUNNER_POLL: &str = "/v1/runner-protocol/poll";
pub const RUNNER_CLAIM: &str = "/v1/runner-protocol/runs/{run_id}/claim";
pub const ATTEMPT_HEARTBEAT: &str = "/v1/runner-protocol/attempts/{attempt_id}/heartbeat";
pub const ATTEMPT_CACHE_FINALIZATION: &str =
    "/v1/runner-protocol/attempts/{attempt_id}/cache-finalization";
pub const ATTEMPT_RECOVERY_STATUS: &str =
    "/v1/runner-protocol/attempts/{attempt_id}/recovery-status";
pub const ATTEMPT_CONTAINER_IMAGE: &str =
    "/v1/runner-protocol/attempts/{attempt_id}/container-image";
pub const ATTEMPT_SOURCE: &str = "/v1/runner-protocol/attempts/{attempt_id}/source";
pub const ATTEMPT_LOGS: &str = "/v1/runner-protocol/attempts/{attempt_id}/logs";
pub const ATTEMPT_COMPLETE: &str = "/v1/runner-protocol/attempts/{attempt_id}/complete";
pub const ATTEMPT_ABANDON: &str = "/v1/runner-protocol/attempts/{attempt_id}/abandon";
pub const ATTEMPT_STEP_START: &str =
    "/v1/runner-protocol/attempts/{attempt_id}/steps/{step_index}/start";
pub const ATTEMPT_STEP_COMPLETE: &str =
    "/v1/runner-protocol/attempts/{attempt_id}/steps/{step_index}/complete";
pub const REPO: &str = "/v1/repos/{owner}/{repo}";
pub const REPO_OPERATIONS: &str = "/v1/repos/{owner}/{repo}/operations";
pub const REPO_RUNS: &str = "/v1/repos/{owner}/{repo}/runs";
pub const REPO_RUN: &str = "/v1/repos/{owner}/{repo}/runs/{run_id}";
pub const REPO_RUN_DETAIL: &str = "/v1/repos/{owner}/{repo}/runs/{run_id}/detail";
pub const REPO_RUN_STEP_LOGS: &str =
    "/v1/repos/{owner}/{repo}/runs/{run_id}/attempts/{attempt_id}/steps/{step_index}/logs";
pub const REPO_RUN_CANCEL: &str = "/v1/repos/{owner}/{repo}/runs/{run_id}/cancel";
pub const REPO_RUN_RETRY: &str = "/v1/repos/{owner}/{repo}/runs/{run_id}/retry";
pub const REPO_RUN_EVENTS: &str = "/v1/repos/{owner}/{repo}/runs/{run_id}/events";
pub const REPO_PUSH_TRIGGER_EVALUATION: &str =
    "/v1/repos/{owner}/{repo}/push-trigger-evaluations/{head_oid}";
pub const REPO_CONFIG: &str = "/v1/repos/{owner}/{repo}/config";
pub const REPO_PUSH_INTENTS: &str = "/v1/repos/{owner}/{repo}/push-intents";
pub const REPO_REQUESTS: &str = "/v1/repos/{owner}/{repo}/requests";
pub const REPO_REQUEST_QUEUE: &str = "/v1/repos/{owner}/{repo}/requests/queue";
pub const REPO_REQUEST: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}";
pub const REPO_REQUEST_SUBMIT: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/submit";
pub const REPO_REQUEST_MERGE: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/merge";
pub const REPO_REQUEST_RATINGS: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/ratings";
pub const REPO_REQUEST_INVITEES: &str = "/v1/repos/{owner}/{repo}/requests/{request_id}/invitees";
pub const REPO_REQUEST_INVITEES_ME: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/invitees/me";
pub const REPO_SESSION: &str = "/v1/repos/{owner}/{repo}/session";
pub const REPO_FILES: &str = "/v1/repos/{owner}/{repo}/files";
pub const REPO_FILE_CONTENT: &str = "/v1/repos/{owner}/{repo}/files/content";
pub const REPO_REQUEST_CHANGE_BLOCK_FILES: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{block_id}";
pub const REPO_REQUEST_CHANGE_BLOCK_FILE_DIFF: &str =
    "/v1/repos/{owner}/{repo}/requests/{request_id}/changes/{block_id}/file-diff";
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
pub const DEV_CLI_SESSION: &str = "/v1/dev/cli-session/{handle}";

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

pub fn runner(runner_id: &str) -> String {
    format!("/v1/runners/{}", path_segment(runner_id))
}

pub fn runner_upgrade(runner_id: &str) -> String {
    format!("{}/upgrade", runner(runner_id))
}

pub fn runner_repository(runner_id: &str, owner: &str, repo: &str) -> String {
    format!(
        "{}/repos/{}/{}",
        runner(runner_id),
        path_segment(owner),
        path_segment(repo)
    )
}

pub fn runner_claim(run_id: &str) -> String {
    format!("/v1/runner-protocol/runs/{}/claim", path_segment(run_id))
}

pub fn attempt_heartbeat(attempt_id: &str) -> String {
    attempt_action(attempt_id, "heartbeat")
}

pub fn attempt_cache_finalization(attempt_id: &str) -> String {
    attempt_action(attempt_id, "cache-finalization")
}

pub fn attempt_recovery_status(attempt_id: &str) -> String {
    attempt_action(attempt_id, "recovery-status")
}

pub fn attempt_container_image(attempt_id: &str) -> String {
    attempt_action(attempt_id, "container-image")
}

pub fn attempt_source(attempt_id: &str) -> String {
    attempt_action(attempt_id, "source")
}

pub fn attempt_logs(attempt_id: &str) -> String {
    attempt_action(attempt_id, "logs")
}

pub fn attempt_complete(attempt_id: &str) -> String {
    attempt_action(attempt_id, "complete")
}

pub fn attempt_abandon(attempt_id: &str) -> String {
    attempt_action(attempt_id, "abandon")
}

pub fn attempt_step_start(attempt_id: &str, step_index: u32) -> String {
    attempt_step_action(attempt_id, step_index, "start")
}

pub fn attempt_step_complete(attempt_id: &str, step_index: u32) -> String {
    attempt_step_action(attempt_id, step_index, "complete")
}

fn attempt_action(attempt_id: &str, action: &str) -> String {
    format!(
        "/v1/runner-protocol/attempts/{}/{}",
        path_segment(attempt_id),
        action
    )
}

fn attempt_step_action(attempt_id: &str, step_index: u32, action: &str) -> String {
    format!(
        "/v1/runner-protocol/attempts/{}/steps/{step_index}/{action}",
        path_segment(attempt_id),
    )
}

pub fn repo_runs(owner: &str, repo: &str) -> String {
    format!("{}/runs", self::repo(owner, repo))
}

pub fn repo_operations(owner: &str, repo: &str) -> String {
    format!("{}/operations", self::repo(owner, repo))
}

pub fn repo_run(owner: &str, repo: &str, run_id: &str) -> String {
    format!("{}/{}", repo_runs(owner, repo), path_segment(run_id))
}

pub fn repo_run_detail(owner: &str, repo: &str, run_id: &str) -> String {
    format!("{}/detail", repo_run(owner, repo, run_id))
}

pub fn repo_run_step_logs(
    owner: &str,
    repo: &str,
    run_id: &str,
    attempt_id: &str,
    step_index: u32,
) -> String {
    format!(
        "{}/attempts/{}/steps/{step_index}/logs",
        repo_run(owner, repo, run_id),
        path_segment(attempt_id),
    )
}

pub fn repo_run_cancel(owner: &str, repo: &str, run_id: &str) -> String {
    format!("{}/cancel", repo_run(owner, repo, run_id))
}

pub fn repo_run_retry(owner: &str, repo: &str, run_id: &str) -> String {
    format!("{}/retry", repo_run(owner, repo, run_id))
}

pub fn repo_run_events(owner: &str, repo: &str, run_id: &str) -> String {
    format!("{}/events", repo_run(owner, repo, run_id))
}

pub fn repo_push_trigger_evaluation(owner: &str, repo: &str, head_oid: &str) -> String {
    format!(
        "{}/push-trigger-evaluations/{}",
        self::repo(owner, repo),
        path_segment(head_oid)
    )
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

pub fn repo_request_submit(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/submit", repo_request(owner, repo, request_id))
}

pub fn repo_request_merge(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/merge", repo_request(owner, repo, request_id))
}

pub fn repo_request_ratings(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/ratings", repo_request(owner, repo, request_id))
}

pub fn repo_request_invitees(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/invitees", repo_request(owner, repo, request_id))
}

pub fn repo_request_invitees_me(owner: &str, repo: &str, request_id: &str) -> String {
    format!("{}/invitees/me", repo_request(owner, repo, request_id))
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
                runner_upgrade("runner/with space"),
                "/v1/runners/runner%2Fwith%20space/upgrade",
            ),
            (
                attempt_cache_finalization("attempt/with space"),
                "/v1/runner-protocol/attempts/attempt%2Fwith%20space/cache-finalization",
            ),
            (
                repo_request("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231",
            ),
            (
                repo_request_submit("an owner", "r/name", "request?#1"),
                "/v1/repos/an%20owner/r%2Fname/requests/request%3F%231/submit",
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
                repo_run_step_logs("an owner", "r/name", "run?#1", "attempt/#1", 3),
                "/v1/repos/an%20owner/r%2Fname/runs/run%3F%231/attempts/attempt%2F%231/steps/3/logs",
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
