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
//! The downloads' half lives in the torrents subsystem (`torrents/vpn.rs`): the gate before a
//! download talks to the swarm, and the kill switch. This module keeps the rule
//! ([`VpnRule`]), which the torrents and the plugins read, and the VPN subsystem
//! ([`VpnSubsystem`]): the setting, the VPN list for the view, and connecting, disconnecting
//! and adding VPNs. The subsystem owns `app.vpn_only` and `app.vpn`.
//!
//! Plugins see the same rule through [`nexus_api::VpnGate`], in their service registry: while
//! the setting is on, a plugin asks it before it fetches what the player downloads in bulk.
//! When the gate tells a plugin to wait, the kill switch checks the VPN every `vpn_poll` too.
//! When the VPN is back, the core reads the catalogs again: the catalog refresh is what waits.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use async_trait::async_trait;
use futures::future::BoxFuture;
use nexus_api::{VpnBlock, VpnCheck, VpnPass};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::{CoreError, CoreResult, Network, Vpn};

/// The setting, and the VPN the player picked.
const VPN_ONLY: [&str; 2] = ["app", "vpn_only"];
const VPN: [&str; 2] = ["app", "vpn"];

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

/// The VPN rule as every area reads it: the setting, the picked VPN, where the VPNs come
/// from, and whether a plugin waits for the VPN. The kernel makes it at boot, since each
/// plugin gets the rule as its [`nexus_api::VpnGate`]; the torrents subsystem and the VPN
/// subsystem share it. Cheap to clone; every clone is the same rule.
#[derive(Clone)]
pub(crate) struct VpnRule {
    inner: Arc<Rule>,
}

struct Rule {
    /// For `app.vpn_only` and `app.vpn`. Only the VPN subsystem writes them.
    config: Arc<ConfigService>,
    /// The app's port at first, then the one of the system plugin that provides the VPNs.
    network: RwLock<Arc<dyn Network>>,
    /// The gate told a plugin to wait, and the kill switch hasn't seen the VPN back.
    waited: AtomicBool,
}

impl VpnRule {
    pub fn new(config: Arc<ConfigService>, network: Arc<dyn Network>) -> Self {
        Self { inner: Arc::new(Rule { config, network: RwLock::new(network), waited: AtomicBool::new(false) }) }
    }

    /// The VPNs, from the system plugin that provides them, or else from the app's port.
    pub fn network(&self) -> Arc<dyn Network> {
        self.inner.network.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn use_network(&self, network: Arc<dyn Network>) {
        *self.inner.network.write().unwrap_or_else(|p| p.into_inner()) = network;
    }

    /// The setting as the player left it.
    fn setting(&self) -> bool {
        self.inner.config.get(&VPN_ONLY).as_bool().unwrap_or(false)
    }

    /// Downloads go only through the VPN. Never where the system can't bind them to one.
    pub fn only(&self) -> bool {
        self.setting() && self.network().problem().is_none()
    }

    /// The VPN the player picked; empty for none.
    pub fn chosen(&self) -> String {
        self.inner.config.get(&VPN).as_str().unwrap_or_default().to_owned()
    }

    /// Reads the VPNs off the UI thread, with the interface downloads may use among them.
    pub async fn read(&self) -> (Vec<Vpn>, Option<String>) {
        let network = self.network();
        let finds = network.finds_vpns();
        let vpns = tokio::task::spawn_blocking(move || network.vpns()).await.unwrap_or_default();
        let route = pick(&vpns, &self.chosen(), finds);
        (vpns, route)
    }

    /// Some plugin waits for the VPN, with the setting on.
    pub fn plugin_waits(&self) -> bool {
        self.only() && self.inner.waited.load(Ordering::SeqCst)
    }

    /// Whether a plugin waited for the VPN. From now on none does.
    pub fn take_waited(&self) -> bool {
        self.inner.waited.swap(false, Ordering::SeqCst)
    }
}

/// "Download only through a VPN" for plugins: the same setting and the same VPN as the
/// downloads. It notes when it tells a plugin to wait.
#[async_trait]
impl VpnCheck for VpnRule {
    async fn check(&self) -> Result<VpnPass, VpnBlock> {
        if !self.setting() {
            return Ok(VpnPass::Open);
        }
        let network = self.network();
        if let Some(reason) = network.problem() {
            return Ok(VpnPass::Unbound { reason });
        }
        let finds = network.finds_vpns();
        let (vpns, route) = self.read().await;
        if route.is_some() {
            return Ok(VpnPass::Open);
        }
        let chosen = self.chosen();
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
            self.inner.waited.store(true, Ordering::SeqCst);
        }
        Err(block)
    }
}

/// What the player asks of the VPN setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VpnCmd {
    /// "Download only through a VPN". On is refused where the system can't bind downloads to
    /// a VPN, and says why.
    SetOnly(bool),
    /// The VPN downloads use, by its name. Empty: the first connected one, where the system
    /// finds VPNs.
    Choose(String),
    /// Connects or disconnects a VPN the system can switch, by its name.
    Connect(String),
    Disconnect(String),
    /// Adds a VPN from a provider's file (WireGuard `.conf`, OpenVPN `.ovpn`) and connects it.
    /// `~/` is the home folder.
    Import(PathBuf),
}

/// What the core does when the setting or the VPNs change: the torrents' kill switch looks at
/// once. It is awaited, so a command's effect is in place when the command returns.
pub(crate) type OnChange = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// The subsystem's state, read whole.
#[derive(Clone, Debug)]
pub(crate) struct VpnSnapshot {
    /// As last read.
    pub vpns: Arc<Vec<Vpn>>,
    /// Why the last connect, disconnect or import failed.
    pub error: Option<String>,
    pub only: bool,
    pub chosen: String,
    /// The interface downloads use among the VPNs as last read, while the setting is on.
    pub route: Option<String>,
    /// Why the setting can't be on here.
    pub problem: Option<String>,
    pub finds: bool,
    pub can_import: bool,
}

#[derive(Default)]
struct State {
    vpns: Arc<Vec<Vpn>>,
    error: Option<String>,
}

struct Shared {
    rule: VpnRule,
    /// For `~/` in a VPN's file.
    home: Option<PathBuf>,
    notifier: Notifier,
    on_change: OnChange,
    state: Mutex<State>,
}

/// The VPN setting and the VPNs. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct VpnSubsystem {
    inner: Arc<Shared>,
}

impl VpnSubsystem {
    pub fn new(rule: VpnRule, home: Option<PathBuf>, notifier: Notifier, on_change: OnChange) -> Self {
        Self { inner: Arc::new(Shared { rule, home, notifier, on_change, state: Mutex::default() }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Downloads go only through the VPN.
    pub fn only(&self) -> bool {
        self.inner.rule.only()
    }

    /// The setting and the VPNs, as they are now.
    pub fn snapshot(&self) -> VpnSnapshot {
        let (vpns, error) = {
            let state = self.state();
            (state.vpns.clone(), state.error.clone())
        };
        let network = self.inner.rule.network();
        let only = self.inner.rule.only();
        let chosen = self.inner.rule.chosen();
        let finds = network.finds_vpns();
        VpnSnapshot {
            route: if only { pick(&vpns, &chosen, finds) } else { None },
            vpns,
            error,
            only,
            chosen,
            problem: network.problem(),
            finds,
            can_import: network.can_import(),
        }
    }

    /// From now on the VPNs come from `network` (a system plugin's).
    pub fn use_network(&self, network: Arc<dyn Network>) {
        self.inner.rule.use_network(network);
    }

    /// Reads the VPNs again, off the UI thread, and keeps them for the view.
    pub async fn refresh(&self) -> Vec<Vpn> {
        let (vpns, _) = self.inner.rule.read().await;
        self.keep(vpns.clone());
        vpns
    }

    /// Keeps the VPNs as just read, for the view.
    pub fn keep(&self, vpns: Vec<Vpn>) {
        let changed = {
            let mut state = self.state();
            let changed = *state.vpns != vpns;
            state.vpns = Arc::new(vpns);
            changed
        };
        if changed {
            self.inner.notifier.changed(Area::Vpn);
        }
    }

    pub async fn handle(&self, command: VpnCmd) -> CoreResult<()> {
        match command {
            VpnCmd::SetOnly(on) => {
                if on && let Some(why) = self.inner.rule.network().problem() {
                    return Err(CoreError::Invalid(why));
                }
                self.save(&VPN_ONLY, Value::Bool(on))?;
                self.changed().await;
                Ok(())
            }
            VpnCmd::Choose(name) => {
                self.save(&VPN, Value::String(name))?;
                self.changed().await;
                Ok(())
            }
            VpnCmd::Connect(name) => self.call(move |n| n.connect(&name)).await,
            VpnCmd::Disconnect(name) => self.call(move |n| n.disconnect(&name)).await,
            VpnCmd::Import(file) => {
                let file = crate::folders::expand(&file.to_string_lossy(), self.inner.home.as_deref())
                    .ok_or_else(|| CoreError::Invalid(format!("{} isn't a full path", file.display())))?;
                self.call(move |n| n.import(&file)).await.map(|_| ())
            }
        }
    }

    fn save(&self, key: &[&str], value: Value) -> CoreResult<()> {
        self.inner.rule.inner.config.set(key, value).map_err(|e| CoreError::Io(e.to_string()))
    }

    /// Runs `call` on the network port off the UI thread, keeps its error for the setting,
    /// and reads the VPNs again.
    async fn call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn Network) -> Result<T, String> + Send + 'static,
    ) -> CoreResult<T> {
        let network = self.inner.rule.network();
        let done = tokio::task::spawn_blocking(move || call(network.as_ref()))
            .await
            .map_err(|e| CoreError::Io(e.to_string()))?;
        self.state().error = done.as_ref().err().cloned();
        self.refresh().await;
        self.changed().await;
        done.map_err(CoreError::Invalid)
    }

    /// The setting or the VPNs changed: the kill switch looks at once. A plugin that waits
    /// for the VPN gets it this way too, for example when the player picks the VPN.
    async fn changed(&self) {
        (self.inner.on_change)().await;
        self.inner.notifier.changed(Area::Vpn);
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

    /// Two VPNs, `wg0` up and `work` down, or a system that can't bind downloads at all.
    struct Vpns {
        problem: Option<String>,
    }

    impl Network for Vpns {
        fn problem(&self) -> Option<String> {
            self.problem.clone()
        }

        fn vpns(&self) -> Vec<Vpn> {
            vec![vpn("wg0", true), vpn("work", false)]
        }
    }

    /// A VPN subsystem over [`Vpns`], and how often it asked the kill switch to look.
    fn subsystem(problem: Option<&str>) -> (VpnSubsystem, Arc<std::sync::atomic::AtomicUsize>) {
        let rule =
            VpnRule::new(Arc::new(ConfigService::in_memory()), Arc::new(Vpns { problem: problem.map(str::to_owned) }));
        let looked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = looked.clone();
        let on_change: OnChange = Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        });
        let notifier = Notifier::new(crate::changes::Changes::new(), crate::EventBus::default());
        (VpnSubsystem::new(rule, None, notifier, on_change), looked)
    }

    #[tokio::test]
    async fn the_setting_on_routes_through_the_connected_vpn_and_wakes_the_kill_switch() {
        let (vpn, looked) = subsystem(None);
        vpn.handle(VpnCmd::SetOnly(true)).await.unwrap();
        vpn.refresh().await;
        let snapshot = vpn.snapshot();
        assert!(snapshot.only);
        assert_eq!(snapshot.route.as_deref(), Some("wg0"));
        assert_eq!(snapshot.vpns.len(), 2);
        assert_eq!(looked.load(Ordering::SeqCst), 1);

        vpn.handle(VpnCmd::Choose("work".into())).await.unwrap();
        assert_eq!(vpn.snapshot().route, None, "the picked VPN is down");
        assert_eq!(looked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn where_downloads_can_t_be_bound_the_setting_stays_off() {
        let (vpn, looked) = subsystem(Some("Not available on Windows"));
        assert!(vpn.handle(VpnCmd::SetOnly(true)).await.is_err());
        let snapshot = vpn.snapshot();
        assert!(!snapshot.only);
        assert_eq!(snapshot.problem.as_deref(), Some("Not available on Windows"));
        assert_eq!(looked.load(Ordering::SeqCst), 0, "nothing changed");
    }

    #[tokio::test]
    async fn a_refused_connect_keeps_why_for_the_setting() {
        let (vpn, looked) = subsystem(None);
        assert!(vpn.handle(VpnCmd::Connect("work".into())).await.is_err());
        let snapshot = vpn.snapshot();
        assert_eq!(snapshot.error.as_deref(), Some("this system can't connect a VPN: use the VPN's own app"));
        assert_eq!(snapshot.vpns.len(), 2, "the VPNs are read again");
        assert_eq!(looked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_plugins_gate_waits_for_the_picked_vpn_and_notes_it() {
        let (vpn, _) = subsystem(None);
        let rule = vpn.inner.rule.clone();
        assert!(matches!(rule.check().await, Ok(VpnPass::Open)), "the setting is off");
        vpn.handle(VpnCmd::SetOnly(true)).await.unwrap();
        vpn.handle(VpnCmd::Choose("work".into())).await.unwrap();
        assert!(matches!(rule.check().await, Err(VpnBlock::Waiting { .. })));
        assert!(rule.plugin_waits());
        assert!(rule.take_waited());
        assert!(!rule.plugin_waits(), "taken once");
    }
}
