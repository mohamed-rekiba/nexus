//! What a system plugin provides: the machine's own settings and devices. Network and Wi-Fi,
//! VPNs, Bluetooth, sound, power and battery, and on Nexus OS its updates and its installer.
//! Each is one capability and one trait; the core uses the first active system plugin that
//! provides it, and shows a capability nobody provides as unavailable.
//!
//! Quick calls answer within the host's call timeout. Long work (joining a network, a
//! Bluetooth scan, pairing, installing the OS) starts in the background and returns at once;
//! it reports through the [`SystemJob`] the host passes in.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PluginResult;

/// A long piece of work, as the host follows it. Each method may be called from any thread.
pub trait SystemJob: Send + Sync {
    /// How far it is: a percent when the plugin knows one, and what it's doing.
    fn progress(&self, percent: Option<u8>, text: &str);
    /// It ended well; `text` says how ("Connected").
    fn done(&self, text: &str);
    /// It ended badly; `reason` says why, for the player.
    fn failed(&self, reason: &str);
    /// The player cancelled it: stop at the next chance, then call [`SystemJob::failed`].
    fn cancelled(&self) -> bool;
}

/// How the machine is online.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    #[default]
    None,
    Wired,
    Wifi,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub connection: Connection,
    /// The Wi-Fi network's name, or the wired connection's.
    pub name: String,
    /// The Wi-Fi signal, 0 to 100.
    pub signal: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0 to 100.
    pub signal: u8,
    /// It needs a password.
    pub secured: bool,
    /// The machine has joined it before and joins it again by itself.
    pub saved: bool,
    /// The machine is on it now.
    pub active: bool,
}

/// Network state and Wi-Fi.
#[async_trait]
pub trait NetworkSettings: Send + Sync {
    /// Why it can't be used here ("NetworkManager isn't installed"), or `None`.
    async fn unavailable(&self) -> Option<String> {
        None
    }
    async fn status(&self) -> PluginResult<NetworkStatus>;
    /// The Wi-Fi networks in reach, strongest first. `rescan` looks again first, which takes a
    /// few seconds.
    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>>;
    /// Starts joining `ssid`, with its password when it's secured and not saved.
    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()>;
    /// Forgets a saved network: it isn't joined by itself any more.
    async fn forget(&self, ssid: &str) -> PluginResult<()>;
}

/// A VPN, or on a system that can't tell VPNs from other networks, any network with an
/// address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vpn {
    /// The name the player knows it by. Connect and disconnect take it.
    pub name: String,
    /// "WireGuard", "OpenVPN", "VPN", or "Network" when the system can't tell.
    pub kind: String,
    /// Up and with an address: traffic can go through it.
    pub connected: bool,
    /// The network interface that downloads go through while it is connected. Empty when the
    /// system doesn't name one.
    pub interface: String,
    /// The system can connect and disconnect it.
    pub switchable: bool,
}

/// VPNs, for "Download only through a VPN". The core keeps the rule itself (which interface
/// torrents bind to, pausing them when the VPN drops); this lists and switches the VPNs.
/// Blocking calls: the host runs them off its async threads.
pub trait VpnSettings: Send + Sync {
    /// Why downloads can't go only through a VPN here, for example "Not available on
    /// Windows". `None` when they can.
    fn problem(&self) -> Option<String> {
        None
    }
    /// The system tells VPNs from other networks. When it can't (macOS), [`VpnSettings::vpns`]
    /// lists every network with an address, and the player picks the one the VPN uses.
    fn finds_vpns(&self) -> bool {
        true
    }
    fn vpns(&self) -> Vec<Vpn>;
    /// The system can add a VPN from a provider's file.
    fn can_import(&self) -> bool {
        false
    }
    /// Adds a VPN from a provider's file (WireGuard `.conf`, OpenVPN `.ovpn`) and connects it.
    /// Returns its name.
    fn import(&self, _file: &std::path::Path) -> Result<String, String> {
        Err("this system can't add a VPN: connect it with the VPN's own app".into())
    }
    fn connect(&self, _name: &str) -> Result<(), String> {
        Err("this system can't connect a VPN: use the VPN's own app".into())
    }
    fn disconnect(&self, _name: &str) -> Result<(), String> {
        Err("this system can't disconnect a VPN: use the VPN's own app".into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

/// Bluetooth: controllers and headsets.
#[async_trait]
pub trait BluetoothSettings: Send + Sync {
    /// Why it can't be used here ("no Bluetooth adapter"), or `None`.
    async fn unavailable(&self) -> Option<String> {
        None
    }
    async fn powered(&self) -> PluginResult<bool>;
    async fn set_powered(&self, on: bool) -> PluginResult<()>;
    /// Paired devices, and the ones the last scan found.
    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>>;
    /// Starts looking for devices to pair for `seconds`.
    async fn scan(&self, seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()>;
    /// Starts pairing, trusting and connecting a device the scan found.
    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()>;
    async fn connect(&self, address: &str) -> PluginResult<()>;
    async fn disconnect(&self, address: &str) -> PluginResult<()>;
    /// Forgets a paired device.
    async fn remove(&self, address: &str) -> PluginResult<()>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutput {
    /// The system's name for it, which [`AudioSettings::set_output`] takes.
    pub name: String,
    /// What the player sees: "HDMI / DisplayPort", "Headset".
    pub description: String,
    pub current: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLevel {
    /// 0 to 100.
    pub volume: u8,
    pub muted: bool,
}

/// Sound: where it goes and how loud.
#[async_trait]
pub trait AudioSettings: Send + Sync {
    async fn unavailable(&self) -> Option<String> {
        None
    }
    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>>;
    async fn set_output(&self, name: &str) -> PluginResult<()>;
    /// The current output's level.
    async fn level(&self) -> PluginResult<AudioLevel>;
    async fn set_volume(&self, volume: u8) -> PluginResult<()>;
    async fn set_muted(&self, muted: bool) -> PluginResult<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    /// 0 to 100.
    pub percent: u8,
    pub charging: bool,
}

/// Power: restarting, shutting down, and a laptop's battery.
#[async_trait]
pub trait PowerSettings: Send + Sync {
    /// Why restart and shut down can't be used here, or `None`. The battery still shows.
    async fn unavailable(&self) -> Option<String> {
        None
    }
    async fn restart(&self) -> PluginResult<()>;
    async fn shut_down(&self) -> PluginResult<()>;
    /// `None` on a PC without a battery.
    async fn battery(&self) -> PluginResult<Option<Battery>>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum OsUpdate {
    #[default]
    Current,
    Checking,
    /// Downloaded: a restart puts it in.
    Staged {
        version: String,
    },
    Failed {
        reason: String,
    },
}

/// Nexus OS's updates: the system downloads them by itself, and a restart puts one in.
#[async_trait]
pub trait OsUpdates: Send + Sync {
    async fn status(&self) -> PluginResult<OsUpdate>;
    /// Looks for an update now rather than at the next daily check.
    async fn check_now(&self) -> PluginResult<()>;
}

/// A disk Nexus OS can be installed on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallDisk {
    pub path: PathBuf,
    pub bytes: u64,
    /// Windows is on it.
    pub windows: bool,
    pub usb: bool,
    pub model: String,
}

/// How an install ended well.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    /// The edition installed: `minimal` or `minimal-nvidia`.
    pub edition: String,
    /// Secure Boot's key screen comes at the restart.
    pub mok: bool,
    /// The PC has an NVIDIA card and gets its driver on the installed system.
    pub nvidia_later: bool,
}

/// Installing Nexus OS from its USB stick.
#[async_trait]
pub trait OsInstall: Send + Sync {
    async fn disks(&self) -> PluginResult<Vec<InstallDisk>>;
    /// Starts erasing `disk` and installing Nexus OS on it. The job's `done` text is
    /// [`Installed`] as JSON.
    async fn install(&self, disk: &std::path::Path, job: Arc<dyn SystemJob>) -> PluginResult<()>;
    /// Starts the installer that lets the player choose partitions (Anaconda). It may restart
    /// the PC into it.
    async fn choose_partitions(&self) -> PluginResult<()>;
}
