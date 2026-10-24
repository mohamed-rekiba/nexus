//! [`FakeSystem`]: a system plugin the test controls. It has a machine's network, Wi-Fi, Bluetooth,
//! sound, power and battery, and Nexus OS's updates and installer, as plain state the test sets
//! and reads. Its long work (joining, scanning, pairing, installing) ends when the test says, or
//! at once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::{
    AudioLevel, AudioOutput, AudioSettings, Battery, BluetoothDevice, BluetoothSettings, InstallDisk, Installed,
    NetworkSettings, NetworkStatus, OsInstall, OsUpdate, OsUpdates, PluginError, PluginResult, PowerSettings, Provided,
    SystemJob, Vpn, VpnSettings, WifiNetwork,
};

use crate::StubPlugin;

/// What the fake machine is, and what was asked of it.
#[derive(Default)]
pub struct FakeMachine {
    pub network: NetworkStatus,
    /// What a plain read lists.
    pub wifi: Vec<WifiNetwork>,
    /// What a rescan finds.
    pub wifi_scanned: Vec<WifiNetwork>,
    /// The password each secured network takes.
    pub passwords: Vec<(String, String)>,
    pub forgotten: Vec<String>,
    pub bluetooth_on: bool,
    pub devices: Vec<BluetoothDevice>,
    /// What a Bluetooth scan finds.
    pub found: Vec<BluetoothDevice>,
    /// Pairing these fails with the reason.
    pub pair_refusals: Vec<(String, String)>,
    pub outputs: Vec<AudioOutput>,
    pub level: AudioLevel,
    pub battery: Option<Battery>,
    pub restarts: usize,
    pub shutdowns: usize,
    pub os_update: OsUpdate,
    pub os_checks: usize,
    pub disks: Vec<InstallDisk>,
    /// The progress an install reports, then how it ends.
    pub install_steps: Vec<(u8, String)>,
    pub install_result: Option<Result<Installed, String>>,
    pub installed_on: Vec<PathBuf>,
    pub partition_choices: usize,
    pub vpns: Vec<Vpn>,
    /// Why a capability is unavailable, by its name ("network", "bluetooth", "audio", "power").
    pub unavailable: Vec<(&'static str, String)>,
    /// Calls that never answer, by method name ("status", "devices", …).
    pub hangs: Vec<&'static str>,
    /// Calls that panic, by method name.
    pub panics: Vec<&'static str>,
    /// Long work held until the test calls [`FakeSystem::finish_jobs`].
    pub hold_jobs: bool,
    held: Vec<(Arc<dyn SystemJob>, Result<String, String>)>,
}

/// A system plugin over a [`FakeMachine`]. Clones share the machine.
#[derive(Clone, Default)]
pub struct FakeSystem {
    machine: Arc<Mutex<FakeMachine>>,
}

impl FakeSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes the machine.
    pub fn with(&self, change: impl FnOnce(&mut FakeMachine)) -> &Self {
        change(&mut self.machine.lock().unwrap());
        self
    }

    /// Reads the machine.
    pub fn read<T>(&self, read: impl FnOnce(&FakeMachine) -> T) -> T {
        read(&self.machine.lock().unwrap())
    }

    /// Ends the held long work, each as it would have.
    pub fn finish_jobs(&self) {
        let held = std::mem::take(&mut self.machine.lock().unwrap().held);
        for (job, result) in held {
            match result {
                Ok(text) => job.done(&text),
                Err(reason) => job.failed(&reason),
            }
        }
    }

    /// The plugin, `fake-system`, providing every system capability.
    pub fn plugin(&self) -> StubPlugin {
        self.plugin_with(&["network", "vpn", "bluetooth", "audio", "power", "os_updates", "os_install"])
    }

    /// The plugin, providing only `capabilities` (as the manifest names them).
    pub fn plugin_with(&self, capabilities: &[&str]) -> StubPlugin {
        let list = capabilities.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
        let manifest = format!(
            "id = \"fake-system\"\nname = \"Fake system\"\nversion = \"1.0.0\"\nkind = \"system\"\n\
             host_api = \"*\"\ncapabilities = [{list}]\n"
        );
        let names: Vec<String> = capabilities.iter().map(|c| c.to_string()).collect();
        let system = self.clone();
        StubPlugin::from_toml(&manifest).provides(move |_| {
            let has = |name: &str| names.iter().any(|n| n == name);
            let me = Arc::new(system.clone());
            Ok(Provided {
                network: has("network").then(|| me.clone() as Arc<dyn NetworkSettings>),
                vpn: has("vpn").then(|| me.clone() as Arc<dyn VpnSettings>),
                bluetooth: has("bluetooth").then(|| me.clone() as Arc<dyn BluetoothSettings>),
                audio: has("audio").then(|| me.clone() as Arc<dyn AudioSettings>),
                power: has("power").then(|| me.clone() as Arc<dyn PowerSettings>),
                os_updates: has("os_updates").then(|| me.clone() as Arc<dyn OsUpdates>),
                os_install: has("os_install").then(|| me.clone() as Arc<dyn OsInstall>),
                ..Provided::default()
            })
        })
    }

    /// Hangs or panics when the test said so for `method`.
    async fn call(&self, method: &'static str) {
        let (hang, panic) = self.read(|m| (m.hangs.contains(&method), m.panics.contains(&method)));
        if panic {
            panic!("the fake system's {method} panicked on purpose");
        }
        if hang {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    }

    fn unavailable(&self, capability: &str) -> Option<String> {
        self.read(|m| m.unavailable.iter().find(|(c, _)| *c == capability).map(|(_, why)| why.clone()))
    }

    /// Ends long work now, or holds it for [`FakeSystem::finish_jobs`].
    fn end(&self, job: Arc<dyn SystemJob>, result: Result<String, String>) {
        let mut machine = self.machine.lock().unwrap();
        if machine.hold_jobs {
            machine.held.push((job, result));
            return;
        }
        drop(machine);
        match result {
            Ok(text) => job.done(&text),
            Err(reason) => job.failed(&reason),
        }
    }
}

#[async_trait]
impl NetworkSettings for FakeSystem {
    async fn unavailable(&self) -> Option<String> {
        FakeSystem::unavailable(self, "network")
    }

    async fn status(&self) -> PluginResult<NetworkStatus> {
        self.call("status").await;
        Ok(self.read(|m| m.network.clone()))
    }

    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>> {
        self.call("wifi").await;
        Ok(self.read(|m| if rescan { m.wifi_scanned.clone() } else { m.wifi.clone() }))
    }

    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        self.call("join").await;
        job.progress(None, &format!("Joining {ssid}"));
        let wanted = self.read(|m| m.passwords.iter().find(|(s, _)| s == ssid).map(|(_, p)| p.clone()));
        let result = match (wanted, password) {
            (Some(wanted), Some(given)) if wanted == given => Ok(()),
            (Some(_), _) => Err("The password is wrong".to_owned()),
            (None, _) => Ok(()),
        };
        if result.is_ok() {
            self.with(|m| {
                let connection = nexus_api::Connection::Wifi;
                m.network = NetworkStatus { connection, name: ssid.into(), signal: Some(80) };
                for w in m.wifi.iter_mut().chain(m.wifi_scanned.iter_mut()) {
                    w.active = w.ssid == ssid;
                    w.saved |= w.ssid == ssid;
                }
            });
        }
        self.end(job, result.map(|()| "Connected".to_owned()));
        Ok(())
    }

    async fn forget(&self, ssid: &str) -> PluginResult<()> {
        self.with(|m| m.forgotten.push(ssid.into()));
        Ok(())
    }
}

impl VpnSettings for FakeSystem {
    fn vpns(&self) -> Vec<Vpn> {
        self.read(|m| m.vpns.clone())
    }
}

#[async_trait]
impl BluetoothSettings for FakeSystem {
    async fn unavailable(&self) -> Option<String> {
        FakeSystem::unavailable(self, "bluetooth")
    }

    async fn powered(&self) -> PluginResult<bool> {
        self.call("powered").await;
        Ok(self.read(|m| m.bluetooth_on))
    }

    async fn set_powered(&self, on: bool) -> PluginResult<()> {
        self.with(|m| m.bluetooth_on = on);
        Ok(())
    }

    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>> {
        self.call("devices").await;
        Ok(self.read(|m| m.devices.clone()))
    }

    async fn scan(&self, _seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Looking for devices");
        self.with(|m| {
            let found = m.found.clone();
            for device in found {
                if !m.devices.iter().any(|d| d.address == device.address) {
                    m.devices.push(device);
                }
            }
        });
        self.end(job, Ok("Done looking".into()));
        Ok(())
    }

    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Pairing");
        let refusal = self.read(|m| m.pair_refusals.iter().find(|(a, _)| a == address).map(|(_, why)| why.clone()));
        if refusal.is_none() {
            self.with(|m| {
                for d in m.devices.iter_mut().filter(|d| d.address == address) {
                    d.paired = true;
                    d.connected = true;
                }
            });
        }
        self.end(job, refusal.map_or(Ok("Connected".into()), Err));
        Ok(())
    }

    async fn connect(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.iter_mut().filter(|d| d.address == address).for_each(|d| d.connected = true));
        Ok(())
    }

    async fn disconnect(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.iter_mut().filter(|d| d.address == address).for_each(|d| d.connected = false));
        Ok(())
    }

    async fn remove(&self, address: &str) -> PluginResult<()> {
        self.with(|m| m.devices.retain(|d| d.address != address));
        Ok(())
    }
}

#[async_trait]
impl AudioSettings for FakeSystem {
    async fn unavailable(&self) -> Option<String> {
        FakeSystem::unavailable(self, "audio")
    }

    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>> {
        self.call("outputs").await;
        Ok(self.read(|m| m.outputs.clone()))
    }

    async fn set_output(&self, name: &str) -> PluginResult<()> {
        if !self.read(|m| m.outputs.iter().any(|o| o.name == name)) {
            return Err(PluginError::NotFound(format!("no output {name}")));
        }
        self.with(|m| m.outputs.iter_mut().for_each(|o| o.current = o.name == name));
        Ok(())
    }

    async fn level(&self) -> PluginResult<AudioLevel> {
        self.call("level").await;
        Ok(self.read(|m| m.level))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        self.with(|m| m.level = AudioLevel { volume, muted: false });
        Ok(())
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.with(|m| m.level.muted = muted);
        Ok(())
    }
}

#[async_trait]
impl PowerSettings for FakeSystem {
    async fn unavailable(&self) -> Option<String> {
        FakeSystem::unavailable(self, "power")
    }

    async fn restart(&self) -> PluginResult<()> {
        self.with(|m| m.restarts += 1);
        Ok(())
    }

    async fn shut_down(&self) -> PluginResult<()> {
        self.with(|m| m.shutdowns += 1);
        Ok(())
    }

    async fn battery(&self) -> PluginResult<Option<Battery>> {
        self.call("battery").await;
        Ok(self.read(|m| m.battery))
    }
}

#[async_trait]
impl OsUpdates for FakeSystem {
    async fn status(&self) -> PluginResult<OsUpdate> {
        Ok(self.read(|m| m.os_update.clone()))
    }

    async fn check_now(&self) -> PluginResult<()> {
        self.with(|m| m.os_checks += 1);
        Ok(())
    }
}

#[async_trait]
impl OsInstall for FakeSystem {
    async fn disks(&self) -> PluginResult<Vec<InstallDisk>> {
        Ok(self.read(|m| m.disks.clone()))
    }

    async fn install(&self, disk: &Path, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        let (steps, result) = self.read(|m| (m.install_steps.clone(), m.install_result.clone()));
        for (percent, text) in steps {
            job.progress(Some(percent), &text);
        }
        self.with(|m| m.installed_on.push(disk.to_path_buf()));
        let result = result.unwrap_or(Ok(Installed { edition: "minimal".into(), mok: false, nvidia_later: false }));
        let result = result.map(|done| nexus_api::serde_json::to_string(&done).expect("Installed is JSON"));
        self.end(job, result);
        Ok(())
    }

    async fn choose_partitions(&self) -> PluginResult<()> {
        self.with(|m| m.partition_choices += 1);
        Ok(())
    }
}
