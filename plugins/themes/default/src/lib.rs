//! The built-in default theme. Every other theme extends it, directly or through a parent, so it
//! has every screen, component and token. It is compiled into the binary, it is the default,
//! and it is what the app falls back to when another theme fails.

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
            .expect("the default theme manifest is valid")
    }
}

impl ThemePlugin for DefaultTheme {
    fn files(&self) -> Arc<dyn Files> {
        Arc::new(FILES)
    }
}

nexus_api::inventory::submit! { nexus_api::PluginFactory::theme(|| Box::new(DefaultTheme)) }
