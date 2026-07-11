pub mod app;
pub mod state;

pub(crate) mod auth;
#[cfg(feature = "local-dev")]
#[path = "../../dev/api/mod.rs"]
pub mod dev;
pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod http;
pub(crate) mod persistence;
pub(crate) mod runtime_budgets;

pub use scope_core::domain;
pub use scope_core::object_store;
pub(crate) use scope_core::{config, db, repo_events};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
#[path = "../tests/workflows/mod.rs"]
mod workflow_tests;

pub use app::router;
pub use state::AppState;

#[cfg(feature = "type-export")]
pub fn export_api_types(output_path: &std::path::Path) {
    http::type_exports::export_api_types(output_path);
}
