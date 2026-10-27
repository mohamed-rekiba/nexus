//! The built-in system plugin: the machine's own settings through the system's own tools.
//!
//! - Linux ([`linux`]): network and Wi-Fi, VPNs, Bluetooth, sound, restart and shut down, the
//!   battery, and on Nexus OS its updates and its USB stick's installer.
//! - macOS: VPNs (every network with an address) and the battery (`pmset`).
//! - Windows: the battery isn't read yet; VPNs say why they can't work there.

#[cfg(target_os = "linux")]
mod linux;
// Mostly the Linux tools' output; macOS reads only pmset's. The tests run everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod parse;
mod vpn;

use std::sync::Arc;

use nexus_api::{
    Capability, CodePlugin, Plugin, PluginContext, PluginFactory, PluginManifest, PluginResult, Provided, inventory,
};

const MANIFEST: &str = include_str!("../plugin.toml");

pub struct HostSystem;

impl Plugin for HostSystem {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid");
        if !cfg!(target_os = "linux") {
            manifest.capabilities.retain(|c| matches!(c, Capability::Vpn | Capability::Power));
        }
        manifest
    }
}

#[nexus_api::async_trait]
impl CodePlugin for HostSystem {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let vpn = Some(Arc::new(vpn::Vpns::new(ctx.programs().clone())) as Arc<dyn nexus_api::VpnSettings>);
        #[cfg(target_os = "linux")]
        {
            let linux = Arc::new(linux::Linux::new(ctx.programs().clone()));
            Ok(Provided {
                network: Some(linux.clone()),
                vpn,
                bluetooth: Some(linux.clone()),
                audio: Some(linux.clone()),
                power: Some(linux.clone()),
                os_updates: Some(linux.clone()),
                os_install: Some(linux),
                ..Provided::default()
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(Provided { vpn, power: Some(Arc::new(other::Power(ctx.programs().clone()))), ..Provided::default() })
        }
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(HostSystem)) }

/// macOS and Windows: the battery, and no restart or shut down (the system's own menu does
/// that, and Nexus isn't the whole screen there).
#[cfg(not(target_os = "linux"))]
mod other {
    use std::sync::Arc;

    use nexus_api::{Battery, Location, PluginError, PluginResult, PowerSettings, Programs};

    pub struct Power(pub Arc<dyn Programs>);

    #[nexus_api::async_trait]
    impl PowerSettings for Power {
        async fn unavailable(&self) -> Option<String> {
            Some("Restart or shut down from the system's own menu".into())
        }

        async fn restart(&self) -> PluginResult<()> {
            Err(PluginError::Other("restart from the system's own menu".into()))
        }

        async fn shut_down(&self) -> PluginResult<()> {
            Err(PluginError::Other("shut down from the system's own menu".into()))
        }

        async fn battery(&self) -> PluginResult<Option<Battery>> {
            if !cfg!(target_os = "macos") {
                return Ok(None);
            }
            let args = ["-g".to_owned(), "batt".to_owned()];
            let out = self.0.output(&Location::System("pmset".into()), &args, nexus_api::MAX_PROGRAM_TIME).await?;
            Ok(crate::parse::pmset(&String::from_utf8_lossy(&out)))
        }
    }
}
