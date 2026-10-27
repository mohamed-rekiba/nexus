//! Plugin lifecycle: discover, validate, resolve, load, init (lazily), deactivate.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use nexus_api::{
    Activation, Capability, CodePlugin, ContextParts, Event, Files, HostInfo, Logger, Platform, PluginBox,
    PluginContext, PluginId, PluginKind, PluginManifest, PluginResult, Provided, Runtime, ServiceRegistry, ThemePlugin,
    VpnGate,
};
use tokio::sync::Mutex;

use crate::config::{ConfigService, ScopedConfig};
use crate::facades::{AllowlistHttp, PluginEvents, TracingLog};
use crate::guard::guarded;
use crate::ports::Ports;
use crate::resolve::{dependency_ids, resolve};
use crate::themes::DataThemeLoader;
use crate::{
    Archives, Clock, CoreError, CoreResult, DirStorage, EventBus, GroupTree, Hardware, HttpTransport, LoadError,
    LoginSession, Network, NoArchives, NoHardware, NoLoginSession, NoNetwork, NoOpener, NoTorrents, NoVpn, NoWindows,
    Opener, ProcessSpawner, ProcessTree, TokioClock, TokioSpawner, Torrents, Windows,
};

/// Settings for one run of the host.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Holds `config.json`, and each plugin's data folder under `plugins/<id>`.
    pub data_dir: PathBuf,
    /// Load built-in plugins only.
    pub safe_mode: bool,
    pub platform: Option<Platform>,
    /// The host runs 32-bit Linux programs: it has their loader, `/lib/ld-linux.so.2`.
    pub runs_32bit: bool,
    /// How this launcher was installed: its package in a release, and how its update goes in.
    pub install: crate::launcher_update::Install,
    /// Whether Nexus is the whole screen, on Nexus OS, or on its USB stick.
    pub place: crate::system::Place,
    pub api_version: semver::Version,
    /// Limit for one call into a plugin.
    pub call_timeout: Duration,
    /// Limit for a plugin's `shutdown`.
    pub shutdown_timeout: Duration,
    /// How long Stop waits for a game that was asked to close before it is killed.
    pub stop_grace: Duration,
    /// How long a started game has to show a window before the launcher stops waiting for it.
    pub show_timeout: Duration,
    /// How long a game another program starts (a store's client) has to show its process: the
    /// client may first have to start, sign in and update itself.
    pub hand_over_timeout: Duration,
    /// How often a running game's processes and windows are read once it shows.
    pub watch_every: Duration,
    /// How many downloads run at once. The rest wait in a queue.
    pub max_downloads: usize,
    /// The player's home folder, for folder defaults like `~/Games`.
    pub home: Option<PathBuf>,
    /// How often a running torrent's state is read.
    pub torrent_poll: Duration,
    /// How often a store's installs (Steam's) are read while one runs, or for a minute after
    /// Nexus asked the store to install; ten times as long otherwise, to see installs started in
    /// the store itself.
    pub store_poll: Duration,
    /// How often the VPN is checked while downloads go only through it.
    pub vpn_poll: Duration,
}

impl HostConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            safe_mode: false,
            platform: Platform::current(),
            runs_32bit: std::path::Path::new("/lib/ld-linux.so.2").exists(),
            install: crate::launcher_update::Install::detect(),
            place: crate::system::Place::detect(),
            api_version: nexus_api::api_version(),
            call_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(5),
            stop_grace: Duration::from_secs(4),
            show_timeout: Duration::from_secs(20),
            hand_over_timeout: Duration::from_secs(120),
            watch_every: Duration::from_secs(1),
            max_downloads: 2,
            home: None,
            torrent_poll: Duration::from_secs(1),
            store_poll: Duration::from_secs(2),
            vpn_poll: Duration::from_secs(2),
        }
    }

    pub fn plugin_dir(&self, plugin: &PluginId) -> PathBuf {
        self.data_dir.join("plugins").join(plugin.as_str())
    }
}

/// Where a plugin came from. Only built-in plugins run in-process without a sandbox, and only
/// they load in safe mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    ThirdParty,
}

/// Turns a manifest found on disk into a plugin, for one kind of [`Runtime`].
pub trait Loader: Send + Sync {
    fn accepts(&self, manifest: &PluginManifest) -> bool;

    /// `dir` is the folder holding the manifest.
    fn load(&self, manifest: &PluginManifest, dir: &Path) -> Result<PluginBox, String>;
}

enum Source {
    Code(PluginBox),
    Dir(PathBuf),
}

struct Candidate {
    name: String,
    origin: Origin,
    manifest: Result<PluginManifest, String>,
    source: Source,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rejection {
    pub plugin: Option<PluginId>,
    /// The plugin id, or the folder for a manifest that couldn't be read.
    pub source: String,
    pub error: LoadError,
}

/// What happened at boot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BootReport {
    /// Loaded plugins, dependencies first.
    pub order: Vec<PluginId>,
    pub rejected: Vec<Rejection>,
}

impl BootReport {
    pub fn rejection(&self, plugin: &PluginId) -> Option<&LoadError> {
        self.rejected.iter().find(|r| r.plugin.as_ref() == Some(plugin)).map(|r| &r.error)
    }
}

pub struct KernelBuilder {
    host: HostConfig,
    candidates: Vec<Candidate>,
    loaders: Vec<Arc<dyn Loader>>,
    http: Arc<dyn HttpTransport>,
    spawner: Arc<dyn ProcessSpawner>,
    processes: Arc<dyn ProcessTree>,
    windows: Arc<dyn Windows>,
    login_session: Arc<dyn LoginSession>,
    opener: Arc<dyn Opener>,
    hardware: Arc<dyn Hardware>,
    torrents: Arc<dyn Torrents>,
    network: Arc<dyn Network>,
    archives: Arc<dyn Archives>,
    clock: Arc<dyn Clock>,
    config: Option<Arc<ConfigService>>,
}

impl KernelBuilder {
    pub fn new(host: HostConfig) -> Self {
        Self {
            host,
            candidates: Vec::new(),
            loaders: vec![Arc::new(DataThemeLoader)],
            http: Arc::new(NoNetwork),
            spawner: Arc::new(TokioSpawner),
            processes: Arc::new(GroupTree),
            windows: Arc::new(NoWindows),
            login_session: Arc::new(NoLoginSession),
            opener: Arc::new(NoOpener),
            hardware: Arc::new(NoHardware),
            torrents: Arc::new(NoTorrents),
            network: Arc::new(NoVpn),
            archives: Arc::new(NoArchives),
            clock: Arc::new(TokioClock),
            config: None,
        }
    }

    /// A built-in, in-process plugin.
    pub fn plugin(self, plugin: PluginBox) -> Self {
        self.code(plugin, Origin::BuiltIn)
    }

    /// An in-process plugin treated as third-party: for tests.
    pub fn third_party(self, plugin: PluginBox) -> Self {
        self.code(plugin, Origin::ThirdParty)
    }

    fn code(mut self, plugin: PluginBox, origin: Origin) -> Self {
        let manifest = plugin.manifest();
        self.candidates.push(Candidate {
            name: manifest.id.to_string(),
            origin,
            manifest: Ok(manifest),
            source: Source::Code(plugin),
        });
        self
    }

    /// Every plugin registered with [`nexus_api::PluginFactory`] in the linked crates.
    pub fn built_in_plugins(mut self) -> Self {
        for factory in nexus_api::PluginFactory::all() {
            self = self.plugin(factory.create());
        }
        self
    }

    /// Each subfolder of `dir` with a `plugin.toml` or `theme.toml` is a third-party plugin.
    pub fn discover(mut self, dir: &Path) -> Self {
        let Ok(entries) = std::fs::read_dir(dir) else { return self };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let file = ["plugin.toml", "theme.toml"].iter().map(|f| folder.join(f)).find(|f| f.is_file());
            let Some(file) = file else { continue };
            let manifest = std::fs::read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| PluginManifest::from_toml(&text).map_err(|e| e.0));
            self.candidates.push(Candidate {
                name: folder.display().to_string(),
                origin: Origin::ThirdParty,
                manifest,
                source: Source::Dir(folder),
            });
        }
        self
    }

    pub fn loader(mut self, loader: Arc<dyn Loader>) -> Self {
        self.loaders.push(loader);
        self
    }

    pub fn http(mut self, http: Arc<dyn HttpTransport>) -> Self {
        self.http = http;
        self
    }

    pub fn spawner(mut self, spawner: Arc<dyn ProcessSpawner>) -> Self {
        self.spawner = spawner;
        self
    }

    /// A running game's processes, beyond its process group.
    pub fn processes(mut self, processes: Arc<dyn ProcessTree>) -> Self {
        self.processes = processes;
        self
    }

    /// The desktop's windows, to bring a game or the launcher to the front.
    pub fn windows(mut self, windows: Arc<dyn Windows>) -> Self {
        self.windows = windows;
        self
    }

    /// The player's login session: start at login, and installing the launcher's package.
    pub fn login_session(mut self, login_session: Arc<dyn LoginSession>) -> Self {
        self.login_session = login_session;
        self
    }

    /// The system's opener, for the links a plugin opens.
    pub fn opener(mut self, opener: Arc<dyn Opener>) -> Self {
        self.opener = opener;
        self
    }

    /// This computer's processor, memory and graphics, for the "This PC" check.
    pub fn hardware(mut self, hardware: Arc<dyn Hardware>) -> Self {
        self.hardware = hardware;
        self
    }

    /// BitTorrent, for the games a catalog offers. Without it no torrent downloads.
    pub fn torrents(mut self, torrents: Arc<dyn Torrents>) -> Self {
        self.torrents = torrents;
        self
    }

    /// The VPNs, for "Download only through a VPN". Without it the setting is off.
    pub fn network(mut self, network: Arc<dyn Network>) -> Self {
        self.network = network;
        self
    }

    /// Unpacking the archives a download holds. Without it an archive can't install.
    pub fn archives(mut self, archives: Arc<dyn Archives>) -> Self {
        self.archives = archives;
        self
    }

    /// The time every timer of the core and every plugin call's timeout runs on. Real time
    /// by default.
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Defaults to `config.json` in the data folder.
    pub fn config(mut self, config: Arc<ConfigService>) -> Self {
        self.config = Some(config);
        self
    }

    pub async fn boot(self) -> (Kernel, BootReport) {
        Kernel::boot(self).await
    }
}

enum State {
    Loaded,
    Active(Box<Provided>),
    Failed(String),
}

struct Record {
    manifest: PluginManifest,
    origin: Origin,
    dir: Option<PathBuf>,
    plugin: Code,
    state: Mutex<State>,
}

/// A loaded plugin's code: a plugin with an init, or a theme.
enum Code {
    Init(Arc<dyn CodePlugin>),
    Theme(Arc<dyn ThemePlugin>),
}

impl From<PluginBox> for Code {
    fn from(plugin: PluginBox) -> Self {
        match plugin {
            PluginBox::Code(p) => Self::Init(Arc::from(p)),
            PluginBox::Theme(p) => Self::Theme(Arc::from(p)),
        }
    }
}

impl Code {
    fn as_plugin(&self) -> &dyn nexus_api::Plugin {
        match self {
            Self::Init(p) => p.as_ref(),
            Self::Theme(p) => p.as_ref(),
        }
    }

    async fn shutdown(&self) -> PluginResult<()> {
        self.as_plugin().shutdown().await
    }
}

/// What the UI shows about a plugin.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub origin: Origin,
    pub status: PluginStatus,
    /// The folder the plugin was loaded from, for plugins found on disk.
    pub dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    Loaded,
    Active,
    Failed(String),
}

struct Inner {
    host: HostConfig,
    bus: EventBus,
    config: Arc<ConfigService>,
    /// The kernel uses the HTTP, process, clock and VPN ports, and hands the bundle on to the
    /// core.
    ports: Ports,
    order: Vec<PluginId>,
    records: BTreeMap<PluginId, Record>,
}

/// The plugin registry and lifecycle manager. Cheap to clone.
#[derive(Clone)]
pub struct Kernel {
    inner: Arc<Inner>,
}

impl Kernel {
    pub fn builder(host: HostConfig) -> KernelBuilder {
        KernelBuilder::new(host)
    }

    async fn boot(builder: KernelBuilder) -> (Kernel, BootReport) {
        let KernelBuilder {
            host,
            candidates,
            loaders,
            http,
            spawner,
            processes,
            windows,
            login_session,
            opener,
            hardware,
            torrents,
            network,
            archives,
            clock,
            config,
        } = builder;
        let mut report = BootReport::default();
        let reject = |report: &mut BootReport, plugin: Option<PluginId>, source: String, error: LoadError| {
            tracing::warn!(plugin = ?plugin, %source, %error, "plugin rejected");
            report.rejected.push(Rejection { plugin, source, error });
        };

        // Validate.
        let mut valid: BTreeMap<PluginId, Candidate> = BTreeMap::new();
        for candidate in candidates {
            let manifest = match &candidate.manifest {
                Ok(m) => m,
                Err(message) => {
                    let error = LoadError::Manifest { source_name: candidate.name.clone(), message: message.clone() };
                    reject(&mut report, None, candidate.name.clone(), error);
                    continue;
                }
            };
            let id = manifest.id.clone();
            if let Err(error) = validate(&host, manifest, candidate.origin) {
                reject(&mut report, Some(id), candidate.name.clone(), error);
                continue;
            }
            match valid.entry(id) {
                Entry::Occupied(entry) => {
                    reject(&mut report, Some(entry.key().clone()), candidate.name.clone(), LoadError::Duplicate)
                }
                Entry::Vacant(entry) => {
                    entry.insert(candidate);
                }
            }
        }

        // Resolve.
        let manifests: Vec<PluginManifest> = valid.values().filter_map(|c| c.manifest.clone().ok()).collect();
        let resolution = resolve(&manifests);
        for (id, error) in resolution.rejected {
            let name = valid.remove(&id).map(|c| c.name).unwrap_or_else(|| id.to_string());
            reject(&mut report, Some(id), name, error);
        }

        // Load.
        let mut records = BTreeMap::new();
        let mut order = Vec::new();
        for id in resolution.order {
            let Some(candidate) = valid.remove(&id) else { continue };
            let manifest = candidate.manifest.clone().expect("validated above");
            let (plugin, dir): (PluginBox, Option<PathBuf>) = match candidate.source {
                Source::Code(plugin) => (plugin, None),
                Source::Dir(dir) => match loaders.iter().find(|l| l.accepts(&manifest)) {
                    Some(loader) => match loader.load(&manifest, &dir) {
                        Ok(plugin) => (plugin, Some(dir)),
                        Err(e) => {
                            reject(&mut report, Some(id), candidate.name, LoadError::Loader(e));
                            continue;
                        }
                    },
                    None => {
                        let error = LoadError::Loader(format!("runtime {:?}", manifest.runtime));
                        reject(&mut report, Some(id), candidate.name, error);
                        continue;
                    }
                },
            };
            if plugin.is_theme() != (manifest.kind == PluginKind::Theme) {
                let code = if plugin.is_theme() { "a theme" } else { "code with an init" };
                let error =
                    LoadError::Invalid(format!("the manifest says {:?}, but the plugin is {code}", manifest.kind));
                reject(&mut report, Some(id), candidate.name, error);
                continue;
            }
            if host.platform.is_some_and(|p| !manifest.supports(p)) {
                tracing::warn!(
                    plugin_id = %id,
                    supports = ?manifest.platforms,
                    "made for other systems: loaded turned off, turn it on to use your own build"
                );
            }
            order.push(id.clone());
            records.insert(
                id,
                Record {
                    manifest,
                    origin: candidate.origin,
                    dir,
                    plugin: plugin.into(),
                    state: Mutex::new(State::Loaded),
                },
            );
        }

        let config = config.unwrap_or_else(|| Arc::new(ConfigService::open(host.data_dir.join("config.json"))));
        let ports = Ports {
            http,
            spawner,
            processes,
            windows,
            login_session,
            opener,
            hardware,
            torrents,
            archives,
            clock,
            vpn: crate::vpn::VpnRule::new(config.clone(), network),
        };
        let kernel = Kernel {
            inner: Arc::new(Inner { host, bus: EventBus::default(), config, ports, order: order.clone(), records }),
        };

        // Init the eager ones.
        for id in &order {
            if kernel.inner.records[id].manifest.activation == Activation::Eager
                && let Err(error) = kernel.activate_inner(id).await
            {
                report.rejected.push(Rejection { plugin: Some(id.clone()), source: id.to_string(), error });
            }
        }
        report.order = order;
        (kernel, report)
    }

    pub fn host(&self) -> &HostConfig {
        &self.inner.host
    }

    pub fn events(&self) -> &EventBus {
        &self.inner.bus
    }

    pub fn config(&self) -> &Arc<ConfigService> {
        &self.inner.config
    }

    /// Every port the app plugged in. The core takes the bundle once, when it starts, and
    /// splits it per subsystem.
    pub(crate) fn ports(&self) -> &Ports {
        &self.inner.ports
    }

    /// Loaded plugins, dependencies first.
    pub fn order(&self) -> &[PluginId] {
        &self.inner.order
    }

    pub fn manifest(&self, plugin: &PluginId) -> CoreResult<&PluginManifest> {
        self.record(plugin).map(|r| &r.manifest)
    }

    /// The player's folders as `plugin` may use them. `None` for a plugin that isn't loaded.
    pub(crate) fn host_folders(&self, plugin: &PluginId) -> Option<crate::folders::HostFolders> {
        let record = self.record(plugin).ok()?;
        Some(crate::folders::HostFolders::new(&record.manifest, &self.inner.config, &self.inner.host))
    }

    pub fn plugin_dir(&self, plugin: &PluginId) -> CoreResult<Option<&Path>> {
        self.record(plugin).map(|r| r.dir.as_deref())
    }

    fn record(&self, plugin: &PluginId) -> CoreResult<&Record> {
        self.inner.records.get(plugin).ok_or_else(|| CoreError::UnknownPlugin(plugin.clone()))
    }

    pub async fn plugins(&self) -> Vec<PluginInfo> {
        let mut out = Vec::new();
        for id in &self.inner.order {
            let record = &self.inner.records[id];
            let status = match &*record.state.lock().await {
                State::Loaded => PluginStatus::Loaded,
                State::Active(_) => PluginStatus::Active,
                State::Failed(e) => PluginStatus::Failed(e.clone()),
            };
            out.push(PluginInfo {
                manifest: record.manifest.clone(),
                origin: record.origin,
                status,
                dir: record.dir.clone(),
            });
        }
        out
    }

    /// Each plugin's status without waiting. A plugin whose `init` is running shows as loaded.
    pub fn statuses(&self) -> BTreeMap<PluginId, PluginStatus> {
        self.inner
            .records
            .iter()
            .map(|(id, record)| {
                let status = match record.state.try_lock().as_deref() {
                    Ok(State::Active(_)) => PluginStatus::Active,
                    Ok(State::Failed(e)) => PluginStatus::Failed(e.clone()),
                    _ => PluginStatus::Loaded,
                };
                (id.clone(), status)
            })
            .collect()
    }

    /// Whether the player has the plugin on. They turn plugins off by id, saved under
    /// `app.disabled_plugins`.
    ///
    /// A plugin made for other systems is off until the player turns it on, saved under
    /// `app.enabled_plugins`.
    pub fn enabled(&self, plugin: &PluginId) -> bool {
        let listed = |key: &str| {
            let ids = self.inner.config.get(&["app", key]);
            ids.as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(plugin.as_str())))
        };
        if self.elsewhere(plugin) { listed("enabled_plugins") } else { !listed("disabled_plugins") }
    }

    /// The plugin is made for other systems than this one.
    pub fn elsewhere(&self, plugin: &PluginId) -> bool {
        let Ok(record) = self.record(plugin) else { return false };
        self.inner.host.platform.is_some_and(|p| !record.manifest.supports(p))
    }

    /// Turned-on plugins whose manifest declares `capability`, in load order.
    pub fn providers(&self, capability: Capability) -> Vec<PluginId> {
        self.inner
            .order
            .iter()
            .filter(|id| self.inner.records[*id].manifest.capabilities.contains(&capability) && self.enabled(id))
            .cloned()
            .collect()
    }

    /// Turned-on plugins of one kind, in load order.
    pub fn of_kind(&self, kind: PluginKind) -> Vec<PluginId> {
        self.inner
            .order
            .iter()
            .filter(|id| self.inner.records[*id].manifest.kind == kind && self.enabled(id))
            .cloned()
            .collect()
    }

    /// The plugin's capabilities, calling `init` first if it hasn't run.
    /// A plugin the player turned off is unavailable.
    pub async fn provided(&self, plugin: &PluginId) -> CoreResult<Provided> {
        self.record(plugin)?;
        if !self.enabled(plugin) {
            return Err(CoreError::PluginUnavailable { plugin: plugin.clone(), reason: "it is turned off".into() });
        }
        self.activate_inner(plugin)
            .await
            .map_err(|error| CoreError::PluginUnavailable { plugin: plugin.clone(), reason: error.to_string() })
    }

    pub async fn activate(&self, plugin: &PluginId) -> CoreResult<()> {
        self.provided(plugin).await.map(|_| ())
    }

    fn activate_inner<'a>(&'a self, plugin: &'a PluginId) -> BoxFuture<'a, Result<Provided, LoadError>> {
        Box::pin(async move {
            let record =
                self.inner.records.get(plugin).ok_or_else(|| LoadError::Invalid(format!("no plugin {plugin}")))?;
            // Dependencies first. They form no cycle: resolution removed those.
            for (dep, _, optional) in dependency_ids(&record.manifest) {
                if optional && !self.inner.records.contains_key(&dep) {
                    continue;
                }
                if self.activate_inner(&dep).await.is_err() && !optional {
                    return Err(LoadError::DependencyRejected { dependency: dep });
                }
            }

            let mut state = record.state.lock().await;
            match &*state {
                State::Active(provided) => return Ok((**provided).clone()),
                State::Failed(reason) => return Err(LoadError::Invalid(reason.clone())),
                State::Loaded => {}
            }

            let (host, clock) = (&self.inner.host, self.inner.ports.clock.as_ref());
            let result = match &record.plugin {
                // A theme has no init: its files are read on demand.
                Code::Theme(_) => Ok(Provided::default()),
                Code::Init(code) => {
                    Self::init_code(plugin, record, code.init(self.context_for(record)), code.as_ref(), host, clock)
                        .await
                }
            };
            match result {
                Ok(provided) => {
                    *state = State::Active(Box::new(provided.clone()));
                    drop(state);
                    tracing::info!(plugin_id = %plugin, "plugin activated");
                    self.inner.bus.publish(Event::PluginActivated { plugin: plugin.clone() });
                    Ok(provided)
                }
                Err(error) => {
                    *state = State::Failed(error.to_string());
                    drop(state);
                    self.inner.bus.publish(Event::PluginFailed { plugin: plugin.clone(), error: error.to_string() });
                    Err(error)
                }
            }
        })
    }

    /// Runs a code plugin's `init` and checks it provides what its manifest declares.
    async fn init_code(
        plugin: &PluginId,
        record: &Record,
        init: BoxFuture<'_, PluginResult<Provided>>,
        code: &dyn nexus_api::Plugin,
        host: &HostConfig,
        clock: &dyn Clock,
    ) -> Result<Provided, LoadError> {
        let provided = guarded(clock, plugin, "init", host.call_timeout, init).await.map_err(LoadError::Init)?;
        let declared: BTreeSet<Capability> = record.manifest.capabilities.clone();
        let got = provided.capabilities();
        if declared == got {
            return Ok(provided);
        }
        if let Err(error) = guarded(clock, plugin, "shutdown", host.shutdown_timeout, code.shutdown()).await {
            tracing::warn!(plugin_id = %plugin, %error, "shutdown failed");
        }
        Err(LoadError::CapabilityMismatch {
            declared: declared.into_iter().collect(),
            provided: got.into_iter().collect(),
        })
    }

    /// A theme plugin's files. Themes stay on, and have no init.
    pub fn theme_files(&self, plugin: &PluginId) -> CoreResult<Arc<dyn Files>> {
        match &self.record(plugin)?.plugin {
            Code::Theme(theme) => Ok(theme.files()),
            Code::Init(_) => {
                Err(CoreError::PluginUnavailable { plugin: plugin.clone(), reason: "it is not a theme".into() })
            }
        }
    }

    fn context_for(&self, record: &Record) -> PluginContext {
        let id = record.manifest.id.clone();
        let host = &self.inner.host;
        let services = ServiceRegistry::builder()
            .insert(Arc::new(HostInfo {
                api_version: host.api_version.to_string(),
                platform: host.platform,
                safe_mode: host.safe_mode,
                runs_32bit: host.runs_32bit,
            }))
            .insert(Arc::new(VpnGate::new(Arc::new(self.inner.ports.vpn.clone()))))
            .build();
        PluginContext::new(ContextParts {
            plugin: id.clone(),
            config: Arc::new(ScopedConfig { plugin: id.clone(), service: self.inner.config.clone() }),
            storage: Arc::new(DirStorage::new(host.plugin_dir(&id))),
            folders: Arc::new(crate::folders::HostFolders::new(&record.manifest, &self.inner.config, host)),
            programs: Arc::new(crate::programs::HostPrograms {
                plugin: id.clone(),
                permissions: record.manifest.permissions.clone(),
                config: self.inner.config.clone(),
                home: host.home.clone(),
                platform: host.platform,
                data_dir: host.plugin_dir(&id),
                spawner: self.inner.ports.spawner.clone(),
            }),
            http: Arc::new(AllowlistHttp {
                plugin: id.clone(),
                permissions: record.manifest.permissions.clone(),
                transport: self.inner.ports.http.clone(),
            }),
            log: Logger::new(Arc::new(TracingLog { plugin: id.clone() })),
            events: Arc::new(PluginEvents { plugin: id, bus: self.inner.bus.clone() }),
            services,
        })
    }

    /// Calls `shutdown` if the plugin is active, after deactivating the plugins that depend
    /// on it. Safe to call any number of times.
    pub fn deactivate<'a>(&'a self, plugin: &'a PluginId) -> BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let record = self.record(plugin)?;
            for dependent in self.dependents(plugin) {
                self.deactivate(&dependent).await?;
            }
            let mut state = record.state.lock().await;
            if !matches!(*state, State::Active(_)) {
                return Ok(());
            }
            *state = State::Loaded;
            drop(state);
            let timeout = self.inner.host.shutdown_timeout;
            if let Err(error) =
                guarded(self.inner.ports.clock.as_ref(), plugin, "shutdown", timeout, record.plugin.shutdown()).await
            {
                tracing::warn!(plugin_id = %plugin, %error, "shutdown failed");
            }
            self.inner.bus.publish(Event::PluginDeactivated { plugin: plugin.clone() });
            Ok(())
        })
    }

    fn dependents(&self, plugin: &PluginId) -> Vec<PluginId> {
        self.inner
            .order
            .iter()
            .filter(|id| dependency_ids(&self.inner.records[*id].manifest).iter().any(|(d, _, _)| d == plugin))
            .cloned()
            .collect()
    }

    /// Deactivates every plugin, dependents first.
    pub async fn shutdown(&self) {
        for id in self.inner.order.iter().rev() {
            let _ = self.deactivate(id).await;
        }
    }

    /// Runs one call into `plugin` with the host's timeout, panic guard and tracing span.
    pub async fn guard<T>(
        &self,
        plugin: &PluginId,
        method: &'static str,
        call: impl Future<Output = PluginResult<T>>,
    ) -> CoreResult<T> {
        guarded(self.inner.ports.clock.as_ref(), plugin, method, self.inner.host.call_timeout, call)
            .await
            .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })
    }
}

fn validate(host: &HostConfig, manifest: &PluginManifest, origin: Origin) -> Result<(), LoadError> {
    if host.safe_mode && origin == Origin::ThirdParty {
        return Err(LoadError::SafeMode);
    }
    if !manifest.host_api.matches(&host.api_version) {
        return Err(LoadError::HostApi {
            required: manifest.host_api.to_string(),
            actual: host.api_version.to_string(),
        });
    }
    // An emulator made for other systems loads turned off: the player may have their own
    // build for this one. A theme can't be turned off, so it is left out.
    if let Some(platform) = host.platform
        && !manifest.supports(platform)
        && manifest.kind == PluginKind::Theme
    {
        return Err(LoadError::Platform { supported: manifest.platforms.clone() });
    }
    if let Some(system) = &manifest.system
        && system.color.is_some()
        && system.rgb().is_none()
    {
        return Err(LoadError::Invalid(format!(
            "[system] color {:?}: write it as #rrggbb",
            system.color.as_deref().unwrap_or_default()
        )));
    }
    for grant in &manifest.permissions.filesystem {
        if !nexus_api::PathGrant::valid_name(&grant.folder) {
            return Err(LoadError::Invalid(format!(
                "folder name {:?}: use lowercase letters, digits and -",
                grant.folder
            )));
        }
        // The folders a file lists are read only: the file's writer, not the player, named them.
        if grant.write && grant.listed_in.is_some() {
            return Err(LoadError::Invalid(format!(
                "folder {:?}: a folder with listed_in can't be written",
                grant.folder
            )));
        }
    }
    if origin == Origin::ThirdParty && manifest.runtime == Runtime::Native {
        return Err(LoadError::Invalid("third-party plugins must be data-only".into()));
    }
    // The kind governs privileges only: any plugin with code may provide the data capabilities.
    let system = manifest.kind == PluginKind::System;
    if let Some(c) = manifest.capabilities.iter().find(|c| !system && Capability::SYSTEM.contains(c)) {
        return Err(LoadError::Invalid(format!("only a system plugin can provide {c:?}")));
    }
    let process = &manifest.permissions;
    if !system && !(process.long_running.is_empty() && process.polkit.is_empty()) {
        return Err(LoadError::Invalid("only a system plugin may run programs long or through polkit".into()));
    }
    if let Some(rule) = process.long_running.iter().chain(&process.polkit).find(|r| !process.process.contains(r)) {
        return Err(LoadError::Invalid(format!("{rule} must also be in permissions.process")));
    }
    let theme = manifest.kind == PluginKind::Theme;
    let a_kind = match manifest.kind {
        PluginKind::Emulator => "an emulator",
        PluginKind::Service => "a service",
        PluginKind::Theme => "a theme",
        PluginKind::System => "a system",
    };
    if theme && manifest.capabilities.iter().any(|c| *c != Capability::Theme) {
        return Err(LoadError::Invalid("a theme plugin provides only the `theme` capability: it has no init".into()));
    }
    if theme && !manifest.capabilities.contains(&Capability::Theme) {
        return Err(LoadError::Invalid("a theme plugin must declare the `theme` capability".into()));
    }
    if !theme && manifest.capabilities.contains(&Capability::Theme) {
        return Err(LoadError::Invalid(format!("{a_kind} plugin can't provide a theme")));
    }
    if !theme && manifest.runtime == Runtime::Data {
        return Err(LoadError::Invalid(format!("{a_kind} plugin needs code")));
    }
    Ok(())
}
