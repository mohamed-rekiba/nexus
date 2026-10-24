//! PC games on Linux, Windows and macOS.
//!
//! - Install: from a download folder the host has unpacked. The archives that are the release's
//!   packaging come from their unpacked folders; everything else, a game's own data archives
//!   too, is copied as it is. Files are probed by their first bytes, programs get the program
//!   flag again, and a manifest file in the game's folder says what Play starts.
//! - Library: the games in `PC/` of the player's game folders that have a manifest file.
//! - Play: what this system can start, from the game's folder; a Windows game on Linux through
//!   umu-launcher (Proton) or Wine, on a Mac through Wine. A release this system can't start
//!   installs anyway, and its Game Hub says why.

pub mod probe;
mod umu;

use std::sync::Arc;

use emulator_kit::archive_sets::{UNPACKED, set_name};
use emulator_kit::installs::{self, Download, DownloadFile, HostTree, Tree};
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

use crate::probe::{Choice, Kind, Probe};

const MANIFEST: &str = include_str!("../plugin.toml");
pub const GAMES: &str = nexus_api::GAMES_FOLDER;
pub const PACKAGES: &str = nexus_api::PACKAGES_FOLDER;
/// Where installs go in the games folder.
pub const CONSOLE_FOLDER: &str = "PC";
/// The manifest file in an installed game's folder.
pub const GAME_FILE: &str = ".nexus-game.json";
/// The most a manifest file may be.
const MAX_GAME_FILE: u64 = 64 * 1024;
/// Where a GOG installer's game says its name, version and build, a line each.
const GOG_INFO: &str = "data/noarch/gameinfo";
/// Where a portable Wine wrapper (Kron4ek's) says its game's name, on its sixth line: beside
/// its start script.
const WRAPPER_INFO: &str = "game_info/game_info.txt";
const WRAPPER_NAME_LINE: usize = 5;
/// The most files of a release whose first bytes are read.
const PROBED: usize = 4000;

fn path(p: impl Into<String>) -> PluginResult<RelPath> {
    RelPath::new(p).map_err(|e| PluginError::InvalidInput(e.to_string()))
}

/// What Play starts, and the other choices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Start {
    pub launch: String,
    pub kind: Kind,
}

/// The manifest file an install leaves in the game's folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct GameManifest {
    pub version: u32,
    pub name: String,
    /// What Play starts, from the game's folder.
    pub launch: String,
    pub kind: Kind,
    #[serde(default)]
    pub alternatives: Vec<Start>,
}

/// A path inside its folder: no root, no `..`, no empty part.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

pub struct PcPlugin;

#[async_trait]
impl Plugin for PcPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl EmulatorPlugin for PcPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let platform = ctx
            .services()
            .get::<HostInfo>()
            .and_then(|h| h.platform)
            .or_else(Platform::current)
            .unwrap_or(Platform::Linux);
        let runs_32bit = ctx.services().get::<HostInfo>().is_some_and(|h| h.runs_32bit);
        let pc = Arc::new(Pc { ctx, platform, runs_32bit });
        Ok(Provided {
            catalog: Some(pc.clone()),
            launcher: Some(pc.clone()),
            configurator: Some(pc.clone()),
            game_installer: Some(pc.clone()),
            slots: Some(pc),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::emulator(|| Box::new(PcPlugin)) }

/// An installed game: its folder's name is its id.
#[derive(Clone, Debug)]
struct Game {
    id: String,
    dir: HostPath,
    manifest: GameManifest,
}

impl Game {
    fn summary(&self) -> GameSummary {
        GameSummary {
            id: self.id.clone(),
            title: self.manifest.name.clone(),
            platform: "PC".into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: None,
            content_id: None,
        }
    }
}

/// The game setting a Windows game runs at: `screen`, or `<width>x<height>`.
const RESOLUTION: &str = "resolution";

/// The resolutions a Windows game can run at, inside gamescope.
mod resolution {
    /// Each choice, with its label. `screen` is the screen's own: the game runs as it is.
    pub const CHOICES: [(&str, &str); 8] = [
        ("screen", "Your screen"),
        ("640x480", "640 × 480"),
        ("800x600", "800 × 600"),
        ("1024x768", "1024 × 768"),
        ("1280x720", "1280 × 720"),
        ("1280x1024", "1280 × 1024"),
        ("1600x900", "1600 × 900"),
        ("1920x1080", "1920 × 1080"),
    ];

    /// The width and height of a choice; `None` for `screen` or one that isn't a choice.
    pub fn size(choice: &str) -> Option<(u32, u32)> {
        if !CHOICES.iter().any(|(c, _)| *c == choice) {
            return None;
        }
        let (w, h) = choice.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }
}

/// The runner as an argument of gamescope: a system program by its name, which gamescope
/// finds on the PATH, and the plugin's own by its path.
fn runner_arg(program: Location) -> LaunchArg {
    match program {
        Location::System(name) => LaunchArg::Text(name),
        other => LaunchArg::path(other),
    }
}

/// What runs a Windows game: umu-launcher (Proton) or Wine.
struct Runner {
    program: Location,
    umu: bool,
}

struct Pc {
    ctx: PluginContext,
    platform: Platform,
    /// The host runs 32-bit Linux programs.
    runs_32bit: bool,
}

impl Pc {
    /// Every installed game, in name order.
    async fn games(&self) -> PluginResult<Vec<Game>> {
        let folders = self.ctx.folders();
        let mut out = Vec::new();
        for root in folders.roots(GAMES).await? {
            let dir = HostPath::new(GAMES, root.root, path(CONSOLE_FOLDER)?);
            for entry in folders.list(&dir).await.unwrap_or_default() {
                if !entry.is_dir {
                    continue;
                }
                let game_dir = dir.join(&entry.name)?;
                let Ok(bytes) = folders.read_range(&game_dir.join(GAME_FILE)?, 0, MAX_GAME_FILE).await else {
                    continue;
                };
                let Ok(mut manifest) = serde_json::from_slice::<GameManifest>(&bytes) else { continue };
                if manifest.version != 1 || !safe_relative(&manifest.launch) {
                    continue;
                }
                // Installed before its name came from GOG's or the wrapper's file: named after
                // the installer or the release's folder.
                let script_dir = manifest.launch.rsplit_once('/').map_or("", |(dir, _)| dir);
                let wrapper = format!("{script_dir}/{WRAPPER_INFO}");
                let named = match self.named_in(&game_dir.join(GOG_INFO)?, 0).await {
                    Some(name) => Some(name),
                    None => self.named_in(&game_dir.join(wrapper.trim_start_matches('/'))?, WRAPPER_NAME_LINE).await,
                };
                if let Some(name) = named {
                    manifest.name = name;
                }
                out.push(Game { id: entry.name, dir: game_dir, manifest });
            }
        }
        out.sort_by_cached_key(|g| g.manifest.name.to_lowercase());
        Ok(out)
    }

    /// The name the file at `file` gives its game on its line `line` (from 0): the first of a
    /// GOG installer's `gameinfo`, the sixth of a portable Wine wrapper's `game_info.txt`.
    async fn named_in(&self, file: &HostPath, line: usize) -> Option<String> {
        let bytes = self.ctx.folders().read_range(file, 0, 4096).await.ok()?;
        let named = String::from_utf8_lossy(&bytes).lines().nth(line)?.trim().to_owned();
        Some(probe::clean_name(&named)).filter(|n| !n.is_empty())
    }

    async fn game(&self, id: &str) -> PluginResult<Game> {
        self.games().await?.into_iter().find(|g| g.id == id).ok_or_else(|| PluginError::NotFound(id.into()))
    }

    /// What runs a Windows game here: on Linux the umu-launcher this plugin installed, then one
    /// on the system, then Wine. `None` when there is none.
    async fn runner(&self) -> Option<Runner> {
        if self.platform == Platform::Linux && self.umu_version().await.is_some() {
            return Some(Runner { program: Location::Data(path(umu::PROGRAM).ok()?), umu: true });
        }
        let candidates: &[&'static str] =
            if self.platform == Platform::Linux { &["umu-run", "wine", "wine64"] } else { &["wine", "wine64"] };
        for name in candidates {
            if matches!(self.ctx.programs().find(&Location::System((*name).into())).await, Ok(Some(_))) {
                return Some(Runner { program: Location::System((*name).into()), umu: *name == "umu-run" });
            }
        }
        None
    }

    /// The version of the umu-launcher this plugin installed, if it did.
    async fn umu_version(&self) -> Option<String> {
        let storage = self.ctx.storage();
        if !storage.exists(&path(umu::PROGRAM).ok()?).await.unwrap_or(false) {
            return None;
        }
        let version = storage.read(&path(umu::VERSION).ok()?).await.ok()?;
        Some(String::from_utf8_lossy(&version).trim().to_owned()).filter(|v| !v.is_empty())
    }

    /// Installs umu-launcher's latest release into the data folder, in place of the one there:
    /// its zipapp, checked against the SHA-256 GitHub lists for it.
    async fn install_umu(&self) -> PluginResult<()> {
        let get = |url: String| async move {
            let answer = self.ctx.http().send(HttpRequest::get(url.clone())).await?;
            if answer.status != 200 {
                return Err(PluginError::Network(format!("{url} answered {}", answer.status)));
            }
            Ok(answer)
        };
        let release: Value = get(umu::LATEST.into()).await?.json()?;
        let found = emulator_kit::github::builds(&release, false, umu::zipapp);
        let (release, asset) = found
            .into_iter()
            .next()
            .ok_or_else(|| PluginError::NotFound("umu-launcher's latest release has no zipapp to install".into()))?;
        let sha256 = asset
            .sha256
            .ok_or_else(|| PluginError::Precondition(format!("GitHub has no checksum for {}", asset.name)))?;
        let tar = get(asset.url).await?.body;
        let fail =
            |why: String| PluginError::Precondition(format!("umu-launcher {} wasn't installed: {why}", release.tag));
        umu::checked(&tar, &sha256).map_err(|why| fail(format!("the download isn't the one GitHub lists, {why}")))?;
        let program = umu::program(&tar).map_err(fail)?;
        // Beside the one in use, then in its place: a failed install leaves it as it was.
        let storage = self.ctx.storage();
        let part = path(format!("{}.part", umu::PROGRAM))?;
        storage.write(&part, &program).await?;
        storage.set_executable(&part).await?;
        storage.rename(&part, &path(umu::PROGRAM)?).await?;
        storage.write(&path(umu::VERSION)?, release.tag.as_bytes()).await?;
        self.ctx.log().info(&format!("installed umu-launcher {}", release.tag));
        Ok(())
    }

    /// Whether `game` runs as a Windows program here, through umu-launcher or Wine: a Windows
    /// game, or a portable Wine wrapper whose own Wine can't run.
    async fn runs_windows(&self, game: &Game) -> bool {
        let kind = game.manifest.kind;
        if kind == Kind::Windows {
            return kind.needs_runner(self.platform);
        }
        let folder = match game.manifest.launch.rsplit_once('/') {
            Some((dir, _)) => game.dir.join(dir),
            None => Ok(game.dir.clone()),
        };
        kind == Kind::Script
            && game.manifest.alternatives.iter().any(|a| a.kind == Kind::Windows)
            && match folder {
                Ok(folder) => self.wine_that_cant_run(&folder).await,
                Err(_) => false,
            }
    }

    /// A portable Wine wrapper's script whose own Wine, `wine/bin/wine` beside it, is a 32-bit
    /// program this host can't run: the script would only say "There is a problem running Wine
    /// binary!".
    async fn wine_that_cant_run(&self, script_folder: &HostPath) -> bool {
        if self.runs_32bit || self.platform != Platform::Linux {
            return false;
        }
        let Ok(wine) = script_folder.join("wine/bin/wine") else { return false };
        let bytes = self.ctx.folders().read_range(&wine, 0, probe::HEAD).await.unwrap_or_default();
        matches!(probe::probe(&bytes), Probe::Elf { x64: false })
    }

    /// Runs the Windows program `launch` (in the game's folder) through the runner, in the
    /// game's own Wine prefix.
    /// With a resolution of its own in its settings (`config`), it runs inside gamescope at that
    /// resolution, which fills the screen with it.
    async fn windows_spec(&self, game: &Game, launch: &str, config: &Value, spec: &mut LaunchSpec) -> PluginResult<()> {
        let runner = self.runner().await.ok_or_else(|| PluginError::Precondition(self.runner_hint().into()))?;
        let exe = LaunchArg::path(game.dir.join(launch)?);
        let resolution = config.get(RESOLUTION).and_then(Value::as_str).and_then(resolution::size);
        match resolution {
            Some((width, height)) => {
                let gamescope = Location::System("gamescope".into());
                if !matches!(self.ctx.programs().find(&gamescope).await, Ok(Some(_))) {
                    return Err(PluginError::Precondition(format!(
                        "To run at {width}×{height} and fill the screen, install gamescope, or set its Resolution \
                         back to Your screen in its settings"
                    )));
                }
                spec.program = gamescope;
                spec.args = ["-f", "-w", &width.to_string(), "-h", &height.to_string(), "--"]
                    .into_iter()
                    .map(LaunchArg::from)
                    .chain([runner_arg(runner.program), exe])
                    .collect();
            }
            None => {
                spec.program = runner.program;
                spec.args = vec![exe];
            }
        }
        spec.cwd = Some(Location::Folder(match launch.rsplit_once('/') {
            Some((dir, _)) => game.dir.join(dir)?,
            None => game.dir.clone(),
        }));
        // Each game in its own Wine prefix, in this plugin's data folder.
        spec.env.insert("WINEPREFIX".into(), LaunchArg::path(path(format!("prefixes/{}", game.id))?));
        if runner.umu {
            // umu downloads and updates Proton (GE-Proton) by itself.
            spec.env.insert("GAMEID".into(), "umu-default".into());
            spec.env.insert("PROTONPATH".into(), "GE-Proton".into());
            // With no 32-bit libraries, Proton's runtime can't give a 32-bit game graphics
            // ("radeonsi: driver missing"): Wine's WoW64 runs it on the 64-bit ones.
            if !self.runs_32bit {
                spec.env.insert("PROTON_USE_WOW64".into(), "1".into());
            }
        }
        Ok(())
    }

    /// The plugin page's "Windows games" section: what runs them, and on Linux the button that
    /// installs or updates umu-launcher.
    async fn windows_panel(&self) -> Vec<SlotItem> {
        let runs = match (self.runner().await, self.umu_version().await) {
            (Some(Runner { umu: true, .. }), Some(version)) => format!("Proton, through umu-launcher {version}"),
            (Some(Runner { umu: true, .. }), None) => "Proton, through the system's umu-launcher".into(),
            (Some(_), _) => "Wine".into(),
            (None, _) if self.platform == Platform::Linux => {
                "Nothing yet: install umu-launcher (Proton) or Wine".into()
            }
            (None, _) => "Nothing yet: install Wine".into(),
        };
        let mut items = vec![SlotItem {
            id: "windows".into(),
            order: Order::default(),
            content: SlotContent::Section {
                title: "Windows games".into(),
                rows: vec![SectionRow { label: "Run with".into(), value: runs }],
            },
        }];
        if self.platform == Platform::Linux {
            let label = if self.umu_version().await.is_some() { "Update umu-launcher" } else { "Install umu-launcher" };
            items.push(SlotItem {
                id: "umu".into(),
                order: Order::default().after("windows"),
                content: SlotContent::Action {
                    label: label.into(),
                    action: "install-umu".into(),
                    args: Value::Null,
                    style: ActionStyle::Secondary,
                    icon: None,
                },
            });
        }
        items
    }

    fn runner_hint(&self) -> &'static str {
        if self.platform == Platform::Linux {
            "This is a Windows game: install umu-launcher (Emulators, then PC: it runs the game with Proton) or Wine to play it"
        } else {
            "This is a Windows game: install Wine to play it"
        }
    }
}

#[async_trait]
impl Catalog for Pc {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games().await?;
        let total = all.len() as u32;
        let items = all.iter().skip(page.offset as usize).take(page.limit as usize).map(Game::summary).collect();
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let text = query.text.to_lowercase();
        Ok(self
            .games()
            .await?
            .iter()
            .filter(|g| g.manifest.name.to_lowercase().contains(&text))
            .take(query.limit as usize)
            .map(Game::summary)
            .collect())
    }

    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let game = self.game(game).await?;
        let size = self.ctx.folders().size(&game.dir).await.unwrap_or(0);
        Ok(GameDetails {
            summary: game.summary(),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: (size > 0).then_some(size),
            background: None,
            facts: vec![GameFact::new("starts", "Starts", &game.manifest.launch)],
            folder: Some(game.dir),
            also: Vec::new(),
        })
    }

    async fn artwork(&self, _game: &str, _kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Launcher for Pc {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let game = self.game(game).await?;
        let kind = game.manifest.kind;
        if !kind.runs_on(self.platform) {
            return Err(PluginError::Precondition(kind.not_here(self.platform)));
        }
        let target = game.dir.join(&game.manifest.launch)?;
        let folder = match game.manifest.launch.rsplit_once('/') {
            Some((dir, _)) => game.dir.join(dir)?,
            None => game.dir.clone(),
        };
        let mut spec = LaunchSpec {
            program: Location::Folder(target.clone()),
            args: Vec::new(),
            env: Default::default(),
            cwd: Some(Location::Folder(folder.clone())),
        };
        match kind {
            // A portable Wine wrapper whose Wine can't run here: the game's Windows program runs
            // through umu-launcher or Wine instead.
            Kind::Script if self.wine_that_cant_run(&folder).await => {
                let exe = game.manifest.alternatives.iter().find(|a| a.kind == Kind::Windows).map(|a| a.launch.clone());
                let hint = || {
                    PluginError::Precondition(format!(
                        "This game's own Wine is a 32-bit program, which this computer can't run. {}",
                        self.runner_hint()
                    ))
                };
                let exe = exe.ok_or_else(hint)?;
                if self.runner().await.is_none() {
                    return Err(hint());
                }
                self.windows_spec(&game, &exe, config, &mut spec).await?;
            }
            // A script without `#!` (some repacks' `start`) runs with sh.
            Kind::Script => {
                let head = self.ctx.folders().read_range(&target, 0, 2).await.unwrap_or_default();
                if head != b"#!" {
                    spec.program = Location::System("sh".into());
                    spec.args = vec![LaunchArg::path(target)];
                }
            }
            // AppImages unpack and run without FUSE, which minimal systems lack.
            Kind::AppImage => {
                spec.env.insert("APPIMAGE_EXTRACT_AND_RUN".into(), "1".into());
            }
            Kind::Windows if kind.needs_runner(self.platform) => {
                let launch = game.manifest.launch.clone();
                self.windows_spec(&game, &launch, config, &mut spec).await?;
            }
            Kind::Native | Kind::MacApp | Kind::Windows => {}
        }
        Ok(spec)
    }
}

#[async_trait]
impl Slots for Pc {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        if request.slot == SlotId::EmulatorPanel {
            return Ok(self.windows_panel().await);
        }
        let Some(id) = request.game.as_deref() else { return Ok(Vec::new()) };
        let game = self.game(id).await?;
        let kind = game.manifest.kind;
        Ok(match request.slot {
            // "Start with <file>": the next choice, by its file name.
            SlotId::GameAction if !game.manifest.alternatives.is_empty() => vec![item(
                "start-something-else",
                SlotContent::Action {
                    label: format!(
                        "Start with {}",
                        probe::short_path(&game.manifest.alternatives[0].launch, &game.manifest.launch)
                    ),
                    action: "start-something-else".into(),
                    args: Value::Null,
                    style: ActionStyle::Secondary,
                    icon: None,
                },
            )],
            SlotId::GameDetailSection => {
                let runs = if !kind.runs_on(self.platform) {
                    kind.not_here(self.platform)
                } else if kind.needs_runner(self.platform) {
                    match self.runner().await {
                        Some(Runner { umu: true, .. }) => "With Proton, through umu-launcher".into(),
                        Some(_) => "With Wine".into(),
                        None => self.runner_hint().into(),
                    }
                } else {
                    "As it is".into()
                };
                vec![item(
                    "pc",
                    SlotContent::Section {
                        title: "PC".into(),
                        rows: vec![
                            SectionRow { label: "Starts".into(), value: game.manifest.launch.clone() },
                            SectionRow { label: "Runs".into(), value: runs },
                        ],
                    },
                )]
            }
            _ => Vec::new(),
        })
    }

    /// "Start with <file>": the first other choice becomes what Play starts, and the one it
    /// replaces goes last.
    async fn run_action(&self, action: &str, game: Option<&str>, _args: &Value) -> PluginResult<()> {
        if action == "install-umu" {
            return self.install_umu().await;
        }
        if action != "start-something-else" {
            return Err(PluginError::Unsupported(format!("no action {action}")));
        }
        let game = self.game(game.ok_or_else(|| PluginError::InvalidInput("which game?".into()))?).await?;
        let mut manifest = game.manifest;
        if manifest.alternatives.is_empty() {
            return Err(PluginError::Precondition("There's nothing else to start".into()));
        }
        let next = manifest.alternatives.remove(0);
        manifest.alternatives.push(Start { launch: manifest.launch, kind: manifest.kind });
        manifest.launch = next.launch;
        manifest.kind = next.kind;
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        self.ctx.folders().write(&game.dir.join(GAME_FILE)?, &bytes).await
    }
}

/// What a set's folder holds beside the set, among the release's own files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Beside {
    Nothing,
    Script,
    Program,
}

/// The files of a release to install: each from where it is, to its path in the game.
struct Plan {
    name: String,
    files: Vec<DownloadFile>,
}

/// The folder all of `files` are in, one level down (a torrent's own folder), or `""`.
fn common_top(files: &[&DownloadFile]) -> String {
    let mut top: Option<&str> = None;
    for f in files {
        let Some((dir, _)) = f.rel.split_once('/') else { return String::new() };
        match top {
            None => top = Some(dir),
            Some(t) if t == dir => {}
            Some(_) => return String::new(),
        }
    }
    top.map(|t| format!("{t}/")).unwrap_or_default()
}

impl Pc {
    async fn head(&self, file: &DownloadFile) -> Probe {
        let bytes = self.ctx.folders().read_range(&file.path, 0, probe::HEAD).await.unwrap_or_default();
        probe::probe(&bytes)
    }

    /// Which files of the download install, and where: the release's own files, with each
    /// archive set that is its packaging replaced by what the host unpacked from it.
    async fn plan(&self, download: &Download) -> PluginResult<Plan> {
        let unpacked_prefix = format!("{UNPACKED}/");
        let own: Vec<&DownloadFile> = download.files.iter().filter(|f| !f.rel.starts_with(&unpacked_prefix)).collect();
        // The sets the host unpacked, by name, with the parts that make them.
        let mut sets: std::collections::BTreeMap<String, Vec<&DownloadFile>> = Default::default();
        for f in &own {
            let gog = f.rel.to_ascii_lowercase().ends_with(".sh") && f.rel.matches('/').count() <= 1;
            let name = set_name(&f.rel).or_else(|| gog.then(|| f.rel[..f.rel.len() - 3].to_owned()));
            // A set's parts can be spelled in another case: the host names its folder as the
            // first part spells it.
            let folder = name.and_then(|name| {
                let wanted = format!("{unpacked_prefix}{name}/").to_ascii_lowercase();
                download
                    .files
                    .iter()
                    .find(|u| u.rel.to_ascii_lowercase().starts_with(&wanted))
                    .map(|u| u.rel[unpacked_prefix.len()..wanted.len() - 1].to_owned())
            });
            if let Some(folder) = folder {
                sets.entry(folder).or_default().push(f);
            }
        }
        let in_set = |f: &DownloadFile| sets.values().any(|parts| parts.iter().any(|p| p.rel == f.rel));
        // What each folder that holds a set has beside its sets.
        let mut beside: std::collections::BTreeMap<String, Beside> = Default::default();
        for f in own.iter().filter(|f| !in_set(f) && f.rel.matches('/').count() <= 1) {
            let folder = f.rel.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
            if !sets.keys().any(|name| name.rsplit_once('/').map(|(d, _)| d).unwrap_or("") == folder) {
                continue;
            }
            let what = match self.head(f).await {
                Probe::Elf { .. } | Probe::Exe | Probe::AppImage | Probe::MachO => Beside::Program,
                Probe::Script => Beside::Script,
                Probe::Other => continue,
            };
            let best = beside.entry(folder).or_insert(what);
            *best = (*best).max(what);
        }
        let mut packaging = Vec::new();
        for (name, parts) in &sets {
            let folder = name.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
            let gog = parts.iter().any(|p| p.rel.to_ascii_lowercase().ends_with(".sh"));
            let unpack = match beside.get(&folder).copied().unwrap_or(Beside::Nothing) {
                Beside::Nothing => true,
                // The script starts what the archive holds.
                Beside::Script => {
                    let inside = format!("{unpacked_prefix}{name}/");
                    let mut holds_program = false;
                    for f in download.files.iter().filter(|f| f.rel.starts_with(&inside)).take(PROBED) {
                        if matches!(self.head(f).await, Probe::Elf { .. } | Probe::Exe | Probe::AppImage | Probe::MachO)
                        {
                            holds_program = true;
                            break;
                        }
                    }
                    holds_program
                }
                // A game's own data beside its program: it reads the archive as it is.
                Beside::Program => false,
            };
            if gog || unpack {
                packaging.push(name.clone());
            }
        }
        let packed = |f: &DownloadFile| packaging.iter().any(|name| sets[name].iter().any(|p| p.rel == f.rel));
        let kept: Vec<&DownloadFile> = own.iter().copied().filter(|f| !packed(f)).collect();
        // The torrent's own folder is the game's folder.
        let top = common_top(&own);
        let mut files: Vec<DownloadFile> = kept
            .iter()
            .map(|f| DownloadFile { path: f.path.clone(), rel: f.rel[top.len()..].to_owned(), size: f.size })
            .collect();
        for name in &packaging {
            let inside = format!("{unpacked_prefix}{name}/");
            // An archive's files go where it was, in the game's folder.
            let at = name.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
            let at = at.strip_prefix(&top).unwrap_or(&at).to_owned();
            for f in download.files.iter().filter(|f| f.rel.starts_with(&inside)) {
                files.push(DownloadFile {
                    path: f.path.clone(),
                    rel: format!("{at}{}", &f.rel[inside.len()..]),
                    size: f.size,
                });
            }
        }
        // A GOG installer names its game in its `gameinfo` file.
        let mut gog_name = None;
        for name in packaging.iter().filter(|n| sets[*n].iter().any(|p| p.rel.to_ascii_lowercase().ends_with(".sh"))) {
            let info = format!("{unpacked_prefix}{name}/{GOG_INFO}");
            if let Some(f) = download.files.iter().find(|f| f.rel == info) {
                gog_name = self.named_in(&f.path, 0).await;
                break;
            }
        }
        let title = if let Some(name) = gog_name {
            name
        } else if !top.is_empty() {
            top.trim_end_matches('/').to_owned()
        } else if let Some(name) = packaging.first().or(sets.keys().next()) {
            name.rsplit('/').next().unwrap_or(name).to_owned()
        } else {
            "Game".into()
        };
        Ok(Plan { name: probe::clean_name(&title), files })
    }
}

#[async_trait]
impl GameInstaller for Pc {
    /// PC games install from their downloads only.
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.ctx.folders().clone();
        let download = Download::scan(folders.as_ref(), source).await?.ok_or_else(|| {
            PluginError::InvalidInput(format!("{} isn't a download folder: a PC game installs from one", source.path))
        })?;
        let plan = self.plan(&download).await?;
        let mut probed = Vec::new();
        for f in plan.files.iter().take(PROBED) {
            probed.push((f.rel.clone(), self.head(f).await));
        }
        let Choice { launch, kind, alternatives } = probe::choose(&probed, &plan.name, self.platform)
            .ok_or_else(|| PluginError::InvalidInput("Nothing in this download starts a game".into()))?;
        // A portable Wine wrapper names its game beside its start script.
        let script_dir = launch.rsplit_once('/').map_or(String::new(), |(dir, _)| format!("{dir}/"));
        let wrapper = format!("{script_dir}{WRAPPER_INFO}");
        let named = match plan.files.iter().find(|f| f.rel == wrapper) {
            Some(f) => self.named_in(&f.path, WRAPPER_NAME_LINE).await,
            None => None,
        };
        let name = named.unwrap_or_else(|| plan.name.clone());
        let into = installs::games_folder(folders.as_ref(), job.as_ref(), CONSOLE_FOLDER).await?;
        let dest = into.join(&probe::folder_name(&name))?;
        if folders.exists(&dest).await? {
            return Err(PluginError::Precondition(format!("{name} is already installed")));
        }
        let work = HostPath::new(GAMES, dest.root, path(format!("{}.part", dest.path))?);
        folders.remove(&work).await?;
        let manifest = GameManifest {
            version: 1,
            name,
            launch,
            kind,
            alternatives: alternatives.into_iter().map(|(launch, kind)| Start { launch, kind }).collect(),
        };
        let runnable: Vec<String> = probed.iter().filter(|(_, p)| p.runnable()).map(|(rel, _)| rel.clone()).collect();
        let flags = self.platform != Platform::Windows;
        let mut tree = HostTree::of(folders.clone(), plan.files);
        let into = work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            use emulator_kit::fail::Error;
            let entries = tree.entries(&installs::cancel_check(job))?;
            installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
            let io = |call: std::pin::Pin<Box<dyn std::future::Future<Output = PluginResult<()>> + Send + '_>>| {
                writer.runtime.block_on(call).map_err(|e| Error::msg(e.to_string()))
            };
            // Downloads drop the program flag.
            if flags {
                for rel in &runnable {
                    let file = into.join(rel).map_err(|e| Error::msg(e.to_string()))?;
                    io(Box::pin(writer.folders.set_executable(&file)))?;
                }
            }
            let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| Error::msg(e.to_string()))?;
            let file = into.join(GAME_FILE).map_err(|e| Error::msg(e.to_string()))?;
            io(Box::pin(writer.folders.write(&file, &bytes)))?;
            writer.rename(&into, &dest)?;
            Ok((dest, manifest.name))
        });
        Ok(work)
    }
}

#[async_trait]
impl Configurator for Pc {
    /// A Windows game on Linux has a Resolution: one of its own runs inside gamescope.
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let ConfigScope::Game(id) = scope else { return Ok(serde_json::json!({ "type": "object", "properties": {} })) };
        if self.platform != Platform::Linux || !self.runs_windows(&self.game(id).await?).await {
            return Ok(serde_json::json!({ "type": "object", "properties": {} }));
        }
        let choices: Vec<Value> = resolution::CHOICES
            .iter()
            .map(|(value, label)| serde_json::json!({ "const": value, "description": label }))
            .collect();
        Ok(serde_json::json!({ "type": "object", "properties": { RESOLUTION: {
            "title": "Resolution",
            "description": "An old game made for a small screen fills yours at its own resolution: it runs inside gamescope, which scales it up.",
            "type": "string",
            "oneOf": choices,
            "default": "screen",
        } } }))
    }

    async fn defaults(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        Ok(serde_json::json!({}))
    }

    async fn validate(&self, _scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        if !value.is_object() {
            return Err(PluginError::InvalidInput("settings are a JSON object".into()));
        }
        match value.get(RESOLUTION).and_then(Value::as_str) {
            Some(choice) if !resolution::CHOICES.iter().any(|(c, _)| *c == choice) => {
                Err(PluginError::InvalidInput(format!("{choice} isn't a resolution to choose")))
            }
            _ => Ok(()),
        }
    }
}
