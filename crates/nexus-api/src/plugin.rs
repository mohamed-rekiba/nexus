use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    Capability, Catalog, Compatibility, Configurator, Controllers, Downloader, Files, GameInstaller, Installer,
    LaunchMiddleware, Launcher, PluginContext, PluginManifest, PluginResult, Saves, Shaders, Slots,
};

/// What every plugin has, emulator or theme: its manifest and its shutdown.
///
/// The host drives the lifecycle:
/// - **load**: construct the plugin and read `manifest()`. No work happens yet.
/// - **init** (activate): for an emulator, called once, at boot for `activation = "eager"`,
///   otherwise the first time the host needs one of its capabilities. A theme has no init: its
///   files are read when the theme manager needs them.
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

/// An emulator plugin: Rust code that returns emulator capabilities.
#[async_trait]
pub trait EmulatorPlugin: Plugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided>;
}

/// A service plugin: Rust code that adds to the games other plugins find (`Metadata`,
/// `Releases`, `Slots`, `Configurator`) and launches none.
#[async_trait]
pub trait ServicePlugin: Plugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided>;
}

/// A system plugin: Rust code that provides the machine's own settings and devices (network,
/// VPNs, Bluetooth, sound, power, and on Nexus OS its updates and installer). See
/// [`crate::NetworkSettings`] and the other traits in that module.
#[async_trait]
pub trait SystemPlugin: Plugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided>;
}

/// A theme plugin: its files (`theme.toml`, Slint screens and components, assets), and no
/// other code.
pub trait ThemePlugin: Plugin {
    fn files(&self) -> Arc<dyn Files>;
}

/// A plugin held by its kind: an emulator, a service, a theme or a system plugin, as the host
/// registers it.
pub enum PluginByKind {
    Emulator(Box<dyn EmulatorPlugin>),
    Service(Box<dyn ServicePlugin>),
    Theme(Box<dyn ThemePlugin>),
    System(Box<dyn SystemPlugin>),
}

impl PluginByKind {
    pub fn emulator(plugin: impl EmulatorPlugin + 'static) -> Self {
        Self::Emulator(Box::new(plugin))
    }

    pub fn service(plugin: impl ServicePlugin + 'static) -> Self {
        Self::Service(Box::new(plugin))
    }

    pub fn theme(plugin: impl ThemePlugin + 'static) -> Self {
        Self::Theme(Box::new(plugin))
    }

    pub fn system(plugin: impl SystemPlugin + 'static) -> Self {
        Self::System(Box::new(plugin))
    }

    /// The root plugin, whichever kind it is.
    pub fn as_plugin(&self) -> &dyn Plugin {
        match self {
            Self::Emulator(p) => p.as_ref(),
            Self::Service(p) => p.as_ref(),
            Self::Theme(p) => p.as_ref(),
            Self::System(p) => p.as_ref(),
        }
    }

    pub fn manifest(&self) -> PluginManifest {
        self.as_plugin().manifest()
    }

    /// The kind its trait makes it. The manifest must say the same.
    pub fn kind(&self) -> crate::PluginKind {
        match self {
            Self::Emulator(_) => crate::PluginKind::Emulator,
            Self::Service(_) => crate::PluginKind::Service,
            Self::Theme(_) => crate::PluginKind::Theme,
            Self::System(_) => crate::PluginKind::System,
        }
    }
}

/// The capabilities an emulator plugin provides. It must match `capabilities` in the manifest.
#[derive(Clone, Default)]
pub struct Provided {
    pub catalog: Option<Arc<dyn Catalog>>,
    pub downloader: Option<Arc<dyn Downloader>>,
    pub installer: Option<Arc<dyn Installer>>,
    pub configurator: Option<Arc<dyn Configurator>>,
    pub launcher: Option<Arc<dyn Launcher>>,
    pub compatibility: Option<Arc<dyn Compatibility>>,
    pub saves: Option<Arc<dyn Saves>>,
    pub controllers: Option<Arc<dyn Controllers>>,
    pub shaders: Option<Arc<dyn Shaders>>,
    pub slots: Option<Arc<dyn Slots>>,
    pub game_installer: Option<Arc<dyn GameInstaller>>,
    pub launch_middleware: Vec<Arc<LaunchMiddleware>>,
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
            (Capability::Saves, self.saves.is_some()),
            (Capability::Controllers, self.controllers.is_some()),
            (Capability::Shaders, self.shaders.is_some()),
            (Capability::Slots, self.slots.is_some()),
            (Capability::GameInstaller, self.game_installer.is_some()),
            (Capability::LaunchMiddleware, !self.launch_middleware.is_empty()),
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

/// Registers a first-party plugin at compile time, one call per kind:
///
/// ```ignore
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::emulator(|| Box::new(MyEmulator)) }
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::service(|| Box::new(MyService)) }
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::theme(|| Box::new(MyTheme)) }
/// nexus_api::inventory::submit! { nexus_api::PluginFactory::system(|| Box::new(MySystem)) }
/// ```
///
/// The crate must be linked into the host binary, or the factory never registers.
#[cfg(feature = "inventory")]
pub enum PluginFactory {
    Emulator(fn() -> Box<dyn EmulatorPlugin>),
    Service(fn() -> Box<dyn ServicePlugin>),
    Theme(fn() -> Box<dyn ThemePlugin>),
    System(fn() -> Box<dyn SystemPlugin>),
}

#[cfg(feature = "inventory")]
impl PluginFactory {
    pub const fn emulator(create: fn() -> Box<dyn EmulatorPlugin>) -> Self {
        Self::Emulator(create)
    }

    pub const fn service(create: fn() -> Box<dyn ServicePlugin>) -> Self {
        Self::Service(create)
    }

    pub const fn theme(create: fn() -> Box<dyn ThemePlugin>) -> Self {
        Self::Theme(create)
    }

    pub const fn system(create: fn() -> Box<dyn SystemPlugin>) -> Self {
        Self::System(create)
    }

    pub fn create(&self) -> PluginByKind {
        match self {
            Self::Emulator(create) => PluginByKind::Emulator(create()),
            Self::Service(create) => PluginByKind::Service(create()),
            Self::Theme(create) => PluginByKind::Theme(create()),
            Self::System(create) => PluginByKind::System(create()),
        }
    }

    pub fn all() -> impl Iterator<Item = &'static PluginFactory> {
        inventory::iter::<PluginFactory>.into_iter()
    }
}

#[cfg(feature = "inventory")]
inventory::collect!(PluginFactory);
