use crate::{git, http, state::AppState};
use axum::{
    Router,
    http::{
        Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
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
            routes::ADMIN_METADATA_RESET,
            post(http::admin::reset_metadata),
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
        .route(
            routes::REPOS,
            get(http::repos::list_repos).post(http::repos::create_repo),
        )
        .route(
            routes::REPO,
            get(http::repos::get_repo).delete(http::repos::delete_repo),
        )
        .route(routes::REPO_CONFIG, get(http::repos::get_repo_config))
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
            routes::REPO_REQUEST,
            get(http::requests::get_request).delete(http::requests::delete_request),
        )
        .route(
            routes::REPO_REQUEST_CHANGES,
            get(http::request_review::get_request_changes),
        )
        .route(
            routes::REPO_REQUEST_FILE_DIFF,
            get(http::request_review::get_request_file_diff),
        )
        .route(
            routes::REPO_REQUEST_SUBMIT,
            post(http::requests::submit_request),
        )
        .route(
            routes::REPO_REQUEST_COMMENTS,
            post(http::requests::comment_request),
        )
        .route(
            routes::REPO_REQUEST_NEEDS_RESPONSE,
            post(http::requests::mark_needs_response),
        )
        .route(
            routes::REPO_REQUEST_RESPOND,
            post(http::requests::respond_to_request),
        )
        .route(
            routes::REPO_REQUEST_RESOLVE,
            post(http::requests::resolve_request),
        )
        .route(
            routes::REPO_REQUEST_MERGE,
            post(http::requests::merge_request),
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
    let router = router.route(
        routes::DEV_BENCH_CLI_SESSION,
        post(crate::dev::create_bench_cli_session),
    );

    router
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::PATCH,
                    Method::DELETE,
                ])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
        .layer(TraceLayer::new_for_http())
}
