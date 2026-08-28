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
mod git_segment_v2_backfill;
pub(crate) mod http;
mod landing_file_backfill;
pub(crate) mod object_store_config;
pub(crate) mod persistence;
pub(crate) mod persistence_ids;
pub(crate) mod product_analytics;
pub(crate) mod push_intents;
pub(crate) mod repo_access;
pub(crate) mod repo_events;
pub(crate) mod run_recovery;
pub(crate) mod run_retention;
pub(crate) mod runtime_budgets;
#[cfg(feature = "smoke-seed")]
pub mod smoke_seed;
pub(crate) mod telemetry;
pub(crate) mod use_cases;
mod workflow_catalog_backfill;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod workflow_tests;

pub use app::router;
pub use git_segment_v2_backfill::backfill_git_segments_v2_for_maintenance;
pub use state::AppState;
pub use workflow_catalog_backfill::validate_repository_workflow_catalogs_for_maintenance;

#[cfg(feature = "type-export")]
pub fn export_api_types(output_path: &std::path::Path) {
    http::type_exports::export_api_types(output_path);
}
