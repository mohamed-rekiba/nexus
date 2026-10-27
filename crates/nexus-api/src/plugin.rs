use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    Capability, Catalog, Compatibility, Configurator, Downloader, Files, GameInstaller, Installer, Launcher,
    PluginContext, PluginManifest, PluginResult, Slots,
};

/// What every plugin has, with code or a theme: its manifest and its shutdown.
///
/// The host drives the lifecycle:
/// - **load**: construct the plugin and read `manifest()`. No work happens yet.
/// - **init** (activate): for a plugin with code, called once, at boot for
///   `activation = "eager"`, otherwise the first time the host needs one of its capabilities. A
///   theme has no init: its files are read when the theme manager needs them.
/// - **shutdown** (deactivate): called at most once per activation, under a timeout. The host
///   makes repeated deactivation safe, so the plugin doesn't have to.
///
/// Calls into a plugin run under a timeout and, in-process, a panic guard.
#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;

    async fn shutdown(&self) -> PluginResult<()> {
        Ok(())
    }
}

/// A plugin with code: an emulator, a service or a system plugin. `init` returns its
/// capabilities. Any of them may provide the data capabilities (`catalog`, `releases`,
/// `metadata`, `slots`, `configurator` and the rest); the manifest's `kind` decides only the
/// privileges: the system capabilities, [`crate::Programs::stream`] and polkit are for system
/// plugins.
#[async_trait]
pub trait CodePlugin: Plugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided>;
}

/// A theme plugin: its files (`theme.toml`, Slint screens and components, assets), and no
/// other code.
pub trait ThemePlugin: Plugin {
    fn files(&self) -> Arc<dyn Files>;
}

/// A plugin as the host registers it: code with an `init`, or a theme. The manifest's `kind`
/// says which kind of code plugin it is.
pub enum PluginBox {
    Code(Box<dyn CodePlugin>),
    Theme(Box<dyn ThemePlugin>),
}

impl PluginBox {
    pub fn code(plugin: impl CodePlugin + 'static) -> Self {
        Self::Code(Box::new(plugin))
    }

    pub fn theme(plugin: impl ThemePlugin + 'static) -> Self {
        Self::Theme(Box::new(plugin))
    }

    /// The root plugin, code or theme.
    pub fn as_plugin(&self) -> &dyn Plugin {
        match self {
            Self::Code(p) => p.as_ref(),
            Self::Theme(p) => p.as_ref(),
        }
    }

    pub fn manifest(&self) -> PluginManifest {
        self.as_plugin().manifest()
    }

    pub fn is_theme(&self) -> bool {
        matches!(self, Self::Theme(_))
    }
}

/// The capabilities a code plugin provides. It must match `capabilities` in the manifest.
#[derive(Clone, Default)]
pub struct Provided {
    pub catalog: Option<Arc<dyn Catalog>>,
    pub downloader: Option<Arc<dyn Downloader>>,
    pub installer: Option<Arc<dyn Installer>>,
    pub configurator: Option<Arc<dyn Configurator>>,
    pub launcher: Option<Arc<dyn Launcher>>,
    pub compatibility: Option<Arc<dyn Compatibility>>,
    pub slots: Option<Arc<dyn Slots>>,
    pub game_installer: Option<Arc<dyn GameInstaller>>,
    pub metadata: Option<Arc<dyn crate::Metadata>>,
    pub releases: Option<Arc<dyn crate::Releases>>,
    pub network: Option<Arc<dyn crate::NetworkSettings>>,
    pub vpn: Option<Arc<dyn crate::VpnSettings>>,
    pub bluetooth: Option<Arc<dyn crate::BluetoothSettings>>,
    pub audio: Option<Arc<dyn crate::AudioSettings>>,
    pub power: Option<Arc<dyn crate::PowerSettings>>,
    pub os_updates: Option<Arc<dyn crate::OsUpdates>>,
    pub os_install: Option<Arc<dyn crate::OsInstall>>,
    /// Images that slot data and catalog entries refer to as [`crate::ImageSource::Asset`].
    /// Plugins loaded from a folder can use the manifest's `assets` folder instead.
    pub assets: Option<Arc<dyn Files>>,
}

impl std::fmt::Debug for Provided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Provided").field(&self.capabilities()).finish()
    }
}

impl Provided {
    pub fn capabilities(&self) -> BTreeSet<Capability> {
        let present = [
            (Capability::Catalog, self.catalog.is_some()),
            (Capability::Downloader, self.downloader.is_some()),
            (Capability::Installer, self.installer.is_some()),
            (Capability::Configurator, self.configurator.is_some()),
            (Capability::Launcher, self.launcher.is_some()),
            (Capability::Compatibility, self.compatibility.is_some()),
            (Capability::Slots, self.slots.is_some()),
            (Capability::GameInstaller, self.game_installer.is_some()),
            (Capability::Metadata, self.metadata.is_some()),
            (Capability::Releases, self.releases.is_some()),
            (Capability::Network, self.network.is_some()),
            (Capability::Vpn, self.vpn.is_some()),
            (Capability::Bluetooth, self.bluetooth.is_some()),
            (Capability::Audio, self.audio.is_some()),
            (Capability::Power, self.power.is_some()),
            (Capability::OsUpdates, self.os_updates.is_some()),
            (Capability::OsInstall, self.os_install.is_some()),
        ];
        present.into_iter().filter_map(|(c, yes)| yes.then_some(c)).collect()
    }
}

/// Registers a first-party plugin at compile time, code or theme:
///
/// ```ignore
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::code(|| Box::new(MyEmulator)) }
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::theme(|| Box::new(MyTheme)) }
/// ```
///
/// The crate must be linked into the host binary, or the factory never registers.
#[cfg(feature = "inventory")]
pub enum PluginFactory {
    Code(fn() -> Box<dyn CodePlugin>),
    Theme(fn() -> Box<dyn ThemePlugin>),
}

#[cfg(feature = "inventory")]
impl PluginFactory {
    pub const fn code(create: fn() -> Box<dyn CodePlugin>) -> Self {
        Self::Code(create)
    }

    pub const fn theme(create: fn() -> Box<dyn ThemePlugin>) -> Self {
        Self::Theme(create)
    }

    pub fn create(&self) -> PluginBox {
        match self {
            Self::Code(create) => PluginBox::Code(create()),
            Self::Theme(create) => PluginBox::Theme(create()),
        }
    }

    pub fn all() -> impl Iterator<Item = &'static PluginFactory> {
        inventory::iter::<PluginFactory>.into_iter()
    }
}

#[cfg(feature = "inventory")]
inventory::collect!(PluginFactory);
