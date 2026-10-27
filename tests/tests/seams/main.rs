//! The seam tests (docs/development.md#tests), in one test binary: one module per area, with
//! the shared helpers in `common`. One binary builds and links once, instead of once per file.
//! `spans.rs` stays its own binary: `tracing` caches per-callsite interest across threads.

mod app;
mod boot;
mod catalog_refresh;
mod common;
mod compare;
mod contracts;
mod default_theme;
mod folders;
mod game_installs;
mod gamepad;
mod groups;
mod hardware;
mod kyty;
mod launcher_update;
mod lifecycle;
mod metadata;
mod pc;
mod prefetch;
mod programs;
mod real_archives;
mod release_archives;
mod releases;
mod screenshots;
mod shadps4;
mod slots;
mod sounds;
mod steam;
mod switching;
mod system;
mod system_theme;
mod themes;
mod timing;
mod torrents;
mod unpack;
mod vpn;
