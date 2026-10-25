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

/// The version of this contract. A theme's `ui_api` range must include it. It is not the crate's
/// version: that is Nexus's, which every release raises. This one changes by hand, with the
/// contract, together with the built-in themes' `ui_api`.
pub const UI_API_VERSION: &str = "0.1.1";

pub fn ui_api_version() -> semver::Version {
    semver::Version::parse(UI_API_VERSION).expect("UI_API_VERSION is valid semver")
}
