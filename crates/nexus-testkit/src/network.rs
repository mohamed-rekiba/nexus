//! The computer's VPNs, as the test sets them: they connect and drop when the test says.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nexus_core::{Network, Vpn};

struct State {
    vpns: Vec<Vpn>,
    finds: bool,
    problem: Option<String>,
    imports: bool,
    /// Every connect, disconnect and import, as "connect wg0".
    calls: Vec<String>,
    /// The next connect, disconnect or import fails with this.
    refuse: Option<String>,
}

/// Fake VPNs. With none set, the computer has no VPN.
#[derive(Clone)]
pub struct FakeNetwork {
    state: Arc<Mutex<State>>,
}

impl Default for FakeNetwork {
    fn default() -> Self {
        let state =
            State { vpns: Vec::new(), finds: true, problem: None, imports: true, calls: Vec::new(), refuse: None };
        Self { state: Arc::new(Mutex::new(state)) }
    }
}

impl FakeNetwork {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// The WireGuard VPN `name` connects, on the interface of the same name.
    pub fn up(&self, name: &str) {
        self.set(name, true);
    }

    /// The VPN `name` drops. It stays in the list, not connected.
    pub fn down(&self, name: &str) {
        self.set(name, false);
    }

    fn set(&self, name: &str, connected: bool) {
        let mut state = self.lock();
        let vpn = Vpn {
            name: name.into(),
            kind: "WireGuard".into(),
            connected,
            interface: if connected { name.into() } else { String::new() },
            switchable: true,
        };
        match state.vpns.iter_mut().find(|v| v.name == name) {
            Some(old) => *old = vpn,
            None => state.vpns.push(vpn),
        }
    }

    /// The system can't tell VPNs from other networks, as on macOS: the player picks one.
    pub fn picks_only(&self) {
        self.lock().finds = false;
    }

    /// The system can't bind downloads to a VPN, and says why.
    pub fn unavailable(&self, why: &str) {
        self.lock().problem = Some(why.into());
    }

    /// The next connect, disconnect or import fails with `error`.
    pub fn refuse(&self, error: &str) {
        self.lock().refuse = Some(error.into());
    }

    /// Every connect, disconnect and import so far, as "connect wg0".
    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    fn call(&self, call: String) -> Result<(), String> {
        let mut state = self.lock();
        state.calls.push(call);
        state.refuse.take().map_or(Ok(()), Err)
    }
}

impl Network for FakeNetwork {
    fn problem(&self) -> Option<String> {
        self.lock().problem.clone()
    }

    fn finds_vpns(&self) -> bool {
        self.lock().finds
    }

    fn vpns(&self) -> Vec<Vpn> {
        self.lock().vpns.clone()
    }

    fn can_import(&self) -> bool {
        self.lock().imports
    }

    /// The file's name without its extension becomes a VPN, connected.
    fn import(&self, file: &Path) -> Result<String, String> {
        self.call(format!("import {}", file.display()))?;
        let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.up(&name);
        Ok(name)
    }

    fn connect(&self, name: &str) -> Result<(), String> {
        self.call(format!("connect {name}"))?;
        self.up(name);
        Ok(())
    }

    fn disconnect(&self, name: &str) -> Result<(), String> {
        self.call(format!("disconnect {name}"))?;
        self.down(name);
        Ok(())
    }
}
