//! The plugin contract of the launcher.
//!
//! Emulator plugins depend on this crate only. Every type that crosses between host and plugin
//! is `serde`-serializable.
//!
//! Versioning: this crate follows semver. A plugin's manifest names the versions it works with
//! in `host_api`, and the host refuses plugins whose range doesn't include [`API_VERSION`].

mod catalog;
mod config;
mod context;
mod error;
mod event;
mod extras;
mod games;
mod id;
mod manifest;
mod middleware;
mod order;
mod path;
mod plans;
mod plugin;
mod releases;
mod slots;
mod system;
mod theme;

pub use async_trait::async_trait;
#[cfg(feature = "inventory")]
pub use inventory;
// Re-exported so plugins need no other dependency. With derives, point them here:
// `#[serde(crate = "nexus_api::serde")]`, `#[schemars(crate = "nexus_api::schemars")]`.
pub use schemars;
pub use semver;
pub use serde;
pub use serde_json;

pub use catalog::*;
pub use config::*;
pub use context::*;
pub use error::*;
pub use event::*;
pub use extras::*;
pub use games::*;
pub use id::*;
pub use manifest::*;
pub use middleware::*;
pub use order::*;
pub use path::*;
pub use plans::*;
pub use plugin::*;
pub use releases::*;
pub use slots::*;
pub use system::*;
pub use theme::*;

/// The version of this API.
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn api_version() -> semver::Version {
    semver::Version::parse(API_VERSION).expect("the crate version is valid semver")
}
