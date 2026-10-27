//! The machine's own settings, from the system plugins: network and Wi-Fi, Bluetooth, sound,
//! power and battery, and on Nexus OS its updates and its installer. For each capability the
//! core uses the first active system plugin that provides it; without one, the capability is
//! unavailable. The state is read again every few seconds and after each action, and long work
//! (joining a network, scanning, pairing, installing) runs as a job the view model lists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nexus_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Capability, InstallDisk, NetworkStatus, OsUpdate, PluginId,
    PluginKind, PluginResult, Provided, SystemJob, WifiNetwork,
};

use crate::changes::Area;
use crate::guard::guarded;
use crate::{Core, CoreError, CoreResult};

/// How often the state is read again while nothing happens.
pub(crate) const REFRESH_EVERY: Duration = Duration::from_secs(5);
/// How long a Bluetooth scan looks for devices.
const BLUETOOTH_SCAN_SECONDS: u32 = 10;
/// A finished job stays listed this long, so the theme can say how it ended.
const JOB_KEPT: Duration = Duration::from_secs(8);

/// Where Nexus runs, which decides what the theme shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// Nexus is the whole screen, with no desktop behind it: the Nexus session or Nexus OS.
    /// The network, sound and Bluetooth controls show then.
    pub whole_screen: bool,
    /// Nexus OS (its marker, `/usr/lib/nexus-os/release`).
    pub nexus_os: bool,
    /// Nexus OS started from its USB stick, which can install it.
    pub stick: bool,
}

impl Place {
    /// This run's place: `NEXUS_SESSION=1` (set by the Nexus session), Nexus OS's marker, and
    /// the stick's live kernel argument with its installer. `NEXUS_OS=1` and `NEXUS_USB=1`
    /// pretend, for screenshots.
    pub fn detect() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
        let nexus_os = set("NEXUS_OS") || Path::new(crate::launcher_update::NEXUS_OS_MARKER).exists();
        let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let live = cmdline.split_whitespace().any(|a| a == "rd.live.image");
        let stick = set("NEXUS_USB") || (nexus_os && live && Path::new(INSTALLER).exists());
        Place { whole_screen: set("NEXUS_SESSION") || nexus_os || stick, nexus_os: nexus_os || stick, stick }
    }
}

/// The installer on Nexus OS's USB stick.
pub const INSTALLER: &str = "/usr/libexec/nexus-os/install";

/// What long work a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    JoinWifi,
    ScanBluetooth,
    Pair,
    InstallOs,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::JoinWifi => "join",
            JobKind::ScanBluetooth => "scan",
            JobKind::Pair => "pair",
            JobKind::InstallOs => "install",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct JobState {
    pub id: u64,
    pub kind: JobKind,
    /// What it works on: the network's name, the device's address, the disk.
    pub target: String,
    pub phase: JobPhase,
    pub percent: Option<u8>,
    pub text: String,
    pub(crate) cancel: Arc<AtomicBool>,
}

/// What the core knows of the machine.
#[derive(Clone, Debug, Default)]
pub struct SystemState {
    pub network: Option<NetworkStatus>,
    pub wifi: Vec<WifiNetwork>,
    pub bluetooth_on: bool,
    pub devices: Vec<BluetoothDevice>,
    pub outputs: Vec<AudioOutput>,
    pub level: Option<AudioLevel>,
    pub battery: Option<Battery>,
    pub os_update: Option<OsUpdate>,
    pub disks: Vec<InstallDisk>,
    /// Each capability a system plugin provides: `None` when it works, else why not.
    pub capabilities: BTreeMap<Capability, Option<String>>,
    pub jobs: Vec<JobState>,
    pub(crate) next_job: u64,
}

impl SystemState {
    /// Why `capability` can't be used: nobody provides it, or its plugin says why.
    pub fn unavailable(&self, capability: Capability) -> Option<String> {
        match self.capabilities.get(&capability) {
            Some(reason) => reason.clone(),
            None => Some("Nexus can't change this on this system".into()),
        }
    }
}

/// The first active system plugin that provides a capability, and that capability.
async fn provider<T: ?Sized>(core: &Core, pick: impl Fn(&Provided) -> Option<Arc<T>>) -> Option<(PluginId, Arc<T>)> {
    let kernel = core.kernel();
    for plugin in kernel.of_kind(PluginKind::System) {
        if let Ok(provided) = kernel.provided(&plugin).await
            && let Some(capability) = pick(&provided)
        {
            return Some((plugin, capability));
        }
    }
    None
}

/// A call into a system plugin, under the guard: a timeout and the panic catch.
async fn call<T>(
    core: &Core,
    plugin: &PluginId,
    method: &'static str,
    future: impl Future<Output = PluginResult<T>>,
) -> CoreResult<T> {
    guarded(core.kernel().clock().as_ref(), plugin, method, core.kernel().host().call_timeout, future)
        .await
        .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })
}

/// The job handle a plugin reports through: it changes the job in the state, and reads the
/// state again when the job ends.
struct HostJob {
    core: std::sync::Weak<crate::app::Inner>,
    id: u64,
    runtime: tokio::runtime::Handle,
    cancel: Arc<AtomicBool>,
}

impl HostJob {
    fn change(&self, change: impl FnOnce(&mut JobState)) {
        let Some(inner) = self.core.upgrade() else { return };
        let core = Core { inner };
        let ended = {
            let mut state = core.state();
            let Some(job) = state.system.jobs.iter_mut().find(|j| j.id == self.id) else { return };
            if job.phase != JobPhase::Running {
                return;
            }
            change(job);
            job.phase != JobPhase::Running
        };
        core.changed(Area::System);
        if ended {
            let id = self.id;
            self.runtime.spawn(async move {
                refresh(&core).await;
                core.kernel().clock().sleep(JOB_KEPT).await;
                core.state().system.jobs.retain(|j| j.id != id || j.phase == JobPhase::Running);
                core.changed(Area::System);
            });
        }
    }
}

impl SystemJob for HostJob {
    fn progress(&self, percent: Option<u8>, text: &str) {
        self.change(|job| {
            job.percent = percent.map(|p| p.min(100));
            job.text = text.to_owned();
        });
    }

    fn done(&self, text: &str) {
        self.change(|job| {
            job.phase = JobPhase::Done;
            job.percent = Some(100);
            job.text = text.to_owned();
        });
    }

    fn failed(&self, reason: &str) {
        self.change(|job| {
            job.phase = JobPhase::Failed;
            job.text = reason.to_owned();
        });
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// A new running job, and the handle its plugin reports through.
fn start_job(core: &Core, kind: JobKind, target: &str) -> (u64, Arc<dyn SystemJob>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let id = {
        let mut state = core.state();
        state.system.next_job += 1;
        let id = state.system.next_job;
        state.system.jobs.push(JobState {
            id,
            kind,
            target: target.to_owned(),
            phase: JobPhase::Running,
            percent: None,
            text: String::new(),
            cancel: cancel.clone(),
        });
        id
    };
    core.changed(Area::System);
    let job = HostJob { core: Arc::downgrade(&core.inner), id, runtime: tokio::runtime::Handle::current(), cancel };
    (id, Arc::new(job))
}

/// A job that couldn't start fails with the reason.
fn failed_to_start(job: &Arc<dyn SystemJob>, error: &CoreError) {
    job.failed(&error.to_string());
}

/// The VPNs come from the system plugin that provides them, if one does; the app's port stays
/// otherwise. Called at boot, before downloads resume, and at each read.
pub(crate) async fn adopt_vpn(core: &Core) {
    if let Some((_, vpn)) = provider(core, |p| p.vpn.clone()).await {
        core.vpn().use_network(Arc::new(crate::ports::PluginVpn(vpn)));
    }
}

/// Reads every system capability again.
pub(crate) async fn refresh(core: &Core) {
    adopt_vpn(core).await;
    let mut found = SystemState::default();
    if let Some((plugin, network)) = provider(core, |p| p.network.clone()).await {
        let reason = call(core, &plugin, "unavailable", async { Ok(network.unavailable().await) }).await;
        let reason = reason.unwrap_or(None);
        if reason.is_none() {
            found.network = call(core, &plugin, "status", network.status()).await.ok();
            found.wifi = call(core, &plugin, "wifi", network.wifi(false)).await.unwrap_or_default();
        }
        found.capabilities.insert(Capability::Network, reason);
    }
    if let Some((plugin, bluetooth)) = provider(core, |p| p.bluetooth.clone()).await {
        let reason = call(core, &plugin, "unavailable", async { Ok(bluetooth.unavailable().await) }).await;
        let reason = reason.unwrap_or(None);
        if reason.is_none() {
            found.bluetooth_on = call(core, &plugin, "powered", bluetooth.powered()).await.unwrap_or(false);
            found.devices = call(core, &plugin, "devices", bluetooth.devices()).await.unwrap_or_default();
        }
        found.capabilities.insert(Capability::Bluetooth, reason);
    }
    if let Some((plugin, audio)) = provider(core, |p| p.audio.clone()).await {
        let reason = call(core, &plugin, "unavailable", async { Ok(audio.unavailable().await) }).await;
        let reason = reason.unwrap_or(None);
        if reason.is_none() {
            found.outputs = call(core, &plugin, "outputs", audio.outputs()).await.unwrap_or_default();
            found.level = call(core, &plugin, "level", audio.level()).await.ok();
        }
        found.capabilities.insert(Capability::Audio, reason);
    }
    if let Some((plugin, power)) = provider(core, |p| p.power.clone()).await {
        let reason = call(core, &plugin, "unavailable", async { Ok(power.unavailable().await) }).await;
        let reason = reason.unwrap_or(None);
        found.battery = call(core, &plugin, "battery", power.battery()).await.ok().flatten();
        found.capabilities.insert(Capability::Power, reason);
    }
    if let Some((plugin, updates)) = provider(core, |p| p.os_updates.clone()).await {
        found.os_update = call(core, &plugin, "status", updates.status()).await.ok();
        found.capabilities.insert(Capability::OsUpdates, None);
    }
    if core.kernel().host().place.stick
        && let Some((plugin, install)) = provider(core, |p| p.os_install.clone()).await
    {
        found.disks = call(core, &plugin, "disks", install.disks()).await.unwrap_or_default();
        found.capabilities.insert(Capability::OsInstall, None);
    }
    {
        let mut state = core.state();
        let system = &mut state.system;
        // A scan found more networks than the plain read: keep them until the next scan.
        if found.wifi.is_empty() {
            found.wifi = std::mem::take(&mut system.wifi);
        }
        found.jobs = std::mem::take(&mut system.jobs);
        found.next_job = system.next_job;
        *system = found;
    }
    core.changed(Area::System);
}

/// Reads the state again every [`REFRESH_EVERY`], while some system plugin is on.
pub(crate) fn schedule_refresh(core: &Core) {
    let weak = Arc::downgrade(&core.inner);
    tokio::spawn(async move {
        loop {
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            if !core.kernel().of_kind(PluginKind::System).is_empty() {
                refresh(&core).await;
            }
            let wait = core.kernel().clock().sleep(REFRESH_EVERY);
            drop(core);
            wait.await;
        }
    });
}

/// What the player asks of the machine.
#[derive(Clone, Debug, PartialEq)]
pub enum SystemCommand {
    /// Reads everything again now.
    Refresh,
    /// Looks for Wi-Fi networks again (a few seconds).
    ScanWifi,
    /// Joins a Wi-Fi network, with its password when it's secured and not saved.
    JoinWifi {
        ssid: String,
        password: Option<String>,
    },
    ForgetWifi(String),
    SetBluetooth(bool),
    /// Looks for devices to pair.
    ScanBluetooth,
    Pair(String),
    ConnectDevice(String),
    DisconnectDevice(String),
    RemoveDevice(String),
    SetOutput(String),
    /// 0 to 100.
    SetVolume(u8),
    SetMuted(bool),
    Restart,
    ShutDown,
    /// Asks Nexus OS to look for an update now.
    CheckOsUpdate,
    /// Erases the disk and installs Nexus OS on it (the USB stick only).
    InstallOs(PathBuf),
    /// Starts the installer that lets the player choose partitions.
    ChoosePartitions,
    /// Cancels a running job, by its id.
    CancelJob(u64),
}

fn missing(capability: Capability) -> CoreError {
    CoreError::Invalid(format!("no system plugin provides {capability:?} here"))
}

pub(crate) async fn run(core: &Core, command: SystemCommand) -> CoreResult<()> {
    use SystemCommand as C;
    match command {
        C::Refresh => {}
        C::ScanWifi => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            let wifi = call(core, &plugin, "wifi", network.wifi(true)).await?;
            core.state().system.wifi = wifi;
            core.changed(Area::System);
            return Ok(());
        }
        C::JoinWifi { ssid, password } => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            let (_, job) = start_job(core, JobKind::JoinWifi, &ssid);
            if let Err(e) = call(core, &plugin, "join", network.join(&ssid, password.as_deref(), job.clone())).await {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ForgetWifi(ssid) => {
            let (plugin, network) = provider(core, |p| p.network.clone()).await.ok_or(missing(Capability::Network))?;
            call(core, &plugin, "forget", network.forget(&ssid)).await?;
        }
        C::SetBluetooth(on) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            call(core, &plugin, "set_powered", bt.set_powered(on)).await?;
        }
        C::ScanBluetooth => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            let (_, job) = start_job(core, JobKind::ScanBluetooth, "");
            if let Err(e) = call(core, &plugin, "scan", bt.scan(BLUETOOTH_SCAN_SECONDS, job.clone())).await {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::Pair(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            let (_, job) = start_job(core, JobKind::Pair, &address);
            if let Err(e) = call(core, &plugin, "pair", bt.pair(&address, job.clone())).await {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ConnectDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            call(core, &plugin, "connect", bt.connect(&address)).await?;
        }
        C::DisconnectDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            call(core, &plugin, "disconnect", bt.disconnect(&address)).await?;
        }
        C::RemoveDevice(address) => {
            let (plugin, bt) = provider(core, |p| p.bluetooth.clone()).await.ok_or(missing(Capability::Bluetooth))?;
            call(core, &plugin, "remove", bt.remove(&address)).await?;
        }
        C::SetOutput(name) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            call(core, &plugin, "set_output", audio.set_output(&name)).await?;
        }
        C::SetVolume(volume) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            let volume = volume.min(100);
            call(core, &plugin, "set_volume", audio.set_volume(volume)).await?;
            // Shown at once: the next read confirms it.
            if let Some(level) = core.state().system.level.as_mut() {
                level.volume = volume;
                level.muted = false;
            }
            core.changed(Area::System);
        }
        C::SetMuted(muted) => {
            let (plugin, audio) = provider(core, |p| p.audio.clone()).await.ok_or(missing(Capability::Audio))?;
            call(core, &plugin, "set_muted", audio.set_muted(muted)).await?;
            if let Some(level) = core.state().system.level.as_mut() {
                level.muted = muted;
            }
            core.changed(Area::System);
        }
        C::Restart => {
            let (plugin, power) = provider(core, |p| p.power.clone()).await.ok_or(missing(Capability::Power))?;
            return call(core, &plugin, "restart", power.restart()).await;
        }
        C::ShutDown => {
            let (plugin, power) = provider(core, |p| p.power.clone()).await.ok_or(missing(Capability::Power))?;
            return call(core, &plugin, "shut_down", power.shut_down()).await;
        }
        C::CheckOsUpdate => {
            let (plugin, updates) =
                provider(core, |p| p.os_updates.clone()).await.ok_or(missing(Capability::OsUpdates))?;
            call(core, &plugin, "check_now", updates.check_now()).await?;
        }
        C::InstallOs(disk) => {
            if !core.kernel().host().place.stick {
                return Err(CoreError::Invalid("Nexus OS installs from its USB stick only".into()));
            }
            let (plugin, install) =
                provider(core, |p| p.os_install.clone()).await.ok_or(missing(Capability::OsInstall))?;
            let (_, job) = start_job(core, JobKind::InstallOs, &disk.display().to_string());
            if let Err(e) = call(core, &plugin, "install", install.install(&disk, job.clone())).await {
                failed_to_start(&job, &e);
            }
            return Ok(());
        }
        C::ChoosePartitions => {
            let (plugin, install) =
                provider(core, |p| p.os_install.clone()).await.ok_or(missing(Capability::OsInstall))?;
            return call(core, &plugin, "choose_partitions", install.choose_partitions()).await;
        }
        C::CancelJob(id) => {
            let state = core.state();
            let job = state.system.jobs.iter().find(|j| j.id == id && j.phase == JobPhase::Running);
            let job = job.ok_or_else(|| CoreError::Invalid(format!("no running job {id}")))?;
            job.cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
    }
    refresh(core).await;
    Ok(())
}
