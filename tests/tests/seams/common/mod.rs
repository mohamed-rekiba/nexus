//! A composed host for seam 3: the core and the theme host, wired the way the app wires them,
//! on Slint's testing backend.

#![allow(dead_code)] // Each test binary uses part of this module.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use i_slint_backend_testing::ElementHandle;
use nexus_api::*;
use nexus_core::Clock;
use nexus_core::{Area, Command, Core};
use nexus_testkit::{FakeClock, FakeHost, StubPlugin, TestCore};
use nexus_ui_host::{ThemeHost, UiRequest};
use slint_interpreter::ComponentHandle;

/// `path` with this system's separator: `~/Games` is `~\Games` on Windows, as the app shows it.
pub fn native(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// A full path on this system for a Unix one: `/x` stays `/x`, and is `C:\x` on Windows.
pub fn full_path(unix: &str) -> String {
    if cfg!(windows) { format!("C:{}", native(unix)) } else { unix.to_owned() }
}

pub fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

pub struct Composed {
    pub rt: tokio::runtime::Runtime,
    pub test: TestCore,
    pub ui: ThemeHost,
    pub requests: Rc<RefCell<Vec<UiRequest>>>,
}

/// Boots a core with the default theme plus whatever `build` adds, and a theme host on it.
/// Starts Slint's testing backend for this thread.
pub fn composed(build: impl FnOnce(FakeHost) -> FakeHost) -> Composed {
    thread_local!(static SLINT: () = quiet_testing_backend());
    SLINT.with(|_| {});
    composed_on_current_platform(build)
}

/// False for the log line Slint's element search writes for the items its interpreter keeps no
/// element info for (see [`quiet_testing_backend`]); every test platform drops it.
pub fn keeps_log_line(arguments: &std::fmt::Arguments) -> bool {
    !arguments.to_string().starts_with("The use of the ElementHandle API requires")
}

/// Slint's testing backend, as `i_slint_backend_testing::init_no_event_loop` sets it up, minus one
/// log line. Slint 1.18's interpreter keeps no element info for the items it adds itself (layout
/// and clip wrappers), so every accessible-label search logs "The use of the ElementHandle API
/// requires the presence of debug info…" for each of them, tens of thousands of times a run;
/// `SLINT_EMIT_DEBUG_INFO` doesn't change that. The search works all the same. Every other log
/// line goes through. It doesn't forward `bind_context`: the backend only needs the context for
/// its event loop, which a backend without threading doesn't have.
fn quiet_testing_backend() {
    use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
    use slint::platform::{Clipboard, EventLoopProxy, Platform, PlatformError, WindowAdapter};

    struct Quiet(TestingBackend);

    impl Platform for Quiet {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            self.0.create_window_adapter()
        }
        fn duration_since_start(&self) -> std::time::Duration {
            self.0.duration_since_start()
        }
        fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
            self.0.set_clipboard_text(text, clipboard)
        }
        fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
            self.0.clipboard_text(clipboard)
        }
        fn run_event_loop(&self) -> Result<(), PlatformError> {
            self.0.run_event_loop()
        }
        fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
            self.0.new_event_loop_proxy()
        }
        fn open_url(&self, url: &str) -> Result<(), PlatformError> {
            self.0.open_url(url)
        }
        fn debug_log(&self, arguments: std::fmt::Arguments) {
            if keeps_log_line(&arguments) {
                self.0.debug_log(arguments)
            }
        }
    }

    // renderer_name exists only when the testing backend is built with a renderer.
    #[allow(clippy::needless_update)] // the update fills renderer_name when it exists
    let options = TestingBackendOptions { mock_time: true, threading: false, ..Default::default() };
    slint::platform::set_platform(Box::new(Quiet(TestingBackend::new(options)))).expect("platform already initialized");
}

/// Like [`composed`], for a test that set its own Slint platform.
pub fn composed_on_current_platform(build: impl FnOnce(FakeHost) -> FakeHost) -> Composed {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let host = build(FakeHost::new().built_in(nexus_api::PluginBox::theme(theme_default::DefaultTheme)));
    let test = rt.block_on(host.boot_core());
    let requests: Rc<RefCell<Vec<UiRequest>>> = Rc::default();
    let sink = requests.clone();
    let ui = rt.block_on(ThemeHost::new(test.core.clone(), Rc::new(move |r| sink.borrow_mut().push(r))));
    Composed { rt, test, ui, requests }
}

impl Composed {
    pub fn core(&self) -> &Core {
        &self.test.core
    }

    /// Runs a command on the core, then renders.
    pub fn dispatch(&mut self, command: Command) {
        self.rt.block_on(self.test.core.dispatch(command)).unwrap();
        self.ui.render().unwrap();
    }

    /// Waits until each catalog on the Catalog is the copy fetched from the catalog address,
    /// then renders: the Game Catalog fetches in the background, and the first read gets the
    /// bundled copy.
    pub fn catalogs_fetched(&mut self) {
        self.rt.block_on(catalogs_fetched(&self.test.core));
        self.ui.render().unwrap();
    }

    /// Runs the core until `done` holds: see [`TestCore::until`]. It doesn't render.
    pub fn until(&self, area: Area, done: impl FnMut(&Core) -> bool) {
        self.rt.block_on(self.test.until(area, done));
    }

    /// Moves the core's clock on by `by`: see [`nexus_testkit::FakeClock::advance`]. It
    /// doesn't render.
    pub fn advance(&self, by: std::time::Duration) {
        self.rt.block_on(self.test.clock.advance(by));
    }

    /// Runs the core until every task waits: see [`nexus_testkit::FakeClock::run_until_idle`].
    /// It doesn't render.
    pub fn run_until_idle(&self) {
        self.rt.block_on(self.test.clock.run_until_idle());
    }

    /// Carries out what the theme asked for, as the app would.
    pub fn pump(&mut self) {
        let requests: Vec<UiRequest> = self.requests.borrow_mut().drain(..).collect();
        for request in requests {
            match request {
                UiRequest::Core(command) => self.dispatch(command),
                UiRequest::SwitchTheme(theme) => self.ui.switch(&theme).unwrap(),
                UiRequest::PlaySound(_)
                | UiRequest::OpenUrl(_)
                | UiRequest::OpenPath(_)
                | UiRequest::Quit
                | UiRequest::Restart
                | UiRequest::LogOut
                | UiRequest::Trailer(_) => {}
                UiRequest::Render => self.ui.render().unwrap(),
            }
        }
    }

    /// Loads every image the last render asked for, then renders again.
    pub fn load_images(&mut self) {
        for key in self.ui.wanted_images() {
            let bytes = self.rt.block_on(self.test.core.image_bytes(&key.0)).map_err(|e| e.to_string());
            self.ui.add_image(key, bytes);
        }
        // As the app: images coming in bind the last view again.
        self.ui.render_images().unwrap();
    }

    pub fn instance(&self) -> &slint_interpreter::ComponentInstance {
        self.ui.instance().expect("a theme is shown")
    }

    /// How many rendered elements carry this accessible label.
    pub fn count(&self, label: &str) -> usize {
        ElementHandle::find_by_accessible_label(self.instance(), label).count()
    }

    pub fn first(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(self.instance(), label).next().unwrap_or_else(|| panic!("no {label}"))
    }

    pub fn resize(&self, width: f32, height: f32) {
        self.instance().window().set_size(slint::LogicalSize::new(width, height));
    }
}

/// Waits until each catalog the core has is the copy fetched from the catalog address: the Game
/// Catalog fetches in the background, and the first read gets the bundled copy. It fails when
/// the catalogs never change to that, rather than letting the test go on with the bundled copy
/// and fail somewhere that doesn't say why.
pub async fn catalogs_fetched(core: &Core) {
    nexus_testkit::until(core, Area::Catalogs, |core| {
        let sources = core.view().catalog.sources;
        !sources.is_empty() && sources.iter().all(|s| s.contains(": fetched from "))
    })
    .await;
}

/// Moves the core's clock one round of the game watcher (the fake host's `watch_every`) at a
/// time until `done` is true, for at most five seconds of it, then shows it: the watcher's
/// rounds are what change the desktop and the game's state.
pub fn watch_until(host: &mut Composed, what: &str, done: impl Fn(&Core) -> bool) {
    let mut rounds = 0;
    while !done(host.core()) {
        assert!(rounds < 250, "not {what} after five seconds of the clock");
        host.advance(std::time::Duration::from_millis(20));
        rounds += 1;
    }
    host.ui.render().unwrap();
    settle(host);
}

/// Lets the theme's moves reach the core, as the app would: a move in the Home row or a grid
/// goes to the core a moment after the last one. It moves the clock twice, as [`settle`], then
/// carries out what the theme asked for.
pub fn rest(host: &mut Composed) {
    for _ in 0..2 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(150));
        host.ui.render().unwrap();
    }
    host.pump();
}

/// Lets entrance animations finish: the testing backend's clock moves only when told to. It
/// moves twice: a step fires the timers due, then runs change handlers, so a timer that a
/// handler starts fires on the next step.
pub fn settle(host: &mut Composed) {
    for _ in 0..2 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(3));
        host.ui.render().unwrap();
    }
}

/// Writes a data-only theme folder `<parent>/<theme_id>` with `theme.toml` built from
/// `theme_section` and the given files.
pub fn write_theme(parent: &Path, theme_id: &str, theme_section: &str, files: &[(&str, &str)]) {
    let dir = parent.join(theme_id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("theme.toml"),
        format!(
            "id = \"{theme_id}\"\nkind = \"theme\"\nname = \"{theme_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
             capabilities = [\"theme\"]\n[runtime]\ntype = \"data\"\n[theme]\nui_api = \"^0.1\"\n{theme_section}\n"
        ),
    )
    .unwrap();
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

struct Games {
    titles: Vec<&'static str>,
    platform: &'static str,
    details_fail: bool,
    /// The details have no description.
    quiet: bool,
}

#[async_trait]
impl Catalog for Games {
    async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
        let items = self
            .titles
            .iter()
            .enumerate()
            .map(|(i, t)| GameSummary {
                id: format!("G{i}"),
                title: (*t).into(),
                platform: self.platform.into(),
                cover: None,
                tags: vec![],
                release_year: None,
                title_id: Some(format!("CUSA0000{i}")),
                content_id: None,
            })
            .collect();
        Ok(Page { items, total: None })
    }
    async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        Ok(self.list(PageRequest::ALL).await?.items)
    }
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        if self.details_fail {
            return Err(PluginError::Other(format!("no details for {game}")));
        }
        let summary = self.list(PageRequest::ALL).await?.items.into_iter().find(|g| g.id == game);
        let summary = summary.ok_or_else(|| PluginError::NotFound(game.into()))?;
        Ok(GameDetails {
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
            description: if self.quiet { String::new() } else { format!("About {}", summary.title) },
            summary,
            developer: None,
            genres: vec![],
            size_bytes: Some(7_610_000_000),
            background: None,
            facts: vec![
                GameFact::new("region", "Region", "Europe"),
                GameFact::new("age", "Age rating", "12+"),
                GameFact::new("version", "Version", "01.000.002"),
                GameFact::new("title-id", "Title ID", game.to_uppercase()),
            ],
        })
    }
    async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Slots for Games {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        Ok(match request.slot {
            SlotId::GameBadge => vec![SlotItem {
                id: "region".into(),
                order: Order::default(),
                content: SlotContent::Badge { label: "EU".into(), short: String::new(), tone: Tone::Info, icon: None },
            }],
            SlotId::GameAction => vec![SlotItem {
                id: "verify".into(),
                order: Order::default(),
                content: SlotContent::Action {
                    label: "Verify files".into(),
                    action: "verify".into(),
                    args: serde_json::Value::Null,
                    style: ActionStyle::Secondary,
                    icon: None,
                },
            }],
            SlotId::GameDetailSection => vec![SlotItem {
                id: "info".into(),
                order: Order::default(),
                content: SlotContent::Section {
                    title: "Emulator".into(),
                    rows: vec![SectionRow { label: "Build".into(), value: "1.0".into() }],
                },
            }],
            _ => vec![],
        })
    }
    async fn run_action(&self, _: &str, _: Option<&str>, _: &serde_json::Value) -> PluginResult<()> {
        Ok(())
    }
}

/// An emulator plugin with a catalog of `titles`, each with a `region` badge.
pub fn games(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    games_on(plugin_id, "PS4", titles)
}

/// Like [`games`], on `platform`.
pub fn games_on(plugin_id: &str, platform: &'static str, titles: Vec<&'static str>) -> StubPlugin {
    catalog(plugin_id, platform, titles, false, false, "")
}

/// Like [`games`], from an emulator whose games need `requires` (TOML for its `[requires]`
/// section).
pub fn games_needing(plugin_id: &str, requires: &str, titles: Vec<&'static str>) -> StubPlugin {
    catalog(plugin_id, "PS5", titles, false, false, requires)
}

/// Like [`games`], with details that have no description.
pub fn games_without_description(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    catalog(plugin_id, "PS4", titles, false, true, "")
}

/// Like [`games`], with a catalog whose details always fail.
pub fn games_without_details(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    catalog(plugin_id, "PS4", titles, true, false, "")
}

fn catalog(
    plugin_id: &str,
    platform: &'static str,
    titles: Vec<&'static str>,
    details_fail: bool,
    quiet: bool,
    requires: &str,
) -> StubPlugin {
    // Its system as the real plugins describe theirs: PS5 neutral, PS4 blue.
    let system = match platform {
        "PS5" => "[system]\nid = \"PS5\"\nname = \"PlayStation 5\"\n",
        "PS4" => "[system]\nid = \"PS4\"\nname = \"PlayStation 4\"\ncolor = \"#8fc2ff\"\n",
        _ => "",
    };
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"{plugin_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"slots\"]\n{system}[requires]\n{requires}"
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform, details_fail, quiet });
        Ok(Provided { catalog: Some(games.clone()), slots: Some(games), ..Provided::default() })
    })
}

/// KytyPS5 with no games of its own, which installs from the packages folder: the emulator a
/// catalog's PS5 offers run with. Its games need `requires` (TOML for its `[requires]` section).
pub fn offering_emulator(requires: &str) -> StubPlugin {
    struct Emu;
    #[async_trait]
    impl Catalog for Emu {
        async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
            Ok(Page { items: vec![], total: None })
        }
        async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
            Ok(vec![])
        }
        async fn details(&self, game: &str) -> PluginResult<GameDetails> {
            Err(PluginError::NotFound(game.into()))
        }
        async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
            Ok(None)
        }
    }
    StubPlugin::from_toml(&format!(
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"PS5\"\nemulator = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         console = \"PS5\"\n\
         capabilities = [\"catalog\", \"launcher\"]\n[permissions]\nprocess = [\"bin/*\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n[requires]\n{requires}"
    ))
    .provides(|_| Ok(Provided { catalog: Some(Arc::new(Emu)), launcher: Some(Arc::new(Play)), ..Provided::default() }))
}

/// The test-only theme in `tests/fixtures/themes`: a grid that extends the default theme.
pub fn fixture_themes() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/themes")
}

struct Play;

#[async_trait]
impl Launcher for Play {
    async fn prepare(&self, game: &str, _: &serde_json::Value) -> PluginResult<LaunchSpec> {
        Ok(LaunchSpec {
            program: RelPath::new("bin/game").unwrap().into(),
            args: vec![game.into()],
            env: Default::default(),
            cwd: None,
            game_process: None,
        })
    }
}

/// Like [`games`], with a launcher, so games can run.
pub fn playable_games(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    playable_games_with(plugin_id, titles, "")
}

/// Like [`playable_games`], with `more` added to its manifest.
pub fn playable_games_with(plugin_id: &str, titles: Vec<&'static str>, more: &str) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"{plugin_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"slots\", \"launcher\"]\n[permissions]\nprocess = [\"bin/*\"]\n{more}"
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform: "PS4", details_fail: false, quiet: false });
        Ok(Provided {
            catalog: Some(games.clone()),
            slots: Some(games),
            launcher: Some(Arc::new(Play)),
            ..Provided::default()
        })
    })
}

/// Like [`games`], with the community's result for each: Playable, last tested on
/// 2026-09-01.
pub fn tested_games(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    struct Tested;
    #[async_trait]
    impl Compatibility for Tested {
        async fn status(&self, _: &str) -> PluginResult<CompatInfo> {
            Ok(CompatInfo {
                level: CompatLevel::Playable,
                notes: Some("Tested on Linux".into()),
                source: Some("Linux".into()),
                page: None,
                tested: Some("2026-09-01".into()),
            })
        }
    }
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"{plugin_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"slots\", \"compatibility\"]"
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform: "PS4", details_fail: false, quiet: false });
        Ok(Provided {
            catalog: Some(games.clone()),
            slots: Some(games),
            compatibility: Some(Arc::new(Tested)),
            ..Provided::default()
        })
    })
}

/// Like [`playable_games`], with keyboard keys in its manifest (`[[keymap]]`).
pub fn games_with_keys(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"{plugin_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"slots\", \"launcher\"]\n[permissions]\nprocess = [\"bin/*\"]\n\
         [[keymap]]\nbutton = \"Cross\"\nkeys = \"J\""
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform: "PS4", details_fail: false, quiet: false });
        Ok(Provided {
            catalog: Some(games.clone()),
            slots: Some(games),
            launcher: Some(Arc::new(Play)),
            ..Provided::default()
        })
    })
}

/// Like [`playable_games`], with an emulator to install first: no build is installed, and
/// version 1.0 is a 300 MB download.
pub fn games_needing_an_emulator(plugin_id: &str, titles: Vec<&'static str>) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"Emu\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"slots\", \"launcher\", \"installer\", \"downloader\"]\n\
         [permissions]\nprocess = [\"bin/*\"]\nnetwork = [\"builds.example\"]"
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform: "PS4", details_fail: false, quiet: false });
        let build = Arc::new(NoBuild);
        Ok(Provided {
            catalog: Some(games.clone()),
            slots: Some(games),
            launcher: Some(Arc::new(Play)),
            installer: Some(build.clone()),
            downloader: Some(build),
            ..Provided::default()
        })
    })
}

/// An emulator plugin named by its console (`name`, such as `PS5`), whose program is `emulator`
/// (`KytyPS5`), with `build` installed, or none for "", and a catalog of `titles`. With none
/// installed, a first play asks to install build 1.0.
pub fn console_emulator(
    plugin_id: &str,
    name: &str,
    emulator: &str,
    build: &'static str,
    titles: Vec<&'static str>,
) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"emulator\"\nname = \"{name}\"\nemulator = \"{emulator}\"\n\
         version = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"catalog\", \"launcher\", \"installer\", \"downloader\"]\n\
         [permissions]\nprocess = [\"bin/*\"]\nnetwork = [\"builds.example\"]"
    ))
    .provides(move |_| {
        let games = Arc::new(Games { titles: titles.clone(), platform: "PS5", details_fail: false, quiet: false });
        let installer = Arc::new(Built(build));
        Ok(Provided {
            catalog: Some(games),
            launcher: Some(Arc::new(Play)),
            installer: Some(installer),
            downloader: Some(Arc::new(NoBuild)),
            ..Provided::default()
        })
    })
}

/// An installer with one build installed, or none for "".
struct Built(&'static str);

#[async_trait]
impl Installer for Built {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        let version = if self.0.is_empty() { "1.0" } else { self.0 };
        Ok(vec![VersionInfo { version: version.into(), notes: None }])
    }
    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        if self.0.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![InstalledVersion { version: self.0.into(), path: RelPath::new("builds/1").unwrap(), own: false }])
    }
    async fn install(&self, _: &str, _: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        Err(PluginError::Unsupported("the test never installs".into()))
    }
    async fn uninstall(&self, _: &str) -> PluginResult<()> {
        Ok(())
    }
}

struct NoBuild;

#[async_trait]
impl Installer for NoBuild {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(vec![VersionInfo { version: "1.0".into(), notes: None }])
    }
    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        Ok(vec![])
    }
    async fn install(&self, _: &str, _: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        Err(PluginError::Unsupported("the test never installs".into()))
    }
    async fn uninstall(&self, _: &str) -> PluginResult<()> {
        Ok(())
    }
}

#[async_trait]
impl Downloader for NoBuild {
    async fn plan(&self, _: &DownloadRequest) -> PluginResult<DownloadPlan> {
        Ok(DownloadPlan {
            label: "Emu 1.0".into(),
            artifacts: vec![Artifact {
                url: "https://builds.example/emu.bin".into(),
                size: Some(300 * 1024 * 1024),
                sha256: "00".repeat(32),
                signature: None,
                dest: RelPath::new("downloads/emu.bin").unwrap(),
                unpack: None,
            }],
        })
    }
}

/// A `Vm` struct property's field, from the live instance.
pub fn vm_field(host: &Composed, property: &str, field: &str) -> slint_interpreter::Value {
    match host.instance().get_global_property("Vm", property).unwrap() {
        slint_interpreter::Value::Struct(s) => s.get_field(field).cloned().unwrap_or_default(),
        other => panic!("Vm.{property} is {other:?}"),
    }
}

/// A store's catalog: the games it knows, by title id, each with a rating (from 3,227 players) and a tagline (""
/// for none), a trailer and two screenshots at art.example, the genres Adventure and Arcade, the publisher MOJANG AB and, when `art` is set,
/// an icon at `art`. It answers after `delay`.
pub fn store(
    plugin_id: &str,
    known: Vec<(&'static str, f32, &'static str)>,
    art: Option<String>,
    delay: std::time::Duration,
) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{plugin_id}\"\nkind = \"service\"\nname = \"{plugin_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"metadata\"]\n[permissions]\nnetwork = [\"art.example\"]"
    ))
    .provides(move |_| {
        let store = Arc::new(Store { known: known.clone(), art: art.clone(), delay });
        Ok(Provided { metadata: Some(store), ..Provided::default() })
    })
}

struct Store {
    known: Vec<(&'static str, f32, &'static str)>,
    art: Option<String>,
    delay: std::time::Duration,
}

#[async_trait]
impl Metadata for Store {
    async fn describe(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>> {
        FakeClock::default().sleep(self.delay).await;
        let Some((_, score, tagline)) = self.known.iter().find(|(id, ..)| *id == game.title_id) else {
            return Ok(None);
        };
        Ok(Some(GameMetadata {
            title: game.title.clone(),
            tagline: (*tagline).into(),
            trailer: format!("https://art.example/{}.mp4", game.title_id),
            screenshots: vec![
                ImageSource::Url("https://art.example/1.png".into()),
                ImageSource::Url("https://art.example/2.png".into()),
            ],
            genres: vec!["Adventure".into(), "Arcade".into()],
            publisher: "MOJANG AB".into(),
            rating: Some(Rating { score: *score, total: 3_227 }),
            icon: self.art.clone().map(ImageSource::Url),
            ..GameMetadata::default()
        }))
    }
}

/// KytyPS5 for the PS5 catalog, with no games of its own and a game installer that writes a
/// quarter of the game and then waits until the player cancels.
pub fn slow_installer() -> StubPlugin {
    struct Emu;
    #[async_trait]
    impl Catalog for Emu {
        async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
            Ok(Page { items: vec![], total: None })
        }
        async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
            Ok(vec![])
        }
        async fn details(&self, game: &str) -> PluginResult<GameDetails> {
            Err(PluginError::NotFound(game.into()))
        }
        async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
            Ok(None)
        }
    }
    #[async_trait]
    impl GameInstaller for Emu {
        async fn candidates(&self) -> PluginResult<Vec<Installable>> {
            Ok(vec![])
        }
        async fn begin(&self, _: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
            job.progress(1, 4);
            tokio::spawn(async move {
                // The job says a cancel only by a flag: it is read on the fake clock's time.
                let mut ticks = FakeClock::default().interval(std::time::Duration::from_millis(5));
                while !job.cancelled() {
                    ticks.next().await;
                }
                job.finished(Err(PluginError::Other("The install was cancelled".into())));
            });
            Ok(HostPath::new("games", 0, RelPath::new("PS5/game.part").unwrap()))
        }
    }
    StubPlugin::from_toml(
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"PS5\"\nemulator = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         console = \"PS5\"\n\
         capabilities = [\"catalog\", \"game_installer\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n",
    )
    .provides(|_| {
        Ok(Provided { catalog: Some(Arc::new(Emu)), game_installer: Some(Arc::new(Emu)), ..Provided::default() })
    })
}
