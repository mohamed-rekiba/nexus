//! The built-in Default theme, the one the app starts with and falls back to. It extends the
//! Classic theme: its Home is a list of the systems the emulators play, with the focused
//! system's cover behind it, and each system opens the Library on its games. Every other screen
//! is the Classic theme's.

use std::sync::Arc;

use nexus_api::{EmbeddedFiles, Files, Plugin, PluginManifest, ThemePlugin, async_trait};

static FILES: EmbeddedFiles = EmbeddedFiles::new(include!(concat!(env!("OUT_DIR"), "/files.rs")));

pub const ID: &str = "default";

pub struct DefaultTheme;

impl DefaultTheme {
    /// The theme's files, for tools and tests.
    pub fn files() -> &'static EmbeddedFiles {
        &FILES
    }
}

#[async_trait]
impl Plugin for DefaultTheme {
    fn manifest(&self) -> PluginManifest {
        let text = FILES.read(&nexus_api::RelPath::new("theme.toml").expect("valid path")).expect("theme.toml");
        PluginManifest::from_toml(std::str::from_utf8(&text).expect("UTF-8"))
            .expect("the Default theme manifest is valid")
    }
}

impl ThemePlugin for DefaultTheme {
    fn files(&self) -> Arc<dyn Files> {
        Arc::new(FILES)
    }
}

nexus_api::inventory::submit! { nexus_api::PluginFactory::theme(|| Box::new(DefaultTheme)) }
