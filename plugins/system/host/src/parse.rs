//! What the system's tools print, read into the system types. Pure functions over text, so the
//! tests run on recorded output anywhere.

use std::path::{Path, PathBuf};

use nexus_api::{AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus};

/// One line of `nmcli -t` output: fields are `:`-separated, with `\:` inside a field.
pub fn nm_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut().expect("one field at least").extend(chars.next()),
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("one field at least").push(c),
        }
    }
    fields
}

/// `nmcli -t -f TYPE,STATE,CONNECTION device`: a cable is used before Wi-Fi.
pub fn network(text: &str) -> NetworkStatus {
    let rows: Vec<Vec<String>> = text.lines().map(nm_fields).collect();
    let connected = |kind: &str| rows.iter().find(|f| f.len() >= 3 && f[0] == kind && f[1] == "connected");
    if let Some(f) = connected("ethernet") {
        return NetworkStatus { connection: Connection::Wired, name: f[2].clone(), signal: None };
    }
    match connected("wifi") {
        Some(f) => NetworkStatus { connection: Connection::Wifi, name: f[2].clone(), signal: None },
        None => NetworkStatus::default(),
    }
}

/// A Wi-Fi network as `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    pub signal: u8,
    pub secured: bool,
    pub in_use: bool,
}

/// Nearby Wi-Fi networks, the one in use first, then strongest first, one entry per name;
/// hidden networks (no name) are left out.
pub fn wifi(text: &str) -> Vec<Wifi> {
    let mut list: Vec<Wifi> = text
        .lines()
        .map(nm_fields)
        .filter(|f| f.len() >= 4 && !f[1].is_empty())
        .map(|f| Wifi {
            in_use: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secured: !f[3].is_empty() && f[3] != "--",
        })
        .collect();
    list.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    let mut seen = std::collections::HashSet::new();
    list.retain(|w| seen.insert(w.ssid.clone()));
    list
}

/// The names of the saved Wi-Fi connections, from `nmcli -t -f NAME,TYPE connection show`.
pub fn saved_wifi(text: &str) -> Vec<String> {
    text.lines().map(nm_fields).filter(|f| f.len() >= 2 && f[1] == "802-11-wireless").map(|f| f[0].clone()).collect()
}

/// The first line of a tool's error, without `Error: `.
pub fn error_line(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it failed");
    first.trim_start_matches("Error: ").to_owned()
}

/// `bluetoothctl devices` (and `devices Paired`, `devices Connected`): address and name.
/// Unnamed devices show their address again, with dashes; they aren't worth listing.
pub fn devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            (address.len() == 17 && !name.trim().is_empty() && name.replace('-', ":") != address)
                .then(|| (address.to_owned(), name.trim().to_owned()))
        })
        .collect()
}

/// Every device, from the three lists: all known, paired, connected. Paired first.
pub fn bluetooth(all: &str, paired: &str, connected: &str) -> Vec<BluetoothDevice> {
    let paired: Vec<String> = devices(paired).into_iter().map(|(a, _)| a).collect();
    let connected: Vec<String> = devices(connected).into_iter().map(|(a, _)| a).collect();
    let mut list: Vec<BluetoothDevice> = devices(all)
        .into_iter()
        .map(|(address, name)| BluetoothDevice {
            paired: paired.contains(&address),
            connected: connected.contains(&address),
            address,
            name,
        })
        .collect();
    list.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.cmp(&b.name)));
    list
}

/// `bluetoothctl show`: whether the adapter is on.
pub fn powered(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "Powered: yes")
}

/// `pactl -f json list sinks` and `pactl get-default-sink`: the outputs, named for the player.
pub fn outputs(json: &str, default: &str) -> Vec<AudioOutput> {
    let Ok(nexus_api::serde_json::Value::Array(sinks)) = nexus_api::serde_json::from_str(json) else {
        return Vec::new();
    };
    let default = default.trim();
    sinks
        .iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?.to_owned();
            let description = s["description"].as_str().unwrap_or(&name);
            let port = s["active_port"].as_str().unwrap_or("");
            // "GA102 High Definition Audio Controller Digital Stereo (HDMI)" says less than this.
            let description = if port.starts_with("hdmi") || description.contains("HDMI") {
                "TV or monitor (HDMI)".to_owned()
            } else if port.contains("headphones") {
                "Headphones".to_owned()
            } else {
                description.replace(" Analog Stereo", "").replace(" Digital Stereo", "")
            };
            Some(AudioOutput { current: name == default, name, description })
        })
        .collect()
}

/// `pactl get-sink-volume @DEFAULT_SINK@` and `pactl get-sink-mute @DEFAULT_SINK@`.
pub fn level(volume: &str, mute: &str) -> AudioLevel {
    let percent = volume
        .split('/')
        .map(str::trim)
        .find_map(|part| part.strip_suffix('%').and_then(|n| n.trim().parse::<u32>().ok()))
        .unwrap_or(0);
    AudioLevel { volume: percent.min(100) as u8, muted: mute.trim() == "Mute: yes" }
}

/// The PC's own batteries under `root` (`/sys/class/power_supply`), as one: laptops with two
/// add up. A controller's or a mouse's battery (`scope=Device`) isn't the PC's.
pub fn battery(root: &Path) -> Option<Battery> {
    let read = |dir: &Path, file: &str| std::fs::read_to_string(dir.join(file)).ok().map(|s| s.trim().to_owned());
    let (mut now, mut full, mut percents) = (0u64, 0u64, Vec::new());
    let (mut charging, mut found) = (false, false);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for dir in entries {
        match read(&dir, "type").as_deref() {
            Some("Mains") | Some("USB") if read(&dir, "online").as_deref() == Some("1") => charging = true,
            Some("Battery") if read(&dir, "scope").as_deref() != Some("Device") => {
                // A removable battery taken out says present=0.
                if read(&dir, "present").as_deref() == Some("0") {
                    continue;
                }
                found = true;
                if matches!(read(&dir, "status").as_deref(), Some("Charging" | "Full" | "Not charging")) {
                    charging = true;
                }
                let num = |f: &str| read(&dir, f).and_then(|s| s.parse::<u64>().ok());
                match (num("energy_now").or(num("charge_now")), num("energy_full").or(num("charge_full"))) {
                    (Some(n), Some(f)) if f > 0 => {
                        now += n;
                        full += f;
                    }
                    _ => percents.extend(num("capacity")),
                }
            }
            _ => {}
        }
    }
    if !found {
        return None;
    }
    // From the energy left of the full charge, or else the average of the batteries' own figures.
    let percent = (now * 100)
        .checked_div(full)
        .or_else(|| percents.iter().sum::<u64>().checked_div(percents.len() as u64))?
        .min(100);
    Some(Battery { percent: percent as u8, charging })
}

#[cfg_attr(target_os = "linux", allow(dead_code))] // macOS reads it; the tests run everywhere.
/// macOS's `pmset -g batt`: `… -InternalBattery-0 (id=…) 76%; discharging; 4:12 remaining …`
/// (a tab before the percent).
pub fn pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let percent = line.split(['\t', ' ', ';']).find_map(|w| w.strip_suffix('%')?.parse::<u8>().ok())?;
    let charging = line.contains("; charging") || line.contains("; charged") || line.contains("AC attached");
    Some(Battery { percent: percent.min(100), charging })
}

/// The installer's `--disks`: `disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…`.
pub fn disks(text: &str) -> Vec<InstallDisk> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(6, ' ');
            if f.next()? != "disk" {
                return None;
            }
            let path = PathBuf::from(f.next()?);
            let bytes = f.next()?.parse().ok()?;
            let windows = f.next()? == "1";
            let usb = f.next()? == "1";
            let model = f.next().unwrap_or("Disk").replace('_', " ").trim().to_owned();
            Some(InstallDisk { path, bytes, windows, usb, model })
        })
        .collect()
}

/// One line the installer printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallLine {
    Progress(u8, String),
    Done(Installed),
    Failed(String),
}

/// `progress PERCENT TEXT`, `done EDITION [mok] [nvidia-later]` or `error TEXT`; anything else
/// is the installer's own chatter.
pub fn install_line(line: &str) -> Option<InstallLine> {
    let line = line.trim();
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    match word {
        "progress" => {
            let (percent, text) = rest.split_once(' ').unwrap_or((rest, ""));
            let percent = percent.parse::<f32>().ok()?.clamp(0.0, 100.0) as u8;
            Some(InstallLine::Progress(percent, text.to_owned()))
        }
        "done" => {
            let mut words = rest.split_whitespace();
            let edition = words.next().unwrap_or("minimal").to_owned();
            let flags: Vec<&str> = words.collect();
            Some(InstallLine::Done(Installed {
                edition,
                mok: flags.contains(&"mok"),
                nvidia_later: flags.contains(&"nvidia-later"),
            }))
        }
        "error" => Some(InstallLine::Failed(rest.to_owned())),
        _ => None,
    }
}

/// `rpm-ostree status --json`: the version a restart starts, when one is staged.
pub fn staged(json: &str) -> Option<Option<String>> {
    let v: nexus_api::serde_json::Value = nexus_api::serde_json::from_str(json).ok()?;
    let deployments = v["deployments"].as_array()?;
    let version = |d: &nexus_api::serde_json::Value| {
        d["version"].as_str().map(str::to_owned).unwrap_or_else(|| {
            let digest = d["container-image-reference-digest"].as_str().unwrap_or_default();
            digest.chars().rev().take(7).collect::<Vec<_>>().into_iter().rev().collect()
        })
    };
    deployments.iter().find(|d| d["booted"].as_bool() == Some(true))?;
    Some(deployments.iter().find(|d| d["staged"].as_bool() == Some(true)).map(version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_networkmanager_s_devices_a_cable_first() {
        let wifi = network("ethernet:unavailable:\nwifi:connected:Home\\: 5G\nloopback:connected (externally):lo\n");
        assert_eq!((wifi.connection, wifi.name.as_str()), (Connection::Wifi, "Home: 5G"));
        let cable = network("ethernet:connected:Wired connection 1\nwifi:connected:Home\n");
        assert_eq!((cable.connection, cable.name.as_str()), (Connection::Wired, "Wired connection 1"));
        assert_eq!(network("wifi:disconnected:\n"), NetworkStatus::default());
    }

    #[test]
    fn reads_wifi_networks_one_per_name_in_use_and_strongest_first() {
        let list = wifi(
            ":Cafe:40:\n*:Home\\: 5G:78:WPA2\n:Neighbour:90:WPA1 WPA2\n:Neighbour:30:WPA2\n::70:WPA2\n:Old:20:WEP\n",
        );
        let seen: Vec<_> = list.iter().map(|w| (w.ssid.as_str(), w.signal, w.secured, w.in_use)).collect();
        assert_eq!(
            seen,
            [
                ("Home: 5G", 78, true, true),
                ("Neighbour", 90, true, false),
                ("Cafe", 40, false, false),
                ("Old", 20, true, false)
            ]
        );
        assert_eq!(saved_wifi("Home\\: 5G:802-11-wireless\nWired connection 1:802-3-ethernet\n"), ["Home: 5G"]);
    }

    #[test]
    fn reads_bluetooth_devices_and_skips_unnamed_ones() {
        let all = "Device 70:5A:6F:6D:EF:70 EarFun Wave Pro\nDevice A0:5A:5C:11:22:33 DualSense Wireless Controller\n\
                   Device 4C:11:22:33:44:55 4C-11-22-33-44-55\n";
        let list = bluetooth(all, "Device A0:5A:5C:11:22:33 DualSense Wireless Controller\n", "");
        let seen: Vec<_> = list.iter().map(|d| (d.name.as_str(), d.paired, d.connected)).collect();
        assert_eq!(seen, [("DualSense Wireless Controller", true, false), ("EarFun Wave Pro", false, false)]);
        assert!(powered("Controller 00:1A:7D:DA:71:13 (public)\n\tPowered: yes\n\tDiscoverable: no\n"));
        assert!(!powered("\tPowered: no\n"));
    }

    #[test]
    fn reads_sound_outputs_named_for_the_player() {
        let json = r#"[{"name":"alsa_output.usb-Shure.analog-stereo","description":"Shure MV7 Analog Stereo","active_port":"analog-output"},
            {"name":"alsa_output.pci.hdmi-stereo","description":"GA102 High Definition Audio Controller Digital Stereo (HDMI)","active_port":"hdmi-output-0"},
            {"name":"bluez_output.70_5A.1","description":"EarFun Wave Pro","active_port":"headset-output"}]"#;
        let list = outputs(json, "alsa_output.pci.hdmi-stereo\n");
        let seen: Vec<_> = list.iter().map(|o| (o.description.as_str(), o.current)).collect();
        assert_eq!(seen, [("Shure MV7", false), ("TV or monitor (HDMI)", true), ("EarFun Wave Pro", false)]);
        let volume = "Volume: front-left: 26214 /  40% / -23.87 dB,   front-right: 26214 /  40% / -23.87 dB\n";
        assert_eq!(level(volume, "Mute: no\n"), AudioLevel { volume: 40, muted: false });
        assert_eq!(
            level("Volume: mono: 98304 / 150% / 10.57 dB", "Mute: yes"),
            AudioLevel { volume: 100, muted: true }
        );
    }

    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
    }

    #[test]
    fn reads_the_pc_s_batteries_but_not_a_controller_s() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(battery(root.path()), None, "a desktop PC");
        supply(root.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        supply(
            root.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Discharging"), ("energy_now", "30"), ("energy_full", "100")],
        );
        supply(
            root.path(),
            "BAT1",
            &[("type", "Battery"), ("status", "Discharging"), ("energy_now", "10"), ("energy_full", "100")],
        );
        supply(root.path(), "ps-controller-battery", &[("type", "Battery"), ("scope", "Device"), ("capacity", "5")]);
        assert_eq!(battery(root.path()), Some(Battery { percent: 20, charging: false }));
        supply(root.path(), "AC", &[("type", "Mains"), ("online", "1")]);
        assert_eq!(battery(root.path()), Some(Battery { percent: 20, charging: true }));
        let mac = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t76%; discharging; 4:12 remaining present: true\n";
        assert_eq!(pmset(mac), Some(Battery { percent: 76, charging: false }));
        assert_eq!(pmset("Now drawing from 'AC Power'\n"), None, "a Mac without a battery");
    }

    #[test]
    fn reads_the_installer_s_disks_and_lines() {
        let list = disks(
            "disk /dev/nvme0n1 512110190592 1 0 Samsung_SSD 970 EVO\ndisk /dev/sdb 2000398934016 0 1 CT2000\nnoise\n",
        );
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].model.as_str(), list[0].bytes, list[0].windows, list[0].usb),
            ("Samsung SSD 970 EVO", 512110190592, true, false)
        );
        assert!(list[1].usb);
        assert_eq!(
            install_line("progress 35 Copying Nexus OS"),
            Some(InstallLine::Progress(35, "Copying Nexus OS".into()))
        );
        let done = |edition: &str, mok, nvidia_later| {
            Some(InstallLine::Done(Installed { edition: edition.into(), mok, nvidia_later }))
        };
        assert_eq!(install_line("done minimal-nvidia mok"), done("minimal-nvidia", true, false));
        assert_eq!(install_line("done minimal nvidia-later"), done("minimal", false, true));
        assert_eq!(
            install_line("error Installing on /dev/sda failed."),
            Some(InstallLine::Failed("Installing on /dev/sda failed.".into()))
        );
        assert_eq!(install_line("Deploying container image"), None);
    }

    #[test]
    fn reads_a_staged_system_update() {
        let json = r#"{"deployments":[{"booted":false,"staged":true,"version":"44.20261008"},{"booted":true,"version":"44.20261007"}]}"#;
        assert_eq!(staged(json), Some(Some("44.20261008".into())));
        assert_eq!(staged(r#"{"deployments":[{"booted":true,"version":"44.1"}]}"#), Some(None));
        assert_eq!(staged("not json"), None);
    }
}
