use crate::{git, http, state::AppState};
use axum::{
    Router,
    http::{
        Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    middleware,
    routing::{delete, get, patch, post},
};
use http::routes;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

pub fn router(state: AppState) -> Router {
    let router = Router::new()
        .route(routes::HEALTH, get(http::account::healthz))
        .route(routes::READINESS, get(http::account::readyz))
        .route(routes::ADMIN_CLEANUP, get(http::admin::get_cleanup_status))
        .route(
            routes::ADMIN_CLEANUP_DRAIN,
            post(http::admin::drain_cleanup),
        )
        .route(
            routes::ADMIN_RUNNER_CUTOVER,
            get(http::admin::get_runner_protocol_cutover),
        )
        .route(
            routes::ADMIN_RUNNER_CUTOVER_ADVANCE,
            post(http::admin::advance_runner_protocol_cutover),
        )
        .route(
            routes::ADMIN_RUNNER_CUTOVER_CANARY,
            post(http::admin::create_runner_protocol_canary),
        )
        .route(
            routes::CLI_DEVICE_LOGIN,
            post(http::device_login::start_cli_device_login),
        )
        .route(
            routes::CLI_DEVICE_LOGIN_COMPLETE,
            post(http::device_login::complete_cli_device_login),
        )
        .route(
            routes::CLI_DEVICE_LOGIN_POLL,
            post(http::device_login::poll_cli_device_login),
        )
        .route(
            routes::CLI_BROWSER_LOGIN,
            post(http::cli_auth::start_cli_browser_login),
        )
        .route(
            routes::CLI_BROWSER_LOGIN_COMPLETE,
            post(http::cli_auth::complete_cli_browser_login),
        )
        .route(
            routes::CLI_BROWSER_LOGIN_EXCHANGE,
            post(http::cli_auth::exchange_cli_browser_login),
        )
        .route(
            routes::CLI_EXCHANGE_GRANTS,
            post(http::cli_auth::create_cli_exchange_grant),
        )
        .route(
            routes::CLI_EXCHANGE_GRANTS_EXCHANGE,
            post(http::cli_auth::exchange_cli_grant),
        )
        .route(routes::CLI_SESSIONS, get(http::cli_auth::list_cli_sessions))
        .route(
            routes::CLI_SESSION_BY_ID,
            delete(http::cli_auth::revoke_cli_session),
        )
        .route(
            routes::CLI_SESSION,
            delete(http::device_login::revoke_current_cli_session),
        )
        .route(
            routes::ACCOUNT_SESSION,
            get(http::account::get_account_session),
        )
        .route(routes::RUNNERS, post(http::runners::register_runner))
        .route(
            routes::RUNNER,
            get(http::runners::get_runner).delete(http::runners::delete_runner),
        )
        .route(
            routes::RUNNER_UPGRADE,
            post(http::runners::upgrade_runner_registration),
        )
        .route(
            routes::RUNNER_REPOSITORY,
            axum::routing::put(http::runners::attach_runner_repository)
                .delete(http::runners::detach_runner_repository),
        )
        .route(routes::RUNNER_POLL, post(http::runner_protocol::poll))
        .route(routes::RUNNER_CLAIM, post(http::runner_protocol::claim))
        .route(
            routes::ATTEMPT_HEARTBEAT,
            post(http::runner_protocol::heartbeat),
        )
        .route(
            routes::ATTEMPT_CACHE_FINALIZATION,
            post(http::runner_protocol::finalize_cache),
        )
        .route(
            routes::ATTEMPT_RECOVERY_STATUS,
            get(http::runner_protocol::recovery_status),
        )
        .route(
            routes::ATTEMPT_CONTAINER_IMAGE,
            post(http::runner_protocol::pin_container_image),
        )
        .route(routes::ATTEMPT_SOURCE, get(http::runner_protocol::source))
        .route(
            routes::ATTEMPT_LOGS,
            post(http::runner_protocol::append_log),
        )
        .route(
            routes::ATTEMPT_COMPLETE,
            post(http::runner_protocol::complete),
        )
        .route(
            routes::ATTEMPT_ABANDON,
            post(http::runner_protocol::abandon),
        )
        .route(
            routes::ATTEMPT_STEP_START,
            post(http::runner_protocol::start_step),
        )
        .route(
            routes::ATTEMPT_STEP_COMPLETE,
            post(http::runner_protocol::complete_step),
        )
        .route(routes::REPOS, post(http::repos::create_repo))
        .route(
            routes::OWNER_REPOSITORIES,
            get(http::repos::get_owner_repositories),
        )
        .route(
            routes::REPO,
            get(http::repos::get_repo).delete(http::repos::delete_repo),
        )
        .route(routes::REPO_CONFIG, get(http::repos::get_repo_config))
        .route(
            routes::REPO_OPERATIONS,
            get(http::runs::get_repository_operations),
        )
        .route(routes::REPO_RUNS, post(http::runs::create_manual_run))
        .route(routes::REPO_RUN, get(http::runs::get_run))
        .route(
            routes::REPO_RUN_DETAIL,
            get(http::runs::get_repository_run_detail),
        )
        .route(
            routes::REPO_RUN_STEP_LOGS,
            get(http::runs::get_repository_run_step_logs),
        )
        .route(routes::REPO_RUN_CANCEL, post(http::runs::cancel_run))
        .route(routes::REPO_RUN_RETRY, post(http::runs::retry_run))
        .route(routes::REPO_RUN_EVENTS, get(http::runs::run_events))
        .route(
            routes::REPO_PUSH_TRIGGER_EVALUATION,
            get(http::runs::get_push_trigger_evaluation),
        )
        .route(
            routes::REPO_PUSH_INTENTS,
            post(http::repos::create_push_intent),
        )
        .route(routes::REPO_SESSION, get(http::account::get_session))
        .route(routes::REPO_FILES, get(http::repos::get_files))
        .route(
            routes::REPO_FILE_CONTENT,
            get(http::repos::get_file_content),
        )
        .route(
            routes::REPO_REQUESTS,
            get(http::requests::list_requests).post(http::requests::start_request),
        )
        .route(
            routes::REPO_REQUEST_QUEUE,
            get(http::request_queue::request_queue),
        )
        .route(
            routes::REPO_REQUEST,
            get(http::requests::get_request)
                .patch(http::requests::edit_request_identity)
                .delete(http::requests::close_request),
        )
        .route(
            routes::REPO_REQUEST_SUBMIT,
            post(http::requests::submit_request),
        )
        .route(
            routes::REPO_REQUEST_MERGE,
            post(http::requests::merge_request),
        )
        .route(
            routes::REPO_REQUEST_RATINGS,
            get(http::request_ratings::list_request_ratings)
                .post(http::request_ratings::create_request_rating),
        )
        .route(
            routes::REPO_REQUEST_INVITEES,
            axum::routing::put(http::requests::add_request_invitee)
                .delete(http::requests::remove_request_invitee),
        )
        .route(
            routes::REPO_REQUEST_INVITEES_ME,
            delete(http::requests::leave_request),
        )
        .route(
            routes::REPO_REQUEST_REVISIONS,
            get(http::request_review::list_request_revisions),
        )
        .route(
            routes::REPO_REQUEST_REVISION_COMMIT,
            get(http::request_review::get_request_revision_commit),
        )
        .route(
            routes::REPO_REQUEST_REVISION_COMMIT_FILE_DIFF,
            get(http::request_review::get_request_revision_commit_file_diff),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSIONS,
            get(http::request_discussions::list_discussions)
                .post(http::request_discussions::create_discussion),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_CHANGES,
            get(http::request_discussions::changed_discussions),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_REPLIES,
            get(http::request_discussions::list_replies)
                .post(http::request_discussions::create_reply),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_RESOLVE,
            post(http::request_discussions::resolve_discussion),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_REOPEN,
            post(http::request_discussions::reopen_discussion),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_REOPEN_AND_REPLY,
            post(http::request_discussions::reopen_and_reply),
        )
        .route(
            routes::REPO_REQUEST_DISCUSSION_READ,
            axum::routing::put(http::request_discussions::mark_read),
        )
        .route(
            routes::REPO_REQUEST_ACTIVITY,
            get(http::request_discussions::activity),
        )
        .route(routes::REPO_EVENTS, get(http::repo_events::repo_events))
        .route(routes::REPO_COMMITS, get(http::history::get_commit_history))
        .route(routes::REPO_COMMIT, get(http::history::get_commit_detail))
        .route(
            routes::REPO_COMMIT_FILE_DIFF,
            get(http::history::get_commit_file_diff),
        )
        .route(
            routes::REPO_MEMBERS,
            get(http::repo_collaboration::list_repository_collaboration),
        )
        .route(
            routes::REPO_INVITES,
            post(http::repo_collaboration::create_repository_invite),
        )
        .route(
            routes::REPO_INVITE,
            delete(http::repo_collaboration::delete_repository_invite),
        )
        .route(
            routes::REPO_MEMBER,
            patch(http::repo_collaboration::update_repository_member)
                .delete(http::repo_collaboration::delete_repository_member),
        )
        .route(
            routes::REPOSITORY_INVITE,
            get(http::repo_collaboration::get_repository_invite),
        )
        .route(
            routes::REPOSITORY_INVITE_ACCEPT,
            post(http::repo_collaboration::accept_repository_invite),
        )
        .route(
            routes::REPO_PROJECTION_PREVIEW,
            get(http::repos::get_projection_preview),
        )
        .route(routes::GIT_INFO_REFS, get(git::git_info_refs))
        .route(routes::GIT_RECEIVE_PACK, post(git::git_receive_pack))
        .route(routes::GIT_UPLOAD_PACK, post(git::git_upload_pack_rpc));

    #[cfg(feature = "local-dev")]
    let router = router
        .route(
            routes::DEV_BENCH_CLI_SESSION,
            post(crate::dev::create_bench_cli_session),
        )
        .route(
            routes::DEV_CLI_SESSION,
            post(crate::dev::create_dev_cli_session),
        );

    router
        .with_state(state)
        .layer(middleware::from_fn(http::cli_compatibility::enforce))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::PATCH,
                    Method::PUT,
                    Method::DELETE,
                ])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
        .layer(TraceLayer::new_for_http())
}
