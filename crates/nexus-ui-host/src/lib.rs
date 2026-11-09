//! The UI host: shows themes with Slint.
//!
//! - [`ThemeSet`] finds the installed themes, the fallback theme among them, and serves each with the kit
//!   beneath its files, with no toolkit code.
//! - [`ThemeHost`] compiles a theme, binds the core's view models to it, and swaps themes
//!   atomically: validate, load assets, build the new view tree, swap, release the old one.
//!   On any failure the old theme stays.
//! - [`AppShell`] owns the theme host on the UI thread: it routes the theme's requests,
//!   coalesces renders, loads images and runs the tick, through a [`Platform`].

mod bind;
mod compile;
#[cfg(test)]
mod drift;
mod host;
mod images;
mod input;
mod loader;
mod shell;
mod themes;
mod values;

pub use bind::Viewport;
pub use compile::{VFS_ROOT, sources};
pub use host::{Requests, ThemeHost, TrailerRequest, TrailerStatus, UiRequest, vm_text};
pub use images::{Crop, ImageKey, ImageSize, MAX_HEIGHT, MAX_WIDTH, Prepared, StillWanted, prepare};
pub use input::{PadAction, PadButton};
pub use loader::sweep_thumbs;
pub use shell::{AppShell, IMAGES_GAP, Platform, RENDER_GAP, RequestQueue, ShellOptions, TICK, TrailerPoll};
pub use themes::{InstalledTheme, ResolvedTheme, ThemeSet};
