pub mod app;
pub mod domain;
pub mod state;

pub(crate) mod auth;
pub(crate) mod config;
pub(crate) mod db;
#[cfg(feature = "local-dev")]
#[path = "../../dev/api/mod.rs"]
pub mod dev;
pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod http;
pub mod object_store;
pub(crate) mod persistence;
pub(crate) mod repo_events;

#[cfg(test)]
mod tests;

pub use app::router;
pub use state::AppState;
