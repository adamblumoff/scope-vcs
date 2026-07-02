pub mod app;
pub mod state;

pub(crate) mod auth;
#[cfg(feature = "local-dev")]
#[path = "../../dev/api/mod.rs"]
pub mod dev;
pub(crate) mod git;
pub(crate) mod http;
pub(crate) mod persistence;
pub(crate) mod runtime_budgets;

pub use scope_core::domain;
pub use scope_core::object_store;
pub(crate) use scope_core::{config, db, error, repo_events};

#[cfg(test)]
mod tests;

pub use app::router;
pub use state::AppState;
