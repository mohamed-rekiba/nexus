//! The launcher's own update. A release publishes `latest.json`: its version, and each
//! package's address and SHA-256 by kind of install (`macos-aarch64`, `linux-x86_64`,
//! `linux-x86_64-appimage`, `linux-x86_64-rpm`, `windows-x86_64-setup`, …: [`package_key`]).
//! The core reads it from the repo in `app.update_repo` ("owner/name", by default
//! `mohamed-rekiba/nexus`), downloads this install's package and checks its SHA-256. Then it
//! goes in the way the install takes ([`Install`]): kept in `updates/` with `ready.json` for the
//! app to swap in or run at its next start ([`apply_staged`]), or handed to the system's
//! package manager. On Nexus OS the system updates the launcher, so nothing is downloaded.
//!
//! The check and the download are a subsystem ([`LauncherUpdateSubsystem`]), in the shape of
//! [`crate::torrents`]: its own state, its HTTP and login session ports, `app.update_repo`, and its
//! changes as [`Area::LauncherUpdate`]. The rest of this module runs before the core starts:
//! [`apply_staged`] swaps a kept update in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use nexus_api::HttpRequest;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::{CoreError, CoreResult, HttpTransport, LoginSession};

/// Where a release's files are served from, after GitHub's redirects.
const HOSTS: [&str; 3] = ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

/// Where the launcher's update is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UpdateState {
    /// No `app.update_repo`: no checks.
    Off,
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
        asset: Asset,
    },
    Downloading {
        version: String,
    },
    /// Kept in `updates/`: it goes in at the next start.
    Ready {
        version: String,
    },
    /// The package manager installed it: a restart starts it.
    Installed {
        version: String,
    },
    /// Nexus OS: it comes with the next system update.
    WithSystem {
        version: String,
    },
    Failed(String),
}

/// How this launcher was installed, which decides its package in `latest.json` and how an
/// update goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// A build that isn't installed, such as `cargo run`'s: it reads this system's package,
    /// but the app never swaps it in.
    Unknown,
    /// The unpacked tar.gz on Linux.
    Folder,
    AppImage,
    /// `Nexus.app` on macOS.
    MacApp,
    /// The unpacked zip on Windows.
    WindowsZip,
    /// Windows' installer: the new installer runs at the next start.
    WindowsSetup,
    Deb,
    Rpm,
    Arch,
    /// The launcher of Nexus OS, which the system updates.
    NexusOs,
}

/// Written by Nexus OS's image.
pub const NEXUS_OS_MARKER: &str = "/usr/lib/nexus-os/release";
/// Beside the binary in a deb, rpm or Arch package: its format.
pub const PACKAGED_MARKER: &str = "packaged";
/// Beside `nexus.exe` when Windows' installer put it there.
pub const SETUP_MARKER: &str = "installed-by-setup";

impl Install {
    /// This run's install, from the environment and the files beside the binary.
    pub fn detect() -> Self {
        if std::env::var_os("APPIMAGE").is_some() {
            return Install::AppImage;
        }
        let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else {
            return Install::Unknown;
        };
        Self::of(&dir, Path::new(NEXUS_OS_MARKER).exists())
    }

    /// The install whose binary is in `dir`; `nexus_os` when the system is Nexus OS.
    pub fn of(dir: &Path, nexus_os: bool) -> Self {
        if let Ok(format) = std::fs::read_to_string(dir.join(PACKAGED_MARKER)) {
            if nexus_os {
                return Install::NexusOs;
            }
            return match format.trim() {
                "deb" => Install::Deb,
                "rpm" => Install::Rpm,
                "arch" => Install::Arch,
                _ => Install::Unknown,
            };
        }
        if dir.join(SETUP_MARKER).is_file() {
            return Install::WindowsSetup;
        }
        if dir.ends_with("Contents/MacOS") {
            return Install::MacApp;
        }
        if dir.join("LICENSE").is_file() && dir.join("extensions").is_dir() {
            return if cfg!(windows) { Install::WindowsZip } else { Install::Folder };
        }
        Install::Unknown
    }

    /// How an update goes in: `swap` at the next start, `installer` run at the next start,
    /// `package` through the package manager, or `system` with Nexus OS's update.
    pub fn method(self) -> &'static str {
        match self {
            Install::WindowsSetup => "installer",
            Install::Deb | Install::Rpm | Install::Arch => "package",
            Install::NexusOs => "system",
            Install::Unknown | Install::Folder | Install::AppImage | Install::MacApp | Install::WindowsZip => "swap",
        }
    }

    fn manager(self) -> Option<crate::PackageManager> {
        match self {
            Install::Rpm => Some(crate::PackageManager::Dnf),
            Install::Deb => Some(crate::PackageManager::Apt),
            Install::Arch => Some(crate::PackageManager::Pacman),
            _ => None,
        }
    }
}

/// What the app keeps for its next start.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Staged {
    pub version: String,
    /// The package file, in `updates/`.
    pub file: PathBuf,
}

/// The install's key in `latest.json`. `None` on Nexus OS, which downloads nothing.
pub fn package_key(install: Install) -> Option<String> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = std::env::consts::ARCH;
    Some(match install {
        Install::Unknown | Install::Folder | Install::MacApp | Install::WindowsZip => format!("{os}-{arch}"),
        Install::AppImage => format!("linux-{arch}-appimage"),
        Install::Deb => format!("linux-{arch}-deb"),
        Install::Rpm => format!("linux-{arch}-rpm"),
        Install::Arch => format!("linux-{arch}-arch"),
        Install::WindowsSetup => format!("windows-{arch}-setup"),
        Install::NexusOs => return None,
    })
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.contains('@')).then_some(host)
}

/// Where Nexus's releases are, unless `app.update_repo` says otherwise.
pub const DEFAULT_REPO: &str = "mohamed-rekiba/nexus";
/// The setting that names the repo.
const UPDATE_REPO: [&str; 2] = ["app", "update_repo"];
/// This launcher's version.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn newer(candidate: &str, current: &str) -> bool {
    match (semver::Version::parse(candidate.trim_start_matches('v')), semver::Version::parse(current)) {
        (Ok(c), Ok(now)) => c > now,
        _ => false,
    }
}

/// What the player asks of the launcher's update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LauncherUpdateCmd {
    /// Reads the newest release of the launcher (`app.update_repo`).
    Check,
    /// Downloads the newer launcher the check found, for the next start.
    Download,
}

/// The ports the launcher's update uses, and nothing else.
pub(crate) struct LauncherUpdatePorts {
    pub http: Arc<dyn HttpTransport>,
    /// Hands a package to the system's package manager.
    pub login_session: Arc<dyn LoginSession>,
}

/// Where this launcher keeps its update, and how it was installed.
pub(crate) struct ThisLauncher {
    /// The data folder: the update goes in its `updates/`.
    pub data_dir: PathBuf,
    pub install: Install,
}

/// The subsystem's state, read whole.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LauncherUpdateSnapshot {
    pub state: UpdateState,
    /// No release repo is set: no checks.
    pub off: bool,
    /// How an update goes in ([`Install::method`]).
    pub method: &'static str,
    /// This launcher's version.
    pub current: &'static str,
}

struct Shared {
    ports: LauncherUpdatePorts,
    here: ThisLauncher,
    /// For `app.update_repo`, which the subsystem reads and never writes.
    config: Arc<ConfigService>,
    notifier: Notifier,
    state: Mutex<UpdateState>,
}

/// The launcher's own update. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct LauncherUpdateSubsystem {
    inner: Arc<Shared>,
}

impl LauncherUpdateSubsystem {
    pub fn new(ports: LauncherUpdatePorts, here: ThisLauncher, config: Arc<ConfigService>, notifier: Notifier) -> Self {
        let state = Mutex::new(UpdateState::Idle);
        Self { inner: Arc::new(Shared { ports, here, config, notifier, state }) }
    }

    fn state(&self) -> MutexGuard<'_, UpdateState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn set(&self, state: UpdateState) {
        *self.state() = state;
        self.inner.notifier.changed(Area::LauncherUpdate);
    }

    /// Where the update is, as it is now.
    pub fn snapshot(&self) -> LauncherUpdateSnapshot {
        LauncherUpdateSnapshot {
            state: self.state().clone(),
            off: self.repo().is_none(),
            method: self.inner.here.install.method(),
            current: CURRENT,
        }
    }

    pub async fn handle(&self, command: LauncherUpdateCmd) -> CoreResult<()> {
        match command {
            LauncherUpdateCmd::Check => {
                self.check().await;
                Ok(())
            }
            LauncherUpdateCmd::Download => self.download().await,
        }
    }

    /// Reads the newest release, and returns what it found.
    pub async fn check(&self) -> UpdateState {
        self.set(UpdateState::Checking);
        let found = self.read_latest().await;
        self.set(found.clone());
        found
    }

    /// Downloads the update the check found.
    async fn download(&self) -> CoreResult<()> {
        let UpdateState::Available { version, asset } = self.state().clone() else {
            return Err(CoreError::Invalid("no launcher update to download".into()));
        };
        self.set(UpdateState::Downloading { version: version.clone() });
        let done = self.update(&version, &asset).await;
        self.set(done.unwrap_or_else(|e| UpdateState::Failed(e.to_string())));
        Ok(())
    }

    /// The repo to read releases from, "owner/name": `app.update_repo`, or [`DEFAULT_REPO`]
    /// when it isn't set. An empty `app.update_repo` turns the checks off.
    fn repo(&self) -> Option<String> {
        let value = self.inner.config.get(&UPDATE_REPO);
        let repo = value.as_str().map(str::trim).unwrap_or(DEFAULT_REPO);
        (repo.split('/').count() == 2 && !repo.contains(' ') && !repo.starts_with('/') && !repo.ends_with('/'))
            .then(|| repo.to_owned())
    }

    /// GET, following redirects only to GitHub's release hosts.
    async fn get(&self, url: &str) -> CoreResult<Vec<u8>> {
        let mut url = url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            if !host_of(&url).is_some_and(|h| HOSTS.contains(&h)) {
                return Err(CoreError::Invalid(format!("updates come from GitHub only, not {url}")));
            }
            let response = self
                .inner
                .ports
                .http
                .send(HttpRequest::get(url.clone()))
                .await
                .map_err(|e| CoreError::Io(e.to_string()))?;
            if (300..400).contains(&response.status) {
                let location =
                    response.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("location")).map(|(_, v)| v.clone());
                url = location.ok_or_else(|| CoreError::Io("a redirect without a Location".into()))?;
                continue;
            }
            if response.status != 200 {
                return Err(CoreError::Io(format!("{url}: HTTP {}", response.status)));
            }
            return Ok(response.body);
        }
        Err(CoreError::Io("too many redirects".into()))
    }

    /// Reads the newest release's `latest.json`.
    async fn read_latest(&self) -> UpdateState {
        let Some(repo) = self.repo() else { return UpdateState::Off };
        let url = format!("https://github.com/{repo}/releases/latest/download/latest.json");
        let manifest = match self
            .get(&url)
            .await
            .and_then(|b| serde_json::from_slice::<Manifest>(&b).map_err(|e| CoreError::Io(e.to_string())))
        {
            Ok(m) => m,
            Err(e) => return UpdateState::Failed(e.to_string()),
        };
        if !newer(&manifest.version, CURRENT) {
            return UpdateState::UpToDate;
        }
        let Some(key) = package_key(self.inner.here.install) else {
            return UpdateState::WithSystem { version: manifest.version };
        };
        match manifest.assets.get(&key) {
            Some(asset) => UpdateState::Available { version: manifest.version, asset: asset.clone() },
            None => UpdateState::Failed(format!("Nexus {} has no package for this system", manifest.version)),
        }
    }

    /// Downloads the package and checks its SHA-256. A packaged install hands it to the
    /// package manager; any other keeps it for the next start.
    async fn update(&self, version: &str, asset: &Asset) -> CoreResult<UpdateState> {
        let staged = self.stage(version, asset).await?;
        let Some(manager) = self.inner.here.install.manager() else {
            return Ok(UpdateState::Ready { version: staged.version });
        };
        // Nothing for the next start: the package manager puts it in now.
        let updates = self.inner.here.data_dir.join("updates");
        let _ = tokio::fs::remove_file(updates.join("ready.json")).await;
        let session = self.inner.ports.login_session.clone();
        let file = staged.file.clone();
        let done = tokio::task::spawn_blocking(move || session.install_package(&file, manager))
            .await
            .map_err(|e| CoreError::Io(e.to_string()))?;
        let _ = tokio::fs::remove_dir_all(&updates).await;
        match done {
            Ok(()) => Ok(UpdateState::Installed { version: staged.version }),
            Err(crate::PackageError::Cancelled) => Err(CoreError::Invalid(format!(
                "the password prompt was closed, so Nexus {} wasn't installed",
                staged.version
            ))),
            Err(crate::PackageError::Failed(why)) => Err(CoreError::Io(why)),
        }
    }

    /// Downloads the package, checks its SHA-256, and keeps it for the next start.
    async fn stage(&self, version: &str, asset: &Asset) -> CoreResult<Staged> {
        let bytes = self.get(&asset.url).await?;
        let sum = hex::encode(sha2::Sha256::digest(&bytes));
        if !sum.eq_ignore_ascii_case(asset.sha256.trim()) {
            return Err(CoreError::Invalid(format!("the download's SHA-256 is {sum}, not {}", asset.sha256)));
        }
        let dir = self.inner.here.data_dir.join("updates");
        let name = asset.url.rsplit('/').next().filter(|n| !n.is_empty() && !n.contains("..")).unwrap_or("package");
        let file = dir.join(name);
        let staged = Staged { version: version.to_owned(), file: file.clone() };
        let io = |e: std::io::Error| CoreError::Io(e.to_string());
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        tokio::fs::write(&file, &bytes).await.map_err(io)?;
        let ready = serde_json::to_vec(&staged).map_err(|e| CoreError::Io(e.to_string()))?;
        tokio::fs::write(dir.join("ready.json"), ready).await.map_err(io)?;
        Ok(staged)
    }
}

/// The update kept for this start, if any.
pub fn staged(data_dir: &Path) -> Option<Staged> {
    let bytes = std::fs::read(data_dir.join("updates/ready.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Swaps the kept package in for `root`: the running `Nexus.app`, the AppImage file, or the
/// package folder. The old one is renamed aside first, and comes back when anything fails.
/// Returns the new version. Blocking; run it before the launcher starts.
pub fn apply_staged(data_dir: &Path, root: &Path) -> Result<Option<String>, String> {
    let Some(staged) = staged(data_dir) else { return Ok(None) };
    let updates = data_dir.join("updates");
    let result = swap(&staged, root);
    // Tried once: a broken package isn't tried at every start.
    let _ = std::fs::remove_dir_all(&updates);
    result.map(|()| Some(staged.version))
}

fn aside(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

fn swap(staged: &Staged, root: &Path) -> Result<(), String> {
    let text = |e: &dyn std::fmt::Display| e.to_string();
    let name = staged.file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let old = aside(root);
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_file(&old);
    if name.ends_with(".AppImage") {
        std::fs::rename(root, &old).map_err(|e| text(&e))?;
        let placed = std::fs::copy(&staged.file, root).map_err(|e| text(&e)).and_then(|_| executable(root));
        if let Err(e) = placed {
            let _ = std::fs::rename(&old, root);
            return Err(e);
        }
        // The old one is still running: the next start removes it (clean_old_files).
        return Ok(());
    }
    if cfg!(windows) {
        return swap_files(staged, root, name);
    }
    // A package archive: its one top folder is the package; Nexus.app is inside it on macOS.
    let unpacked =
        root.with_file_name(format!("{}.update", root.file_name().and_then(|n| n.to_str()).unwrap_or("nexus")));
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|e| text(&e))?;
    let unpacked_ok = if name.ends_with(".tar.gz") {
        crate::unpack::untar_gz(&staged.file, &unpacked).map_err(|e| text(&e))
    } else if name.ends_with(".zip") {
        crate::unpack::unzip(&staged.file, &unpacked, None).map(|_| ()).map_err(|e| text(&e))
    } else {
        Err(format!("{name}: not a package this launcher can take"))
    };
    let new_root = unpacked_ok.and_then(|()| {
        let top = std::fs::read_dir(&unpacked)
            .map_err(|e| text(&e))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .ok_or_else(|| "the package has no folder".to_owned())?;
        let is_bundle = root.extension().is_some_and(|e| e == "app");
        let candidate = if is_bundle { top.join(root.file_name().unwrap_or_default()) } else { top };
        if candidate.exists() { Ok(candidate) } else { Err(format!("the package has no {}", candidate.display())) }
    });
    let new_root = match new_root {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&unpacked);
            return Err(e);
        }
    };
    std::fs::rename(root, &old).map_err(|e| text(&e))?;
    if let Err(e) = std::fs::rename(&new_root, root) {
        let _ = std::fs::rename(&old, root);
        let _ = std::fs::remove_dir_all(&unpacked);
        return Err(text(&e));
    }
    let _ = std::fs::remove_dir_all(&unpacked);
    // The old version still runs from it: the next start removes it (clean_old_files).
    Ok(())
}

/// Windows: a running program's folder can't be renamed, but its files can. Each new file
/// goes in; one that is there is renamed to `<name>.old` first (the running nexus.exe too).
fn swap_files(staged: &Staged, root: &Path, name: &str) -> Result<(), String> {
    if !name.ends_with(".zip") {
        return Err(format!("{name}: not a Windows package"));
    }
    let unpacked = root.with_file_name("nexus.update");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    crate::unpack::unzip(&staged.file, &unpacked, None).map_err(|e| e.to_string())?;
    let top = std::fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| "the package has no folder".to_owned())?;
    // What changed, to put back when a file fails: (the new file, its old one if any).
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = copy_over(&top, root, &mut done);
    if result.is_err() {
        for (placed, old) in done.into_iter().rev() {
            let _ = std::fs::remove_file(&placed);
            if let Some(old) = old {
                let _ = std::fs::rename(&old, &placed);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&unpacked);
    result
}

/// Copies every file under `top` into `root`, renaming each one already there to `.old`,
/// and notes each change in `done`.
fn copy_over(top: &Path, root: &Path, done: &mut Vec<(PathBuf, Option<PathBuf>)>) -> Result<(), String> {
    let mut todo = vec![top.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            let from = entry.path();
            let to = root.join(from.strip_prefix(top).map_err(|e| e.to_string())?);
            if from.is_dir() {
                std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                todo.push(from);
                continue;
            }
            let old = if to.exists() {
                let old = aside(&to);
                let _ = std::fs::remove_file(&old);
                std::fs::rename(&to, &old).map_err(|e| e.to_string())?;
                Some(old)
            } else {
                None
            };
            let copied = std::fs::copy(&from, &to);
            done.push((to, old));
            copied.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Removes what an update left once the old program has stopped: the old install beside
/// `root` (`<root>.old`), and on Windows the `.old` files in it.
pub fn clean_old_files(root: &Path) {
    let old = aside(root);
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_file(&old);
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "old") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_higher_version_is_newer() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
        assert!(!newer("not-a-version", "0.1.0"));
    }

    #[test]
    fn updates_come_only_from_github() {
        assert_eq!(host_of("https://github.com/a/b/releases"), Some("github.com"));
        assert_eq!(host_of("https://github.com@evil.example/x"), None);
        assert_eq!(host_of("http://github.com/a"), None);
    }

    /// Serves one `latest.json` behind GitHub's redirect, and counts the requests.
    struct Releases {
        latest: String,
        asked: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl HttpTransport for Releases {
        async fn send(&self, request: HttpRequest) -> nexus_api::PluginResult<nexus_api::HttpResponse> {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let served = "https://objects.githubusercontent.com/latest.json";
            Ok(if request.url == served {
                nexus_api::HttpResponse { status: 200, headers: vec![], body: self.latest.clone().into_bytes() }
            } else {
                nexus_api::HttpResponse { status: 302, headers: vec![("Location".into(), served.into())], body: vec![] }
            })
        }

        async fn open(&self, url: &str, offset: u64) -> nexus_api::PluginResult<crate::Opened> {
            crate::NoNetwork.open(url, offset).await
        }
    }

    fn launcher(repo: Option<&str>, version: &str) -> (LauncherUpdateSubsystem, Arc<Releases>) {
        let key = package_key(Install::Folder).unwrap();
        let latest = serde_json::json!({ "version": version,
            "assets": { key: { "url": "https://github.com/n/n/releases/download/x/nexus.tar.gz", "sha256": "00" } } });
        let http = Arc::new(Releases { latest: latest.to_string(), asked: Default::default() });
        let config = Arc::new(ConfigService::in_memory());
        if let Some(repo) = repo {
            config.set(&UPDATE_REPO, serde_json::Value::String(repo.into())).unwrap();
        }
        let notifier = Notifier::new(crate::changes::Changes::new(), crate::EventBus::default());
        let ports = LauncherUpdatePorts { http: http.clone(), login_session: Arc::new(crate::NoLoginSession) };
        let here = ThisLauncher { data_dir: PathBuf::from("/nowhere"), install: Install::Folder };
        (LauncherUpdateSubsystem::new(ports, here, config, notifier), http)
    }

    #[tokio::test]
    async fn a_higher_release_with_this_install_s_package_is_available() {
        let (update, _) = launcher(None, "999.0.0");
        let found = update.check().await;
        assert!(matches!(&found, UpdateState::Available { version, .. } if version == "999.0.0"), "{found:?}");
        let snapshot = update.snapshot();
        assert_eq!(snapshot.state, found);
        assert!(!snapshot.off);
        assert_eq!(snapshot.method, "swap");
    }

    #[tokio::test]
    async fn the_same_release_is_up_to_date() {
        let (update, _) = launcher(Some("me/nexus"), CURRENT);
        assert_eq!(update.check().await, UpdateState::UpToDate);
    }

    #[tokio::test]
    async fn an_empty_repo_turns_the_checks_off_without_asking() {
        let (update, http) = launcher(Some(""), "999.0.0");
        assert!(update.snapshot().off);
        assert_eq!(update.check().await, UpdateState::Off);
        assert_eq!(http.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(update.handle(LauncherUpdateCmd::Download).await.is_err(), "nothing to download");
    }
}
