use crate::{git, http, state::AppState};
use axum::{
    Router,
    routing::{get, patch, post},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(http::account::healthz))
        .route("/readyz", get(http::account::readyz))
        .route("/v1/session", get(http::account::get_account_session))
        .route(
            "/v1/repos",
            get(http::repos::list_repos).post(http::repos::create_repo),
        )
        .route(
            "/v1/repos/{owner}/{repo}",
            get(http::repos::get_repo).delete(http::repos::delete_repo),
        )
        .route(
            "/v1/repos/{owner}/{repo}/setup",
            get(http::setup::get_repo_setup),
        )
        .route(
            "/v1/repos/{owner}/{repo}/setup-token",
            get(http::setup::get_repo_setup).post(http::setup::regenerate_first_push_token),
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
            "/v1/repos/{owner}/{repo}/pending-import",
            get(http::review::get_pending_import_review),
        )
        .route(
            "/v1/repos/{owner}/{repo}/publish",
            post(http::review::publish_repo),
        )
        .route(
            "/v1/repos/{owner}/{repo}/files/visibility",
            patch(http::repos::update_file_visibility),
        )
        .route(
            "/v1/repos/{owner}/{repo}/settings",
            get(http::repos::get_settings).patch(http::repos::update_settings),
        )
        .route(
            "/v1/repos/{owner}/{repo}/staged-update",
            get(http::review::get_staged_update),
        )
        .route(
            "/v1/repos/{owner}/{repo}/staged-update/files/visibility",
            patch(http::review::update_staged_file_visibility),
        )
        .route(
            "/v1/repos/{owner}/{repo}/staged-update/apply",
            post(http::review::apply_staged_update),
        )
        .route(
            "/v1/repos/{owner}/{repo}/staged-update/reject",
            post(http::review::reject_staged_update),
        )
        .route(
            "/v1/repos/{owner}/{repo}/projections",
            get(http::repos::get_projection),
        )
        .route(
            "/v1/repos/{owner}/{repo}/git-projections",
            get(http::repos::get_git_projection),
        )
        .route("/git/{org}/{repo}/info/refs", get(git::git_info_refs))
        .route(
            "/git/{org}/{repo}/git-receive-pack",
            post(git::git_receive_pack),
        )
        .route(
            "/git/{org}/{repo}/git-upload-pack",
            post(git::git_upload_pack_rpc),
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
