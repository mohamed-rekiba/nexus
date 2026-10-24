//! The UI host: shows themes with Slint.
//!
//! - [`ThemeSet`] finds the installed themes and resolves inheritance, with no toolkit code.
//! - [`ThemeHost`] compiles a theme, binds the core's view models to it, and swaps themes
//!   atomically: validate, load assets, build the new view tree, swap, release the old one.
//!   On any failure the old theme stays.

mod bind;
mod compile;
mod host;
mod images;
mod input;
mod themes;

pub use bind::Viewport;
pub use compile::{VFS_ROOT, sources};
pub use host::{BASE_THEME, Requests, ThemeHost, TrailerRequest, TrailerStatus, UiRequest, vm_text};
pub use images::{Crop, ImageKey, ImageSize, MAX_HEIGHT, MAX_WIDTH, Prepared, StillWanted, prepare};
pub use input::{PadAction, PadButton};
pub use themes::{ResolvedTheme, ThemeLayer, ThemeSet};
