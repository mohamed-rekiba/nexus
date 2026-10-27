//! Plugins built for a test from a few choices: [`FakeCatalog`], an emulator with a catalog of
//! games, [`FakeStore`], a store that describes games, and [`FakeService`], a service that
//! fills slots.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use nexus_api::*;
use nexus_core::Clock;

use crate::{FakeClock, StubPlugin};

/// The emulator build a [`FakeCatalog`] downloads and installs.
#[derive(Clone, Copy)]
enum Build {
    /// No installer and no downloader.
    None,
    /// No build is installed, and version 1.0 is a 300 MB download.
    ToInstall,
    /// This build is installed, or none for "". A first play asks to install build 1.0.
    Installed(&'static str),
}

/// An emulator plugin with a catalog of games, built for a test. Its games are `G0`, `G1`, …
/// in the order given, on PS4 unless [`FakeCatalog::system`] or [`FakeCatalog::platform`]
/// says other. Each game's details have a description, a size and four facts. Its slots give
/// each game a `region` badge, a `verify` action and an `Emulator` section.
///
/// It turns into the plugin with [`FakeCatalog::build`].
#[derive(Clone)]
pub struct FakeCatalog {
    id: String,
    name: String,
    emulator: Option<String>,
    console: Option<String>,
    titles: Vec<String>,
    platform: String,
    system: bool,
    details_fail: bool,
    quiet: bool,
    slots: bool,
    launcher: bool,
    compatibility: bool,
    build: Build,
    packages: bool,
    waiting_installer: bool,
    keymap: Vec<(String, String)>,
    requires: Option<String>,
}

impl FakeCatalog {
    /// An emulator named `id`, with slots and an empty catalog.
    pub fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            name: id.into(),
            emulator: None,
            console: None,
            titles: Vec::new(),
            platform: "PS4".into(),
            system: false,
            details_fail: false,
            quiet: false,
            slots: true,
            launcher: false,
            compatibility: false,
            build: Build::None,
            packages: false,
            waiting_installer: false,
            keymap: Vec::new(),
            requires: None,
        }
    }

    /// KytyPS5 for the PS5 catalog, `emu-kyty`, named PS5: no games of its own and no slots. It
    /// may read the packages folder, where a catalog's downloads go.
    pub fn kyty() -> Self {
        Self::new("emu-kyty").named("PS5").emulator("KytyPS5").console("PS5").without_slots().reads_packages()
    }

    /// The games in its catalog, by title.
    pub fn games<'a>(mut self, titles: impl IntoIterator<Item = &'a str>) -> Self {
        self.titles = titles.into_iter().map(str::to_owned).collect();
        self
    }

    /// Its games are on `platform`, and the manifest describes the system as the real plugins
    /// do theirs: PS5 neutral, PS4 blue.
    pub fn system(mut self, platform: &str) -> Self {
        self.platform = platform.into();
        self.system = true;
        self
    }

    /// Its games are on `platform`, with no system in the manifest.
    pub fn platform(mut self, platform: &str) -> Self {
        self.platform = platform.into();
        self
    }

    /// The manifest's `name`: the console it is named by, such as `PS5`.
    pub fn named(mut self, name: &str) -> Self {
        self.name = name.into();
        self
    }

    /// The manifest's `emulator`: its program, such as `KytyPS5`.
    pub fn emulator(mut self, program: &str) -> Self {
        self.emulator = Some(program.into());
        self
    }

    /// The manifest's `console`: the console whose catalog offers it runs.
    pub fn console(mut self, console: &str) -> Self {
        self.console = Some(console.into());
        self
    }

    /// What its games need, as TOML for the manifest's `[requires]` section.
    pub fn requires(mut self, requires: &str) -> Self {
        self.requires = Some(requires.into());
        self
    }

    /// Its games' details have no description.
    pub fn without_description(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Its catalog's details always fail.
    pub fn without_details(mut self) -> Self {
        self.details_fail = true;
        self
    }

    /// It has no slots.
    pub fn without_slots(mut self) -> Self {
        self.slots = false;
        self
    }

    /// A launcher, so its games can run: it runs `bin/game <game id>`.
    pub fn playable(mut self) -> Self {
        self.launcher = true;
        self
    }

    /// The community's result for each game: Playable, last tested on 2026-09-01.
    pub fn tested(mut self) -> Self {
        self.compatibility = true;
        self
    }

    /// A keyboard key for a controller button, in its `[[keymap]]`.
    pub fn key(mut self, button: &str, keys: &str) -> Self {
        self.keymap.push((button.into(), keys.into()));
        self
    }

    /// An emulator to install first: no build is installed, and version 1.0 is a 300 MB
    /// download from `builds.example`. It never installs.
    pub fn needing_its_emulator(mut self) -> Self {
        self.build = Build::ToInstall;
        self
    }

    /// Like [`FakeCatalog::needing_its_emulator`], with `build` installed, or none for "".
    pub fn installed(mut self, build: &'static str) -> Self {
        self.build = Build::Installed(build);
        self
    }

    /// It may read the packages folder, where a catalog's downloads go.
    pub fn reads_packages(mut self) -> Self {
        self.packages = true;
        self
    }

    /// A game installer that writes a quarter of the game into the games folder, then waits
    /// until the player cancels.
    pub fn waiting_installer(mut self) -> Self {
        self.waiting_installer = true;
        self
    }

    /// The manifest, as TOML.
    fn manifest(&self) -> String {
        let mut capabilities = vec!["catalog"];
        if self.slots {
            capabilities.push("slots");
        }
        if self.compatibility {
            capabilities.push("compatibility");
        }
        if self.launcher {
            capabilities.push("launcher");
        }
        if !matches!(self.build, Build::None) {
            capabilities.extend(["installer", "downloader"]);
        }
        if self.waiting_installer {
            capabilities.push("game_installer");
        }
        let capabilities: Vec<String> = capabilities.iter().map(|c| format!("\"{c}\"")).collect();

        let mut toml = format!("id = \"{}\"\nkind = \"emulator\"\nname = \"{}\"\n", self.id, self.name);
        if let Some(emulator) = &self.emulator {
            toml += &format!("emulator = \"{emulator}\"\n");
        }
        toml += "version = \"1.0.0\"\nhost_api = \"^0.2\"\n";
        if let Some(console) = &self.console {
            toml += &format!("console = \"{console}\"\n");
        }
        toml += &format!("capabilities = [{}]\n", capabilities.join(", "));
        if self.system {
            toml += match self.platform.as_str() {
                "PS5" => "[system]\nid = \"PS5\"\nname = \"PlayStation 5\"\n",
                "PS4" => "[system]\nid = \"PS4\"\nname = \"PlayStation 4\"\ncolor = \"#8fc2ff\"\n",
                _ => "",
            };
        }
        toml += "[permissions]\n";
        if self.launcher {
            toml += "process = [\"bin/*\"]\n";
        }
        if !matches!(self.build, Build::None) {
            toml += "network = [\"builds.example\"]\n";
        }
        if self.packages {
            toml += "[[permissions.filesystem]]\nfolder = \"packages\"\n";
        }
        if self.waiting_installer {
            toml += "[[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n";
        }
        for (button, keys) in &self.keymap {
            toml += &format!("[[keymap]]\nbutton = \"{button}\"\nkeys = \"{keys}\"\n");
        }
        if let Some(requires) = &self.requires {
            toml += &format!("[requires]\n{requires}\n");
        }
        toml
    }

    /// The plugin.
    pub fn build(self) -> StubPlugin {
        let manifest = self.manifest();
        StubPlugin::from_toml(&manifest).provides(move |_| {
            let games = Arc::new(Games {
                titles: self.titles.clone(),
                platform: self.platform.clone(),
                details_fail: self.details_fail,
                quiet: self.quiet,
            });
            let mut provided = Provided { catalog: Some(games.clone()), ..Provided::default() };
            if self.slots {
                provided.slots = Some(games);
            }
            if self.launcher {
                provided.launcher = Some(Arc::new(Play));
            }
            if self.compatibility {
                provided.compatibility = Some(Arc::new(Tested));
            }
            match self.build {
                Build::None => {}
                Build::ToInstall => {
                    provided.installer = Some(Arc::new(NoBuild));
                    provided.downloader = Some(Arc::new(NoBuild));
                }
                Build::Installed(build) => {
                    provided.installer = Some(Arc::new(Built(build)));
                    provided.downloader = Some(Arc::new(NoBuild));
                }
            }
            if self.waiting_installer {
                provided.game_installer = Some(Arc::new(WaitingInstaller));
            }
            Ok(provided)
        })
    }
}

struct Games {
    titles: Vec<String>,
    platform: String,
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
                title: t.clone(),
                platform: self.platform.clone(),
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

struct WaitingInstaller;

#[async_trait]
impl GameInstaller for WaitingInstaller {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        Ok(vec![])
    }
    async fn begin(&self, _: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        job.progress(1, 4);
        tokio::spawn(async move {
            // The job says a cancel only by a flag: it is read on the fake clock's time.
            let mut ticks = FakeClock::default().interval(Duration::from_millis(5));
            while !job.cancelled() {
                ticks.next().await;
            }
            job.finished(Err(PluginError::Other("The install was cancelled".into())));
        });
        Ok(HostPath::new("games", 0, RelPath::new("PS5/game.part").unwrap()))
    }
}

/// A store service that describes games, built for a test.
#[derive(Clone)]
pub struct FakeStore {
    id: String,
    known: Vec<(String, f32, String)>,
    art: Option<String>,
    covers: bool,
    delay: Duration,
}

impl FakeStore {
    /// A store named `id` that knows no game.
    pub fn new(id: &str) -> Self {
        Self { id: id.into(), known: Vec::new(), art: None, covers: false, delay: Duration::ZERO }
    }

    /// It knows the game with this title id: its rating is `score` from 3,227 players, its
    /// tagline is `tagline` ("" for none), it has a trailer and two screenshots at art.example,
    /// the genres Adventure and Arcade, and the publisher MOJANG AB.
    pub fn knows(mut self, title_id: &str, score: f32, tagline: &str) -> Self {
        self.known.push((title_id.into(), score, tagline.into()));
        self
    }

    /// Each game it knows has an icon at `url`.
    pub fn icon(mut self, url: impl Into<String>) -> Self {
        self.art = Some(url.into());
        self
    }

    /// It gives every game, known or not, only a cover at `https://art.example/<title id>.png`.
    pub fn covers(mut self) -> Self {
        self.covers = true;
        self
    }

    /// It answers after `delay` of the core's clock.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// The plugin.
    pub fn build(self) -> StubPlugin {
        StubPlugin::from_toml(&format!(
            "id = \"{}\"\nkind = \"service\"\nname = \"{}\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
             capabilities = [\"metadata\"]\n[permissions]\nnetwork = [\"art.example\"]",
            self.id, self.id
        ))
        .provides(move |_| Ok(Provided { metadata: Some(Arc::new(self.clone())), ..Provided::default() }))
    }
}

#[async_trait]
impl Metadata for FakeStore {
    async fn describe(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>> {
        if self.covers {
            return Ok(Some(GameMetadata {
                icon: Some(ImageSource::Url(format!("https://art.example/{}.png", game.title_id))),
                ..GameMetadata::default()
            }));
        }
        FakeClock::default().sleep(self.delay).await;
        let Some((_, score, tagline)) = self.known.iter().find(|(id, ..)| *id == game.title_id) else {
            return Ok(None);
        };
        Ok(Some(GameMetadata {
            title: game.title.clone(),
            tagline: tagline.clone(),
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

/// A service plugin that fills slots, built for a test.
#[derive(Clone)]
pub struct FakeService {
    id: String,
    name: String,
    items: Vec<(SlotId, SlotItem)>,
}

impl FakeService {
    /// A service with the id `id` and the name `name`, with no slot items.
    pub fn new(id: &str, name: &str) -> Self {
        Self { id: id.into(), name: name.into(), items: Vec::new() }
    }

    /// A section in `slot`: its id, its title and its rows as (label, value).
    pub fn section(mut self, slot: SlotId, id: &str, title: &str, rows: &[(&str, &str)]) -> Self {
        let rows = rows.iter().map(|(label, value)| SectionRow { label: (*label).into(), value: (*value).into() });
        let content = SlotContent::Section { title: title.into(), rows: rows.collect() };
        self.items.push((slot, SlotItem { id: id.into(), order: Order::default(), content }));
        self
    }

    /// The plugin.
    pub fn build(self) -> StubPlugin {
        StubPlugin::from_toml(&format!(
            "id = \"{}\"\nkind = \"service\"\nname = \"{}\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
             capabilities = [\"slots\"]",
            self.id, self.name
        ))
        .provides(move |_| Ok(Provided { slots: Some(Arc::new(self.clone())), ..Provided::default() }))
    }
}

#[async_trait]
impl Slots for FakeService {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        Ok(self.items.iter().filter(|(slot, _)| *slot == request.slot).map(|(_, item)| item.clone()).collect())
    }
    async fn run_action(&self, _: &str, _: Option<&str>, _: &serde_json::Value) -> PluginResult<()> {
        Ok(())
    }
}
