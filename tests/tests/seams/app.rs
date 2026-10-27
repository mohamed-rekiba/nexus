//! Seam 1, core app API: commands in, view models and events out.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use nexus_api::*;
use nexus_core::Front;
use nexus_core::{Area, Clock, Command, Core, CoreError, Route};
use nexus_testkit::{FakeHost, Shown, Signal, SpawnedProcess, StubPlugin, TestCore};
use nexus_ui_api::ScreenId;

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

fn key(s: &str) -> GameKey {
    s.parse().unwrap()
}

/// A small emulator for driving the core: a catalog of titles, one build to download, an
/// installer that writes into storage, and a launcher.
#[derive(Clone)]
struct FakeEmu {
    titles: Vec<&'static str>,
    build_url: String,
    build_sha: String,
    build_size: Option<u64>,
    storage: Arc<Mutex<Option<Arc<dyn Storage>>>>,
    exits: Arc<Mutex<Vec<(String, ExitInfo)>>>,
    broken_catalog: bool,
    /// The newest build GitHub would list.
    latest: Arc<Mutex<String>>,
    /// Its launch hands the game over to a process with this text in its command line.
    game_process: Option<String>,
}

impl FakeEmu {
    fn new(titles: Vec<&'static str>) -> Self {
        Self {
            titles,
            build_url: "https://builds.example/emu-1.0.bin".into(),
            build_sha: String::new(),
            build_size: None,
            storage: Arc::default(),
            exits: Arc::default(),
            broken_catalog: false,
            latest: Arc::new(Mutex::new("1.0".into())),
            game_process: None,
        }
    }

    /// Its launch hands the game over, as `steam steam://rungameid/…` does.
    fn handing_over(mut self, game_process: &str) -> Self {
        self.game_process = Some(game_process.into());
        self
    }

    fn storage(&self) -> Arc<dyn Storage> {
        self.storage.lock().unwrap().clone().expect("init ran")
    }

    fn summary(&self, i: usize) -> GameSummary {
        GameSummary {
            id: format!("G{i}"),
            title: self.titles[i].into(),
            platform: "PS4".into(),
            cover: Some(ImageSource::Asset(RelPath::new(format!("covers/{i}.png")).unwrap())),
            tags: vec![],
            release_year: None,
            title_id: None,
            content_id: None,
        }
    }
}

#[async_trait]
impl Catalog for FakeEmu {
    async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
        if self.broken_catalog {
            return Err(PluginError::Network("catalog server is down".into()));
        }
        Ok(Page { items: (0..self.titles.len()).map(|i| self.summary(i)).collect(), total: None })
    }
    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let q = query.text.to_lowercase();
        Ok((0..self.titles.len())
            .filter(|i| self.titles[*i].to_lowercase().contains(&q))
            .map(|i| self.summary(i))
            .collect())
    }
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let i: usize = game.trim_start_matches('G').parse().map_err(|_| PluginError::NotFound(game.into()))?;
        let summary = self.titles.get(i).map(|_| self.summary(i)).ok_or_else(|| PluginError::NotFound(game.into()))?;
        Ok(GameDetails {
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
            summary,
            description: "A game.".into(),
            developer: Some("Dev".into()),
            genres: vec!["Action".into()],
            size_bytes: None,
            background: None,
            facts: vec![],
        })
    }
    async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Downloader for FakeEmu {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan> {
        match request {
            DownloadRequest::EmulatorBuild { version } => {
                // Each version downloads to its own file, as a real build would.
                let version = version.clone().unwrap_or_else(|| self.latest.lock().unwrap().clone());
                Ok(DownloadPlan {
                    label: format!("Emu {version}"),
                    artifacts: vec![Artifact {
                        url: self.build_url.clone(),
                        size: self.build_size,
                        sha256: self.build_sha.clone(),
                        signature: None,
                        dest: RelPath::new(format!("downloads/emu-{version}.bin")).unwrap(),
                        unpack: None,
                    }],
                })
            }
            _ => Err(PluginError::Unsupported("only builds".into())),
        }
    }
}

#[async_trait]
impl Installer for FakeEmu {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(vec![VersionInfo { version: self.latest.lock().unwrap().clone(), notes: None }])
    }
    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        let names = self.storage().list(&RelPath::new("builds").unwrap()).await?;
        Ok(names
            .into_iter()
            .map(|v| InstalledVersion { path: RelPath::new(format!("builds/{v}")).unwrap(), version: v, own: false })
            .collect())
    }
    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let path = RelPath::new(format!("builds/{version}")).unwrap();
        let bin = path.join("emu").unwrap();
        let bytes = self.storage().read(&files[0].dest).await?;
        self.storage().write(&bin, &bytes).await?;
        self.storage().set_executable(&bin).await?;
        Ok(InstalledVersion { version: version.into(), path, own: false })
    }
    async fn uninstall(&self, version: &str) -> PluginResult<()> {
        self.storage().remove(&RelPath::new(format!("builds/{version}")).unwrap()).await
    }
}

#[async_trait]
impl Launcher for FakeEmu {
    async fn prepare(&self, game: &str, config: &serde_json::Value) -> PluginResult<LaunchSpec> {
        let installed = self.installed().await?;
        let build = installed.last().ok_or_else(|| PluginError::Precondition("install the emulator first".into()))?;
        let mut env = BTreeMap::new();
        if let Some(scale) = config.get("scale") {
            env.insert("EMU_SCALE".into(), scale.to_string().into());
        }
        Ok(LaunchSpec {
            program: build.path.join("emu").unwrap().into(),
            args: vec![game.into()],
            env,
            cwd: None,
            game_process: self.game_process.clone(),
        })
    }
    async fn on_exit(&self, game: &str, exit: &ExitInfo) -> PluginResult<()> {
        self.exits.lock().unwrap().push((game.into(), exit.clone()));
        Ok(())
    }
}

#[async_trait]
impl Slots for FakeEmu {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        Ok(match request.slot {
            SlotId::GameBadge => vec![SlotItem {
                id: "region".into(),
                order: Order::default(),
                content: SlotContent::Badge { label: "EU".into(), short: String::new(), tone: Tone::Info, icon: None },
            }],
            _ => vec![],
        })
    }
    async fn run_action(&self, _: &str, _: Option<&str>, _: &serde_json::Value) -> PluginResult<()> {
        Ok(())
    }
}

/// The fake host's `watch_every`: how often the game watcher looks at the desktop.
const WATCH_EVERY: Duration = Duration::from_millis(20);

const PERMISSIONS: &str = "network = [\"builds.example\"]\nprocess = [\"builds/*/emu\"]";

fn emu_plugin(plugin_id: &str, emu: FakeEmu) -> StubPlugin {
    emu_plugin_with(plugin_id, emu, PERMISSIONS)
}

fn emu_plugin_with(plugin_id: &str, emu: FakeEmu, permissions: &str) -> StubPlugin {
    emu_plugin_named(plugin_id, &format!("name = \"{plugin_id}\""), emu, permissions)
}

/// Like [`emu_plugin_with`], with `names`: the manifest's `name` line, and an `emulator` line
/// if it has one.
fn emu_plugin_named(plugin_id: &str, names: &str, emu: FakeEmu, permissions: &str) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\n{names}\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"downloader\", \"installer\", \"launcher\", \"slots\"]\n[permissions]\n{permissions}\n"
    ))
    .provides(move |ctx| {
        *emu.storage.lock().unwrap() = Some(ctx.storage().clone());
        let e = Arc::new(emu.clone());
        Ok(Provided {
            catalog: Some(e.clone()),
            downloader: Some(e.clone()),
            installer: Some(e.clone()),
            launcher: Some(e.clone()),
            slots: Some(e),
            ..Provided::default()
        })
    })
}

/// Boots a core with `emu` as `emu-a`, with its build served over fake HTTP.
async fn core_with(mut emu: FakeEmu) -> (TestCore, FakeEmu) {
    let host = FakeHost::new();
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let core = host.plugin(emu_plugin("emu-a", emu.clone())).boot_core().await;
    (core, emu)
}

async fn install(core: &Core) {
    core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap();
}

#[tokio::test]
async fn the_library_merges_every_catalog_sorted_by_title_with_badges() {
    let a = FakeEmu::new(vec!["Zeta", "Alpha"]);
    let b = FakeEmu::new(vec!["Mu"]);
    let t = FakeHost::new().plugin(emu_plugin("emu-a", a)).plugin(emu_plugin("emu-b", b)).boot_core().await;

    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let home = t.core.view().home;
    let titles: Vec<_> = home.games.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Alpha", "Mu", "Zeta"]);
    assert_eq!(home.games[0].key, "emu-a:G1");
    assert_eq!(home.games[1].key, "emu-b:G0");
    assert_eq!(home.games[0].badges[0].label, "emu-a:region");
    assert_eq!(home.games[0].badges[0].text, "EU");
    // No plugin here describes PS4: its code is its name, in the theme's neutral look.
    assert_eq!(home.games[0].system, nexus_ui_api::GameSystemVm { name: "PS4".into(), color: None });
}

#[tokio::test]
async fn a_broken_catalog_leaves_a_notice_and_the_other_catalogs_still_show() {
    let mut broken = FakeEmu::new(vec!["Hidden"]);
    broken.broken_catalog = true;
    let ok = FakeEmu::new(vec!["Shown"]);
    let t = FakeHost::new().plugin(emu_plugin("emu-a", broken)).plugin(emu_plugin("emu-b", ok)).boot_core().await;

    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Shown"]);
    assert!(view.notices.iter().any(|n| n.contains("catalog server is down")), "{:?}", view.notices);
}

#[tokio::test]
async fn search_opens_the_catalog_on_its_text() {
    let a = FakeEmu::new(vec!["The Last Road", "Road Rage"]);
    let t = FakeHost::new().plugin(emu_plugin("emu-a", a)).boot_core().await;

    t.core.dispatch(Command::Search("road".into())).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::Catalog);
    assert_eq!(view.catalog.games.query, "road");
    assert_eq!(view.home.library.query, "", "the Library keeps its own search");
}

#[tokio::test]
async fn installing_downloads_checks_and_installs_the_build() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    let mut events = t.core.events();

    install(&t.core).await;

    let installed = emu.installed().await.unwrap();
    assert_eq!(installed[0].version, "1.0");
    let emulators = t.core.view().emulators;
    assert_eq!(emulators.emulators[0].installed, vec!["1.0"]);
    assert!(!emulators.emulators[0].busy);
    let mut phases = vec![];
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.next()).await {
        if let Event::InstallChanged { state, .. } = event {
            phases.push(state);
        }
    }
    assert_eq!(phases, [InstallPhase::Downloading, InstallPhase::Installing, InstallPhase::Installed]);
}

#[tokio::test]
async fn an_interrupted_download_resumes_from_where_it_stopped() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    host.http().cut_next_after(&emu.build_url, 9_000);
    let t = host.plugin(emu_plugin("emu-a", emu.clone())).boot_core().await;

    let first = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    assert!(first.is_err(), "the first attempt fails mid-way");
    install(&t.core).await;

    let offsets: Vec<u64> = t.http.requests().into_iter().map(|(_, offset)| offset).collect();
    assert_eq!(offsets, [0, 9_000]);
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_server_that_ignores_ranges_gets_a_clean_restart() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, (0..20_000u32).map(|i| i as u8).collect::<Vec<_>>());
    host.http().cut_next_after(&emu.build_url, 9_000);
    host.http().ignore_ranges();
    let t = host.plugin(emu_plugin("emu-a", emu.clone())).boot_core().await;

    let _ = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    install(&t.core).await;

    assert_eq!(emu.installed().await.unwrap().len(), 1, "the checksum matched after the restart");
}

#[tokio::test]
async fn a_file_with_the_wrong_checksum_is_deleted_and_never_installed() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    host.http().serve(&emu.build_url, vec![1u8; 100]);
    emu.build_sha = "00".repeat(32);
    let t = host.plugin(emu_plugin("emu-a", emu.clone())).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("checksum")), "{err}");
    assert!(emu.installed().await.unwrap().is_empty());
    let plugin_dir = t.data_dir().join("plugins/emu-a/downloads");
    assert!(std::fs::read_dir(&plugin_dir).map(|d| d.count()).unwrap_or(0) == 0, "no partial or bad file is left");
    assert_eq!(t.core.view().downloads.jobs[0].state, "failed");
}

#[tokio::test]
async fn a_download_from_a_host_outside_the_allowlist_is_refused() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_url = "https://evil.example/emu.bin".into();
    emu.build_sha = host.http().serve(&emu.build_url, vec![1u8; 10]);
    let t = host.plugin(emu_plugin("emu-a", emu)).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("evil.example")), "{err}");
    assert!(t.http.requests().is_empty(), "nothing was fetched");
}

#[tokio::test]
async fn an_install_a_crash_interrupted_resumes_at_the_next_start() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    let body = vec![7u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    // What the last run left: a half-downloaded part file and the saved install phase.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body[..5_000]).unwrap();
    std::fs::write(
        host.data_dir().join("config.json"),
        r#"{"app": {"installs": {"emu-a": {"version": "1.0", "from": null, "phase": "downloading"}}}}"#,
    )
    .unwrap();
    let (builder, dir, http, spawner, _) = host.plugin(emu_plugin("emu-a", emu.clone())).into_builder();
    let (kernel, _) = builder.boot().await;
    let mut events = kernel.events().stream();

    let _core = Core::start(kernel).await;

    loop {
        match events.next().await {
            Some(Event::InstallChanged { state: InstallPhase::Installed, .. }) => break,
            Some(_) => {}
            None => panic!("the events ended before the install did"),
        }
    }
    assert_eq!(http.requests(), [(emu.build_url.clone(), 5_000)], "it continued the part file");
    assert_eq!(emu.installed().await.unwrap()[0].version, "1.0");
    drop((dir, spawner));
}

#[tokio::test]
async fn a_download_a_crash_interrupted_resumes_at_the_next_start() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    let body = vec![9u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    // What the last run left: half the file, and the saved download.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body[..5_000]).unwrap();
    let record = serde_json::json!({"plugin": "emu-a", "request": {"type": "emulator_build", "version": null}});
    let config = serde_json::json!({"app": {"downloads": {"pending": record}}});
    std::fs::write(host.data_dir().join("config.json"), config.to_string()).unwrap();
    let (builder, dir, http, spawner, _) = host.plugin(emu_plugin("emu-a", emu.clone())).into_builder();
    let (kernel, _) = builder.boot().await;
    let mut events = kernel.events().stream();

    let core = Core::start(kernel).await;

    let finished = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(Event::DownloadFinished { .. }) => return,
                Some(Event::DownloadFailed { error, .. }) => panic!("{error}"),
                Some(_) => {}
                None => panic!("the events ended before the download did"),
            }
        }
    });
    finished.await.expect("the download resumed and finished");
    assert_eq!(http.requests(), [(emu.build_url.clone(), 5_000)], "it continued the part file");
    assert_eq!(std::fs::read(downloads.join("emu-1.0.bin")).unwrap(), body);
    let saved = core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_none_or(|m| m.is_empty()), "the finished download is no longer saved: {saved}");
    drop((dir, spawner));
}

/// Waits for the first event that `pick` maps to something.
async fn next_event<T>(
    events: &mut futures::stream::BoxStream<'static, Event>,
    pick: impl Fn(&Event) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.next().await.expect("the bus lives as long as the core");
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    })
    .await
    .expect("the event came")
}

fn job_state(core: &Core, job: u64) -> String {
    core.view().downloads.jobs.iter().find(|j| j.id == job).map(|j| j.state.clone()).unwrap_or_default()
}

#[tokio::test]
async fn a_download_is_saved_while_it_runs_and_forgotten_when_it_ends() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    let gate = t.http.hold_after(&emu.build_url, 0);
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };

    t.core.dispatch(Command::Download { plugin: id("emu-a"), request }).await.unwrap();

    let saved = t.core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_some_and(|m| m.len() == 1), "dispatch returns while it runs: {saved}");
    gate.release();
    next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    let saved = t.core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_some_and(|m| m.is_empty()), "{saved}");
}

#[tokio::test]
async fn downloads_beyond_the_limit_wait_in_the_queue() {
    let host = FakeHost::new().max_downloads(1);
    let mut a = FakeEmu::new(vec!["A"]);
    a.build_sha = host.http().serve(&a.build_url, vec![1u8; 10_000]);
    let mut b = FakeEmu::new(vec!["B"]);
    b.build_url = "https://builds.example/b-1.0.bin".into();
    b.build_sha = host.http().serve(&b.build_url, vec![2u8; 10_000]);
    let gate = host.http().hold_after(&a.build_url, 0);
    let t = host.plugin(emu_plugin("emu-a", a)).plugin(emu_plugin("emu-b", b)).boot_core().await;
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };

    t.core.dispatch(Command::Download { plugin: id("emu-a"), request: request.clone() }).await.unwrap();
    t.core.dispatch(Command::Download { plugin: id("emu-b"), request }).await.unwrap();
    next_event(&mut events, |e| matches!(e, Event::DownloadQueued { job: JobId(2), .. }).then_some(())).await;

    assert_eq!(job_state(&t.core, 2), "queued", "one runs at a time");
    gate.release();
    for _ in 0..2 {
        next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    }
    assert_eq!(t.http.requests().len(), 2);
    assert_eq!(job_state(&t.core, 2), "done");
}

#[tokio::test]
async fn a_paused_download_keeps_its_part_and_resumes_where_it_stopped() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    let _gate = t.http.hold_after(&emu.build_url, 8_192);
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };
    t.core.dispatch(Command::Download { plugin: id("emu-a"), request }).await.unwrap();
    let job = next_event(&mut events, |e| match e {
        Event::DownloadProgress { job, received: 8_192, .. } => Some(*job),
        _ => None,
    })
    .await;

    t.core.dispatch(Command::PauseDownload(job)).await.unwrap();

    next_event(&mut events, |e| matches!(e, Event::DownloadPaused { .. }).then_some(())).await;
    assert_eq!(job_state(&t.core, job.0), "paused");
    let part = t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin.part");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), 8_192, "the part file stays");

    t.core.dispatch(Command::ResumeDownload(job)).await.unwrap();

    next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    let offsets: Vec<u64> = t.http.requests().into_iter().map(|(_, offset)| offset).collect();
    assert_eq!(offsets, [0, 8_192]);
    assert_eq!(std::fs::read(t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin")).unwrap(), vec![7u8; 20_000]);
}

#[tokio::test]
async fn cancelling_a_download_deletes_its_part_and_fails_the_install_that_waits_for_it() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    let _gate = t.http.hold_after(&emu.build_url, 4_096);
    let mut events = t.core.events();
    let core = t.core.clone();
    let install =
        tokio::spawn(
            async move { core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await },
        );
    let job = next_event(&mut events, |e| match e {
        Event::DownloadProgress { job, received: 4_096, .. } => Some(*job),
        _ => None,
    })
    .await;

    t.core.dispatch(Command::CancelDownload(job)).await.unwrap();

    let err = install.await.unwrap().unwrap_err();
    assert!(matches!(err, CoreError::Cancelled(j) if j == job), "{err}");
    assert_eq!(job_state(&t.core, job.0), "cancelled");
    assert!(!t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin.part").exists(), "the part file is gone");
    assert!(emu.installed().await.unwrap().is_empty());
    let resumed = t.core.dispatch(Command::ResumeDownload(job)).await;
    assert!(resumed.is_err(), "a cancelled download can't resume");
}

#[tokio::test]
async fn a_part_file_that_already_has_every_byte_is_checked_without_a_request() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    let body = vec![7u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    emu.build_size = Some(body.len() as u64);
    // The last run stopped after the last byte, before the check.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body).unwrap();
    let t = host.plugin(emu_plugin("emu-a", emu.clone())).boot_core().await;

    install(&t.core).await;

    assert!(t.http.requests().is_empty(), "no request: {:?}", t.http.requests());
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn launching_spawns_the_program_from_the_plugin_folder_and_tracks_the_process() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    let plugin = id("emu-a");
    t.core
        .dispatch(Command::SetConfig {
            plugin: plugin.clone(),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({"scale": 2}),
        })
        .await
        .unwrap();
    let mut events = t.core.events();

    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();

    let process = t.spawner.wait_for(1).await;
    assert_eq!(process.command.program, t.data_dir().join("plugins/emu-a/builds/1.0/emu"));
    assert_eq!(process.command.args, ["G0"]);
    assert_eq!(process.command.env.get("EMU_SCALE").map(String::as_str), Some("2"));
    assert!(matches!(events.next().await, Some(Event::ProcessStarted { game, .. }) if game == key("emu-a:G0")));
    assert_eq!(t.core.view().screen, ScreenId::Running);

    process.exit(0);
    loop {
        if let Event::ProcessExited { exit, .. } = events.next().await.expect("the events ended first") {
            assert!(exit.success);
            break;
        }
    }
    assert_eq!(emu.exits.lock().unwrap()[0].0, "G0", "the plugin hears about the exit");
    assert_ne!(t.core.view().screen, ScreenId::Running);
}

#[tokio::test]
async fn stop_kills_a_running_game() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    let mut events = t.core.events();
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let session = loop {
        if let Event::ProcessStarted { session, .. } = events.next().await.expect("the events ended first") {
            break session;
        }
    };

    t.core.dispatch(Command::Stop(session)).await.unwrap();

    loop {
        if let Event::ProcessExited { session: s, exit, .. } = events.next().await.expect("the events ended first") {
            assert_eq!(s, session);
            assert!(!exit.success);
            break;
        }
    }
}

/// Installs the emulator and starts `emu-a:G0`.
async fn play(t: &TestCore) -> (SessionId, SpawnedProcess) {
    install(&t.core).await;
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    let session = SessionId(t.core.view().running.expect("a running game").session);
    (session, process)
}

/// Moves the clock one round of the game watcher at a time until `done` is true, for at most
/// five seconds of it: the watcher's rounds are what change the desktop and the game's state.
async fn until(t: &TestCore, what: &str, done: impl Fn() -> bool) {
    for _ in 0..250 {
        if done() {
            return;
        }
        t.clock.advance(WATCH_EVERY).await;
    }
    panic!("not {what} after five seconds of the clock");
}

async fn closed(events: &mut futures::stream::BoxStream<'static, Event>) -> ExitInfo {
    loop {
        if let Event::ProcessExited { exit, .. } = events.next().await.expect("the events ended first") {
            return exit;
        }
    }
}

#[tokio::test]
async fn a_started_game_comes_to_the_front_and_the_launcher_comes_back_when_it_closes() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;

    until(&t, "the game is in front", || t.core.game_in_front()).await;
    assert_eq!(t.windows.shown(), [Shown::Game(process.id)]);
    let running = t.core.view().running.unwrap();
    assert_eq!(running.state, "running", "its window showed");
    assert!(running.in_front);

    process.exit(0);
    closed(&mut events).await;

    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));
    assert!(!t.core.game_in_front());
    assert_eq!(process.signals(), [Signal::Kill], "what the game left running stops with it");
}

#[tokio::test]
async fn a_game_whose_window_closed_is_stopped_and_the_launcher_comes_back() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    // On macOS a program can keep running after its last window closed.
    t.windows.windows_end_games();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.windows.no_windows();
    closed(&mut events).await;

    assert_eq!(process.signals().first(), Some(&Signal::Terminate), "asked to close, as Stop does");
    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));
    assert!(t.core.view().notices.is_empty(), "the player closed it: no crash notice");
}

#[tokio::test]
async fn a_launcher_s_own_window_closing_before_the_game_s_opens_leaves_the_game_running() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the launcher's menu is in front", || t.core.game_in_front()).await;

    // The player picked "Start Game": the menu closed, and the game takes a while to open.
    t.windows.no_windows();
    t.clock.advance(Duration::from_millis(300)).await;
    assert!(process.signals().is_empty(), "nothing stopped it: {:?}", process.signals());
    assert!(t.core.view().running.is_some(), "the game keeps its session");
}

#[tokio::test]
async fn a_game_a_start_script_left_running_keeps_the_session_until_it_ends() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    // The script starts the game in the background and exits.
    let game = process.id + 1000;
    t.processes.add(process.id, game);
    process.exit(0);

    t.clock.advance(Duration::from_millis(200)).await;
    assert!(t.core.view().running.is_some(), "the game still runs");
    assert!(process.signals().is_empty(), "nothing was killed: {:?}", process.signals());

    t.processes.exit(game);
    let exit = closed(&mut events).await;
    assert!(exit.success, "the script's own exit");
}

#[tokio::test]
async fn the_window_in_front_is_the_game_s_when_a_process_below_it_owns_it() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;
    // A sandbox's process, out of the game's group, draws the game.
    let sandboxed = process.id + 1000;
    t.processes.add(process.id, sandboxed);

    t.windows.set_front(Front::Launcher);
    until(&t, "the launcher is in front", || !t.core.game_in_front()).await;
    t.windows.set_front(Front::Other(sandboxed));
    until(&t, "the sandboxed window counts as the game's", || t.core.game_in_front()).await;
}

#[tokio::test]
async fn stop_reaches_the_processes_a_sandbox_moved_out_of_the_game_s_group() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (session, process) = play(&t).await;
    let sandboxed = process.id + 1000;
    t.processes.add(process.id, sandboxed);
    t.processes.stubborn();

    t.core.dispatch(Command::Stop(session)).await.unwrap();
    closed(&mut events).await;

    let ended = t.processes.ended();
    assert!(ended.first().is_some_and(|(ids, force)| ids.contains(&sandboxed) && !force), "asked first: {ended:?}");
    assert!(ended.iter().any(|(ids, force)| ids.contains(&sandboxed) && *force), "then killed: {ended:?}");
}

/// The command line Steam gives game 7's first process.
const STEAM_GAME: &str = "reaper SteamLaunch AppId=7 -- /games/seven/run";

#[tokio::test]
async fn a_game_another_program_starts_runs_until_its_own_process_ends() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    let (_, launcher) = play(&t).await;
    // `steam steam://rungameid/7` hands the game to Steam and exits.
    launcher.exit(0);
    t.clock.advance(Duration::from_millis(100)).await;
    assert!(t.core.view().running.is_some(), "waiting for the game");
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    t.processes.add(game, game + 1);

    t.clock.advance(Duration::from_millis(700)).await;
    assert!(t.core.view().running.is_some(), "the game runs, past the time a game has to show");
    until(&t, "the game's window came to the front", || t.windows.shown().contains(&Shown::Game(game))).await;

    t.processes.exit(game + 1);
    t.processes.exit(game);
    let exit = closed(&mut events).await;
    assert!(exit.success, "{exit:?}");
    assert!(launcher.signals().is_empty(), "Steam was left alone: {:?}", launcher.signals());
}

#[tokio::test]
async fn stop_ends_the_game_another_program_started_and_leaves_that_program_running() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    // Steam wasn't running: the launch started Steam's client, which stays.
    let (session, steam) = play(&t).await;
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.core.dispatch(Command::Stop(session)).await.unwrap();
    closed(&mut events).await;
    assert_eq!(t.processes.ended().first(), Some(&(vec![game], false)), "the game was asked to close");
    assert!(steam.signals().is_empty(), "Steam's client was left running: {:?}", steam.signals());
}

#[tokio::test]
async fn a_game_another_program_never_starts_ends_with_a_notice() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"]).handing_over("AppId=7 ")).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let mut events = t.core.events();
    let (_, launcher) = play(&t).await;
    launcher.exit(0);

    let exit = closed(&mut events).await;
    assert!(!exit.success, "{exit:?}");
    let notices = t.core.view().notices;
    assert!(notices.iter().any(|n| n.contains("A didn't start")), "{notices:?}");
    assert_eq!(t.core.view().result_prompt, None, "nothing was played");
    assert_eq!(emu.exits.lock().unwrap().len(), 1, "the plugin hears of it");
}

#[tokio::test]
async fn stop_before_the_game_another_program_starts_shows_ends_with_nothing_played() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"]).handing_over("AppId=7 ")).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let mut events = t.core.events();
    let (session, launcher) = play(&t).await;
    // Steam is still starting, and the player gives up.
    t.clock.advance(Duration::from_millis(100)).await;
    t.core.dispatch(Command::Stop(session)).await.unwrap();

    closed(&mut events).await;
    let view = t.core.view();
    assert!(view.notices.is_empty(), "the player stopped it: {:?}", view.notices);
    assert_eq!(view.result_prompt, None, "nothing was played");
    let stats = t.core.kernel().config().get(&["app", "playtime", "emu-a:G0"]);
    assert!(stats.is_null(), "no play time: {stats}");
    assert!(launcher.signals().is_empty(), "Steam was left alone");
}

#[tokio::test]
async fn the_same_game_already_running_before_play_isn_t_taken_for_the_one_play_starts() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    // Started from Steam itself, before Nexus.
    t.processes.start(8000, STEAM_GAME);
    let (_, launcher) = play(&t).await;
    launcher.exit(0);
    t.clock.advance(Duration::from_millis(100)).await;
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    until(&t, "the new game came to the front", || t.windows.shown().contains(&Shown::Game(game))).await;

    t.processes.exit(8000);
    t.clock.advance(Duration::from_millis(200)).await;
    assert!(t.core.view().running.is_some(), "the other one ending doesn't end this one");
    t.processes.exit(game);
    closed(&mut events).await;
}

#[tokio::test]
async fn only_the_newest_three_notices_stay_and_each_closes_on_its_own() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    for n in 1..=4 {
        t.core.report(format!("Problem {n}"));
    }
    let view = t.core.view();
    assert_eq!(view.notices, ["Problem 2", "Problem 3", "Problem 4"], "the oldest goes");
    assert!(view.notice_cards.iter().all(|n| n.kind == nexus_ui_api::NoticeKind::Error), "a problem is an error");
    let ids: Vec<u64> = view.notice_cards.iter().map(|n| n.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "newest last, each its own id: {ids:?}");

    t.core.dispatch(Command::DismissNotice(ids[1])).await.unwrap();
    assert_eq!(t.core.view().notices, ["Problem 2", "Problem 4"], "only that one closes");
    t.core.dispatch(Command::OpenNotice(ids[2])).await.unwrap();
    assert_eq!(t.core.view().notices, ["Problem 2"], "opening one closes only it");
    assert_eq!(t.core.view().screen, ScreenId::Home, "it had nothing to open");
}

#[tokio::test]
async fn what_the_desktop_is_missing_is_said_once_a_run() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.windows.set_problem("Install xdotool so games come to the front and the launcher comes back");
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the notice shows", || !t.core.view().notices.is_empty()).await;
    assert_eq!(t.core.view().notices, ["Install xdotool so games come to the front and the launcher comes back"]);
    assert_eq!(t.core.view().notice_cards[0].kind, nexus_ui_api::NoticeKind::Info, "advice, not a failure");

    process.exit(0);
    closed(&mut events).await;
    t.core.dispatch(Command::DismissNotices).await.unwrap();
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    t.spawner.wait_for(2).await;
    t.clock.advance(Duration::from_millis(100)).await;

    assert!(t.core.view().notices.is_empty(), "not again in the same run: {:?}", t.core.view().notices);
}

#[tokio::test]
async fn after_playing_the_player_says_how_far_it_got_and_it_is_kept() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"));

    t.core.dispatch(Command::ReportResult(Some("menus".into()))).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus");
}

#[tokio::test]
async fn not_now_keeps_no_result() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;

    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "");
}

/// Plays `emu-a:G0` once more, as the `n`th run, and waits until it closes.
async fn play_run(t: &TestCore, n: usize) {
    let mut events = t.core.events();
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    t.spawner.wait_for(n).await.exit(0);
    closed(&mut events).await;
}

#[tokio::test]
async fn how_far_it_got_is_asked_once_for_each_build() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(Some("menus".into()))).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "this build has a result");

    *emu.latest.lock().unwrap() = "2.0".into();
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus", "the old result stays until then");
}

#[tokio::test]
async fn not_now_is_kept_for_the_build() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "not again for this build");
    t.core.dispatch(Command::AskResult(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "Rate how it runs still asks");
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    *emu.latest.lock().unwrap() = "2.0".into();
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
}

#[tokio::test]
async fn two_presses_of_play_start_one_game() {
    let (t, _) = core_with(FakeEmu::new(vec!["A", "B"])).await;
    install(&t.core).await;

    let (a, b) = tokio::join!(
        t.core.dispatch(Command::Launch(key("emu-a:G0"))),
        t.core.dispatch(Command::Launch(key("emu-a:G1"))),
    );

    assert!(a.is_ok() != b.is_ok(), "one starts, one is refused: {a:?} {b:?}");
    t.spawner.wait_for(1).await;
    assert_eq!(t.spawner.spawned().len(), 1);
}

#[tokio::test]
async fn a_desktop_that_can_t_say_leaves_the_pad_to_the_game() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    // As on Windows: no window can be shown, and the front is unknown.
    t.windows.set_front(Front::Unknown);
    t.windows.no_windows();
    let (_, _process) = play(&t).await;

    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;
    assert!(t.core.game_in_front(), "the game's window took the screen as it opened");
    t.core.dispatch(Command::ToggleFocus).await.unwrap();
    assert!(!t.core.game_in_front(), "the PS button gives the pad back");
}

#[tokio::test]
async fn a_game_whose_window_the_desktop_never_saw_is_not_taken_for_closed() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    // As on Wayland: xdotool sees no window of a game that draws to a Wayland surface.
    t.windows.set_front(Front::Unknown);
    t.windows.no_windows();
    let (_, process) = play(&t).await;
    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;

    // Longer than the window may be gone before the game counts as closed.
    t.clock.advance(Duration::from_millis(300)).await;
    assert!(process.signals().is_empty(), "the game keeps running: {:?}", process.signals());
    assert!(t.core.view().running.is_some(), "the game keeps running");
}

#[tokio::test]
async fn the_log_of_the_run_before_is_kept() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let log = t.data_dir().join("logs/emu-a/G0.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, "the crash").unwrap();

    let (_, _process) = play(&t).await;

    let previous = t.data_dir().join("logs/emu-a/G0.previous.log");
    assert_eq!(std::fs::read_to_string(previous).unwrap(), "the crash");
}

#[tokio::test]
async fn the_running_screen_says_starting_until_the_game_shows_a_window() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.windows.no_windows();
    let (_, _process) = play(&t).await;

    assert_eq!(t.core.view().running.unwrap().state, "starting");
    // The host's show_timeout is short in tests: the launcher stops waiting.
    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;
    assert!(t.windows.shown().is_empty());
    assert!(!t.core.game_in_front(), "no window came to the front");
}

#[tokio::test]
async fn the_launcher_stays_behind_a_closed_game_when_the_player_says_so() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::SetReturnOnExit(false)).await.unwrap();
    assert!(!t.core.view().settings.return_on_exit);
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    process.exit(0);
    closed(&mut events).await;

    assert!(!t.windows.shown().contains(&Shown::Launcher), "{:?}", t.windows.shown());
}

#[tokio::test]
async fn the_ps_button_goes_from_the_game_to_the_launcher_and_back() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.core.dispatch(Command::ToggleFocus).await.unwrap();
    assert!(!t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));

    t.core.dispatch(Command::ToggleFocus).await.unwrap();
    assert!(t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Game(process.id)));
}

#[tokio::test]
async fn the_desktop_says_which_window_is_in_front() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    let (session, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    // The player clicked the launcher's window.
    t.windows.set_front(Front::Launcher);
    until(&t, "the launcher is in front", || !t.core.game_in_front()).await;

    // Resume, from the Running screen.
    t.core.dispatch(Command::ShowGame(session)).await.unwrap();
    assert!(t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Game(process.id)));
}

#[tokio::test]
async fn without_the_desktop_s_answer_the_last_window_shown_counts() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.windows.set_front(Front::Unknown);
    let (_, _process) = play(&t).await;

    until(&t, "the game is in front", || t.core.game_in_front()).await;
    t.core.dispatch(Command::ToggleFocus).await.unwrap();
    assert!(!t.core.game_in_front());
}

#[tokio::test]
async fn stop_asks_the_game_to_close_before_it_kills_it() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.spawner.ignore_terminate();
    let mut events = t.core.events();
    let (session, process) = play(&t).await;

    t.core.dispatch(Command::Stop(session)).await.unwrap();
    closed(&mut events).await;

    // Asked, killed when it ignored that, then what it left running swept up.
    assert_eq!(process.signals(), [Signal::Terminate, Signal::Kill, Signal::Kill]);
    assert!(t.core.view().notices.is_empty(), "the player stopped it: no crash notice");
}

#[tokio::test]
async fn a_game_that_closes_right_after_it_starts_leaves_a_notice_with_its_log() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    // Joined a part at a time, as the core does: the notice shows its text, and on Windows a
    // joined "logs/emu-a/G0.log" keeps its forward slashes.
    let log = t.data_dir().join("logs").join("emu-a").join("G0.log");
    assert_eq!(process.command.log.as_deref(), Some(log.as_path()), "its output goes to its log");

    process.exit(3);
    closed(&mut events).await;

    let notices = t.core.view().notices;
    assert_eq!(notices, [format!("A closed right after it started (code 3). Its log: {}", log.display())]);
}

#[tokio::test]
async fn one_game_runs_at_a_time() {
    let (t, _) = core_with(FakeEmu::new(vec!["A", "B"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let (_, _process) = play(&t).await;

    let err = t.core.dispatch(Command::Launch(key("emu-a:G1"))).await.unwrap_err();

    assert_eq!(err.to_string(), "A is running: stop it first");
    assert_eq!(t.spawner.spawned().len(), 1);
}

#[tokio::test]
async fn play_time_adds_up_and_the_game_page_shows_it() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!((detail.played_secs, detail.last_played_days), (0, None), "never played");
    let mut events = t.core.events();
    let (_, process) = play(&t).await;

    // A session shorter than three seconds doesn't count.
    t.clock.advance(Duration::from_millis(3100)).await;
    process.exit(0);
    closed(&mut events).await;

    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!((detail.played_secs, detail.last_played_days), (3, Some(0)));
}

#[tokio::test]
async fn a_program_outside_the_process_allowlist_is_not_spawned() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 10]);
    // Same plugin, but its manifest lets it spawn nothing.
    let t = host.plugin(emu_plugin_with("emu-a", emu, "network = [\"builds.example\"]")).boot_core().await;
    install(&t.core).await;

    let err = t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap_err();

    assert!(matches!(err, CoreError::PermissionDenied { .. }), "{err}");
    assert!(t.spawner.spawned().is_empty());
}

struct AddEnv;
#[async_trait]
impl Middleware<LaunchRequest, LaunchSpec> for AddEnv {
    fn name(&self) -> &str {
        "add-env"
    }
    async fn call(
        &self,
        mut request: LaunchRequest,
        next: Next<'_, LaunchRequest, LaunchSpec>,
    ) -> PluginResult<LaunchSpec> {
        request.spec.env.insert("GAMEMODE".into(), "1".into());
        next.run(request).await
    }
}

struct NeedFirmware;
#[async_trait]
impl Middleware<LaunchRequest, LaunchSpec> for NeedFirmware {
    fn name(&self) -> &str {
        "need-firmware"
    }
    fn order(&self) -> Order {
        Order::priority(10)
    }
    async fn call(
        &self,
        request: LaunchRequest,
        next: Next<'_, LaunchRequest, LaunchSpec>,
    ) -> PluginResult<LaunchSpec> {
        if request.spec.args.contains(&"G1".into()) {
            return Err(PluginError::Precondition("firmware missing".into()));
        }
        next.run(request).await
    }
}

#[tokio::test]
async fn launch_middleware_can_change_the_spec_or_stop_the_launch() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A", "B"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 10]);
    let tools = StubPlugin::from_toml(
        "id = \"tools\"\nkind = \"emulator\"\nname = \"Tools\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"launch_middleware\"]\nactivation = \"eager\"",
    )
    .provides(|_| {
        Ok(Provided { launch_middleware: vec![Arc::new(AddEnv), Arc::new(NeedFirmware)], ..Provided::default() })
    });
    let t = host.plugin(emu_plugin("emu-a", emu)).plugin(tools).boot_core().await;
    install(&t.core).await;

    let mut events = t.core.events();
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    assert_eq!(process.command.env.get("GAMEMODE").map(String::as_str), Some("1"));
    // One game runs at a time: close this one first.
    process.exit(0);
    while !matches!(events.next().await, Some(Event::ProcessExited { .. })) {}

    let err = t.core.dispatch(Command::Launch(key("emu-a:G1"))).await.unwrap_err();
    assert!(err.to_string().contains("firmware missing"), "{err}");
    assert_eq!(t.spawner.spawned().len(), 1);
    // The player sees why Play did nothing.
    assert_eq!(t.core.view().notices, ["Can't start G1: firmware missing"]);
}

#[tokio::test]
async fn the_first_game_is_selected_with_its_details_once_the_library_loads() {
    let (t, _) = core_with(FakeEmu::new(vec!["Beta", "Alpha"])).await;

    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.selected, Some(0), "the first game by title");
    assert_eq!(view.detail.map(|d| d.title), Some("Alpha".to_owned()), "its details are in, for the hero");
}

#[tokio::test]
async fn navigation_and_selection_live_in_the_core() {
    let (t, _) = core_with(FakeEmu::new(vec!["A", "B"])).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    t.core.dispatch(Command::Select(key("emu-a:G1"))).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G1"))).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::GameDetail);
    assert_eq!(view.detail.as_ref().unwrap().title, "B");
    assert!(view.can_go_back);

    t.core.dispatch(Command::Navigate(Route::Downloads)).await.unwrap();
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::GameDetail);
    t.core.dispatch(Command::Back).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::Home);
    assert_eq!(view.home.selected, Some(1));
    assert!(view.home.games[1].selected);
}

#[tokio::test]
async fn settings_are_checked_by_the_plugin_before_they_are_stored() {
    struct Strict;
    #[async_trait]
    impl Configurator for Strict {
        async fn schema(&self, _: &ConfigScope) -> PluginResult<serde_json::Value> {
            Ok(serde_json::json!({"type": "object", "properties": {"scale": {"type": "integer", "title": "Scale"}}}))
        }
        async fn defaults(&self, _: &ConfigScope) -> PluginResult<serde_json::Value> {
            Ok(serde_json::json!({"scale": 1}))
        }
        async fn validate(&self, _: &ConfigScope, value: &serde_json::Value) -> PluginResult<()> {
            match value.get("scale").and_then(|s| s.as_i64()) {
                Some(1..=4) => Ok(()),
                _ => Err(PluginError::InvalidInput("scale must be 1 to 4".into())),
            }
        }
    }
    let plugin = StubPlugin::from_toml(
        "id = \"cfg\"\nkind = \"emulator\"\nname = \"Cfg\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\ncapabilities = [\"configurator\"]",
    )
    .provides(|_| Ok(Provided { configurator: Some(Arc::new(Strict)), ..Provided::default() }));
    let t = FakeHost::new().plugin(plugin).boot_core().await;
    let set = |v| Command::SetConfig { plugin: id("cfg"), scope: ConfigScope::Emulator, value: v };

    let err = t.core.dispatch(set(serde_json::json!({"scale": 9}))).await.unwrap_err();
    assert!(err.to_string().contains("scale must be 1 to 4"), "{err}");
    t.core.dispatch(set(serde_json::json!({"scale": 3}))).await.unwrap();
    assert_eq!(t.core.config_value(&id("cfg"), &ConfigScope::Emulator).await.unwrap(), serde_json::json!({"scale": 3}));
}

#[tokio::test]
async fn a_download_follows_redirects_inside_the_allowlist_and_keeps_its_range() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    let mirror = "https://cdn.builds.example/files/emu-1.0.bin";
    emu.build_sha = host.http().serve(mirror, vec![3u8; 20_000]);
    host.http().redirect(&emu.build_url, "//cdn.builds.example/files/emu-1.0.bin");
    host.http().cut_next_after(mirror, 9_000);
    let t = host.plugin(emu_plugin_with("emu-a", emu.clone(), "network = [\"*.builds.example\"]")).boot_core().await;

    let _ = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    install(&t.core).await;

    let requests = t.http.requests();
    let expected = [
        (emu.build_url.clone(), 0),
        (mirror.to_owned(), 0),
        (emu.build_url.clone(), 9_000),
        (mirror.to_owned(), 9_000),
    ];
    assert_eq!(requests, expected, "each hop asks for the same range");
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_out_of_the_allowlist_is_refused_before_it_is_fetched() {
    let host = FakeHost::new();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_sha = host.http().serve("https://evil.example/emu.bin", vec![1u8; 10]);
    host.http().redirect(&emu.build_url, "https://evil.example/emu.bin");
    let t = host.plugin(emu_plugin("emu-a", emu)).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("evil.example")), "{err}");
    let fetched: Vec<String> = t.http.requests().into_iter().map(|(url, _)| url).collect();
    assert_eq!(fetched, ["https://builds.example/emu-1.0.bin"], "nothing past the refused hop");
}

#[tokio::test]
async fn a_plugin_s_own_requests_follow_redirects_only_inside_its_allowlist() {
    let host = FakeHost::new();
    host.http().serve("https://cdn.builds.example/index.json", b"{}".to_vec());
    host.http().redirect("https://builds.example/index.json", "https://cdn.builds.example/index.json");
    host.http().redirect("https://builds.example/away", "https://evil.example/x");
    let http: Arc<Mutex<Option<Arc<dyn HttpClient>>>> = Arc::default();
    let slot = http.clone();
    let plugin = StubPlugin::from_toml(
        "id = \"web\"\nkind = \"emulator\"\nname = \"W\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         [permissions]\nnetwork = [\"*.builds.example\"]",
    )
    .provides(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.http().clone());
        Ok(Provided::default())
    });
    let t = host.plugin(plugin).boot_core().await;
    t.core.kernel().provided(&id("web")).await.unwrap();
    let http = http.lock().unwrap().clone().unwrap();

    let ok = http.send(HttpRequest::get("https://builds.example/index.json")).await.unwrap();
    assert_eq!((ok.status, ok.body), (200, b"{}".to_vec()));
    let err = http.send(HttpRequest::get("https://builds.example/away")).await.unwrap_err();
    assert!(matches!(&err, PluginError::PermissionDenied(m) if m.contains("evil.example")), "{err:?}");
    assert!(!t.http.sent().iter().any(|u| u.contains("evil")), "{:?}", t.http.sent());
}

/// What the Emulators screen shows as installed for `emu-a`.
async fn installed(core: &Core) -> Vec<String> {
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = core.view();
    view.emulators.emulators.into_iter().find(|e| e.plugin == id("emu-a")).map(|e| e.installed).unwrap_or_default()
}

#[tokio::test]
async fn playing_a_game_whose_emulator_isn_t_installed_asks_first_then_installs_and_plays() {
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_size = Some(20_000);
    let (t, _) = core_with(emu).await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!((prompt.emulator.as_str(), prompt.version.as_str()), ("emu-a", "1.0"));
    assert_eq!((prompt.game.as_str(), prompt.size_bytes), ("A", Some(20_000)));
    assert!(installed(&t.core).await.is_empty() && t.spawner.spawned().is_empty(), "nothing happens yet");

    t.core.dispatch(Command::ConfirmInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    let process = t.spawner.wait_for(1).await;
    assert!(process.command.program.ends_with("builds/1.0/emu"), "{:?}", process.command.program);
}

#[tokio::test]
async fn a_plugin_named_by_its_console_names_its_program_where_a_sentence_means_the_program() {
    let host = FakeHost::new().first_run();
    let mut emu = FakeEmu::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let names = "name = \"PS5\"\nemulator = \"KytyPS5\"";
    let t = host.plugin(emu_plugin_named("emu-a", names, emu, PERMISSIONS)).boot_core().await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let steps: Vec<String> = t.core.view().boot.steps.into_iter().map(|s| s.label).collect();
    assert!(steps.contains(&"KytyPS5 emulator".to_owned()), "the first start's step: {steps:?}");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.expect("the Game Hub");
    assert_eq!(detail.launch_hint, "Play installs KytyPS5 first");

    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!(prompt.emulator, "KytyPS5", "the program downloads, not the console");
    t.core.dispatch(Command::CancelInstall).await.unwrap();

    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!((row(&t).name, row(&t).build_label), ("PS5".to_owned(), String::new()), "the list names the console");
    let builds = t.core.view().emulators.emulators.into_iter().find(|e| e.plugin == id("emu-a")).unwrap();
    assert_eq!(builds.name, "KytyPS5", "the builds are the program's");
    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!((row(&t).build, row(&t).build_label), ("1.0".to_owned(), "KytyPS5 1.0".to_owned()));

    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    t.spawner.wait_for(1).await;
    let running = t.core.view().running.expect("the game runs");
    assert_eq!(
        (running.emulator.as_str(), running.page.as_str()),
        ("KytyPS5", "PS5"),
        "the keys' page is the plugin's"
    );
}

#[tokio::test]
async fn not_now_leaves_the_emulator_uninstalled() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    t.core.dispatch(Command::CancelInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    assert!(installed(&t.core).await.is_empty());
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_newer_build_installs_by_itself_when_no_game_runs() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    *emu.latest.lock().unwrap() = "2.0".into();

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
}

#[tokio::test]
async fn an_update_waits_until_the_emulator_s_game_closes() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    *emu.latest.lock().unwrap() = "2.0".into();

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"], "not while the game runs");

    process.exit(0);
    for _ in 0..100 {
        if installed(&t.core).await.contains(&"2.0".to_owned()) {
            return;
        }
        t.clock.advance(WATCH_EVERY).await;
    }
    panic!("the update didn't run after the game closed");
}

#[tokio::test]
async fn automatic_updates_can_be_turned_off() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    assert!(t.core.view().settings.auto_update, "on by default, as in the old launcher");
    t.core.dispatch(Command::SetAutoUpdate(false)).await.unwrap();
    *emu.latest.lock().unwrap() = "2.0".into();

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"]);
    assert!(!t.core.view().settings.auto_update);
}

#[tokio::test]
async fn the_size_of_everything_is_saved_and_stays_between_0_8_and_1_4() {
    let t = FakeHost::new().boot_core().await;
    assert_eq!(t.core.view().settings.scale, 1.0, "the theme's own size by default");
    t.core.dispatch(Command::SetScale(1.2)).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 1.2);
    t.core.dispatch(Command::SetScale(3.0)).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 1.4);
    t.core.dispatch(Command::SetScale(0.1)).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 0.8);
    assert!(t.core.dispatch(Command::SetScale(f32::NAN)).await.is_err());
    assert_eq!(t.core.view().settings.scale, 0.8);
}

#[tokio::test]
async fn start_at_login_goes_through_the_desktop_and_stays_off_when_the_system_refuses() {
    let t = FakeHost::new().boot_core().await;
    let settings = t.core.view().settings;
    assert!(settings.can_start_at_login);
    assert!(!settings.start_at_login, "off by default");
    t.core.dispatch(Command::SetStartAtLogin(true)).await.unwrap();
    assert!(t.core.view().settings.start_at_login);
    t.core.dispatch(Command::SetStartAtLogin(false)).await.unwrap();
    assert!(!t.core.view().settings.start_at_login);
    assert_eq!(t.login_session.logins(), [true, false]);

    t.login_session.refuse_login("reg.exe: access is denied");
    let refused = t.core.dispatch(Command::SetStartAtLogin(true)).await.unwrap_err();
    assert!(refused.to_string().contains("access is denied"), "{refused}");
    let settings = t.core.view().settings;
    assert!(!settings.start_at_login, "it stays off");
    assert_eq!(settings.start_at_login_error, "reg.exe: access is denied");
}

#[tokio::test]
async fn start_at_login_is_hidden_when_this_build_isn_t_installed() {
    let host = FakeHost::new();
    host.login_session().no_login();
    let t = host.boot_core().await;
    assert!(!t.core.view().settings.can_start_at_login);
    assert!(t.core.dispatch(Command::SetStartAtLogin(true)).await.is_err());
    assert!(t.login_session.logins().is_empty(), "the desktop is not asked");
}

#[tokio::test]
async fn a_game_doesn_t_start_while_its_emulator_updates() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    install(&t.core).await;
    *emu.latest.lock().unwrap() = "2.0".into();
    let gate = t.http.hold_after(&emu.build_url, 10);

    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;
    let err = t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap_err();
    assert!(err.to_string().contains("updating"), "{err}");
    assert!(t.spawner.spawned().is_empty());

    gate.release();
    update.await.unwrap().unwrap();
    t.core.dispatch(Command::Launch(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.spawner.wait_for(1).await.command.args, ["G0"]);
}

#[tokio::test]
async fn opening_another_plugin_replaces_the_open_one_so_back_goes_to_the_list() {
    let t = FakeHost::new()
        .plugin(emu_plugin("emu-a", FakeEmu::new(vec!["A"])))
        .plugin(emu_plugin("emu-b", FakeEmu::new(vec!["B"])))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-a")))).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-b")))).await.unwrap();
    assert_eq!(t.core.view().plugin_page.map(|p| p.plugin.id), Some(id("emu-b")));

    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn emulators_and_themes_each_have_their_own_list() {
    let t = FakeHost::new().plugin(emu_plugin("emu-a", FakeEmu::new(vec!["A"]))).boot_core().await;
    t.core.dispatch(Command::Navigate(Route::Themes)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Themes);
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn an_emulator_s_row_shows_its_installed_build_not_the_plugin_s_version() {
    let (t, _emu) = core_with(FakeEmu::new(vec!["A"])).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!(row(&t).build, "", "not installed yet");

    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(row(&t).build, "1.0");
}

#[tokio::test]
async fn the_game_watcher_reads_the_desktop_every_round_on_the_clock_and_not_between() {
    let (t, _) = core_with(FakeEmu::new(vec!["A"])).await;
    play(&t).await;
    t.until(Area::Sessions, |core| core.game_in_front()).await;

    // A round just saw the game in front: the next is `watch_every` away.
    t.windows.set_front(Front::Launcher);
    t.clock.advance(WATCH_EVERY / 2).await;
    assert!(t.core.game_in_front(), "not read between two rounds");
    t.clock.advance(WATCH_EVERY / 2 + Duration::from_millis(3)).await;
    assert!(!t.core.game_in_front(), "the next round read the launcher in front");
}

#[tokio::test]
async fn emulator_updates_are_looked_for_a_minute_after_the_start_on_the_clock() {
    let (t, emu) = core_with(FakeEmu::new(vec!["A"])).await;
    let started = t.clock.now();
    install(&t.core).await;
    *emu.latest.lock().unwrap() = "2.0".into();

    t.clock.advance(Duration::from_secs(59) - (t.clock.now() - started)).await;
    assert_eq!(installed(&t.core).await, ["1.0"], "not before the minute");
    t.clock.advance(Duration::from_secs(2)).await;
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()), "the check found and installed 2.0");
}
