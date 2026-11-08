//! The built-in Classic theme, the old launcher's look. Every other theme extends it, directly or
//! through a parent, so it has every screen, component and token. It is compiled into the
//! binary. The Default theme (`theme-default`) extends it.

use std::sync::Arc;

use nexus_api::{EmbeddedFiles, Files, Plugin, PluginManifest, ThemePlugin, async_trait};

static FILES: EmbeddedFiles = EmbeddedFiles::new(include!(concat!(env!("OUT_DIR"), "/files.rs")));

pub const ID: &str = "classic";

pub struct ClassicTheme;

impl ClassicTheme {
    /// The theme's files, for tools and tests.
    pub fn files() -> &'static EmbeddedFiles {
        &FILES
    }
}

#[async_trait]
impl Plugin for ClassicTheme {
    fn manifest(&self) -> PluginManifest {
        let text = FILES.read(&nexus_api::RelPath::new("theme.toml").expect("valid path")).expect("theme.toml");
        PluginManifest::from_toml(std::str::from_utf8(&text).expect("UTF-8"))
            .expect("the Classic theme manifest is valid")
    }
}

impl ThemePlugin for ClassicTheme {
    fn files(&self) -> Arc<dyn Files> {
        Arc::new(FILES)
    }
}

nexus_api::inventory::submit! { nexus_api::PluginFactory::theme(|| Box::new(ClassicTheme)) }
