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
        .route("/healthz", get(http::account::healthz))
        .route("/readyz", get(http::account::readyz))
        .route("/v1/admin/cleanup", get(http::admin::get_cleanup_status))
        .route("/v1/admin/cleanup/drain", post(http::admin::drain_cleanup))
        .route(
            "/v1/admin/metadata/reset",
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
            "/v1/repos",
            get(http::repos::list_repos).post(http::repos::create_repo),
        )
        .route(
            "/v1/repos/{owner}/{repo}",
            get(http::repos::get_repo).delete(http::repos::delete_repo),
        )
        .route(
            "/v1/repos/{owner}/{repo}/clone-credential",
            post(http::repos::create_clone_credential),
        )
        .route(
            "/v1/repos/{owner}/{repo}/push-intents",
            post(http::repos::create_push_intent),
        )
        .route(
            "/v1/repos/{owner}/{repo}/session",
            get(http::account::get_session),
        )
        .route(
            "/v1/repos/{owner}/{repo}/files",
            get(http::repos::get_files),
        )
        .route(
            "/v1/repos/{owner}/{repo}/events",
            get(http::repo_events::repo_events),
        )
        .route(
            "/v1/repos/{owner}/{repo}/commits",
            get(http::history::get_commit_history),
        )
        .route(
            "/v1/repos/{owner}/{repo}/commits/{commit_id}",
            get(http::history::get_commit_detail),
        )
        .route(
            "/v1/repos/{owner}/{repo}/commits/{commit_id}/file-diff",
            get(http::history::get_commit_file_diff),
        )
        .route(
            "/v1/repos/{owner}/{repo}/members",
            get(http::repo_collaboration::list_repository_collaboration),
        )
        .route(
            "/v1/repos/{owner}/{repo}/invites",
            post(http::repo_collaboration::create_repository_invite),
        )
        .route(
            "/v1/repos/{owner}/{repo}/invites/{invite_id}",
            delete(http::repo_collaboration::delete_repository_invite),
        )
        .route(
            "/v1/repos/{owner}/{repo}/members/{member_user_id}",
            patch(http::repo_collaboration::update_repository_member)
                .delete(http::repo_collaboration::delete_repository_member),
        )
        .route(
            "/v1/repository-invites/{token}",
            get(http::repo_collaboration::get_repository_invite),
        )
        .route(
            "/v1/repository-invites/{token}/accept",
            post(http::repo_collaboration::accept_repository_invite),
        )
        .route(
            "/v1/repos/{owner}/{repo}/projection-preview",
            get(http::repos::get_projection_preview),
        )
        .route("/git/{org}/{repo}/info/refs", get(git::git_info_refs))
        .route(
            "/git/{org}/{repo}/git-receive-pack",
            post(git::git_receive_pack),
        )
        .route(
            "/git/{org}/{repo}/git-upload-pack",
            post(git::git_upload_pack_rpc),
        );

    #[cfg(feature = "local-dev")]
    let router = router.route(
        "/v1/dev/bench/cli-session",
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
