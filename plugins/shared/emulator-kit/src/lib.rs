//! Shared code for the emulator plugins (`emu-kyty`, `emu-shadps4`, and parts for `emu-pc`).
//! Not a plugin.
//!
//! A console emulator describes itself, and the kit runs the rest: a [`builds::Flavor`] for
//! its builds, which the [`builds::BuildManager`] installs, updates and rolls back, and a
//! [`console::ConsoleSpec`] for its games, compatibility list and page, which the
//! [`console::ConsoleEmulator`] serves. The plugin keeps its settings type, its launch
//! arguments and its game installs.
//!
//! The kit reaches the host only through the narrow facades in [`host::Host`]: storage in the
//! plugin's data folder, HTTP within the allowlist, named host folders, programs, settings and
//! the log. It depends on `nexus-api` like a plugin, plus `tokio` (to read host folders from
//! blocking code, and for the lock on `versions.json`) and the Kraken decoder for PS5 packages.

pub mod archive_sets;
pub mod args;
pub mod builds;
pub mod cache;
pub mod compat;
pub mod console;
pub mod controls;
pub mod exfat;
pub mod fail;
pub mod games;
pub mod github;
pub mod host;
pub mod installs;
pub mod pkg5;
pub mod read_at;
pub mod settings;
pub mod sfo;
#[cfg(test)]
mod testing;
pub mod versions;
