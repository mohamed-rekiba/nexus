//! "Download only through a VPN" (`app.vpn_only`, off by default), through the host's
//! [`crate::Network`] port.
//!
//! The VPN in use is the one the player picked (`app.vpn`). With none picked, it is the first
//! connected VPN, where the system tells VPNs from other networks. Where it can't (macOS),
//! only a picked network counts. The torrent port binds every connection to the VPN's
//! interface before a download talks to the swarm. Without that VPN, a download waits: it is
//! paused and marked waiting.
//!
//! The kill switch: while a download runs or waits, the core checks the VPN every
//! `vpn_poll`. When the VPN drops, or another one takes its place, the core pauses every
//! running download, marks it waiting, and closes the torrent port's connections. When a VPN
//! is back, only the waiting downloads start again. One that hadn't read its files yet reads
//! them, then waits for the player as always. A restart forgets the waiting: the downloads
//! come back paused.
//!
//! Plugins see the same rule through [`HostVpnGate`], in their service registry: while the
//! setting is on, a plugin asks it before it fetches what the player downloads in bulk. When
//! the gate tells a plugin to wait, the kill switch checks the VPN every `vpn_poll` too. When
//! the VPN is back, the core reads the catalogs again: the catalog refresh is what waits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use nexus_api::{VpnBlock, VpnCheck, VpnPass};

use crate::app::Core;
use crate::changes::Area;
use crate::config::ConfigService;
use crate::{Network, Vpn};

/// The interface of the VPN downloads use: the picked one when it is connected, or else the
/// first connected one when the system `finds` VPNs.
pub(crate) fn pick(vpns: &[Vpn], chosen: &str, finds: bool) -> Option<String> {
    let mut connected = vpns.iter().filter(|v| v.connected && !v.interface.is_empty());
    let vpn = if !chosen.is_empty() {
        connected.find(|v| v.name == chosen)
    } else if finds {
        connected.next()
    } else {
        None
    };
    vpn.map(|v| v.interface.clone())
}

/// Downloads go only through the VPN. Never where the system can't bind them to one.
pub(crate) fn vpn_only(config: &ConfigService, network: &dyn Network) -> bool {
    config.get(&["app", "vpn_only"]).as_bool().unwrap_or(false) && network.problem().is_none()
}

/// The VPN the player picked; empty for none.
pub(crate) fn vpn_chosen(config: &ConfigService) -> String {
    config.get(&["app", "vpn"]).as_str().unwrap_or_default().to_owned()
}

/// "Download only through a VPN" for plugins: the same setting and the same VPN as the
/// downloads. It sets `waited` when it tells a plugin to wait.
pub(crate) struct HostVpnGate {
    pub config: Arc<ConfigService>,
    pub network: NetworkSource,
    pub waited: Arc<AtomicBool>,
}

/// Where the VPNs come from: the app's port at first, then the system plugin that provides
/// them. Shared, so a plugin's VPN gate sees the change.
#[derive(Clone)]
pub(crate) struct NetworkSource(Arc<std::sync::RwLock<Arc<dyn Network>>>);

impl NetworkSource {
    pub fn new(network: Arc<dyn Network>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(network)))
    }

    pub fn get(&self) -> Arc<dyn Network> {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set(&self, network: Arc<dyn Network>) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = network;
    }
}

#[async_trait]
impl VpnCheck for HostVpnGate {
    async fn check(&self) -> Result<VpnPass, VpnBlock> {
        if !self.config.get(&["app", "vpn_only"]).as_bool().unwrap_or(false) {
            return Ok(VpnPass::Open);
        }
        let network = self.network.get();
        if let Some(reason) = network.problem() {
            return Ok(VpnPass::Unbound { reason });
        }
        let finds = network.finds_vpns();
        let vpns = tokio::task::spawn_blocking(move || network.vpns()).await.unwrap_or_default();
        let chosen = vpn_chosen(&self.config);
        if pick(&vpns, &chosen, finds).is_some() {
            return Ok(VpnPass::Open);
        }
        let block = if !chosen.is_empty() {
            VpnBlock::Waiting { reason: format!("{chosen} isn't connected") }
        } else if !finds {
            VpnBlock::Waiting { reason: "pick the VPN in Settings".into() }
        } else if !vpns.is_empty() {
            VpnBlock::Waiting { reason: "no VPN is connected".into() }
        } else {
            VpnBlock::NoVpn { reason: "there is no VPN".into() }
        };
        if matches!(block, VpnBlock::Waiting { .. }) {
            self.waited.store(true, Ordering::SeqCst);
        }
        Err(block)
    }
}

impl Core {
    /// Downloads go only through the VPN. Never where the system can't bind them to one.
    pub(crate) fn vpn_only(&self) -> bool {
        vpn_only(self.kernel().config(), self.kernel().network().as_ref())
    }

    /// The VPN the player picked; empty for none.
    pub(crate) fn vpn_chosen(&self) -> String {
        vpn_chosen(self.kernel().config())
    }

    /// Reads the VPNs again, off the UI thread, and keeps them for the view.
    pub(crate) async fn refresh_vpns(&self) -> Vec<Vpn> {
        let network = self.kernel().network();
        let vpns = tokio::task::spawn_blocking(move || network.vpns()).await.unwrap_or_default();
        self.keep_vpns(vpns.clone());
        vpns
    }

    /// Keeps the VPNs as just read, for the view.
    pub(crate) fn keep_vpns(&self, vpns: Vec<Vpn>) {
        let changed = {
            let mut state = self.state();
            let changed = state.vpns != vpns;
            state.vpns = vpns;
            changed
        };
        if changed {
            self.changed(Area::Vpn);
        }
    }

    /// Runs `call` on the network port off the UI thread, keeps its error for the setting,
    /// and reads the VPNs again.
    pub(crate) async fn vpn_call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn crate::Network) -> Result<T, String> + Send + 'static,
    ) -> crate::CoreResult<T> {
        let network = self.kernel().network();
        let done = tokio::task::spawn_blocking(move || call(network.as_ref()))
            .await
            .map_err(|e| crate::CoreError::Io(e.to_string()))?;
        self.state().vpn_error = done.as_ref().err().cloned();
        self.refresh_vpns().await;
        self.vpn_changed().await;
        done.map_err(crate::CoreError::Invalid)
    }

    /// The setting or the VPNs changed: the kill switch looks at once. A plugin that waits
    /// for the VPN gets it this way too, for example when the player picks the VPN.
    pub(crate) async fn vpn_changed(&self) {
        self.torrents().vpn_changed().await;
        self.changed(Area::Vpn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vpn(name: &str, connected: bool) -> Vpn {
        Vpn {
            name: name.into(),
            kind: "WireGuard".into(),
            connected,
            interface: if connected { name.into() } else { String::new() },
            switchable: false,
        }
    }

    #[test]
    fn the_picked_vpn_counts_only_while_it_is_connected() {
        let vpns = [vpn("proton0", false), vpn("tun0", true), vpn("wg-se", true)];
        assert_eq!(pick(&vpns, "", true).as_deref(), Some("tun0"));
        assert_eq!(pick(&vpns, "wg-se", true).as_deref(), Some("wg-se"));
        assert_eq!(pick(&vpns, "proton0", true), None, "a picked VPN that is down means: wait");
        assert_eq!(pick(&vpns, "", false), None, "where the system can't tell a VPN, only a pick counts");
        assert_eq!(pick(&vpns, "tun0", false).as_deref(), Some("tun0"));
    }
}
