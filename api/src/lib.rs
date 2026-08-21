pub mod app;
pub mod state;

pub(crate) mod auth;
pub(crate) mod cache_grants;
pub(crate) mod config;
#[cfg(any(test, feature = "local-dev", feature = "smoke-seed"))]
#[path = "dev/seed.rs"]
pub(crate) mod demo_seed;
#[cfg(feature = "local-dev")]
pub mod dev;
pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod http;
pub(crate) mod object_store_config;
pub(crate) mod persistence;
pub(crate) mod persistence_ids;
pub(crate) mod push_intents;
pub(crate) mod repo_access;
pub(crate) mod repo_cleanup;
pub(crate) mod repo_events;
pub(crate) mod run_recovery;
pub(crate) mod run_retention;
pub(crate) mod runtime_budgets;
#[cfg(feature = "smoke-seed")]
pub mod smoke_seed;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod workflow_tests;

pub use app::router;
pub use state::AppState;

#[cfg(feature = "type-export")]
pub fn export_api_types(output_path: &std::path::Path) {
    http::type_exports::export_api_types(output_path);
}
