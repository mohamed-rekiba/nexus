//! Shared code for the emulator plugins (`emu-kyty`, `emu-shadps4`). Not a plugin.
//!
//! Everything here goes through the plugin's [`nexus_api::PluginContext`]: HTTP within the
//! allowlist, storage in the plugin's data folder, and named host folders. It depends on
//! `nexus-api` like a plugin, plus `tokio` (to read host folders from blocking code) and
//! the Kraken decoder for PS5 packages.

pub mod archive_sets;
pub mod args;
pub mod cache;
pub mod compat;
pub mod controls;
pub mod exfat;
pub mod fail;
pub mod games;
pub mod github;
pub mod installs;
pub mod pkg5;
pub mod read_at;
pub mod sfo;
pub mod versions;
