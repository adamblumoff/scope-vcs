//! Delivery contracts shared by the API and its Rust clients.
//!
//! Durable policy stays in `scope-domain`; this crate owns only serialized shapes
//! and route construction.

mod repo_config;
mod types;
mod wire;

pub mod routes;
pub use repo_config::*;
pub use types::*;
pub use wire::*;
