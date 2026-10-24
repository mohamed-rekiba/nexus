//! Linux: NetworkManager (`nmcli`), BlueZ (`bluetoothctl`), PipeWire's PulseAudio server
//! (`pactl`), systemd (`systemctl`), the kernel's power supply folder, and on Nexus OS
//! `rpm-ostree` and the USB stick's installer. Every program runs through the host's
//! [`Programs`], so the manifest's `permissions.process` decides what may run.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nexus_api::{
    AudioLevel, AudioOutput, AudioSettings, Battery, BluetoothDevice, BluetoothSettings, Connection, InstallDisk,
    Location, NetworkSettings, NetworkStatus, OsInstall, OsUpdate, OsUpdates, PluginError, PluginResult, PowerSettings,
    Programs, SystemJob, WifiNetwork,
};

use crate::parse;

/// How long a quick read may take: the host's own limit.
const QUICK: Duration = nexus_api::MAX_PROGRAM_TIME;
/// Nexus OS's marker, and its USB stick's installer.
const NEXUS_OS: &str = "/usr/lib/nexus-os/release";
const INSTALLER: &str = "/usr/libexec/nexus-os/install";
/// The service that looks for a Nexus OS update (a polkit rule lets the player start it).
const UPDATE_SERVICE: &str = "nexus-os-update.service";

#[derive(Clone)]
pub struct Linux {
    programs: Arc<dyn Programs>,
    /// Where the power supplies are: `/sys/class/power_supply`, or a test's folder.
    power_supply: std::path::PathBuf,
    /// Where Bluetooth adapters are: `/sys/class/bluetooth`.
    bluetooth_class: std::path::PathBuf,
}

impl Linux {
    pub fn new(programs: Arc<dyn Programs>) -> Self {
        Self {
            programs,
            power_supply: "/sys/class/power_supply".into(),
            bluetooth_class: "/sys/class/bluetooth".into(),
        }
    }

    /// Runs a system tool briefly, and returns what it printed.
    async fn run(&self, program: &str, args: &[&str]) -> PluginResult<String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = self.programs.output(&Location::System(program.into()), &args, QUICK).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Whether a system tool is installed.
    async fn has(&self, program: &str) -> bool {
        self.programs.find(&Location::System(program.into())).await.ok().flatten().is_some()
    }

    /// Runs a tool line by line, with no time limit, until it ends or `cancelled` says so; its
    /// exit code and its last line.
    async fn exec(
        &self,
        program: &str,
        args: &[&str],
        elevated: bool,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
        each: impl Fn(&str) + Send + Sync,
    ) -> PluginResult<(i32, String)> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let last = std::sync::Mutex::new(String::new());
        let line = |text: &str| {
            if !text.trim().is_empty() {
                *last.lock().expect("not poisoned") = text.trim().to_owned();
            }
            each(text);
        };
        let program = Location::System(program.into());
        let code = self.programs.stream(&program, &args, elevated, &line, cancelled).await?;
        let last = last.into_inner().expect("not poisoned");
        Ok((code, last))
    }

    /// [`Linux::exec`] for a job, which the player can cancel.
    async fn stream(
        &self,
        program: &str,
        args: &[&str],
        elevated: bool,
        job: &Arc<dyn SystemJob>,
        each: impl Fn(&str) + Send + Sync,
    ) -> PluginResult<(i32, String)> {
        self.exec(program, args, elevated, &|| job.cancelled(), each).await
    }

    /// Runs a tool that changes something, and fails with its error when it says no.
    async fn act(&self, program: &str, args: &[&str]) -> PluginResult<()> {
        let (code, last) = self.exec(program, args, false, &|| false, |_| {}).await?;
        if code == 0 { Ok(()) } else { Err(PluginError::Other(parse::error_line(&last))) }
    }

    /// Runs `work` in the background, ending `job` with its outcome.
    fn background(job: Arc<dyn SystemJob>, work: impl Future<Output = Result<String, String>> + Send + 'static) {
        tokio::spawn(async move {
            match work.await {
                Ok(text) => job.done(&text),
                Err(_) if job.cancelled() => job.failed("Cancelled"),
                Err(reason) => job.failed(&reason),
            }
        });
    }

    async fn missing(&self, program: &str, what: &str) -> Option<String> {
        (!self.has(program).await).then(|| format!("{what} ({program}) isn't installed"))
    }
}

fn failed(error: PluginError) -> String {
    match error {
        PluginError::NotFound(_) => "the system's tool isn't installed".into(),
        other => other.to_string(),
    }
}

#[nexus_api::async_trait]
impl NetworkSettings for Linux {
    async fn unavailable(&self) -> Option<String> {
        self.missing("nmcli", "NetworkManager").await
    }

    async fn status(&self) -> PluginResult<NetworkStatus> {
        let mut status = parse::network(&self.run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).await?);
        if status.connection == Connection::Wifi {
            let list =
                self.run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list"]).await?;
            status.signal = parse::wifi(&list).into_iter().find(|w| w.in_use).map(|w| w.signal);
        }
        Ok(status)
    }

    async fn wifi(&self, rescan: bool) -> PluginResult<Vec<WifiNetwork>> {
        let rescan = if rescan { "yes" } else { "no" };
        let args = ["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", rescan];
        let list = parse::wifi(&self.run("nmcli", &args).await?);
        let saved = parse::saved_wifi(&self.run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).await?);
        Ok(list
            .into_iter()
            .map(|w| WifiNetwork {
                saved: saved.contains(&w.ssid),
                ssid: w.ssid,
                signal: w.signal,
                secured: w.secured,
                active: w.in_use,
            })
            .collect())
    }

    async fn join(&self, ssid: &str, password: Option<&str>, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, &format!("Joining {ssid}"));
        let me = self.clone();
        let (ssid, password) = (ssid.to_owned(), password.filter(|p| !p.is_empty()).map(str::to_owned));
        let handle = job.clone();
        Linux::background(job, async move {
            let mut args = vec!["--wait", "45", "device", "wifi", "connect", ssid.as_str()];
            if let Some(password) = &password {
                args.extend(["password", password.as_str()]);
            }
            let (code, last) = me.stream("nmcli", &args, false, &handle, |_| {}).await.map_err(failed)?;
            if code == 0 {
                Ok("Connected".into())
            } else if last.contains("Secrets were required") || last.contains("802-11-wireless-security") {
                Err("The password is wrong".into())
            } else {
                Err(parse::error_line(&last))
            }
        });
        Ok(())
    }

    async fn forget(&self, ssid: &str) -> PluginResult<()> {
        self.act("nmcli", &["connection", "delete", "id", ssid]).await
    }
}

#[nexus_api::async_trait]
impl BluetoothSettings for Linux {
    async fn unavailable(&self) -> Option<String> {
        if let Some(missing) = self.missing("bluetoothctl", "BlueZ").await {
            return Some(missing);
        }
        let adapter = std::fs::read_dir(&self.bluetooth_class)
            .is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("hci")));
        (!adapter).then(|| "This PC has no Bluetooth adapter".into())
    }

    async fn powered(&self) -> PluginResult<bool> {
        Ok(parse::powered(&self.run("bluetoothctl", &["show"]).await?))
    }

    async fn set_powered(&self, on: bool) -> PluginResult<()> {
        self.act("bluetoothctl", &["power", if on { "on" } else { "off" }]).await
    }

    async fn devices(&self) -> PluginResult<Vec<BluetoothDevice>> {
        let all = self.run("bluetoothctl", &["devices"]).await?;
        let paired = self.run("bluetoothctl", &["devices", "Paired"]).await?;
        let connected = self.run("bluetoothctl", &["devices", "Connected"]).await?;
        Ok(parse::bluetooth(&all, &paired, &connected))
    }

    async fn scan(&self, seconds: u32, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Looking for devices: put yours in pairing mode");
        let me = self.clone();
        let handle = job.clone();
        Linux::background(job, async move {
            let _ = me.run("bluetoothctl", &["power", "on"]).await;
            let seconds = seconds.to_string();
            me.stream("bluetoothctl", &["--timeout", &seconds, "scan", "on"], false, &handle, |_| {})
                .await
                .map_err(failed)?;
            Ok("Done looking".into())
        });
        Ok(())
    }

    async fn pair(&self, address: &str, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(None, "Pairing");
        let me = self.clone();
        let address = address.to_owned();
        let handle = job.clone();
        Linux::background(job, async move {
            let steps: [(&str, &[&str]); 3] = [
                ("Pairing", &["--timeout", "25", "pair", &address]),
                // Trusted, it connects again by itself.
                ("Trusting", &["trust", &address]),
                ("Connecting", &["--timeout", "15", "connect", &address]),
            ];
            for (what, args) in steps {
                handle.progress(None, what);
                let (code, last) = me.stream("bluetoothctl", args, false, &handle, |_| {}).await.map_err(failed)?;
                let refused = ["Failed", "not available", "org.bluez.Error"].iter().any(|w| last.contains(w));
                if code != 0 || refused {
                    return Err(format!("{what} didn't work: {}", parse::error_line(&last)));
                }
            }
            Ok("Connected".into())
        });
        Ok(())
    }

    async fn connect(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["--timeout", "8", "connect", address]).await
    }

    async fn disconnect(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["disconnect", address]).await
    }

    async fn remove(&self, address: &str) -> PluginResult<()> {
        self.act("bluetoothctl", &["remove", address]).await
    }
}

#[nexus_api::async_trait]
impl AudioSettings for Linux {
    async fn unavailable(&self) -> Option<String> {
        self.missing("pactl", "PipeWire's sound tools").await
    }

    async fn outputs(&self) -> PluginResult<Vec<AudioOutput>> {
        let sinks = self.run("pactl", &["-f", "json", "list", "sinks"]).await?;
        let default = self.run("pactl", &["get-default-sink"]).await?;
        Ok(parse::outputs(&sinks, &default))
    }

    async fn set_output(&self, name: &str) -> PluginResult<()> {
        self.act("pactl", &["set-default-sink", name]).await
    }

    async fn level(&self) -> PluginResult<AudioLevel> {
        let volume = self.run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await?;
        let mute = self.run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).await?;
        Ok(parse::level(&volume, &mute))
    }

    async fn set_volume(&self, volume: u8) -> PluginResult<()> {
        let volume = format!("{}%", volume.min(100));
        self.act("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"]).await?;
        self.act("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &volume]).await
    }

    async fn set_muted(&self, muted: bool) -> PluginResult<()> {
        self.act("pactl", &["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]).await
    }
}

#[nexus_api::async_trait]
impl PowerSettings for Linux {
    async fn unavailable(&self) -> Option<String> {
        self.missing("systemctl", "systemd").await
    }

    async fn restart(&self) -> PluginResult<()> {
        self.act("systemctl", &["reboot"]).await
    }

    async fn shut_down(&self) -> PluginResult<()> {
        self.act("systemctl", &["poweroff"]).await
    }

    async fn battery(&self) -> PluginResult<Option<Battery>> {
        Ok(parse::battery(&self.power_supply))
    }
}

#[nexus_api::async_trait]
impl OsUpdates for Linux {
    async fn status(&self) -> PluginResult<OsUpdate> {
        if !Path::new(NEXUS_OS).exists() {
            return Err(PluginError::NotFound("this isn't Nexus OS".into()));
        }
        let json = self.run("rpm-ostree", &["status", "--json"]).await?;
        match parse::staged(&json) {
            Some(Some(version)) => Ok(OsUpdate::Staged { version }),
            Some(None) => Ok(OsUpdate::Current),
            None => Ok(OsUpdate::Failed { reason: "rpm-ostree's status couldn't be read".into() }),
        }
    }

    async fn check_now(&self) -> PluginResult<()> {
        self.act("systemctl", &["start", "--no-block", UPDATE_SERVICE]).await
    }
}

#[nexus_api::async_trait]
impl OsInstall for Linux {
    async fn disks(&self) -> PluginResult<Vec<InstallDisk>> {
        if !Path::new(INSTALLER).exists() {
            return Ok(Vec::new());
        }
        // The stick has the installer on its PATH as nexus-os-install (docs/nexus-os.md).
        Ok(parse::disks(&self.run("nexus-os-install", &["--disks"]).await?))
    }

    async fn install(&self, disk: &Path, job: Arc<dyn SystemJob>) -> PluginResult<()> {
        job.progress(Some(0), "Getting ready");
        let me = self.clone();
        let disk = disk.display().to_string();
        let handle = job.clone();
        Linux::background(job, async move {
            let end = std::sync::Mutex::new(None);
            let each = |line: &str| match parse::install_line(line) {
                Some(parse::InstallLine::Progress(percent, text)) => handle.progress(Some(percent), &text),
                Some(other) => *end.lock().expect("not poisoned") = Some(other),
                None => {}
            };
            let (code, _) = me.stream("nexus-os-install", &[&disk], true, &handle, each).await.map_err(failed)?;
            match end.into_inner().expect("not poisoned") {
                Some(parse::InstallLine::Done(installed)) => {
                    Ok(nexus_api::serde_json::to_string(&installed).expect("Installed is JSON"))
                }
                Some(parse::InstallLine::Failed(reason)) => Err(reason),
                _ if code == 126 || code == 127 => Err("The installer wasn't allowed to run".into()),
                _ => Err("The installer stopped without finishing".into()),
            }
        });
        Ok(())
    }

    async fn choose_partitions(&self) -> PluginResult<()> {
        // The installer knows how this stick reaches Anaconda (see docs/nexus-os.md).
        let (code, last) = self.exec("nexus-os-install", &["--choose-partitions"], true, &|| false, |_| {}).await?;
        if code == 0 { Ok(()) } else { Err(PluginError::Other(parse::error_line(&last))) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Programs that are all missing: what a system without the tools looks like.
    struct Nothing;

    #[nexus_api::async_trait]
    impl Programs for Nothing {
        async fn find(&self, _program: &Location) -> PluginResult<Option<String>> {
            Ok(None)
        }

        async fn output(&self, program: &Location, _args: &[String], _timeout: Duration) -> PluginResult<Vec<u8>> {
            Err(PluginError::NotFound(format!("{program} isn't there")))
        }
    }

    #[tokio::test]
    async fn without_the_tools_each_setting_says_which_one_is_missing() {
        let linux = Linux::new(Arc::new(Nothing));
        assert_eq!(
            NetworkSettings::unavailable(&linux).await.as_deref(),
            Some("NetworkManager (nmcli) isn't installed")
        );
        assert_eq!(
            BluetoothSettings::unavailable(&linux).await.as_deref(),
            Some("BlueZ (bluetoothctl) isn't installed")
        );
        assert_eq!(
            AudioSettings::unavailable(&linux).await.as_deref(),
            Some("PipeWire's sound tools (pactl) isn't installed")
        );
        assert!(NetworkSettings::status(&linux).await.is_err());
    }
}
