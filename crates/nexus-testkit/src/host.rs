use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nexus_api::PluginByKind;
use nexus_core::{BootReport, Core, HostConfig, Kernel, KernelBuilder, Loader};
use tempfile::TempDir;

use crate::{FakeArchives, FakeDesktop, FakeHttp, FakeNetwork, FakeSpawner, FakeTorrents, StubPlugin};

/// Builds a real core over a temporary data folder, with fake HTTP and fake processes.
pub struct FakeHost {
    host: HostConfig,
    dir: TempDir,
    plugins: Vec<(PluginByKind, bool)>,
    discover: Vec<std::path::PathBuf>,
    loaders: Vec<Arc<dyn Loader>>,
    http: FakeHttp,
    spawner: FakeSpawner,
    desktop: FakeDesktop,
    torrents: FakeTorrents,
    network: FakeNetwork,
    archives: FakeArchives,
    hardware: Option<nexus_core::Specs>,
    free_space: Option<u64>,
    /// Folders on a disk of their own, and its free bytes.
    disks: Vec<(PathBuf, u64)>,
    first_run: bool,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHost {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let mut host = HostConfig::new(dir.path());
        // Not the computer the tests run on: a host that runs everything, unless a test says.
        host.runs_32bit = true;
        // An installed launcher, not the test binary's own folder.
        host.install = nexus_core::launcher_update::Install::Folder;
        // A desktop, not Nexus OS, whatever the computer running the tests is.
        host.place = nexus_core::system::Place::default();
        host.call_timeout = Duration::from_millis(500);
        host.shutdown_timeout = Duration::from_millis(200);
        host.stop_grace = Duration::from_millis(200);
        host.show_timeout = Duration::from_millis(500);
        host.torrent_poll = Duration::from_millis(10);
        host.vpn_poll = Duration::from_millis(10);
        Self {
            host,
            dir,
            plugins: Vec::new(),
            discover: Vec::new(),
            loaders: Vec::new(),
            http: FakeHttp::default(),
            spawner: FakeSpawner::default(),
            desktop: FakeDesktop::default(),
            torrents: FakeTorrents::default(),
            network: FakeNetwork::default(),
            archives: FakeArchives::default(),
            hardware: None,
            free_space: None,
            disks: Vec::new(),
            first_run: false,
        }
    }

    /// The archives the host unpacks, to say what each holds before the core starts: a
    /// download unpacks as soon as it finishes.
    pub fn archives(&self) -> FakeArchives {
        self.archives.clone()
    }

    /// The computer the "This PC" check sees. Without it the check finds nothing.
    pub fn hardware(mut self, specs: nexus_core::Specs) -> Self {
        self.hardware = Some(specs);
        self
    }

    /// The free bytes every disk has. Without it the disks don't say.
    pub fn free_space(mut self, bytes: u64) -> Self {
        self.free_space = Some(bytes);
        self
    }

    /// `folder` and every folder in it are on a disk of their own, with `bytes` free.
    pub fn disk(mut self, folder: impl Into<PathBuf>, bytes: u64) -> Self {
        self.disks.push((folder.into(), bytes));
        self
    }

    pub fn plugin(mut self, plugin: StubPlugin) -> Self {
        self.plugins.push((plugin.boxed(), false));
        self
    }

    /// Any built-in plugin, such as a real emulator: `PluginByKind::emulator(KytyPlugin)`.
    pub fn built_in(mut self, plugin: PluginByKind) -> Self {
        self.plugins.push((plugin, false));
        self
    }

    pub fn third_party(mut self, plugin: PluginByKind) -> Self {
        self.plugins.push((plugin, true));
        self
    }

    pub fn discover(mut self, dir: &Path) -> Self {
        self.discover.push(dir.to_path_buf());
        self
    }

    pub fn loader(mut self, loader: Arc<dyn Loader>) -> Self {
        self.loaders.push(loader);
        self
    }

    /// A first run: the player hasn't been welcomed yet. Otherwise the test host starts as a
    /// player who has been.
    pub fn first_run(mut self) -> Self {
        self.first_run = true;
        self
    }

    pub fn safe_mode(mut self) -> Self {
        self.host.safe_mode = true;
        self
    }

    /// The home folder for `~/` folder defaults.
    pub fn home(mut self, home: &Path) -> Self {
        self.host.home = Some(home.to_path_buf());
        self
    }

    /// The platform the host says it runs on (`None`: any).
    pub fn platform(mut self, platform: Option<nexus_api::Platform>) -> Self {
        self.host.platform = platform;
        self
    }

    /// Whether the host runs 32-bit Linux programs.
    pub fn runs_32bit(mut self, runs: bool) -> Self {
        self.host.runs_32bit = runs;
        self
    }

    /// How the launcher was installed, which decides how its update goes in.
    pub fn install(mut self, install: nexus_core::launcher_update::Install) -> Self {
        self.host.install = install;
        self
    }

    /// Where Nexus runs: the whole screen, Nexus OS, its USB stick.
    pub fn place(mut self, place: nexus_core::system::Place) -> Self {
        self.host.place = place;
        self
    }

    /// How often the core checks the VPN while downloads go only through it.
    pub fn vpn_poll(mut self, every: Duration) -> Self {
        self.host.vpn_poll = every;
        self
    }

    /// The computer's VPNs, before boot.
    pub fn network(&self) -> &FakeNetwork {
        &self.network
    }

    pub fn max_downloads(mut self, max: usize) -> Self {
        self.host.max_downloads = max;
        self
    }

    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.host.call_timeout = timeout;
        self
    }

    pub fn http(&self) -> &FakeHttp {
        &self.http
    }

    /// The data folder, before boot: for tests that set up files a previous run left.
    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    /// The kernel builder this host would boot, for tests that wire more on top.
    pub fn desktop(&self) -> &FakeDesktop {
        &self.desktop
    }

    pub fn into_builder(self) -> (KernelBuilder, TempDir, FakeHttp, FakeSpawner, FakeDesktop) {
        let mut builder = Kernel::builder(self.host)
            .http(Arc::new(self.http.clone()))
            .spawner(Arc::new(self.spawner.clone()))
            .desktop(Arc::new(self.desktop.clone()))
            .torrents(Arc::new(self.torrents.clone()))
            .network(Arc::new(self.network.clone()))
            .archives(Arc::new(self.archives.clone()));
        if self.hardware.is_some() || self.free_space.is_some() || !self.disks.is_empty() {
            let hardware = FixedHardware(self.hardware.unwrap_or_default(), self.free_space, self.disks);
            builder = builder.hardware(Arc::new(hardware));
        }
        for (plugin, third_party) in self.plugins {
            builder = if third_party { builder.third_party(plugin) } else { builder.plugin(plugin) };
        }
        for dir in &self.discover {
            builder = builder.discover(dir);
        }
        for loader in self.loaders {
            builder = builder.loader(loader);
        }
        (builder, self.dir, self.http, self.spawner, self.desktop)
    }

    pub async fn boot(self) -> TestHost {
        let torrents = self.torrents.clone();
        let network = self.network.clone();
        let archives = self.archives.clone();
        let (builder, dir, http, spawner, desktop) = self.into_builder();
        let (kernel, report) = builder.boot().await;
        TestHost { kernel, report, http, spawner, desktop, torrents, network, archives, _dir: dir }
    }

    /// Boots the kernel and starts the app on it.
    pub async fn boot_core(self) -> TestCore {
        let first_run = self.first_run;
        let host = self.boot().await;
        if !first_run {
            host.kernel.config().set(&["app", "welcomed"], serde_json::Value::Bool(true)).expect("config");
        }
        let core = Core::start(host.kernel.clone()).await;
        TestCore { core, host }
    }
}

/// A started app on a fake host.
pub struct TestCore {
    pub core: Core,
    pub host: TestHost,
}

impl std::ops::Deref for TestCore {
    type Target = TestHost;
    fn deref(&self) -> &TestHost {
        &self.host
    }
}

/// A booted fake host. The data folder is deleted when it drops.
pub struct TestHost {
    pub kernel: Kernel,
    pub report: BootReport,
    pub http: FakeHttp,
    pub spawner: FakeSpawner,
    pub desktop: FakeDesktop,
    pub torrents: FakeTorrents,
    pub network: FakeNetwork,
    pub archives: FakeArchives,
    _dir: TempDir,
}

impl TestHost {
    pub fn report(&self) -> &BootReport {
        &self.report
    }

    pub fn data_dir(&self) -> &Path {
        self._dir.path()
    }
}

/// A computer with these specs, this much free space on each disk, and the disks of their own.
struct FixedHardware(nexus_core::Specs, Option<u64>, Vec<(PathBuf, u64)>);

impl nexus_core::Hardware for FixedHardware {
    fn specs(&self) -> nexus_core::Specs {
        self.0.clone()
    }

    fn free_bytes(&self, path: &std::path::Path) -> Option<u64> {
        self.2.iter().find(|(disk, _)| path.starts_with(disk)).map(|(_, free)| *free).or(self.1)
    }
}
