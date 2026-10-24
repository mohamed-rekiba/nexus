//! `PluginContext`: the only way a plugin reaches the host.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{ConfigScope, Event, Location, PluginError, PluginId, PluginResult, RelPath};

/// The plugin's settings, stored by the host per scope.
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// `Null` when nothing is stored for the scope.
    async fn get(&self, scope: &ConfigScope) -> PluginResult<serde_json::Value>;
    async fn set(&self, scope: &ConfigScope, value: serde_json::Value) -> PluginResult<()>;
}

/// Files in the plugin's own data folder. Paths outside it can't be written down: see
/// [`RelPath`].
#[async_trait]
pub trait Storage: Send + Sync {
    async fn read(&self, path: &RelPath) -> PluginResult<Vec<u8>>;
    /// Creates parent folders as needed.
    async fn write(&self, path: &RelPath, data: &[u8]) -> PluginResult<()>;
    /// Writes `data` at `offset`, as [`Folders::write_at`] does: for files too big to hold in
    /// memory, in pieces of at most [`MAX_FOLDER_READ`].
    async fn write_at(&self, path: &RelPath, offset: u64, data: &[u8]) -> PluginResult<()>;
    async fn exists(&self, path: &RelPath) -> PluginResult<bool>;
    /// Names of the entries in a folder, sorted. Empty when the folder doesn't exist.
    async fn list(&self, dir: &RelPath) -> PluginResult<Vec<String>>;
    /// Removes a file, or a folder and everything in it. No error if it doesn't exist.
    async fn remove(&self, path: &RelPath) -> PluginResult<()>;
    async fn rename(&self, from: &RelPath, to: &RelPath) -> PluginResult<()>;
    async fn set_executable(&self, path: &RelPath) -> PluginResult<()>;
}

/// A path in a host folder: the folder's name from `permissions.filesystem`, which of the
/// player's folders for that name, and a path inside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HostPath {
    pub folder: String,
    pub root: u32,
    pub path: RelPath,
}

impl HostPath {
    pub fn new(folder: impl Into<String>, root: u32, path: RelPath) -> Self {
        Self { folder: folder.into(), root, path }
    }

    pub fn join(&self, name: &str) -> PluginResult<Self> {
        Ok(Self { folder: self.folder.clone(), root: self.root, path: self.path.join(name)? })
    }
}

/// One of the player's folders for a name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRoot {
    pub root: u32,
    /// The real path, to show the player.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// 0 for a folder.
    pub size: u64,
}

/// The most [`Folders::read_range`] returns in one call.
pub const MAX_FOLDER_READ: u64 = 16 * 1024 * 1024;
/// The most entries [`Folders::size`] counts.
pub const MAX_SIZE_FILES: usize = 200_000;

/// Host folders outside the data folder, by the names the manifest asks for in
/// `permissions.filesystem`. The host refuses a name the manifest doesn't ask for, a write
/// without `write = true`, and a symlink out of the folder.
#[async_trait]
pub trait Folders: Send + Sync {
    /// The player's folders for `folder`. Empty until the player maps it (and it has no
    /// default).
    async fn roots(&self, folder: &str) -> PluginResult<Vec<FolderRoot>>;
    /// Entries of a folder, sorted by name. Empty when it doesn't exist.
    async fn list(&self, dir: &HostPath) -> PluginResult<Vec<DirEntry>>;
    /// The bytes of every file under `dir`, counting at most [`MAX_SIZE_FILES`] entries.
    /// Linked folders are not entered. 0 when it doesn't exist.
    async fn size(&self, dir: &HostPath) -> PluginResult<u64>;
    async fn exists(&self, path: &HostPath) -> PluginResult<bool>;
    /// Up to `len` bytes from `offset`, at most [`MAX_FOLDER_READ`]. Fewer at the end of the
    /// file.
    async fn read_range(&self, path: &HostPath, offset: u64, len: u64) -> PluginResult<Vec<u8>>;
    /// Creates parent folders as needed.
    async fn write(&self, path: &HostPath, data: &[u8]) -> PluginResult<()>;
    /// Writes `data` at `offset`, creating the file and its parent folders as needed. The rest
    /// of the file stays; a gap before `offset` reads as zeros. For files too big to hold in
    /// memory: write them in pieces of at most [`MAX_FOLDER_READ`].
    async fn write_at(&self, path: &HostPath, offset: u64, data: &[u8]) -> PluginResult<()>;
    /// Moves a file or folder to another path in the same folder and root, replacing nothing:
    /// it fails if `to` exists. Needs `write = true`.
    async fn rename(&self, from: &HostPath, to: &HostPath) -> PluginResult<()>;
    /// Removes a file, or a folder and everything in it. No error if it doesn't exist.
    async fn remove(&self, path: &HostPath) -> PluginResult<()>;
    /// Marks a file as a program (downloads often lose the flag). Needs `write = true`.
    async fn set_executable(&self, path: &HostPath) -> PluginResult<()>;
}

/// The most bytes of output [`Programs::output`] keeps.
pub const MAX_PROGRAM_OUTPUT: usize = 64 * 1024;
/// The longest [`Programs::output`] lets a program run.
pub const MAX_PROGRAM_TIME: Duration = Duration::from_secs(10);

/// The programs a plugin's manifest may launch (`permissions.process`), found and run briefly:
/// to see whether a build is there and read its version. The host refuses any other program.
#[async_trait]
pub trait Programs: Send + Sync {
    /// The program's real path, or `None` when it isn't there. A system program is looked up
    /// on the system's `PATH`.
    async fn find(&self, program: &Location) -> PluginResult<Option<String>>;
    /// Runs the program in its own folder with `args` and no input, and returns what it wrote
    /// to its standard output: at most [`MAX_PROGRAM_OUTPUT`] bytes. After `timeout` (at most
    /// [`MAX_PROGRAM_TIME`]) the host stops it, and this fails with [`PluginError::Timeout`].
    async fn output(&self, program: &Location, args: &[String], timeout: Duration) -> PluginResult<Vec<u8>>;
    /// Runs a program the manifest lets run long (`permissions.long_running`) with `args` and no
    /// input, and hands each line it writes (output and errors) to `line`, until it ends. When
    /// `cancelled` says so, the host stops it. `elevated` runs it as the administrator through
    /// polkit (`pkexec`), which `permissions.polkit` must allow. Returns its exit code (pkexec's
    /// 126 or 127 when the player dismissed the password prompt).
    async fn stream(
        &self,
        program: &Location,
        _args: &[String],
        _elevated: bool,
        _line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        Err(PluginError::PermissionDenied(format!("{program} can't run long here")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Head,
    Post,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self { method: Method::Get, url: url.into(), headers: Vec::new(), body: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json<T: DeserializeOwned>(&self) -> PluginResult<T> {
        if !(200..300).contains(&self.status) {
            return Err(PluginError::Network(format!("HTTP {}", self.status)));
        }
        serde_json::from_slice(&self.body).map_err(|e| PluginError::Network(format!("bad JSON: {e}")))
    }
}

/// HTTP limited to the hosts in the manifest's `permissions.network`.
#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

pub trait LogSink: Send + Sync {
    fn log(&self, level: LogLevel, message: &str);
}

/// Writes to the host log, tagged with the plugin's id.
#[derive(Clone)]
pub struct Logger(Arc<dyn LogSink>);

impl Logger {
    pub fn new(sink: Arc<dyn LogSink>) -> Self {
        Self(sink)
    }
    pub fn debug(&self, message: &str) {
        self.0.log(LogLevel::Debug, message);
    }
    pub fn info(&self, message: &str) {
        self.0.log(LogLevel::Info, message);
    }
    pub fn warn(&self, message: &str) {
        self.0.log(LogLevel::Warn, message);
    }
    pub fn error(&self, message: &str) {
        self.0.log(LogLevel::Error, message);
    }
}

/// The plugin's handle on the event bus.
pub trait EventSink: Send + Sync {
    /// Sends an [`Event::Custom`] from this plugin.
    fn publish(&self, topic: &str, payload: serde_json::Value) -> PluginResult<()>;
    fn subscribe(&self) -> PluginResult<BoxStream<'static, Event>>;
}

/// Host services, looked up by type. The host decides per plugin which ones are present.
/// Plugins never reach each other through it.
#[derive(Clone, Default)]
pub struct ServiceRegistry {
    services: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl ServiceRegistry {
    pub fn builder() -> ServiceRegistryBuilder {
        ServiceRegistryBuilder::default()
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.services.get(&TypeId::of::<T>()).cloned().and_then(|s| s.downcast().ok())
    }
}

#[derive(Default)]
pub struct ServiceRegistryBuilder {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl ServiceRegistryBuilder {
    pub fn insert<T: Any + Send + Sync>(mut self, service: Arc<T>) -> Self {
        self.services.insert(TypeId::of::<T>(), service);
        self
    }

    pub fn build(self) -> ServiceRegistry {
        ServiceRegistry { services: Arc::new(self.services) }
    }
}

/// Facts about the host, always in the service registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub api_version: String,
    pub platform: Option<crate::Platform>,
    pub safe_mode: bool,
    /// It runs 32-bit Linux programs: it has their loader.
    #[serde(default)]
    pub runs_32bit: bool,
}

/// "Download only through a VPN", as the host sees it when asked. The host's [`VpnGate`] is in
/// the service registry of every plugin. A plugin asks it before it fetches what the player
/// downloads in bulk, for example the forum lists a catalog refresh reads.
///
/// The gate tells only whether that VPN is connected. The plugin's HTTP does not go through the
/// VPN's interface: only the downloads' torrent port is bound to it.
#[async_trait]
pub trait VpnCheck: Send + Sync {
    /// `Ok` while the setting is off, the VPN is connected, or this computer can't bind traffic
    /// to a VPN (see [`VpnPass`]). Else why not, for the player: see [`VpnBlock`].
    async fn check(&self) -> Result<VpnPass, VpnBlock>;
}

/// Why a plugin may fetch now, under "Download only through a VPN".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VpnPass {
    /// The setting is off, or the VPN is connected.
    Open,
    /// The setting is on, but this computer can't bind traffic to a VPN, so the setting doesn't
    /// apply here. `reason` has the words the downloads' setting shows, for the player.
    Unbound { reason: String },
}

/// Why a plugin may not fetch now, under "Download only through a VPN".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VpnBlock {
    /// The computer has a VPN, but not connected: the plugin waits, and tries again later.
    Waiting { reason: String },
    /// The computer has no VPN: the plugin skips what it fetches.
    NoVpn { reason: String },
}

impl VpnBlock {
    pub fn reason(&self) -> &str {
        match self {
            Self::Waiting { reason } | Self::NoVpn { reason } => reason,
        }
    }
}

/// The host's [`VpnCheck`], in the service registry.
#[derive(Clone)]
pub struct VpnGate(Arc<dyn VpnCheck>);

impl VpnGate {
    pub fn new(check: Arc<dyn VpnCheck>) -> Self {
        Self(check)
    }

    pub async fn check(&self) -> Result<VpnPass, VpnBlock> {
        self.0.check().await
    }
}

/// Everything the host gives a plugin at `init`.
#[derive(Clone)]
pub struct PluginContext {
    plugin: PluginId,
    config: Arc<dyn ConfigStore>,
    storage: Arc<dyn Storage>,
    folders: Arc<dyn Folders>,
    programs: Arc<dyn Programs>,
    http: Arc<dyn HttpClient>,
    log: Logger,
    events: Arc<dyn EventSink>,
    services: ServiceRegistry,
}

/// The parts of a [`PluginContext`]. Hosts and test harnesses fill it.
pub struct ContextParts {
    pub plugin: PluginId,
    pub config: Arc<dyn ConfigStore>,
    pub storage: Arc<dyn Storage>,
    pub folders: Arc<dyn Folders>,
    pub programs: Arc<dyn Programs>,
    pub http: Arc<dyn HttpClient>,
    pub log: Logger,
    pub events: Arc<dyn EventSink>,
    pub services: ServiceRegistry,
}

impl PluginContext {
    pub fn new(parts: ContextParts) -> Self {
        let ContextParts { plugin, config, storage, folders, programs, http, log, events, services } = parts;
        Self { plugin, config, storage, folders, programs, http, log, events, services }
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }
    pub fn config(&self) -> &Arc<dyn ConfigStore> {
        &self.config
    }
    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }
    pub fn folders(&self) -> &Arc<dyn Folders> {
        &self.folders
    }
    pub fn programs(&self) -> &Arc<dyn Programs> {
        &self.programs
    }
    pub fn http(&self) -> &Arc<dyn HttpClient> {
        &self.http
    }
    pub fn log(&self) -> &Logger {
        &self.log
    }
    pub fn events(&self) -> &Arc<dyn EventSink> {
        &self.events
    }
    pub fn services(&self) -> &ServiceRegistry {
        &self.services
    }
}
