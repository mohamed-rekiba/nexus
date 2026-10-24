//! The contract between the core and themes.
//!
//! - [`vm`]: headless view models the core builds.
//! - Theme manifest, design tokens and the Slint binding contract: what a theme ships and
//!   what it may rely on.
//!
//! Themes depend on this crate and `nexus-api` only. It has no toolkit code.

pub mod contract;
pub mod theme;
pub mod tokens;
pub mod vm;

pub use contract::ComponentId;
pub use theme::*;
pub use tokens::*;
pub use vm::*;

/// The version of this contract. A theme's `ui_api` range must include it.
pub const UI_API_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn ui_api_version() -> semver::Version {
    semver::Version::parse(UI_API_VERSION).expect("the crate version is valid semver")
}
