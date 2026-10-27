//! An emulator's builds: official ones from GitHub releases, and your own build.
//!
//! A [`Flavor`] describes one emulator: where its releases are, which asset fits this system,
//! where the program is in a build, how a test run looks, and where your own build may be.
//! The [`BuildManager`] runs the flow over it for every emulator alike:
//!
//! - The host downloads a release's asset, checks its SHA-256 and unpacks it into
//!   `builds/<tag>`.
//! - A new build runs once with `--help` before it becomes current. One that fails the test
//!   is removed, and the build before stays current.
//! - An update keeps the build before it for a rollback; older builds go.
//! - Your own build (`own_build` in the settings) runs instead of a downloaded one. Installing
//!   an official build turns it off.
//!
//! The flavor's hooks ([`Flavor::on_switch`], [`Flavor::on_leave_own`]) run when the current
//! build changes, so an emulator that keeps its data next to its program can move it.

use std::time::Duration;

use nexus_api::serde_json::Value;
use nexus_api::{
    ActionStyle, ArchiveFormat, Artifact, DownloadPlan, DownloadRequest, DownloadedArtifact, Downloader, HostPath,
    InstalledVersion, Installer, Location, MAX_PROGRAM_TIME, Order, PluginError, PluginResult, RelPath, SlotContent,
    SlotItem, Storage, Unpack, VersionInfo, async_trait,
};

use crate::host::Host;
use crate::settings::{Settings, SettingsStore};
use crate::versions::{self, Versions};
use crate::{cache, github};

/// How long release lists are kept before they are fetched again.
pub const LISTS_TTL: Duration = Duration::from_secs(6 * 3600);
/// What the Emulators screen calls your own build.
pub const OWN_LABEL: &str = "Your own build";
/// Where your own build runs when it comes from your PATH, so its data has a home.
pub const OWN_DATA: &str = "own";

/// Where an emulator's releases are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseSource {
    /// The releases list on GitHub's API.
    pub url: String,
    /// Where the list is kept in storage.
    pub cache: RelPath,
    /// Pre-releases count too.
    pub prereleases: bool,
}

/// Where your own build may be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnBuild {
    /// The host folder you map to your own build.
    pub folder: &'static str,
    /// The mapping may be the program itself.
    pub itself: bool,
    /// Where the program may be in that folder; the first one there wins.
    pub files: Vec<String>,
    /// The program's name on your PATH.
    pub system: &'static str,
}

/// What a game starts: the program, the folder it runs in (`None`: the program's folder), and
/// the downloaded build's tag (`None` for your own build).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub program: Location,
    pub cwd: Option<Location>,
    pub tag: Option<String>,
}

/// One emulator's builds, as the [`BuildManager`] needs them.
#[async_trait]
pub trait Flavor: Send + Sync + 'static {
    type Settings: Settings;

    /// The emulator's name, for the player: "KytyPS5".
    const NAME: &'static str;
    /// The console's name ("PS5"). It names the console's page on the Emulators screen, its
    /// games' platform, and its folder inside the games and packages folders.
    const CONSOLE: &'static str;
    /// Your own build's page shows its version, from its `--help` ([`Flavor::version`]).
    const OWN_VERSION: bool = false;
    /// The style of "Switch to official builds" on the page.
    const OFFICIAL_STYLE: ActionStyle = ActionStyle::Secondary;

    fn release_source(&self, settings: &Self::Settings) -> PluginResult<ReleaseSource>;

    /// Whether a release asset is the build for this system.
    fn asset_matches(&self, name: &str) -> bool;

    /// How the host unpacks the asset.
    fn archive_format(&self, asset: &str) -> ArchiveFormat;

    /// A tag as the player reads it.
    fn pretty(&self, tag: &str) -> String {
        tag.to_owned()
    }

    /// The program in a build's folder.
    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath>;

    /// Whether the build ran: its `--help` printed `help`.
    fn runs(&self, help: &str) -> bool;

    /// A build's version from its `--help`, for your own build's page.
    fn version(&self, _help: &str) -> Option<String> {
        None
    }

    fn own_build(&self) -> OwnBuild;

    /// The folder a downloaded build runs in. `None`: the program's folder.
    async fn work_dir(&self, _storage: &dyn Storage, _tag: &str) -> PluginResult<Option<RelPath>> {
        Ok(None)
    }

    /// The current build changes from `from` to `to` (`None`: no build). Runs before an
    /// install becomes current, before the current build is removed, and after a rollback.
    async fn on_switch(&self, _host: &Host, _from: Option<&str>, _to: Option<&str>) -> PluginResult<()> {
        Ok(())
    }

    /// Your own build (at `own`, when it is there) gives way to the official build `to`
    /// (`None`: none installed). Runs before `own_build` turns off.
    async fn on_leave_own(&self, _host: &Host, _own: Option<&Location>, _to: Option<&str>) -> PluginResult<()> {
        Ok(())
    }
}

/// Own build is on, but it isn't there. `page`: the console's page; `folder`: the host folder
/// for your own build; `program`: its name on your PATH. A downloaded build is the likely way
/// out, so with one the message says so first.
// TODO(step 8): a typed precondition (`OwnBuildMissing { has_downloaded }`) that the core
// words, once the plugin API can carry it.
pub fn own_build_missing(page: &str, folder: &str, program: &str, has_downloaded: bool) -> PluginError {
    let fix = if has_downloaded {
        format!(
            "turn off Own build on the {page} page to play with the downloaded build, or map the {folder} folder, or put {program} on your PATH"
        )
    } else {
        format!("map the {folder} folder on the {page} page, or put {program} on your PATH")
    };
    PluginError::Precondition(format!("Own build is on, but your build isn't there: {fix}"))
}

fn path(p: impl Into<String>) -> PluginResult<RelPath> {
    Ok(RelPath::new(p)?)
}

/// Runs an emulator's builds over its [`Flavor`].
pub struct BuildManager<F: Flavor> {
    flavor: F,
    host: Host,
    settings: SettingsStore<F::Settings>,
    versions: Versions,
}

impl<F: Flavor> BuildManager<F> {
    pub fn new(flavor: F, host: Host) -> Self {
        let settings = SettingsStore::new(host.config.clone(), host.platform);
        let versions = Versions::new(host.storage.clone());
        Self { flavor, host, settings, versions }
    }

    pub fn flavor(&self) -> &F {
        &self.flavor
    }

    pub fn settings(&self) -> &SettingsStore<F::Settings> {
        &self.settings
    }

    pub fn versions(&self) -> &Versions {
        &self.versions
    }

    async fn source(&self) -> PluginResult<ReleaseSource> {
        self.flavor.release_source(&self.settings.load().await)
    }

    /// Releases with a build for this system, newest first.
    pub async fn builds(&self) -> PluginResult<Vec<(github::Release, github::Asset)>> {
        let source = self.source().await?;
        let host = &self.host;
        let json = cache::cached_json(
            host.storage.as_ref(),
            host.http.as_ref(),
            &host.log,
            &source.url,
            &source.cache,
            LISTS_TTL,
        )
        .await?;
        Ok(github::builds(&json, source.prereleases, |name| self.flavor.asset_matches(name)))
    }

    /// Your own build's program: the mapping itself when the flavor allows it, else the first
    /// of the flavor's files in the mapped folder, or else one on your PATH.
    pub async fn own_program(&self) -> Option<Location> {
        let own = self.flavor.own_build();
        let programs = &self.host.programs;
        for root in self.host.folders.roots(own.folder).await.unwrap_or_default() {
            if own.itself {
                let itself = Location::Folder(HostPath::new(own.folder, root.root, RelPath::root()));
                if let Ok(Some(_)) = programs.find(&itself).await {
                    return Some(itself);
                }
            }
            for file in &own.files {
                let Ok(file) = RelPath::new(file.as_str()) else { continue };
                let location = Location::Folder(HostPath::new(own.folder, root.root, file));
                if let Ok(Some(_)) = programs.find(&location).await {
                    return Some(location);
                }
            }
        }
        let system = Location::System(own.system.into());
        matches!(programs.find(&system).await, Ok(Some(_))).then_some(system)
    }

    /// What `program --help` prints, or `None` (said in the log) when it doesn't run.
    async fn help(&self, program: &Location, timeout: Duration) -> Option<String> {
        match self.host.programs.output(program, &["--help".into()], timeout).await {
            Ok(out) => Some(String::from_utf8_lossy(&out).into_owned()),
            Err(e) => {
                self.host.log.warn(&format!("--help of {program}: {e}"));
                None
            }
        }
    }

    /// Whether your own build runs now: it is on, and it is there.
    async fn own_on(&self) -> bool {
        self.settings.load().await.own_build()
    }

    /// What a game starts. With own build on, your own build, which must be there; else the
    /// current downloaded build.
    pub async fn run(&self) -> PluginResult<Run> {
        if self.own_on().await {
            let Some(program) = self.own_program().await else {
                let own = self.flavor.own_build();
                let downloaded = !self.versions.installed().await?.is_empty();
                return Err(own_build_missing(F::CONSOLE, own.folder, own.system, downloaded));
            };
            // It writes its data next to itself; from PATH, into the plugin's folder. Mapped as
            // the program itself, its folder is above the mapping: the host runs it there.
            let cwd = match &program {
                Location::Folder(file) if file.path.is_root() => None,
                Location::Folder(file) => {
                    let dir = file.path.as_str().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    let dir = if dir.is_empty() { RelPath::root() } else { path(dir)? };
                    Some(Location::Folder(HostPath::new(&file.folder, file.root, dir)))
                }
                _ => {
                    self.host.storage.write(&path(format!("{OWN_DATA}/.keep"))?, b"").await?;
                    Some(Location::Data(path(OWN_DATA)?))
                }
            };
            return Ok(Run { program, cwd, tag: None });
        }
        let tag = self
            .versions
            .state()
            .await
            .current
            .ok_or_else(|| PluginError::Precondition(format!("install {} first", F::NAME)))?;
        let storage = self.host.storage.as_ref();
        let program = self.flavor.program(storage, &tag).await?.into();
        let cwd = self.flavor.work_dir(storage, &tag).await?.map(Location::Data);
        Ok(Run { program, cwd, tag: Some(tag) })
    }

    /// The build games start, for a game's page: your own build or the current one's tag.
    pub async fn build_label(&self) -> Option<String> {
        if self.own_on().await {
            return Some(OWN_LABEL.to_owned());
        }
        self.versions.state().await.current.map(|t| self.flavor.pretty(&t))
    }

    /// The current downloaded build, as the player reads it.
    pub async fn current_label(&self) -> Option<String> {
        self.versions.state().await.current.map(|t| self.flavor.pretty(&t))
    }

    /// The build panel's buttons that go above the rest: use your own build, or switch back.
    pub async fn switch_items(&self) -> Vec<SlotItem> {
        let mut items = Vec::new();
        let own_on = self.own_on().await;
        if !own_on
            && let Some(program) = self.own_program().await
            && let Ok(Some(found)) = self.host.programs.find(&program).await
        {
            items.push(action("use-own", format!("Use the {} at {found}", F::NAME), ActionStyle::Secondary));
        }
        if own_on && self.versions.state().await.current.is_some() {
            items.push(action("use-official", "Switch to official builds".into(), F::OFFICIAL_STYLE));
        }
        items
    }

    /// The build panel's buttons that go below the rest: check for new builds, and roll back.
    pub async fn update_items(&self) -> Vec<SlotItem> {
        let mut items = vec![action("check", "Check for new builds".into(), ActionStyle::Secondary)];
        if let Some(previous) = self.versions.state().await.previous {
            items.push(action(
                "rollback",
                format!("Go back to {}", self.flavor.pretty(&previous)),
                ActionStyle::Secondary,
            ));
        }
        items
    }

    /// Runs one of the build panel's actions.
    pub async fn run_action(&self, name: &str) -> PluginResult<()> {
        match name {
            // The release list is kept for hours; this asks GitHub now.
            "check" => cache::forget(self.host.storage.as_ref(), &self.source().await?.cache).await,
            "use-own" => self.settings.set_own_build(true).await,
            "use-official" => self.leave_own(self.versions.state().await.current.as_deref()).await,
            "rollback" => {
                let state = self.versions.state().await;
                let (Some(current), Some(_)) = (state.current, state.previous) else {
                    return Err(PluginError::Precondition("no earlier build to go back to".into()));
                };
                let previous = self.versions.rollback().await?;
                self.flavor.on_switch(&self.host, Some(&current), Some(&previous)).await
            }
            other => Err(PluginError::NotFound(format!("action {other}"))),
        }
    }

    /// Your own build gives way to the official build `to`, and turns off.
    async fn leave_own(&self, to: Option<&str>) -> PluginResult<()> {
        let own = self.own_program().await;
        self.flavor.on_leave_own(&self.host, own.as_ref(), to).await?;
        self.settings.set_own_build(false).await
    }
}

fn action(id: &str, label: String, style: ActionStyle) -> SlotItem {
    SlotItem {
        id: id.into(),
        order: Order::default(),
        content: SlotContent::Action { label, action: id.into(), args: Value::Null, style, icon: None },
    }
}

#[async_trait]
impl<F: Flavor> Downloader for BuildManager<F> {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan> {
        let DownloadRequest::EmulatorBuild { version } = request else {
            return Err(PluginError::Unsupported(format!("{} downloads its own builds only", F::NAME)));
        };
        let builds = self.builds().await?;
        let (release, asset) = match version {
            Some(tag) => builds.into_iter().find(|(r, _)| &r.tag == tag),
            None => builds.into_iter().next(),
        }
        .ok_or_else(|| PluginError::NotFound(format!("no {} build for this system", F::NAME)))?;
        let sha256 = asset
            .sha256
            .clone()
            .ok_or_else(|| PluginError::Precondition(format!("GitHub has no checksum for {}", asset.name)))?;
        Ok(DownloadPlan {
            label: format!("{} {}", F::NAME, self.flavor.pretty(&release.tag)),
            artifacts: vec![Artifact {
                url: asset.url.clone(),
                size: (asset.size > 0).then_some(asset.size),
                sha256,
                signature: None,
                dest: path(format!("downloads/{}", asset.name))?,
                unpack: Some(Unpack {
                    format: self.flavor.archive_format(&asset.name),
                    into: versions::folder(&release.tag)?,
                }),
            }],
        })
    }
}

#[async_trait]
impl<F: Flavor> Installer for BuildManager<F> {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(self
            .builds()
            .await?
            .into_iter()
            .map(|(r, _)| VersionInfo { notes: Some(self.flavor.pretty(&r.tag)), version: r.tag })
            .collect())
    }

    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        if self.own_on().await
            && let Some(program) = self.own_program().await
        {
            let version = match F::OWN_VERSION {
                true => self.help(&program, Duration::from_secs(3)).await.and_then(|h| self.flavor.version(&h)),
                false => None,
            };
            let version = match version {
                Some(v) => format!("{OWN_LABEL} · {v}"),
                None => OWN_LABEL.to_owned(),
            };
            return Ok(vec![InstalledVersion { version, path: path(OWN_DATA)?, own: true }]);
        }
        let mut out = Vec::new();
        for tag in self.versions.installed().await? {
            out.push(InstalledVersion { path: versions::folder(&tag)?, version: tag, own: false });
        }
        Ok(out)
    }

    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let folder = versions::folder(version)?;
        if !files.iter().any(|f| f.unpacked.as_ref() == Some(&folder)) {
            return Err(PluginError::InvalidInput(format!("the download of {version} was not unpacked")));
        }
        let storage = self.host.storage.as_ref();
        let program = self.flavor.program(storage, version).await?;
        // Downloads can lose the flag.
        storage.set_executable(&program).await?;
        // Before anything moves, so a build that doesn't run never takes the data.
        let ran = self.help(&Location::Data(program), MAX_PROGRAM_TIME).await;
        if !ran.is_some_and(|help| self.flavor.runs(&help)) {
            self.versions.discard(version).await?;
            return Err(PluginError::Precondition(format!(
                "{} {} does not run on this system",
                F::NAME,
                self.flavor.pretty(version)
            )));
        }
        let current = self.versions.state().await.current;
        self.flavor.on_switch(&self.host, current.as_deref(), Some(version)).await?;
        // Switching from your own build: the official build runs.
        if self.own_on().await {
            self.leave_own(Some(version)).await?;
        }
        self.versions.activate(version).await?;
        Ok(InstalledVersion { version: version.into(), path: folder, own: false })
    }

    /// Installs `to` and keeps `from` as the build to go back to; older builds go.
    async fn update(&self, _from: &str, to: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let installed = self.install(to, files).await?;
        self.versions.prune().await?;
        Ok(installed)
    }

    async fn uninstall(&self, version: &str) -> PluginResult<()> {
        // Your own build is the one installed while it is on; it isn't a downloaded build.
        // It isn't removed: the launcher stops using it.
        if !self.versions.has(version).await? && self.own_on().await {
            return self.settings.set_own_build(false).await;
        }
        let state = self.versions.state().await;
        if state.current.as_deref() == Some(version) {
            self.flavor.on_switch(&self.host, Some(version), state.previous.as_deref()).await?;
        }
        self.versions.remove(version).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nexus_api::serde_json::json;
    use nexus_api::{ConfigScope, ConfigStore};

    use super::*;
    use crate::settings::tests::TestSettings;
    use crate::testing::TestHost;

    const RELEASES: &str = "https://api.github.com/repos/emu/emu/releases";

    /// An emulator whose builds hold `emu`, which prints "TestEmu 1.0" when it runs.
    #[derive(Default)]
    struct TestFlavor {
        switches: Arc<Mutex<Vec<Switch>>>,
        left_own: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait]
    impl Flavor for TestFlavor {
        type Settings = TestSettings;
        const NAME: &'static str = "TestEmu";
        const CONSOLE: &'static str = "PS9";
        const OWN_VERSION: bool = true;

        fn release_source(&self, settings: &TestSettings) -> PluginResult<ReleaseSource> {
            Ok(ReleaseSource {
                url: RELEASES.into(),
                cache: path(format!("cache/releases-{}.json", settings.repo))?,
                prereleases: false,
            })
        }

        fn asset_matches(&self, name: &str) -> bool {
            name.ends_with("-linux.zip")
        }

        fn archive_format(&self, _asset: &str) -> ArchiveFormat {
            ArchiveFormat::Zip
        }

        async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
            let program = versions::folder(tag)?.join("emu")?;
            match storage.exists(&program).await? {
                true => Ok(program),
                false => Err(PluginError::NotFound(format!("no emu in {tag}"))),
            }
        }

        fn runs(&self, help: &str) -> bool {
            help.starts_with("TestEmu")
        }

        fn version(&self, help: &str) -> Option<String> {
            help.strip_prefix("TestEmu ").map(str::to_owned)
        }

        fn own_build(&self) -> OwnBuild {
            OwnBuild { folder: "emu-build", itself: false, files: vec!["emu".into()], system: "emu" }
        }

        async fn on_switch(&self, _host: &Host, from: Option<&str>, to: Option<&str>) -> PluginResult<()> {
            self.switches.lock().unwrap().push((from.map(Into::into), to.map(Into::into)));
            Ok(())
        }

        async fn on_leave_own(&self, _host: &Host, _own: Option<&Location>, to: Option<&str>) -> PluginResult<()> {
            self.left_own.lock().unwrap().push(to.map(Into::into));
            Ok(())
        }
    }

    type Switch = (Option<String>, Option<String>);

    fn switch(from: Option<&str>, to: Option<&str>) -> Switch {
        (from.map(Into::into), to.map(Into::into))
    }

    struct Test {
        host: TestHost,
        switches: Arc<Mutex<Vec<Switch>>>,
        left_own: Arc<Mutex<Vec<Option<String>>>>,
        builds: BuildManager<TestFlavor>,
    }

    fn test() -> Test {
        let host = TestHost::default();
        let flavor = TestFlavor::default();
        let (switches, left_own) = (flavor.switches.clone(), flavor.left_own.clone());
        let builds = BuildManager::new(flavor, host.host());
        Test { host, switches, left_own, builds }
    }

    impl Test {
        /// Unpacks build `tag` as the host would; `runs`: its program passes the test run.
        fn unpack(&self, tag: &str, runs: bool) -> Vec<DownloadedArtifact> {
            let program = format!("builds/{tag}/emu");
            self.host.storage.put(&program, b"#!");
            let out = runs.then_some("TestEmu 1.0\nusage");
            self.host.programs.add(&Location::Data(path(program.as_str()).unwrap()), &program, out);
            vec![DownloadedArtifact {
                dest: path(format!("downloads/{tag}.zip")).unwrap(),
                sha256: String::new(),
                size: 0,
                unpacked: Some(versions::folder(tag).unwrap()),
            }]
        }

        async fn install(&self, tag: &str) {
            let files = self.unpack(tag, true);
            self.builds.install(tag, &files).await.unwrap();
        }

        async fn installed(&self) -> Vec<String> {
            self.builds.installed().await.unwrap().into_iter().map(|v| v.version).collect()
        }

        async fn state(&self) -> versions::State {
            self.builds.versions().state().await
        }

        fn switches(&self) -> Vec<Switch> {
            self.switches.lock().unwrap().clone()
        }

        async fn own_build(&self, on: bool) {
            self.host.config.set(&ConfigScope::Emulator, json!({ "own_build": on })).await.unwrap();
        }

        fn own_on_path(&self) {
            self.host.programs.add(&Location::System("emu".into()), "/usr/bin/emu", Some("TestEmu 2.0-dev"));
        }
    }

    #[tokio::test]
    async fn a_build_that_runs_becomes_current_and_runs_games() {
        let t = test();
        t.install("v1").await;
        assert!(t.host.storage.is_executable("builds/v1/emu"), "downloads can lose the flag");
        assert_eq!(t.state().await.current.as_deref(), Some("v1"));
        assert_eq!(t.installed().await, ["v1"]);
        assert_eq!(t.switches(), [switch(None, Some("v1"))], "the data may move before it is current");
        let run = t.builds.run().await.unwrap();
        assert_eq!(
            run,
            Run { program: Location::Data(path("builds/v1/emu").unwrap()), cwd: None, tag: Some("v1".into()) }
        );
    }

    #[tokio::test]
    async fn a_build_needs_its_unpacked_files() {
        let t = test();
        t.unpack("v1", true);
        let error = t.builds.install("v1", &[]).await.unwrap_err();
        assert_eq!(error, PluginError::InvalidInput("the download of v1 was not unpacked".into()));
    }

    #[tokio::test]
    async fn a_build_that_fails_its_test_run_is_removed_and_the_build_before_stays() {
        let t = test();
        t.install("v1").await;
        let files = t.unpack("v2", false);
        let error = t.builds.install("v2", &files).await.unwrap_err();
        assert_eq!(error, PluginError::Precondition("TestEmu v2 does not run on this system".into()));
        assert_eq!(t.state().await.current.as_deref(), Some("v1"));
        assert_eq!(t.installed().await, ["v1"]);
        assert!(!t.host.storage.paths().iter().any(|p| p.starts_with("builds/v2")), "its folder is gone");
        assert_eq!(t.switches(), [switch(None, Some("v1"))], "nothing moved for the broken build");
    }

    #[tokio::test]
    async fn an_update_keeps_the_build_before_and_prunes_older_ones() {
        let t = test();
        t.install("v1").await;
        t.install("v2").await;
        let files = t.unpack("v3", true);
        t.builds.update("v2", "v3", &files).await.unwrap();
        assert_eq!(t.state().await, versions::State { current: Some("v3".into()), previous: Some("v2".into()) });
        assert_eq!(t.installed().await, ["v2", "v3"], "the current one last");
        assert_eq!(t.switches().last(), Some(&switch(Some("v2"), Some("v3"))));
    }

    #[tokio::test]
    async fn a_rollback_goes_back_to_the_build_before_and_moves_the_data_after() {
        let t = test();
        t.install("v1").await;
        t.install("v2").await;
        let rollback = t.builds.update_items().await.into_iter().find(|i| i.id == "rollback").unwrap();
        assert!(matches!(rollback.content, SlotContent::Action { label, .. } if label == "Go back to v1"));
        t.builds.run_action("rollback").await.unwrap();
        assert_eq!(t.state().await, versions::State { current: Some("v1".into()), previous: Some("v2".into()) });
        assert_eq!(t.switches().last(), Some(&switch(Some("v2"), Some("v1"))));
    }

    #[tokio::test]
    async fn a_rollback_without_a_build_before_says_so() {
        let t = test();
        t.install("v1").await;
        let error = t.builds.run_action("rollback").await.unwrap_err();
        assert_eq!(error, PluginError::Precondition("no earlier build to go back to".into()));
        assert!(t.builds.update_items().await.iter().all(|i| i.id != "rollback"));
    }

    #[tokio::test]
    async fn removing_the_current_build_hands_over_to_the_build_before() {
        let t = test();
        t.install("v1").await;
        t.install("v2").await;
        t.builds.uninstall("v2").await.unwrap();
        assert_eq!(t.state().await.current.as_deref(), Some("v1"));
        assert_eq!(t.switches().last(), Some(&switch(Some("v2"), Some("v1"))));
    }

    #[tokio::test]
    async fn your_own_build_when_it_is_there_runs_games_and_shows_its_version() {
        let t = test();
        t.own_on_path();
        assert_eq!(t.builds.switch_items().await.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["use-own"]);
        t.builds.run_action("use-own").await.unwrap();
        let installed = t.builds.installed().await.unwrap();
        assert_eq!(
            installed,
            [InstalledVersion { version: "Your own build · 2.0-dev".into(), path: path("own").unwrap(), own: true }]
        );
        let run = t.builds.run().await.unwrap();
        assert_eq!(
            run,
            Run { program: Location::System("emu".into()), cwd: Some(Location::Data(path("own").unwrap())), tag: None }
        );
        assert!(t.host.storage.file("own/.keep").is_some(), "its working folder exists");
        assert_eq!(t.builds.build_label().await.as_deref(), Some("Your own build"));
    }

    #[tokio::test]
    async fn uninstalling_your_own_build_turns_it_off_and_keeps_it() {
        let t = test();
        t.own_on_path();
        t.own_build(true).await;
        t.builds.uninstall("Your own build · 2.0-dev").await.unwrap();
        assert_eq!(t.host.config.emulator()["own_build"], false);
        assert!(t.switches().is_empty());
    }

    #[tokio::test]
    async fn installing_an_official_build_leaves_your_own_build() {
        let t = test();
        t.own_on_path();
        t.own_build(true).await;
        t.install("v1").await;
        assert_eq!(t.host.config.emulator()["own_build"], false);
        assert_eq!(*t.left_own.lock().unwrap(), [Some("v1".to_owned())]);
        assert_eq!(t.state().await.current.as_deref(), Some("v1"));
    }

    #[tokio::test]
    async fn switching_to_official_builds_hands_over_to_the_current_one() {
        let t = test();
        t.install("v1").await;
        t.own_on_path();
        t.own_build(true).await;
        let items = t.builds.switch_items().await;
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["use-official"]);
        t.builds.run_action("use-official").await.unwrap();
        assert_eq!(*t.left_own.lock().unwrap(), [Some("v1".to_owned())]);
        assert_eq!(t.host.config.emulator()["own_build"], false);
    }

    #[tokio::test]
    async fn own_build_on_without_one_says_how_out() {
        let t = test();
        t.own_build(true).await;
        let missing = t.builds.run().await.unwrap_err();
        assert_eq!(
            missing,
            PluginError::Precondition(
                "Own build is on, but your build isn't there: map the emu-build folder on the PS9 page, or put emu on your PATH".into()
            )
        );
        t.unpack("v1", true);
        let missing = t.builds.run().await.unwrap_err();
        assert_eq!(
            missing,
            PluginError::Precondition(
                "Own build is on, but your build isn't there: turn off Own build on the PS9 page to play with the downloaded build, or map the emu-build folder, or put emu on your PATH".into()
            ),
            "with a downloaded build, turning own build off comes first"
        );
        assert!(t.builds.installed().await.unwrap().iter().all(|v| !v.own));
    }

    #[tokio::test]
    async fn no_build_installed_says_to_install_one() {
        let t = test();
        assert_eq!(t.builds.run().await.unwrap_err(), PluginError::Precondition("install TestEmu first".into()));
    }

    #[tokio::test]
    async fn a_plan_takes_the_newest_build_for_this_system_and_its_checksum() {
        let t = test();
        let asset = |name: &str, digest: bool| {
            let mut a = json!({ "name": name, "browser_download_url": format!("https://x/{name}"), "size": 5 });
            if digest {
                a["digest"] = json!(format!("sha256:{}", "ab".repeat(32)));
            }
            a
        };
        let list = json!([
            { "tag_name": "v2", "published_at": "2026-10-02", "assets": [asset("emu-mac.zip", true)] },
            { "tag_name": "v1", "published_at": "2026-10-01", "assets": [asset("emu-linux.zip", true)] },
            { "tag_name": "v0", "published_at": "2026-09-01", "assets": [asset("emu-linux.zip", false)] },
        ]);
        t.host.http.page(RELEASES, &list.to_string());
        let plan = t.builds.plan(&DownloadRequest::EmulatorBuild { version: None }).await.unwrap();
        assert_eq!(plan.label, "TestEmu v1");
        assert_eq!(
            plan.artifacts[0].unpack,
            Some(Unpack { format: ArchiveFormat::Zip, into: path("builds/v1").unwrap() })
        );
        let old = t.builds.plan(&DownloadRequest::EmulatorBuild { version: Some("v0".into()) }).await.unwrap_err();
        assert_eq!(old, PluginError::Precondition("GitHub has no checksum for emu-linux.zip".into()));
        assert!(t.host.storage.file("cache/releases-upstream.json").is_some(), "the list is kept");
        t.builds.run_action("check").await.unwrap();
        assert!(t.host.storage.file("cache/releases-upstream.json").is_none(), "check forgets the kept list");
    }
}
