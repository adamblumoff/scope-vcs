//! Delivery contracts shared by the API and its Rust clients.
//!
//! Durable policy stays in `scope-domain`; this crate owns only serialized shapes
//! and route construction.

mod repo_config;
mod runs;
mod types;
mod wire;

mod cli_compatibility;

pub mod routes;
pub use cli_compatibility::*;
pub use repo_config::*;
pub use runs::*;
pub use types::*;
pub use wire::*;
