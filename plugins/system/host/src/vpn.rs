//! The machine's VPNs: [`VpnSettings`], for "Download only through a VPN".
//!
//! - Linux: a VPN is a WireGuard or OpenVPN interface in `/sys/class/net`, or another layer-3
//!   tunnel. Tunnels that don't hide the player's address, such as Tailscale or Docker's, are
//!   skipped. "Connected" means up and with an address. NetworkManager (`nmcli`) adds a VPN
//!   from a provider's file, and connects and disconnects the VPNs it knows.
//! - macOS: the system keeps `utun` interfaces up for its own services with no VPN, so the
//!   adapter can't tell a VPN. It lists every interface that is up and has an address, and
//!   the player picks the one the VPN app made. There is no import: the VPN's own app
//!   connects it.
//! - Windows: nothing. librqbit 9.0.1 can't bind its connections to an interface there.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use nexus_api::{Programs, Vpn, VpnSettings};

/// Interfaces that look like tunnels but don't hide the player's address from the internet.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
const NOT_PRIVACY: [&str; 9] = ["tailscale", "zt", "docker", "virbr", "veth", "br-", "vnet", "lxc", "waydroid"];

/// The VPN calls are blocking (the host runs them off its async threads), so programs run
/// through the host's [`Programs`] on its runtime, waited for here.
pub struct Vpns {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Only Linux runs nmcli.
    programs: Arc<dyn Programs>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    runtime: tokio::runtime::Handle,
    /// NetworkManager is there, as the last [`VpnSettings::vpns`] found.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Only Linux has NetworkManager.
    nmcli: std::sync::OnceLock<bool>,
}

impl Vpns {
    /// On the host's runtime, which `init` runs on.
    pub fn new(programs: Arc<dyn Programs>) -> Self {
        Self { programs, runtime: tokio::runtime::Handle::current(), nmcli: std::sync::OnceLock::new() }
    }
}

/// A VPN interface found in `/sys/class/net`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
struct Found {
    name: String,
    kind: &'static str,
    /// Up and with an address.
    connected: bool,
}

/// The VPN interfaces under `root` (`/sys/class/net`), connected or not. `addressed` holds the
/// interfaces with an address.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
fn list_in(root: &Path, addressed: &BTreeSet<String>) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return out };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" || NOT_PRIVACY.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let dir = entry.path();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
        let kind = if read("uevent").lines().any(|l| l.trim() == "DEVTYPE=wireguard") {
            "WireGuard"
        } else if dir.join("tun_flags").exists() {
            "OpenVPN"
        } else if read("type").trim() == "65534" {
            // A layer-3 tunnel with no hardware address.
            "VPN"
        } else {
            continue;
        };
        let flags = u32::from_str_radix(read("flags").trim().trim_start_matches("0x"), 16).unwrap_or(0);
        let up = flags & 0x1 != 0;
        out.push(Found { connected: up && addressed.contains(&name), name, kind });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A VPN connection NetworkManager knows (its WireGuard or VPN plugin type).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
struct Saved {
    name: String,
    wireguard: bool,
    active: bool,
}

/// `nmcli -t -f NAME,TYPE,ACTIVE connection show`, the VPNs only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
fn parse_saved(text: &str) -> Vec<Saved> {
    text.lines()
        .filter_map(|line| {
            // A NAME can hold escaped colons; TYPE and ACTIVE can't.
            let mut parts = line.rsplitn(3, ':');
            let active = parts.next()? == "yes";
            let kind = parts.next()?;
            let name = parts.next()?.replace("\\:", ":");
            matches!(kind, "wireguard" | "vpn").then_some(Saved { name, wireguard: kind == "wireguard", active })
        })
        .collect()
}

/// The interfaces, and the connections NetworkManager knows, as one list. A connection with
/// the name of an interface is that interface. One without has no interface to bind to.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux reads it; the tests run everywhere.
fn merge(found: Vec<Found>, saved: Vec<Saved>) -> Vec<Vpn> {
    let mut vpns: Vec<Vpn> = found
        .into_iter()
        .map(|f| Vpn {
            interface: if f.connected { f.name.clone() } else { String::new() },
            switchable: saved.iter().any(|s| s.name == f.name),
            kind: f.kind.into(),
            connected: f.connected,
            name: f.name,
        })
        .collect();
    for s in saved {
        if !vpns.iter().any(|v| v.name == s.name) {
            let kind = if s.wireguard { "WireGuard" } else { "VPN" };
            vpns.push(Vpn {
                name: s.name,
                kind: kind.into(),
                connected: s.active,
                interface: String::new(),
                switchable: true,
            });
        }
    }
    vpns.sort_by(|a, b| a.name.cmp(&b.name));
    vpns
}

/// The interfaces that are up, aren't the loopback, and have an IPv4 or a global IPv6
/// address.
#[cfg(unix)]
#[allow(unsafe_code)] // getifaddrs: the list it makes is read, then freed, in this function.
fn addressed() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `list` is a writable pointer; on success it holds a list freed below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return names;
    }
    let mut cursor = list;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a node of the list getifaddrs made, not freed yet.
        let entry = unsafe { &*cursor };
        let up = entry.ifa_flags & libc::IFF_UP as libc::c_uint != 0;
        let loopback = entry.ifa_flags & libc::IFF_LOOPBACK as libc::c_uint != 0;
        if up && !loopback && !entry.ifa_addr.is_null() {
            // SAFETY: `ifa_addr` is not null, and points to a sockaddr of the family it names.
            let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
            let usable = match family {
                libc::AF_INET => true,
                libc::AF_INET6 => {
                    // SAFETY: the family is AF_INET6, so the address is a sockaddr_in6.
                    let v6 = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                    let bytes = v6.sin6_addr.s6_addr;
                    // Not link-local (fe80::/10).
                    !(bytes[0] == 0xfe && bytes[1] & 0xc0 == 0x80)
                }
                _ => false,
            };
            if usable {
                // SAFETY: `ifa_name` is a nul-terminated string in the list.
                let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }.to_string_lossy().into_owned();
                names.insert(name);
            }
        }
        cursor = entry.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and nothing reads it after this.
    unsafe { libc::freeifaddrs(list) };
    names
}

#[cfg(target_os = "linux")]
impl Vpns {
    /// Runs `nmcli` with `args`, through the host. The error is its first line of output,
    /// without "Error: ".
    fn nmcli(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let text = std::sync::Mutex::new(String::new());
        let line = |l: &str| {
            let mut text = text.lock().expect("not poisoned");
            text.push_str(l);
            text.push('\n');
        };
        let never = || false;
        let program = nexus_api::Location::System("nmcli".into());
        let code = self
            .runtime
            .block_on(self.programs.stream(&program, &args, false, &line, &never))
            .map_err(|_| "NetworkManager (nmcli) isn't installed".to_owned())?;
        let text = text.into_inner().expect("not poisoned");
        if code == 0 {
            return Ok(text);
        }
        Err(crate::parse::error_line(&text))
    }
}

#[cfg(target_os = "linux")]
impl VpnSettings for Vpns {
    fn vpns(&self) -> Vec<Vpn> {
        let found = list_in(Path::new("/sys/class/net"), &addressed());
        let saved = self.nmcli(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"]);
        let _ = self.nmcli.set(saved.is_ok());
        merge(found, saved.map(|text| parse_saved(&text)).unwrap_or_default())
    }

    fn can_import(&self) -> bool {
        self.nmcli.get().copied().unwrap_or(false)
    }

    fn import(&self, file: &Path) -> Result<String, String> {
        let kind = match file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("conf") => "wireguard",
            Some("ovpn") => "openvpn",
            _ => return Err("Choose a WireGuard .conf or OpenVPN .ovpn file".into()),
        };
        let out = self.nmcli(&["connection", "import", "type", kind, "file", &file.to_string_lossy()])?;
        // "Connection 'wg-se' (<uuid>) successfully added."
        let name = out.split('\'').nth(1).map(str::to_owned).ok_or("NetworkManager didn't name the connection")?;
        self.connect(&name)?;
        Ok(name)
    }

    fn connect(&self, name: &str) -> Result<(), String> {
        self.nmcli(&["connection", "up", "id", name]).map(|_| ())
    }

    fn disconnect(&self, name: &str) -> Result<(), String> {
        self.nmcli(&["connection", "down", "id", name]).map(|_| ())
    }
}

#[cfg(target_os = "macos")]
impl VpnSettings for Vpns {
    fn finds_vpns(&self) -> bool {
        false
    }

    fn vpns(&self) -> Vec<Vpn> {
        addressed()
            .into_iter()
            .map(|name| Vpn {
                kind: "Network".into(),
                connected: true,
                interface: name.clone(),
                switchable: false,
                name,
            })
            .collect()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl VpnSettings for Vpns {
    fn problem(&self) -> Option<String> {
        Some("Not available on Windows".into())
    }

    fn vpns(&self) -> Vec<Vpn> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh `/sys/class/net` stand-in for test `name`.
    fn fixtures(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nexus-vpn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn iface(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (f, v) in files {
            std::fs::write(dir.join(f), v).unwrap();
        }
    }

    #[test]
    fn finds_vpn_tunnels_but_not_networks_that_don_t_hide_the_address() {
        let root = fixtures("tunnels");
        let r = root.as_path();
        iface(r, "lo", &[("type", "772"), ("flags", "0x9")]);
        iface(r, "enp3s0", &[("type", "1"), ("flags", "0x1003")]);
        iface(r, "wg-se", &[("type", "65534"), ("flags", "0x91"), ("uevent", "DEVTYPE=wireguard\nINTERFACE=wg-se\n")]);
        iface(r, "tun0", &[("type", "65534"), ("flags", "0x1091"), ("tun_flags", "0x1001")]);
        // Down.
        iface(r, "proton0", &[("type", "65534"), ("flags", "0x90")]);
        iface(r, "tailscale0", &[("type", "65534"), ("flags", "0x91"), ("tun_flags", "0x1001")]);
        iface(r, "docker0", &[("type", "1"), ("flags", "0x1003")]);
        let addressed: BTreeSet<String> = ["wg-se", "tun0", "enp3s0", "tailscale0"].map(String::from).into();

        let found = list_in(r, &addressed);
        let _ = std::fs::remove_dir_all(r);

        let summary: Vec<_> = found.iter().map(|v| (v.name.as_str(), v.kind, v.connected)).collect();
        assert_eq!(summary, [("proton0", "VPN", false), ("tun0", "OpenVPN", true), ("wg-se", "WireGuard", true)]);
    }

    #[test]
    fn up_but_with_no_address_is_not_connected() {
        let root = fixtures("no-address");
        iface(&root, "wg0", &[("type", "65534"), ("flags", "0x91"), ("uevent", "DEVTYPE=wireguard\n")]);
        let found = list_in(&root, &BTreeSet::new());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(found, [Found { name: "wg0".into(), kind: "WireGuard", connected: false }]);
    }

    #[test]
    fn reads_networkmanager_s_vpn_connections() {
        let text = "Wired connection 1:802-3-ethernet:yes\nwg-se:wireguard:yes\nWork\\: office:vpn:no\nhome wifi:802-11-wireless:no\n";
        assert_eq!(
            parse_saved(text),
            [
                Saved { name: "wg-se".into(), wireguard: true, active: true },
                Saved { name: "Work: office".into(), wireguard: false, active: false },
            ]
        );
    }

    #[test]
    fn a_saved_connection_with_an_interface_s_name_is_that_interface() {
        let found = vec![
            Found { name: "wg-se".into(), kind: "WireGuard", connected: true },
            Found { name: "tun0".into(), kind: "OpenVPN", connected: true },
        ];
        let saved = vec![
            Saved { name: "wg-se".into(), wireguard: true, active: true },
            Saved { name: "Work".into(), wireguard: false, active: false },
        ];
        let vpns = merge(found, saved);
        let summary: Vec<_> =
            vpns.iter().map(|v| (v.name.as_str(), v.connected, v.interface.as_str(), v.switchable)).collect();
        assert_eq!(summary, [("Work", false, "", true), ("tun0", true, "tun0", false), ("wg-se", true, "wg-se", true)]);
    }

    /// Opt-in, `NEXUS_VPN_TESTS=1`: reads this computer's interfaces. It connects nothing.
    #[cfg(unix)]
    #[test]
    fn lists_this_computer_s_networks() {
        if std::env::var_os("NEXUS_VPN_TESTS").is_none() {
            return;
        }
        let found = list_in(Path::new("/sys/class/net"), &addressed());
        assert!(found.iter().all(|v| !v.name.is_empty()), "{found:?}");
        println!("{found:?}");
    }
}
