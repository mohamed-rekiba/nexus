//! shadPS4, the PS4 emulator, as a plugin.
//!
//! - Builds: shadPS4's official release. On Linux, a zip that holds an AppImage: the host
//!   checks the zip's SHA-256, takes the AppImage out and runs its `--appimage-extract`, so
//!   `builds/<tag>/squashfs-root/AppRun` runs without FUSE. On Windows, a zip with
//!   `shadPS4.exe`. On macOS, a zip with `shadps4` next to its Vulkan libraries. A new build
//!   runs once with `--help` before it becomes current; one that doesn't print shadPS4's
//!   name is removed.
//! - Games: PS4 game folders (`sce_sys/param.sfo`, category `gd`) in the player's
//!   game folders. Updates and add-ons have a param.sfo too, and are skipped.
//! - shadPS4 keeps saves and settings in its own folder (`~/.local/share/shadPS4`, on macOS
//!   `~/Library/Application Support/shadPS4`), which
//!   every build shares, so a build change moves nothing.
//! - Compatibility: the community list from `shadps4-compatibility`.

use std::sync::Arc;
use std::time::Duration;

use emulator_kit::games::{self, Console, GameIndex};
use emulator_kit::installs::{self, Entry, Format, Tree, Writer};
use emulator_kit::{cache, compat, github, versions};

pub mod pkg4;
mod pkg4_data;
use nexus_api::schemars::{self, JsonSchema};
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");
// Nightly pre-releases come out daily; 30 still reaches the last official release.
pub const RELEASES: &str = "https://api.github.com/repos/shadps4-emu/shadPS4/releases?per_page=30";
/// The community list's page, for the player.
pub const LIST_PAGE: &str = "https://github.com/shadps4-compatibility/shadps4-game-compatibility/issues";
pub const COMPATIBILITY: &str = "https://github.com/shadps4-compatibility/shadps4-game-compatibility/releases/latest/download/compatibility_data.json";
pub const GAMES: &str = nexus_api::GAMES_FOLDER;
/// Where game installs go inside the first game folder.
const CONSOLE_FOLDER: &str = "PS4";
/// Where you keep packages to install.
pub const PACKAGES: &str = nexus_api::PACKAGES_FOLDER;
/// shadPS4's data folder, where DLC goes (`addcont/<title id>/<label>`).
pub const DATA: &str = "shadps4-data";
const LISTS_TTL: Duration = Duration::from_secs(6 * 3600);

/// shadPS4 settings. Its other settings live in its own config file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars", extend("x-order" = ["own_build", "fullscreen"]))]
pub struct Settings {
    #[serde(default = "yes")]
    #[schemars(title = "Full screen")]
    pub fullscreen: bool,
    /// Runs the shadPS4 you built or installed: from the folder you map as shadps4-build, or
    /// from your PATH. Installing an official build turns this off. Saves are shared: shadPS4
    /// keeps them in its own folder.
    #[serde(default)]
    #[schemars(title = "Use your own build")]
    pub own_build: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { fullscreen: true, own_build: false }
    }
}

/// Per-game settings: anything set here wins over the shadPS4 settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Full screen")]
    pub fullscreen: Option<bool>,
}

/// The official build for this system: a zip with an AppImage on Linux, a zip with
/// `shadPS4.exe` on Windows, a zip with `shadps4` and its Vulkan libraries on macOS.
pub fn asset_matches(name: &str, platform: Option<Platform>) -> bool {
    let prefix = match platform {
        Some(Platform::Windows) => "shadps4-win64",
        Some(Platform::Macos) => "shadps4-macos",
        _ => "shadps4-linux",
    };
    name.starts_with(prefix) && name.ends_with(".zip")
}

/// "v.0.18.0" -> "0.18.0".
pub fn pretty(tag: &str) -> String {
    tag.trim_start_matches('v').trim_start_matches('.').to_owned()
}

fn path(p: impl Into<String>) -> PluginResult<RelPath> {
    Ok(RelPath::new(p)?)
}

/// The program a build starts.
fn program(tag: &str, platform: Option<Platform>) -> PluginResult<RelPath> {
    match platform {
        Some(Platform::Windows) => Ok(versions::folder(tag)?.join("shadPS4.exe")?),
        Some(Platform::Macos) => Ok(versions::folder(tag)?.join("shadps4")?),
        _ => Ok(versions::folder(tag)?.join("squashfs-root")?.join("AppRun")?),
    }
}

// shadPS4's default key layout, as the old app's keyboard screen listed it. shadPS4 keeps the
// layout in its own settings file, so the player's may differ.
pub const KEYS: &[(&str, &str)] = &[
    ("Cross", "N (or keypad 2)"),
    ("Circle", "B (or keypad 6)"),
    ("Square", "V (or keypad 4)"),
    ("Triangle", "C (or keypad 8)"),
    ("D-pad", "Arrow keys"),
    ("Options", "Enter"),
    ("Left stick", "W A S D"),
    ("Right stick", "I J K L"),
    ("L1 / R1", "Q / U"),
    ("L2 / R2", "E / O"),
    ("L3 / R3", "X / M"),
    ("Touchpad", "Space"),
];

/// The folder you map to your own shadPS4 build.
pub const OWN_BUILD: &str = "shadps4-build";
/// What your own build's program may be called, in its folder.
const OWN_PROGRAMS: [&str; 5] =
    ["Shadps4-sdl.AppImage", "Shadps4-qt.AppImage", "shadps4", "build/shadps4", "shadPS4.exe"];
/// What the Emulators screen calls your own build.
const OWN_LABEL: &str = "Your own build";

pub struct ShadPlugin;

#[async_trait]
impl Plugin for ShadPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl EmulatorPlugin for ShadPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let platform = ctx.services().get::<HostInfo>().and_then(|h| h.platform);
        let shad = Arc::new(Shad { ctx, platform, games: GameIndex::new(GAMES, Console::Ps4) });
        Ok(Provided {
            catalog: Some(shad.clone()),
            downloader: Some(shad.clone()),
            installer: Some(shad.clone()),
            configurator: Some(shad.clone()),
            launcher: Some(shad.clone()),
            compatibility: Some(shad.clone()),
            game_installer: Some(shad.clone()),
            slots: Some(shad),
            controllers: Some(Arc::new(emulator_kit::controls::Controls { keys: KEYS })),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::emulator(|| Box::new(ShadPlugin)) }

struct Shad {
    ctx: PluginContext,
    platform: Option<Platform>,
    games: GameIndex,
}

impl Shad {
    async fn settings(&self) -> Settings {
        let stored = self.ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        serde_json::from_value(stored).unwrap_or_default()
    }

    async fn set_own_build(&self, on: bool) -> PluginResult<()> {
        let mut stored = self.ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        if !stored.is_object() {
            stored = serde_json::json!({});
        }
        stored["own_build"] = Value::Bool(on);
        self.ctx.config().set(&ConfigScope::Emulator, stored).await
    }

    /// Your own build's program: the first one in the shadps4-build folder, or else one on PATH.
    async fn own_program(&self) -> Option<Location> {
        let programs = self.ctx.programs();
        for root in self.ctx.folders().roots(OWN_BUILD).await.unwrap_or_default() {
            for name in OWN_PROGRAMS {
                let Ok(file) = RelPath::new(name) else { continue };
                let location = Location::Folder(HostPath::new(OWN_BUILD, root.root, file));
                if let Ok(Some(_)) = programs.find(&location).await {
                    return Some(location);
                }
            }
        }
        let system = Location::System("shadps4".into());
        matches!(programs.find(&system).await, Ok(Some(_))).then_some(system)
    }

    /// A test run of a new build before games start on it: `--help` must print shadPS4's name.
    async fn runs(&self, program: &RelPath) -> bool {
        let location = Location::Data(program.clone());
        match self.ctx.programs().output(&location, &["--help".into()], MAX_PROGRAM_TIME).await {
            Ok(out) => String::from_utf8_lossy(&out).contains("shadPS4"),
            Err(e) => {
                self.ctx.log().warn(&format!("test run of {program}: {e}"));
                false
            }
        }
    }

    async fn builds(&self) -> PluginResult<Vec<(github::Release, github::Asset)>> {
        let json = cache::cached_json(&self.ctx, RELEASES, &path("cache/releases.json")?, LISTS_TTL).await?;
        let platform = self.platform;
        Ok(github::builds(&json, false, |name| asset_matches(name, platform)))
    }

    async fn compat_list(&self) -> std::collections::HashMap<String, CompatInfo> {
        let Ok(cache) = path("cache/compatibility.json") else { return Default::default() };
        match cache::cached_json(&self.ctx, COMPATIBILITY, &cache, LISTS_TTL).await {
            Ok(json) => compat::shad(&json, self.platform),
            Err(e) => {
                self.ctx.log().warn(&format!("compatibility list: {e}"));
                Default::default()
            }
        }
    }
}

#[async_trait]
impl Catalog for Shad {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games.refresh(self.ctx.folders().as_ref()).await?;
        let total = all.len() as u32;
        let items = all.iter().skip(page.offset as usize).take(page.limit as usize).map(|g| g.summary("PS4")).collect();
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let all = self.games.refresh(self.ctx.folders().as_ref()).await?;
        Ok(games::search(all, query).iter().map(|g| g.summary("PS4")).collect())
    }

    /// With its update (`<folder>-patch` beside it) and its add-ons (`addcont/<title id>`),
    /// which go when it is uninstalled.
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        let folders = self.ctx.folders();
        let mut details = self.games.details(folders.as_ref(), game, "PS4").await?;
        let mut also = Vec::new();
        if let Some(dir) = &details.folder {
            also.push(HostPath::new(GAMES, dir.root, path(format!("{}-patch", dir.path))?));
        }
        if let (Some(title_id), Some(data)) = (&details.summary.title_id, self.root_of(DATA).await?) {
            also.push(HostPath::new(DATA, data, path(format!("addcont/{title_id}"))?));
        }
        for folder in also {
            if folders.exists(&folder).await.unwrap_or(false) {
                details.also.push(folder);
            }
        }
        Ok(details)
    }

    async fn artwork(&self, game: &str, kind: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
        Ok(match kind {
            ArtworkKind::Background => found.background,
            _ => found.icon,
        }
        .map(ImageSource::Folder))
    }
}

#[async_trait]
impl Downloader for Shad {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan> {
        let DownloadRequest::EmulatorBuild { version } = request else {
            return Err(PluginError::Unsupported("shadPS4 downloads its own builds only".into()));
        };
        let builds = self.builds().await?;
        let (release, asset) = match version {
            Some(tag) => builds.into_iter().find(|(r, _)| &r.tag == tag),
            None => builds.into_iter().next(),
        }
        .ok_or_else(|| PluginError::NotFound("no shadPS4 build for this system".into()))?;
        let sha256 = asset
            .sha256
            .clone()
            .ok_or_else(|| PluginError::Precondition(format!("GitHub has no checksum for {}", asset.name)))?;
        Ok(DownloadPlan {
            label: format!("shadPS4 {}", pretty(&release.tag)),
            artifacts: vec![Artifact {
                url: asset.url.clone(),
                size: (asset.size > 0).then_some(asset.size),
                sha256,
                signature: None,
                dest: path(format!("downloads/{}", asset.name))?,
                unpack: Some(Unpack {
                    format: if self.platform == Some(Platform::Linux) || self.platform.is_none() {
                        ArchiveFormat::AppImageInZip
                    } else {
                        ArchiveFormat::Zip
                    },
                    into: versions::folder(&release.tag)?,
                }),
            }],
        })
    }
}

#[async_trait]
impl Installer for Shad {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(self
            .builds()
            .await?
            .into_iter()
            .map(|(r, _)| VersionInfo { notes: Some(pretty(&r.tag)), version: r.tag })
            .collect())
    }

    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        if self.settings().await.own_build && self.own_program().await.is_some() {
            return Ok(vec![InstalledVersion { version: OWN_LABEL.into(), path: path("own")?, own: true }]);
        }
        let mut out = Vec::new();
        for tag in versions::installed(&self.ctx).await? {
            out.push(InstalledVersion { path: versions::folder(&tag)?, version: tag, own: false });
        }
        Ok(out)
    }

    async fn install(&self, version: &str, _files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let program = program(version, self.platform)?;
        if !self.ctx.storage().exists(&program).await? {
            return Err(PluginError::Other(format!("shadPS4 {version} has no {program}")));
        }
        self.ctx.storage().set_executable(&program).await?;
        if !self.runs(&program).await {
            versions::discard(&self.ctx, version).await?;
            return Err(PluginError::Precondition(format!("shadPS4 {} does not run on this system", pretty(version))));
        }
        if self.settings().await.own_build {
            self.set_own_build(false).await?;
        }
        versions::activate(&self.ctx, version).await?;
        Ok(InstalledVersion { version: version.into(), path: versions::folder(version)?, own: false })
    }

    /// Installs `to` and keeps `from` as the build to go back to; older builds go.
    async fn update(&self, _from: &str, to: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let installed = self.install(to, files).await?;
        versions::prune(&self.ctx).await?;
        Ok(installed)
    }

    async fn uninstall(&self, version: &str) -> PluginResult<()> {
        if version == OWN_LABEL {
            // Your own build isn't removed; the launcher stops using it.
            return self.set_own_build(false).await;
        }
        versions::remove(&self.ctx, version).await
    }
}

/// A PS4 package's tree.
struct Ps4Package<R: emulator_kit::read_at::ReadAt> {
    package: pkg4::Package<R>,
    nodes: Vec<pkg4::Node>,
}

impl<R: emulator_kit::read_at::ReadAt> Tree for Ps4Package<R> {
    fn entries(
        &mut self,
        cancel: &dyn Fn() -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<Vec<Entry>> {
        self.nodes = self.package.list(cancel)?;
        Ok(self.nodes.iter().map(|n| Entry { path: n.path.clone(), directory: n.directory, size: n.size }).collect())
    }

    fn copy(
        &mut self,
        index: usize,
        sink: &mut dyn FnMut(&[u8]) -> emulator_kit::fail::Result<()>,
    ) -> emulator_kit::fail::Result<()> {
        use emulator_kit::fail::Context;
        let node = self.nodes.get(index).context("No such file in the package")?.clone();
        self.package.copy(&node, sink)
    }
}

impl Shad {
    async fn root_of(&self, folder: &str) -> PluginResult<Option<u32>> {
        Ok(self.ctx.folders().roots(folder).await?.first().map(|r| r.root))
    }
}

#[async_trait]
impl GameInstaller for Shad {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        let folders = self.ctx.folders();
        let mut out = Vec::new();
        for root in folders.roots(PACKAGES).await? {
            // This console's packages are in its folder, as installs go to the games folder.
            let dir = HostPath::new(PACKAGES, root.root, path(CONSOLE_FOLDER)?);
            let Ok(entries) = folders.list(&dir).await else { continue };
            for entry in entries {
                if entry.is_dir {
                    continue;
                }
                let file = dir.join(&entry.name)?;
                let head = folders.read_range(&file, 0, 512).await.unwrap_or_default();
                if installs::format(&head) == Some(Format::Ps4Package) {
                    out.push(Installable { source: file, label: entry.name });
                }
            }
        }
        Ok(out)
    }

    /// Opens each package to learn what it is (a game, an update or DLC) and where it goes,
    /// then copies them in the background as one job: the game, then its update, then its
    /// DLC. An update needs its game installed first, or in the same download. A download can
    /// also hold a game folder that is already unpacked: it is copied as it is.
    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.ctx.folders().clone();
        let files = match installs::Download::scan(folders.as_ref(), source).await? {
            None => {
                let head = folders.read_range(source, 0, 512).await?;
                if installs::format(&head) != Some(Format::Ps4Package) {
                    return Err(PluginError::InvalidInput(format!("{} isn't a PS4 package", source.path)));
                }
                vec![(source.clone(), installs::file_size(folders.as_ref(), source).await?)]
            }
            Some(download) => {
                if let Some(work) = self.begin_folder(&download, job.clone()).await? {
                    return Ok(work);
                }
                let found = download.packages(folders.as_ref(), |f| f == Format::Ps4Package).await?;
                if found.is_empty() {
                    return Err(download.nothing_to_install());
                }
                found.into_iter().map(|(f, _)| (f.path, f.size)).collect()
            }
        };
        let mut packages = Vec::new();
        for (path, size) in files {
            let file = emulator_kit::read_at::HostFile::new(folders.clone(), path.clone(), size);
            let package = installs::blocking(move || pkg4::Package::open(file, &|| Ok(())))
                .await
                .map_err(|e| PluginError::InvalidInput(format!("{}: {e}", path.path)))?;
            let root = package.root().map_err(PluginError::from)?;
            packages.push((package, root));
        }
        let count = |kind: pkg4::Kind| packages.iter().filter(|(p, _)| p.kind() == kind).count();
        let (games, updates) = (count(pkg4::Kind::Game), count(pkg4::Kind::Update));
        if games > 1 {
            return Err(PluginError::InvalidInput(format!(
                "This download has {games} games; install one game at a time"
            )));
        }
        if updates > 1 {
            return Err(PluginError::InvalidInput(format!(
                "This download has {updates} updates for one game; it can install only one"
            )));
        }
        // The game first, then its update, then its DLC.
        packages.sort_by_key(|(p, _)| match p.kind() {
            pkg4::Kind::Game => 0,
            pkg4::Kind::Update => 1,
            pkg4::Kind::Dlc => 2,
        });
        let mut steps: Vec<Step> = Vec::new();
        for (package, root) in packages {
            let game = steps.first().filter(|s| s.kind == pkg4::Kind::Game).map(|s| s.dest.clone());
            let kind = package.kind();
            let dest = self.dest(kind, &root, game.as_ref(), job.as_ref()).await?;
            if folders.exists(&dest).await? {
                if kind == pkg4::Kind::Dlc {
                    continue; // an add-on that is installed stays as it is
                }
                return Err(PluginError::Precondition(format!("{} is already installed", dest.path)));
            }
            let work = HostPath::new(&dest.folder, dest.root, path(format!("{}.part", dest.path))?);
            folders.remove(&work).await?;
            steps.push(Step { package, root, kind, dest, work });
        }
        let Some(first) = steps.first() else {
            return Err(PluginError::Precondition("every add-on in this download is already installed".into()));
        };
        let work = first.work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| install_steps(writer, job, steps));
        Ok(work)
    }
}

/// One package of an install, and where it goes.
struct Step {
    package: pkg4::Package<emulator_kit::read_at::HostFile>,
    root: String,
    kind: pkg4::Kind,
    dest: HostPath,
    work: HostPath,
}

/// Copies each step's package into its work folder, then moves it into place, with one
/// progress for them all. On an error the work folders go; what is in place stays.
fn install_steps(
    writer: &Writer,
    job: &dyn InstallJob,
    steps: Vec<Step>,
) -> emulator_kit::fail::Result<(HostPath, String)> {
    let cancel = installs::cancel_check(job);
    let works: Vec<HostPath> = steps.iter().map(|s| s.work.clone()).collect();
    let run = || -> emulator_kit::fail::Result<(HostPath, String)> {
        let mut trees = Vec::new();
        for step in steps {
            let mut tree = Ps4Package { package: step.package, nodes: Vec::new() };
            let entries = tree.entries(&cancel)?;
            trees.push((tree, entries, step.root, step.dest, step.work));
        }
        let total = trees.iter().map(|(_, entries, ..)| installs::tree_size(entries, "")).sum();
        let mut done = 0;
        job.progress(0, total);
        let mut first: Option<(HostPath, String)> = None;
        for (mut tree, entries, root, dest, into) in trees {
            let title = entries
                .iter()
                .position(|e| e.path == "sce_sys/param.sfo")
                .and_then(|at| Writer::read_entry(&mut tree, at, 1 << 20).ok())
                .and_then(|bytes| emulator_kit::sfo::parse(&bytes))
                .and_then(|sfo| sfo.get("TITLE").map(|t| t.text().to_owned()))
                .filter(|t| !t.is_empty())
                .unwrap_or(root);
            installs::write_tree_counted(writer, &mut tree, &entries, "", &into, job, (&mut done, total))?;
            writer.rename(&into, &dest)?;
            first.get_or_insert((dest, title));
        }
        first.ok_or_else(|| emulator_kit::fail::Error::msg("nothing to install"))
    };
    let result = run();
    if result.is_err() {
        for work in works.iter().skip(1) {
            let _ = writer.remove(work);
        }
    }
    result
}

impl Shad {
    /// Where a package of `kind` with `root` goes. An update goes next to its game: `game`
    /// when the same download installs it, else the installed one.
    async fn dest(
        &self,
        kind: pkg4::Kind,
        root: &str,
        game: Option<&HostPath>,
        job: &dyn InstallJob,
    ) -> PluginResult<HostPath> {
        let need = |what: &str, folder: &str| {
            PluginError::Precondition(format!("map your {folder} folder in Settings first: {what} goes there"))
        };
        Ok(match kind {
            pkg4::Kind::Game => {
                if job.games_folder().is_none() && self.root_of(GAMES).await?.is_none() {
                    return Err(need("the game", GAMES));
                }
                installs::games_folder(self.ctx.folders().as_ref(), job, CONSOLE_FOLDER).await?.join(root)?
            }
            pkg4::Kind::Update => {
                let title_id = root.trim_end_matches("-patch");
                if let Some(game) = game {
                    if game.path.file_name() != Some(title_id) {
                        return Err(PluginError::InvalidInput(format!(
                            "this download's update is for {title_id}, not for its game"
                        )));
                    }
                    return Ok(HostPath::new(GAMES, game.root, path(format!("{}-patch", game.path))?));
                }
                let installed = self.games.refresh(self.ctx.folders().as_ref()).await?;
                let game = installed.iter().find(|g| g.title_id == title_id).ok_or_else(|| {
                    PluginError::Precondition(format!("this is an update for {title_id}: install the game first"))
                })?;
                let folder = game.dir.path.file_name().unwrap_or(title_id).to_owned();
                let beside = game.dir.path.as_str().rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                HostPath::new(GAMES, game.dir.root, path(format!("{beside}{folder}-patch"))?)
            }
            pkg4::Kind::Dlc => {
                let data = self.root_of(DATA).await?.ok_or_else(|| need("DLC", DATA))?;
                HostPath::new(DATA, data, path(format!("addcont/{root}"))?)
            }
        })
    }

    /// Installs the unpacked game folder in `download`, if it has one: a game's
    /// `sce_sys/param.sfo` (not an update's) with a non-empty `eboot.bin`. `None` when it has
    /// none.
    async fn begin_folder(
        &self,
        download: &installs::Download,
        job: Arc<dyn InstallJob>,
    ) -> PluginResult<Option<HostPath>> {
        let folders = self.ctx.folders().clone();
        for prefix in download.game_folders("param.sfo") {
            let bytes = download.read(folders.as_ref(), &format!("{prefix}sce_sys/param.sfo"), 1 << 20).await?;
            let Some(sfo) = emulator_kit::sfo::parse(&bytes) else { continue };
            let text = |key: &str| sfo.get(key).map(|v| v.text().to_owned()).unwrap_or_default();
            if !text("CATEGORY").starts_with("gd") || text("TITLE_ID").is_empty() {
                continue;
            }
            let title_id = text("TITLE_ID");
            let title = Some(text("TITLE")).filter(|t| !t.is_empty()).unwrap_or_else(|| title_id.clone());
            let dest = self.dest(pkg4::Kind::Game, &title_id, None, job.as_ref()).await?;
            if folders.exists(&dest).await? {
                return Err(PluginError::Precondition(format!("{title} is already installed")));
            }
            let work = HostPath::new(&dest.folder, dest.root, path(format!("{}.part", dest.path))?);
            folders.remove(&work).await?;
            let mut tree = download.tree(folders.clone(), &prefix);
            let into = work.clone();
            installs::spawn(folders, work.clone(), job, move |writer, job| {
                let entries = tree.entries(&installs::cancel_check(job))?;
                installs::write_tree(writer, &mut tree, &entries, "", &into, job)?;
                writer.rename(&into, &dest)?;
                Ok((dest, title))
            });
            return Ok(Some(work));
        }
        Ok(None)
    }
}

#[async_trait]
impl Configurator for Shad {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let schema = match scope {
            ConfigScope::Emulator => schemars::schema_for!(Settings),
            ConfigScope::Game(_) => schemars::schema_for!(GameSettings),
        };
        Ok(serde_json::to_value(schema)?)
    }

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Ok(match scope {
            ConfigScope::Emulator => serde_json::to_value(Settings::default())?,
            ConfigScope::Game(_) => serde_json::to_value(GameSettings::default())?,
        })
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        let result = match scope {
            ConfigScope::Emulator => serde_json::from_value::<Settings>(value.clone()).map(|_| ()),
            ConfigScope::Game(_) => serde_json::from_value::<GameSettings>(value.clone()).map(|_| ()),
        };
        result.map_err(|e| PluginError::InvalidInput(e.to_string()))
    }
}

#[async_trait]
impl Launcher for Shad {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
        let settings = self.settings().await;
        let fullscreen = config.get("fullscreen").and_then(Value::as_bool).unwrap_or(settings.fullscreen);
        // shadPS4 takes the game folder with -g; -f true|false sets full screen.
        let args = vec!["-g".into(), LaunchArg::path(found.dir.clone()), "-f".into(), fullscreen.to_string().into()];
        if settings.own_build {
            let Some(program) = self.own_program().await else {
                // A downloaded build is the likely way out: say so first.
                let downloaded = !versions::installed(&self.ctx).await?.is_empty();
                let fix = if downloaded {
                    "turn off Own build on the PS4 page to play with the downloaded build, or map the shadps4-build folder, or put shadps4 on your PATH"
                } else {
                    "map the shadps4-build folder on the PS4 page, or put shadps4 on your PATH"
                };
                return Err(PluginError::Precondition(format!("Own build is on, but your build isn't there: {fix}")));
            };
            let cwd = match &program {
                Location::Folder(file) => {
                    let dir = file.path.as_str().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    Location::Folder(HostPath::new(
                        OWN_BUILD,
                        file.root,
                        if dir.is_empty() { RelPath::root() } else { path(dir)? },
                    ))
                }
                // From PATH it needs a working folder; shadPS4 keeps its data in its own folder.
                _ => {
                    self.ctx.storage().write(&path("own/.keep")?, b"").await?;
                    Location::Data(path("own")?)
                }
            };
            return Ok(LaunchSpec { program, args, env: Default::default(), cwd: Some(cwd) });
        }
        let tag = versions::state(&self.ctx)
            .await
            .current
            .ok_or_else(|| PluginError::Precondition("install shadPS4 first".into()))?;
        Ok(LaunchSpec { program: program(&tag, self.platform)?.into(), args, env: Default::default(), cwd: None })
    }
}

#[async_trait]
impl Compatibility for Shad {
    async fn status(&self, game: &str) -> PluginResult<CompatInfo> {
        let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
        let page = Some(LIST_PAGE.to_owned());
        Ok(match self.compat_list().await.remove(&found.title_id.to_uppercase()) {
            Some(info) => CompatInfo { page, ..info },
            None => CompatInfo { level: CompatLevel::Unknown, notes: None, source: None, page, tested: None },
        })
    }
}

#[async_trait]
impl Slots for Shad {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        let action = |label: String, action: &str| SlotContent::Action {
            label,
            action: action.into(),
            args: Value::Null,
            style: ActionStyle::Secondary,
            icon: None,
        };
        Ok(match (request.slot, request.game.as_deref()) {
            (SlotId::GameBadge, Some(game)) => {
                vec![item("compat", games::compat_badge(&self.status(game).await?, self.platform))]
            }
            (SlotId::GameDetailSection, Some(game)) => {
                let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
                let build = versions::state(&self.ctx).await.current.map(|t| pretty(&t));
                vec![item(
                    "shadps4",
                    SlotContent::Section {
                        title: "shadPS4".into(),
                        rows: vec![
                            SectionRow { label: "Title id".into(), value: found.title_id },
                            SectionRow { label: "Version".into(), value: found.version },
                            SectionRow {
                                label: "Build".into(),
                                value: build.unwrap_or_else(|| "not installed".into()),
                            },
                        ],
                    },
                )]
            }
            (SlotId::EmulatorPanel, None) => {
                let mut items = Vec::new();
                let own_build = self.settings().await.own_build;
                if !own_build
                    && let Some(program) = self.own_program().await
                    && let Ok(Some(found)) = self.ctx.programs().find(&program).await
                {
                    items.push(item("use-own", action(format!("Use the shadPS4 at {found}"), "use-own")));
                }
                if own_build && versions::state(&self.ctx).await.current.is_some() {
                    items.push(item("use-official", action("Switch to official builds".into(), "use-official")));
                }
                for found in self.candidates().await.unwrap_or_default() {
                    items.push(item(
                        &format!("install-{}", found.label),
                        SlotContent::Action {
                            label: format!("Install {}", found.label),
                            action: HOST_INSTALL_GAME.into(),
                            args: serde_json::to_value(&found.source)?,
                            style: ActionStyle::Secondary,
                            icon: None,
                        },
                    ));
                }
                items.extend([
                    emulator_kit::controls::Controls { keys: KEYS }.keyboard_panel(),
                    item("check", action("Check for new builds".into(), "check")),
                ]);
                if let Some(previous) = versions::state(&self.ctx).await.previous {
                    items.push(item("rollback", action(format!("Go back to {}", pretty(&previous)), "rollback")));
                }
                items
            }
            (SlotId::SettingsPanel, None) => vec![item(
                "settings",
                SlotContent::SettingsPanel { title: "shadPS4".into(), scope: ConfigScope::Emulator },
            )],
            _ => Vec::new(),
        })
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        match action {
            "check" => cache::forget(&self.ctx, &path("cache/releases.json")?).await,
            "use-own" => self.set_own_build(true).await,
            "use-official" => self.set_own_build(false).await,
            "rollback" => versions::rollback(&self.ctx).await.map(|_| ()),
            other => Err(PluginError::NotFound(format!("action {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_zip_and_its_tag() {
        let linux = Some(Platform::Linux);
        assert!(asset_matches("shadps4-linux-sdl-0.20.0.zip", linux));
        assert!(!asset_matches("shadps4-win64-sdl-2026-0.20.0.zip", linux));
        assert!(!asset_matches("shadps4-linux-0.18.0.tar.gz", linux));
        assert!(asset_matches("shadps4-win64-sdl-2026-0.20.0.zip", Some(Platform::Windows)));
        let mac = Some(Platform::Macos);
        assert!(asset_matches("shadps4-macos-sdl-2026-0.20.0.zip", mac));
        assert!(asset_matches("shadps4-macos-sdl-2026-10-17-e6c79a5.zip", mac));
        assert!(!asset_matches("shadps4-linux-sdl-0.20.0.zip", mac));
        assert_eq!(pretty("v.0.18.0"), "0.18.0");
    }
}
