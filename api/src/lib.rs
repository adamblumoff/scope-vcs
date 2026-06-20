pub mod app;
pub mod domain;
pub mod state;

pub(crate) mod auth;
pub(crate) mod config;
pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod http;
pub(crate) mod persistence;

#[cfg(test)]
mod tests;

pub use app::router;
pub use state::AppState;
