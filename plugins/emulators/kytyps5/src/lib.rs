//! KytyPS5, the PS5 emulator, as a plugin.
//!
//! - Builds: KytyPS5's GitHub releases (Linux `.tar.gz`, macOS `.zip`). The host downloads,
//!   checks the release's SHA-256 and unpacks into `builds/<tag>`; the plugin keeps the
//!   current and the previous build for a rollback. A new build runs once with `--help`
//!   before it becomes current; one that prints no banner is removed.
//! - Games: PS5 game folders (`sce_sys/param.json`) in the player's game folders.
//! - KytyPS5 writes saves and caches relative to its working folder, which is the build's
//!   folder. When the current build changes, those folders move to the new build, so saves
//!   and shader caches survive updates and rollbacks.
//! - Compatibility: the community list KytyPS5's own launcher uses.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use emulator_kit::games::{self, Console, GameIndex};
use emulator_kit::installs::{self, ExfatImage, Format, Ps5Package, Tree, Writer};
use emulator_kit::read_at::HostFile;
use emulator_kit::{args, cache, compat, github, versions};
use nexus_api::schemars::{self, JsonSchema};
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");
/// KytyPS5's releases on GitHub: the default release repo.
pub const RELEASES: &str = "https://api.github.com/repos/KytyPS5/KytyPS5/releases?per_page=10";
const UPSTREAM_REPO: &str = "KytyPS5/KytyPS5";
/// The macOS default until upstream PR #1469 (run on MoltenVK) is merged: its test builds,
/// published as pre-releases. Without it every game stops at start on macOS. When the PR is
/// merged, delete this and the macOS case in [`release_source`].
const MACOS_TEST_REPO: &str = "mohamed-rekiba/KytyPS5";
/// The community list's page, for the player.
pub const LIST_PAGE: &str = "https://kytyps5.github.io/";
pub const COMPATIBILITY: &str = "https://kytyps5.github.io/data/compatibility.json";
pub const GAMES: &str = nexus_api::GAMES_FOLDER;
/// Where installs go inside the first game folder.
const CONSOLE_FOLDER: &str = "PS5";
/// Where you keep packages and disc images to install.
pub const PACKAGES: &str = nexus_api::PACKAGES_FOLDER;
/// Where macOS builds keep the emulator binary.
const MAC_BUNDLE: &str = "KytyPS5.app/Contents/MacOS";
/// Folders KytyPS5 writes relative to its working folder.
pub const DATA_DIRS: [&str; 6] = ["_SaveData", "_PipelineCache", "_DownloadData", "_TempData", "_Textures", "_Patches"];
/// Where the data folders wait while no build is installed.
const PARKED: &str = "data";
/// The folder you map to your own KytyPS5 build.
pub const OWN_BUILD: &str = "kyty-build";
/// Where a build's binary may be in your own build's folder: at its top, in KytyPS5's usual
/// build folders, or in a macOS app bundle.
const OWN_PATHS: [&str; 6] = ["", "_Build/release-linux", "_Build/linux/install", "_Build/linux", "build", MAC_BUNDLE];
/// Where your own build runs when it comes from your PATH, so its saves have a home.
const OWN_DATA: &str = "own";
/// What the Emulators screen calls your own build.
const OWN_LABEL: &str = "Your own build";
const LISTS_TTL: Duration = Duration::from_secs(6 * 3600);

/// What KytyPS5 tells a game its screen is (`--video-out-resolution`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub enum VideoOut {
    /// Game default
    #[default]
    Title,
    /// 1080p (Full HD)
    FullHd,
    /// 4K (Ultra HD)
    Uhd,
}

/// KytyPS5's window size, as the old launcher offered it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub enum Resolution {
    /// 1280×720
    #[serde(rename = "1280x720")]
    Hd,
    /// 1600×900
    #[serde(rename = "1600x900")]
    HdPlus,
    /// 1920×1080
    #[default]
    #[serde(rename = "1920x1080")]
    FullHd,
    /// 2560×1440
    #[serde(rename = "2560x1440")]
    Qhd,
    /// 3840×2160
    #[serde(rename = "3840x2160")]
    Uhd,
}

impl Resolution {
    const ALL: [Resolution; 5] = [Self::Hd, Self::HdPlus, Self::FullHd, Self::Qhd, Self::Uhd];

    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Hd => (1280, 720),
            Self::HdPlus => (1600, 900),
            Self::FullHd => (1920, 1080),
            Self::Qhd => (2560, 1440),
            Self::Uhd => (3840, 2160),
        }
    }

    /// The preset closest to a window size.
    pub fn nearest(width: u64, height: u64) -> Self {
        let off = |r: &Resolution| {
            let (w, h) = r.size();
            (u64::from(w)).abs_diff(width) + (u64::from(h)).abs_diff(height)
        };
        Self::ALL.into_iter().min_by_key(off).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub enum PresentMode {
    /// Mailbox (low latency)
    #[default]
    Mailbox,
    /// Fifo (V-Sync)
    Fifo,
    /// Immediate (uncapped)
    Immediate,
}

/// KytyPS5 settings.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars", extend("x-order" = ["own_build", "release_repo", "prereleases", "fullscreen", "resolution", "video_out", "present_mode", "amd_cpu", "extra_args"]))]
pub struct Settings {
    #[serde(default = "yes")]
    #[schemars(title = "Full screen")]
    pub fullscreen: bool,
    /// The window's size. The game's screen size is "Video out".
    #[serde(default)]
    #[schemars(title = "Resolution")]
    pub resolution: Resolution,
    #[serde(default)]
    #[schemars(title = "Present mode")]
    pub present_mode: PresentMode,
    /// What the game is told its screen is.
    #[serde(default)]
    #[schemars(title = "Video out")]
    pub video_out: VideoOut,
    /// Turn on for AMD processors.
    #[serde(default)]
    #[schemars(title = "AMD processor")]
    pub amd_cpu: bool,
    /// Passed last, so they can override the rest.
    #[serde(default)]
    #[schemars(title = "Extra arguments")]
    pub extra_args: String,
    /// Runs the KytyPS5 you built or installed: from the folder you map as kyty-build, or
    /// from your PATH. Installing an official build turns this off and copies your saves.
    #[serde(default)]
    #[schemars(title = "Use your own build")]
    pub own_build: bool,
    /// Where official builds come from, as owner/repo. Empty: KytyPS5/KytyPS5, or on macOS its macOS test builds at mohamed-rekiba/KytyPS5.
    #[serde(default)]
    #[schemars(title = "Release repo")]
    pub release_repo: String,
    /// Also offer pre-releases from the release repo. On by default on macOS, where the test builds are pre-releases.
    #[serde(default)]
    #[schemars(title = "Pre-releases")]
    pub prereleases: bool,
}

fn yes() -> bool {
    true
}

/// Settings stored before Resolution replaced the window width and height: the nearest preset
/// instead. `None` when there is nothing to change.
pub fn migrate_settings(stored: &Value) -> Option<Value> {
    let old = stored.as_object()?;
    if !old.contains_key("width") && !old.contains_key("height") {
        return None;
    }
    let mut new = old.clone();
    let width = new.remove("width").and_then(|v| v.as_u64()).unwrap_or(1920);
    let height = new.remove("height").and_then(|v| v.as_u64()).unwrap_or(1080);
    if !new.contains_key("resolution") {
        new.insert("resolution".into(), serde_json::to_value(Resolution::nearest(width, height)).ok()?);
    }
    Some(Value::Object(new))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen: true,
            resolution: Resolution::default(),
            present_mode: PresentMode::default(),
            video_out: VideoOut::default(),
            amd_cpu: false,
            extra_args: String::new(),
            own_build: false,
            release_repo: String::new(),
            prereleases: false,
        }
    }
}

/// Per-game settings: anything set here wins over the KytyPS5 settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Video out")]
    pub video_out: Option<VideoOut>,
    #[serde(default)]
    #[schemars(title = "Extra arguments")]
    pub extra_args: Option<String>,
}

/// The command line KytyPS5 gets for a game. The window size only sizes the window;
/// `--video-out-resolution` is what the game is told its screen is. Extra arguments come last.
/// `supported`: the options the build knows (its `--help`); a setting it doesn't know is left
/// out, since KytyPS5 stops at an unknown option. `None` keeps them all.
pub fn kyty_args(settings: &Settings, game: LaunchArg, supported: Option<&BTreeSet<String>>) -> Vec<LaunchArg> {
    let mut out: Vec<LaunchArg> = vec!["--game".into(), game];
    for (option, value) in setting_options(settings) {
        if supported.is_some_and(|known| !known.contains(option)) {
            continue;
        }
        out.push(option.into());
        out.extend(value.map(LaunchArg::from));
    }
    out.extend(args::shell_split(&settings.extra_args).into_iter().map(LaunchArg::from));
    out
}

/// The options the settings become, each with its value if it takes one.
fn setting_options(settings: &Settings) -> Vec<(&'static str, Option<String>)> {
    let video_out = match settings.video_out {
        VideoOut::Title => "Title",
        VideoOut::FullHd => "FullHd",
        VideoOut::Uhd => "Uhd",
    };
    let present = match settings.present_mode {
        PresentMode::Mailbox => "Mailbox",
        PresentMode::Fifo => "Fifo",
        PresentMode::Immediate => "Immediate",
    };
    let (width, height) = settings.resolution.size();
    let mut options: Vec<(&'static str, Option<String>)> = vec![
        ("--screen-width", Some(width.to_string())),
        ("--screen-height", Some(height.to_string())),
        ("--present-mode", Some(present.into())),
        ("--video-out-resolution", Some(video_out.into())),
    ];
    if settings.fullscreen {
        options.push(("--fullscreen", None));
    }
    if settings.amd_cpu {
        options.push(("--amd-cpu", None));
    }
    options
}

/// The options a KytyPS5 `--help` lists: each on its own line, indented, starting with `--`.
pub fn help_options(help: &str) -> BTreeSet<String> {
    help.lines()
        .filter(|line| line.starts_with(' '))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| word.starts_with("--"))
        .map(str::to_owned)
        .collect()
}

/// Whether a release asset is the build for this system. Linux builds are named like
/// `…Linux…x86_64….tar.gz`, Windows builds `…Windows-x64.zip`. macOS builds are x86-64 and run
/// under Rosetta 2, so on macOS any archive whose name says macOS (or Darwin, OSX) matches.
pub fn asset_matches(name: &str, platform: Option<Platform>) -> bool {
    match platform {
        Some(Platform::Windows) => name.contains("Windows") && name.ends_with(".zip"),
        Some(Platform::Macos) => {
            let n = name.to_ascii_lowercase();
            ["macos", "darwin", "osx"].iter().any(|k| n.contains(k))
                && [".tar.gz", ".tgz", ".zip"].iter().any(|e| n.ends_with(e))
        }
        _ => name.contains("Linux") && name.contains("x86_64") && name.ends_with(".tar.gz"),
    }
}

/// The settings before you change any: on macOS pre-releases are on, for the test builds.
pub fn default_settings(platform: Option<Platform>) -> Settings {
    Settings { prereleases: platform == Some(Platform::Macos), ..Settings::default() }
}

/// The release repo and whether its pre-releases count, from the settings and this system.
pub fn release_source(settings: &Settings, platform: Option<Platform>) -> PluginResult<(String, bool)> {
    let repo = settings.release_repo.trim();
    if repo.is_empty() {
        let repo = if platform == Some(Platform::Macos) { MACOS_TEST_REPO } else { UPSTREAM_REPO };
        return Ok((repo.to_owned(), settings.prereleases));
    }
    let part = |p: &str| {
        !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    match repo.split_once('/') {
        Some((owner, name)) if part(owner) && part(name) => Ok((repo.to_owned(), settings.prereleases)),
        _ => Err(PluginError::InvalidInput(format!("release repo {repo:?}: write it as owner/repo"))),
    }
}

/// The releases list of `repo` (owner/repo) on GitHub's API.
fn releases_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases?per_page=10")
}

/// Where the releases list of `repo` is cached, so a new release repo is not served the old list.
fn releases_cache(repo: &str) -> PluginResult<RelPath> {
    path(format!("cache/releases-{}.json", repo.replace('/', "-")))
}

/// "KytyPS5-2026-09-29-59a1760" -> "2026-09-29 · 59a1760".
pub fn pretty(tag: &str) -> String {
    let t = tag.trim_start_matches("KytyPS5-");
    match t.rsplit_once('-') {
        Some((date, sha)) if sha.len() >= 7 && date.len() == 10 => format!("{date} · {sha}"),
        _ => t.to_owned(),
    }
}

/// "2026-09-29 (6799ecb)" from the first line of `kyty_emulator --help`, which reads
/// "... git = 6799ecb, date = 2026.09.29".
pub fn banner_version(banner: &str) -> Option<String> {
    let line = banner.lines().next()?;
    let field =
        |k: &str| line.split(&format!("{k} = ")).nth(1).map(|s| s.split(',').next().unwrap_or("").trim().to_owned());
    let git = field("git").filter(|g| !g.is_empty())?;
    match field("date").filter(|d| !d.is_empty()) {
        Some(date) => Some(format!("{} ({git})", date.replace('.', "-"))),
        None => Some(git),
    }
}

fn path(p: impl Into<String>) -> PluginResult<RelPath> {
    Ok(RelPath::new(p)?)
}

// The old app's keyboard screen, for KytyPS5's default layout.
pub const KEYS: &[(&str, &str)] = &[
    ("Cross", "J"),
    ("Circle", "L"),
    ("Square", "K"),
    ("Triangle", "I"),
    ("D-pad", "Arrow keys"),
    ("Options", "Enter"),
    ("Left stick", "W A S D"),
    ("Right stick", "T F G H"),
    ("L1 / R1", "Q / E"),
    ("L2 / R2", "Z / C"),
    ("L3 / R3", "Left Shift / Left Ctrl"),
    ("Touchpad", "Backspace / Tab"),
    ("Mouse as right stick", "F7"),
];

pub struct KytyPlugin;

#[async_trait]
impl Plugin for KytyPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl EmulatorPlugin for KytyPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let platform = ctx.services().get::<HostInfo>().and_then(|h| h.platform);
        let stored = ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        if let Some(migrated) = migrate_settings(&stored) {
            ctx.config().set(&ConfigScope::Emulator, migrated).await?;
        }
        let kyty =
            Arc::new(Kyty { ctx, platform, games: GameIndex::new(GAMES, Console::Ps5), options: Default::default() });
        Ok(Provided {
            catalog: Some(kyty.clone()),
            downloader: Some(kyty.clone()),
            installer: Some(kyty.clone()),
            configurator: Some(kyty.clone()),
            launcher: Some(kyty.clone()),
            compatibility: Some(kyty.clone()),
            game_installer: Some(kyty.clone()),
            slots: Some(kyty),
            controllers: Some(Arc::new(emulator_kit::controls::Controls { keys: KEYS })),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::emulator(|| Box::new(KytyPlugin)) }

struct Kyty {
    ctx: PluginContext,
    platform: Option<Platform>,
    games: GameIndex,
    /// The options each downloaded build knows ([`help_options`]), by tag: a build never changes.
    options: std::sync::Mutex<HashMap<String, BTreeSet<String>>>,
}

impl Kyty {
    /// The stored settings over this system's defaults.
    async fn settings(&self) -> Settings {
        let defaults = default_settings(self.platform);
        let stored = self.ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        let (Ok(Value::Object(mut merged)), Value::Object(stored)) = (serde_json::to_value(&defaults), stored) else {
            return defaults;
        };
        merged.extend(stored);
        serde_json::from_value(Value::Object(merged)).unwrap_or(defaults)
    }

    async fn set_own_build(&self, on: bool) -> PluginResult<()> {
        let mut stored = self.ctx.config().get(&ConfigScope::Emulator).await.unwrap_or(Value::Null);
        if !stored.is_object() {
            stored = serde_json::to_value(default_settings(self.platform))?;
        }
        stored["own_build"] = Value::Bool(on);
        self.ctx.config().set(&ConfigScope::Emulator, stored).await
    }

    fn binary_name(&self) -> &'static str {
        if self.platform == Some(Platform::Windows) { "kyty_emulator.exe" } else { "kyty_emulator" }
    }

    /// Your own build's binary: the kyty-build path itself when it is the program, else the
    /// first one in the kyty-build folder, or else one on PATH.
    async fn own_binary(&self) -> Option<Location> {
        let programs = self.ctx.programs();
        let name = self.binary_name();
        for root in self.ctx.folders().roots(OWN_BUILD).await.unwrap_or_default() {
            let itself = Location::Folder(HostPath::new(OWN_BUILD, root.root, RelPath::root()));
            if let Ok(Some(_)) = programs.find(&itself).await {
                return Some(itself);
            }
            for dir in OWN_PATHS {
                let file = if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
                let Ok(file) = RelPath::new(file) else { continue };
                let location = Location::Folder(HostPath::new(OWN_BUILD, root.root, file));
                if let Ok(Some(_)) = programs.find(&location).await {
                    return Some(location);
                }
            }
        }
        let system = Location::System("kyty_emulator".into());
        matches!(programs.find(&system).await, Ok(Some(_))).then_some(system)
    }

    /// Your own build's version, from its `--help` banner.
    async fn own_version(&self, binary: &Location) -> Option<String> {
        self.banner(binary, Duration::from_secs(3)).await
    }

    /// The options a build knows, from its `--help`; `None` when it doesn't answer, and then no
    /// setting is left out. `tag` caches a downloaded build's answer.
    async fn options(&self, binary: &Location, tag: Option<&str>) -> Option<BTreeSet<String>> {
        if let Some(known) = tag.and_then(|t| self.options.lock().ok()?.get(t).cloned()) {
            return Some(known);
        }
        let out = match self.ctx.programs().output(binary, &["--help".into()], Duration::from_secs(5)).await {
            Ok(out) => out,
            Err(e) => {
                self.ctx.log().warn(&format!("--help of {binary}: {e}"));
                return None;
            }
        };
        let known = help_options(&String::from_utf8_lossy(&out));
        if known.is_empty() {
            return None;
        }
        if let (Some(tag), Ok(mut cache)) = (tag, self.options.lock()) {
            cache.insert(tag.to_owned(), known.clone());
        }
        Some(known)
    }

    /// The command line for `binary`, without the settings it doesn't know (said in the log).
    async fn args_for(
        &self,
        binary: &Location,
        tag: Option<&str>,
        settings: &Settings,
        game: LaunchArg,
    ) -> Vec<LaunchArg> {
        let known = self.options(binary, tag).await;
        if let Some(known) = &known {
            for (option, _) in setting_options(settings).into_iter().filter(|(o, _)| !known.contains(*o)) {
                self.ctx.log().info(&format!("KytyPS5 at {binary} has no {option}: that setting is left out"));
            }
        }
        kyty_args(settings, game, known.as_ref())
    }

    /// A build's version, from its `--help` banner. `None` when it doesn't run in `timeout` or
    /// prints no banner.
    async fn banner(&self, binary: &Location, timeout: Duration) -> Option<String> {
        let out = match self.ctx.programs().output(binary, &["--help".into()], timeout).await {
            Ok(out) => out,
            Err(e) => {
                self.ctx.log().warn(&format!("--help of {binary}: {e}"));
                return None;
            }
        };
        banner_version(&String::from_utf8_lossy(&out))
    }

    /// Copies your own build's data folders ([`DATA_DIRS`]: saves, the shader cache and the
    /// rest) into `to`, each one only when `to` has none of it yet. Your build keeps its own
    /// copy.
    async fn adopt_own_data(&self, to: &RelPath) -> PluginResult<()> {
        let storage = self.ctx.storage();
        let empty = |target: RelPath| async move {
            PluginResult::Ok(!(storage.exists(&target).await? && !storage.list(&target).await?.is_empty()))
        };
        let binary = match self.own_binary().await {
            Some(Location::Folder(binary)) => binary,
            // From PATH it ran in the plugin's own folder: its data is already in the data
            // folder, so it moves instead of copying.
            Some(_) => {
                for name in DATA_DIRS {
                    let (from, target) = (path(format!("{OWN_DATA}/{name}"))?, to.join(name)?);
                    if storage.exists(&from).await? && empty(target.clone()).await? {
                        storage.remove(&target).await?;
                        storage.rename(&from, &target).await?;
                    }
                }
                return Ok(());
            }
            None => return Ok(()),
        };
        let dir = binary.path.as_str().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let dir = match dir.strip_suffix(&format!("/{MAC_BUNDLE}")).or(if dir == MAC_BUNDLE { Some("") } else { None })
        {
            Some(build) => build.to_owned(),
            None => dir.to_owned(),
        };
        for name in DATA_DIRS {
            let from = if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
            let source = HostPath::new(OWN_BUILD, binary.root, path(from)?);
            let target = to.join(name)?;
            if !self.ctx.folders().exists(&source).await? {
                continue;
            }
            if !empty(target.clone()).await? {
                self.ctx.log().warn(&format!("official build already has {name}; your own build's was not copied"));
                continue;
            }
            self.copy_tree(&source, &target).await?;
        }
        Ok(())
    }

    /// Copies a folder from a host folder into the data folder, file by file, each file in
    /// pieces of at most [`MAX_FOLDER_READ`], so a big cache never fills the memory.
    async fn copy_tree(&self, from: &HostPath, to: &RelPath) -> PluginResult<()> {
        let folders = self.ctx.folders();
        let storage = self.ctx.storage();
        let mut pending = vec![(from.clone(), to.clone())];
        while let Some((dir, target)) = pending.pop() {
            for entry in folders.list(&dir).await? {
                let source = HostPath::new(&dir.folder, dir.root, dir.path.join(&entry.name)?);
                let dest = target.join(&entry.name)?;
                if entry.is_dir {
                    pending.push((source, dest));
                    continue;
                }
                // An empty file is written too.
                storage.write(&dest, &[]).await?;
                let mut done = 0u64;
                loop {
                    let piece = folders.read_range(&source, done, MAX_FOLDER_READ).await?;
                    storage.write_at(&dest, done, &piece).await?;
                    done += piece.len() as u64;
                    if (piece.len() as u64) < MAX_FOLDER_READ {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Releases with a build for this system, newest first.
    async fn builds(&self) -> PluginResult<Vec<(github::Release, github::Asset)>> {
        let platform = self.platform;
        let (repo, prereleases) = release_source(&self.settings().await, platform)?;
        let json = cache::cached_json(&self.ctx, &releases_url(&repo), &releases_cache(&repo)?, LISTS_TTL).await?;
        Ok(github::builds(&json, prereleases, |name| asset_matches(name, platform)))
    }

    /// The emulator binary in a build's folder: at its top, one folder down, or in the macOS
    /// app bundle.
    async fn binary(&self, tag: &str) -> PluginResult<RelPath> {
        let storage = self.ctx.storage();
        let build = versions::folder(tag)?;
        let binary = self.binary_name();
        let top = build.join(binary)?;
        if storage.exists(&top).await? {
            return Ok(top);
        }
        let bundled = path(format!("{build}/{MAC_BUNDLE}/{binary}"))?;
        if storage.exists(&bundled).await? {
            return Ok(bundled);
        }
        for name in storage.list(&build).await? {
            let inner = build.join(&name)?.join(binary)?;
            if storage.exists(&inner).await? {
                return Ok(inner);
            }
        }
        Err(PluginError::NotFound(format!("no {binary} in KytyPS5 {tag}")))
    }

    /// The folder KytyPS5 runs in for a build: where its binary is, or for a macOS app bundle
    /// the build's folder, so saves don't go inside the signed bundle.
    async fn work_dir(&self, tag: &str) -> PluginResult<RelPath> {
        let binary = self.binary(tag).await?;
        let parent = binary.as_str().rsplit_once('/').map(|(dir, _)| dir).unwrap_or_default();
        match parent.strip_suffix(&format!("/{MAC_BUNDLE}")) {
            Some(build) => path(build),
            None => path(parent),
        }
    }

    /// Moves the data folders from `from` to `to`. When `to` already has one (a build that
    /// ships it), the entries it lacks move into it; an entry both have stays in `from`, so
    /// nothing is lost.
    async fn move_data(&self, from: &RelPath, to: &RelPath) -> PluginResult<()> {
        let storage = self.ctx.storage();
        for dir in DATA_DIRS {
            let (source, target) = (from.join(dir)?, to.join(dir)?);
            if !storage.exists(&source).await? {
                continue;
            }
            if !storage.exists(&target).await? {
                storage.rename(&source, &target).await?;
                continue;
            }
            for name in storage.list(&source).await? {
                let (from_entry, to_entry) = (source.join(&name)?, target.join(&name)?);
                if storage.exists(&to_entry).await? {
                    self.ctx.log().warn(&format!("{to_entry} already exists; {from_entry} stays where it is"));
                } else {
                    storage.rename(&from_entry, &to_entry).await?;
                }
            }
        }
        Ok(())
    }

    /// The data folders' home now: the current build's folder, or the parking folder.
    async fn data_home(&self) -> PluginResult<RelPath> {
        match versions::state(&self.ctx).await.current {
            Some(tag) => match self.work_dir(&tag).await {
                Ok(dir) => Ok(dir),
                Err(_) => path(PARKED),
            },
            None => path(PARKED),
        }
    }

    async fn compat_list(&self) -> std::collections::HashMap<String, CompatInfo> {
        let Ok(cache) = path("cache/compatibility.json") else { return Default::default() };
        match cache::cached_json(&self.ctx, COMPATIBILITY, &cache, LISTS_TTL).await {
            Ok(json) => compat::kyty(&json, self.platform),
            Err(e) => {
                self.ctx.log().warn(&format!("compatibility list: {e}"));
                Default::default()
            }
        }
    }
}

#[async_trait]
impl Catalog for Kyty {
    async fn list(&self, page: PageRequest) -> PluginResult<Page<GameSummary>> {
        let all = self.games.refresh(self.ctx.folders().as_ref()).await?;
        let total = all.len() as u32;
        let items = all.iter().skip(page.offset as usize).take(page.limit as usize).map(|g| g.summary("PS5")).collect();
        Ok(Page { items, total: Some(total) })
    }

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        let all = self.games.refresh(self.ctx.folders().as_ref()).await?;
        Ok(games::search(all, query).iter().map(|g| g.summary("PS5")).collect())
    }

    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        self.games.details(self.ctx.folders().as_ref(), game, "PS5").await
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
impl Downloader for Kyty {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan> {
        let DownloadRequest::EmulatorBuild { version } = request else {
            return Err(PluginError::Unsupported("KytyPS5 downloads its own builds only".into()));
        };
        let builds = self.builds().await?;
        let (release, asset) = match version {
            Some(tag) => builds.into_iter().find(|(r, _)| &r.tag == tag),
            None => builds.into_iter().next(),
        }
        .ok_or_else(|| PluginError::NotFound("no KytyPS5 build for this system".into()))?;
        let sha256 = asset
            .sha256
            .clone()
            .ok_or_else(|| PluginError::Precondition(format!("GitHub has no checksum for {}", asset.name)))?;
        let format = if asset.name.ends_with(".zip") { ArchiveFormat::Zip } else { ArchiveFormat::TarGz };
        Ok(DownloadPlan {
            label: format!("KytyPS5 {}", pretty(&release.tag)),
            artifacts: vec![Artifact {
                url: asset.url.clone(),
                size: (asset.size > 0).then_some(asset.size),
                sha256,
                signature: None,
                dest: path(format!("downloads/{}", asset.name))?,
                unpack: Some(Unpack { format, into: versions::folder(&release.tag)? }),
            }],
        })
    }
}

#[async_trait]
impl Installer for Kyty {
    async fn available(&self) -> PluginResult<Vec<VersionInfo>> {
        Ok(self
            .builds()
            .await?
            .into_iter()
            .map(|(r, _)| VersionInfo { notes: Some(pretty(&r.tag)), version: r.tag })
            .collect())
    }

    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>> {
        if self.settings().await.own_build
            && let Some(binary) = self.own_binary().await
        {
            let version = match self.own_version(&binary).await {
                Some(v) => format!("{OWN_LABEL} · {v}"),
                None => OWN_LABEL.to_owned(),
            };
            return Ok(vec![InstalledVersion { version, path: path(OWN_DATA)?, own: true }]);
        }
        let mut out = Vec::new();
        for tag in versions::installed(&self.ctx).await? {
            out.push(InstalledVersion { path: versions::folder(&tag)?, version: tag, own: false });
        }
        Ok(out)
    }

    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let folder = versions::folder(version)?;
        if !files.iter().any(|f| f.unpacked.as_ref() == Some(&folder)) {
            return Err(PluginError::InvalidInput(format!("the download of {version} was not unpacked")));
        }
        let binary = self.binary(version).await?;
        // Downloads can lose the flag.
        self.ctx.storage().set_executable(&binary).await?;
        // Before the data moves, so a build that doesn't run never takes the saves.
        if self.banner(&Location::Data(binary.clone()), MAX_PROGRAM_TIME).await.is_none() {
            versions::discard(&self.ctx, version).await?;
            return Err(PluginError::Precondition(format!("KytyPS5 {} does not run on this system", pretty(version))));
        }
        let from = self.data_home().await?;
        let to = self.work_dir(version).await?;
        if from != to {
            self.move_data(&from, &to).await?;
        }
        // Switching from your own build: its saves and caches come along, and the official
        // build runs.
        if self.settings().await.own_build {
            self.adopt_own_data(&to).await?;
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
        if version.starts_with(OWN_LABEL) {
            // Your own build isn't removed; the launcher stops using it.
            return self.set_own_build(false).await;
        }
        let state = versions::state(&self.ctx).await;
        if state.current.as_deref() == Some(version) {
            // The data goes to the build that takes over, or waits in the parking folder.
            let next = match state.previous.as_deref() {
                Some(previous) => self.work_dir(previous).await.unwrap_or(path(PARKED)?),
                None => path(PARKED)?,
            };
            self.move_data(&self.work_dir(version).await?, &next).await?;
        }
        versions::remove(&self.ctx, version).await
    }
}

/// What a game installs from: a package or image file, or an unpacked game folder in a
/// download (its tree and its path in the download).
enum Source {
    File(HostFile, Format),
    Folder(installs::HostTree, String),
}

/// The package or image in `download` that holds the game: the one nearest the top. Update
/// and add-on packages hold no game, so they are skipped. A package that fails to open at or
/// above that depth might be the game, so its error stops the install; two at the same depth
/// are never guessed.
async fn game_package(folders: &Arc<dyn Folders>, download: &installs::Download) -> PluginResult<Source> {
    let mut best: Option<(usize, Vec<String>, HostFile, Format)> = None;
    let mut failure: Option<(usize, PluginError)> = None;
    for (file, format) in
        download.packages(folders.as_ref(), |f| matches!(f, Format::Ps5Package | Format::Exfat)).await?
    {
        let depth = file.depth();
        let reader = HostFile::new(folders.clone(), file.path.clone(), file.size);
        let opened = installs::blocking(move || {
            let mut tree: Box<dyn Tree> = match format {
                Format::Ps5Package => Box::new(Ps5Package::open(reader, &|| Ok(()))?),
                _ => Box::new(ExfatImage::open(reader)?),
            };
            Ok(installs::holds_game(&tree.entries(&|| Ok(()))?))
        })
        .await;
        match opened {
            Ok(true) => match &mut best {
                Some((top, names, ..)) if depth == *top => names.push(file.rel.clone()),
                Some((top, ..)) if depth > *top => (),
                _ => {
                    let reader = HostFile::new(folders.clone(), file.path.clone(), file.size);
                    best = Some((depth, vec![file.rel.clone()], reader, format));
                }
            },
            Ok(false) => (),
            Err(e) => {
                if failure.as_ref().is_none_or(|f| depth < f.0) {
                    failure = Some((depth, PluginError::InvalidInput(format!("{}: {e}", file.rel))));
                }
            }
        }
    }
    if let Some((depth, e)) = failure
        && best.as_ref().is_none_or(|b| depth <= b.0)
    {
        return Err(e);
    }
    let Some((_, names, reader, format)) = best else { return Err(download.nothing_to_install()) };
    if names.len() > 1 {
        return Err(PluginError::InvalidInput(format!(
            "Several packages hold a game ({}); choose a single release folder",
            names.join(", ")
        )));
    }
    Ok(Source::File(reader, format))
}

#[async_trait]
impl GameInstaller for Kyty {
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
                if matches!(installs::format(&head), Some(Format::Ps5Package | Format::Exfat)) {
                    out.push(Installable { source: file, label: entry.name });
                }
            }
        }
        Ok(out)
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.ctx.folders().clone();
        let base = installs::games_folder(folders.as_ref(), job.as_ref(), CONSOLE_FOLDER).await?;
        // A download is a folder: an unpacked game in it, else the package that holds the game.
        let from = match installs::Download::scan(folders.as_ref(), source).await? {
            None => {
                let head = folders.read_range(source, 0, 512).await?;
                match installs::format(&head) {
                    Some(f @ (Format::Ps5Package | Format::Exfat)) => {
                        let size = installs::file_size(folders.as_ref(), source).await?;
                        Source::File(HostFile::new(folders.clone(), source.clone(), size), f)
                    }
                    _ => {
                        return Err(PluginError::InvalidInput(format!(
                            "{} isn't a PS5 package or exFAT image",
                            source.path
                        )));
                    }
                }
            }
            Some(download) => {
                let found = download.game_folders("param.json");
                let depth = |p: &String| p.matches('/').count();
                // Two at the same depth are never guessed, as for packages.
                if let [first, second, ..] = found.as_slice()
                    && depth(first) == depth(second)
                {
                    return Err(PluginError::InvalidInput(format!(
                        "Several folders hold a game ({first}, {second}); choose a single release folder"
                    )));
                }
                match found.first() {
                    Some(prefix) => Source::Folder(download.tree(folders.clone(), prefix), prefix.clone()),
                    None => game_package(&folders, &download).await?,
                }
            }
        };
        let stem = match &from {
            Source::File(file, _) => {
                let name = file.path.path.file_name().unwrap_or("game");
                name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_owned()
            }
            Source::Folder(_, prefix) => {
                let prefix = prefix.trim_end_matches('/');
                prefix.rsplit('/').next().filter(|p| !p.is_empty()).unwrap_or("game").to_owned()
            }
        };
        // An unpacked folder may move rather than be copied, when the host made it.
        let folder = match &from {
            Source::Folder(_, prefix) => Some(source.join(prefix.trim_end_matches('/'))?),
            Source::File(..) => None,
        };
        let work = base.join(&format!("{stem}.part"))?;
        folders.remove(&work).await?;
        let into = work.clone();
        installs::spawn(folders, work.clone(), job, move |writer, job| {
            use emulator_kit::fail::Context;
            let cancel = installs::cancel_check(job);
            let mut tree: Box<dyn Tree> = match from {
                Source::File(file, Format::Ps5Package) => Box::new(Ps5Package::open(file, &cancel)?),
                Source::File(file, _) => Box::new(ExfatImage::open(file)?),
                Source::Folder(tree, _) => Box::new(tree),
            };
            let entries = tree.entries(&cancel)?;
            let prefix = installs::game_folder(&entries, "param.json")
                .context("This holds no PS5 game: it has no sce_sys/param.json")?;
            let at =
                entries.iter().position(|e| e.path == format!("{prefix}sce_sys/param.json")).context("param.json")?;
            let param = Writer::read_entry(tree.as_mut(), at, 1 << 20)?;
            let (title_id, title) = games::ps5_param(&param).context("Its param.json has no usable title id")?;
            let dest = base.join(&format!("{title_id}-app0")).map_err(emulator_kit::fail::Error::msg)?;
            emulator_kit::ensure!(!writer.exists(&dest)?, "{title} is already installed");
            let total = installs::tree_size(&entries, &prefix);
            if prefix.is_empty() && folder.is_some_and(|f| job.take(&f, &into)) {
                job.progress(total, total);
            } else {
                installs::write_tree(writer, tree.as_mut(), &entries, &prefix, &into, job)?;
            }
            writer.rename(&into, &dest)?;
            Ok((dest, title))
        });
        Ok(work)
    }
}

#[async_trait]
impl Configurator for Kyty {
    async fn schema(&self, scope: &ConfigScope) -> PluginResult<Value> {
        let schema = match scope {
            ConfigScope::Emulator => schemars::schema_for!(Settings),
            ConfigScope::Game(_) => schemars::schema_for!(GameSettings),
        };
        Ok(serde_json::to_value(schema)?)
    }

    async fn defaults(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Ok(match scope {
            ConfigScope::Emulator => serde_json::to_value(default_settings(self.platform))?,
            ConfigScope::Game(_) => serde_json::to_value(GameSettings::default())?,
        })
    }

    async fn validate(&self, scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        let result = match scope {
            ConfigScope::Emulator => {
                let settings = serde_json::from_value::<Settings>(value.clone())
                    .map_err(|e| PluginError::InvalidInput(e.to_string()))?;
                return release_source(&settings, self.platform).map(|_| ());
            }
            ConfigScope::Game(_) => serde_json::from_value::<GameSettings>(value.clone()).map(|_| ()),
        };
        result.map_err(|e| PluginError::InvalidInput(e.to_string()))
    }
}

#[async_trait]
impl Launcher for Kyty {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
        // `config` is the emulator settings with the game's settings on top; the game's
        // fields are optional, so null ones fall back to the emulator's.
        let mut merged = serde_json::to_value(self.settings().await)?;
        if let (Some(base), Some(over)) = (merged.as_object_mut(), config.as_object()) {
            for (k, v) in over {
                if !v.is_null() {
                    base.insert(k.clone(), v.clone());
                }
            }
        }
        let settings: Settings = serde_json::from_value(merged).unwrap_or_default();
        if settings.own_build {
            let Some(binary) = self.own_binary().await else {
                // A downloaded build is the likely way out: say so first.
                let downloaded = !versions::installed(&self.ctx).await?.is_empty();
                let fix = if downloaded {
                    "turn off Own build on the PS5 page to play with the downloaded build, or map the kyty-build folder, or put kyty_emulator on your PATH"
                } else {
                    "map the kyty-build folder on the PS5 page, or put kyty_emulator on your PATH"
                };
                return Err(PluginError::Precondition(format!("Own build is on, but your build isn't there: {fix}")));
            };
            // It writes its data next to itself; from PATH, into the plugin's folder. Mapped as
            // the program itself, its folder is above the mapping: the host runs it there.
            let cwd = match &binary {
                Location::Folder(file) if file.path.is_root() => None,
                Location::Folder(file) => {
                    let dir = file.path.as_str().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    Some(Location::Folder(HostPath::new(
                        OWN_BUILD,
                        file.root,
                        if dir.is_empty() { RelPath::root() } else { path(dir)? },
                    )))
                }
                _ => {
                    self.ctx.storage().write(&path(format!("{OWN_DATA}/.keep"))?, b"").await?;
                    Some(Location::Data(path(OWN_DATA)?))
                }
            };
            let args = self.args_for(&binary, None, &settings, LaunchArg::path(found.dir)).await;
            return Ok(LaunchSpec { program: binary, args, env: Default::default(), cwd, game_process: None });
        }
        let tag = versions::state(&self.ctx)
            .await
            .current
            .ok_or_else(|| PluginError::Precondition("install KytyPS5 first".into()))?;
        let program: Location = self.binary(&tag).await?.into();
        let args = self.args_for(&program, Some(&tag), &settings, LaunchArg::path(found.dir)).await;
        Ok(LaunchSpec {
            program,
            args,
            env: Default::default(),
            cwd: Some(self.work_dir(&tag).await?.into()),
            game_process: None,
        })
    }
}

#[async_trait]
impl Compatibility for Kyty {
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
impl Slots for Kyty {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        Ok(match (request.slot, request.game.as_deref()) {
            (SlotId::GameBadge, Some(game)) => {
                vec![item("compat", games::compat_badge(&self.status(game).await?, self.platform))]
            }
            (SlotId::GameDetailSection, Some(game)) => {
                let found = self.games.find(self.ctx.folders().as_ref(), game).await?;
                let build = if self.settings().await.own_build {
                    Some(OWN_LABEL.to_owned())
                } else {
                    versions::state(&self.ctx).await.current.map(|t| pretty(&t))
                };
                vec![item(
                    "kyty",
                    SlotContent::Section {
                        title: "KytyPS5".into(),
                        rows: vec![
                            SectionRow { label: "Title id".into(), value: found.title_id },
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
                let settings = self.settings().await;
                let official = versions::state(&self.ctx).await.current.is_some();
                if !settings.own_build
                    && let Some(binary) = self.own_binary().await
                    && let Ok(Some(found)) = self.ctx.programs().find(&binary).await
                {
                    items.push(item(
                        "use-own",
                        SlotContent::Action {
                            label: format!("Use the KytyPS5 at {found}"),
                            action: "use-own".into(),
                            args: Value::Null,
                            style: ActionStyle::Secondary,
                            icon: None,
                        },
                    ));
                }
                if settings.own_build && official {
                    items.push(item(
                        "use-official",
                        SlotContent::Action {
                            label: "Switch to official builds".into(),
                            action: "use-official".into(),
                            args: Value::Null,
                            style: ActionStyle::Primary,
                            icon: None,
                        },
                    ));
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
                    item(
                        "check",
                        SlotContent::Action {
                            label: "Check for new builds".into(),
                            action: "check".into(),
                            args: Value::Null,
                            style: ActionStyle::Secondary,
                            icon: None,
                        },
                    ),
                ]);
                if let Some(previous) = versions::state(&self.ctx).await.previous {
                    items.push(item(
                        "rollback",
                        SlotContent::Action {
                            label: format!("Go back to {}", pretty(&previous)),
                            action: "rollback".into(),
                            args: Value::Null,
                            style: ActionStyle::Secondary,
                            icon: None,
                        },
                    ));
                }
                items
            }
            (SlotId::SettingsPanel, None) => vec![item(
                "settings",
                SlotContent::SettingsPanel { title: "KytyPS5".into(), scope: ConfigScope::Emulator },
            )],
            _ => Vec::new(),
        })
    }

    async fn run_action(&self, action: &str, _game: Option<&str>, _args: &Value) -> PluginResult<()> {
        match action {
            // The release list is kept for hours; this asks GitHub now.
            "check" => {
                let (repo, _) = release_source(&self.settings().await, self.platform)?;
                cache::forget(&self.ctx, &releases_cache(&repo)?).await
            }
            "use-own" => self.set_own_build(true).await,
            // Back to the installed official build, with your saves and caches where it has none.
            "use-official" => {
                let home = self.data_home().await?;
                self.adopt_own_data(&home).await?;
                self.set_own_build(false).await
            }
            "rollback" => {
                let state = versions::state(&self.ctx).await;
                let (Some(current), Some(previous)) = (state.current, state.previous) else {
                    return Err(PluginError::Precondition("no earlier build to go back to".into()));
                };
                let (from, to) = (self.work_dir(&current).await?, self.work_dir(&previous).await?);
                versions::rollback(&self.ctx).await?;
                self.move_data(&from, &to).await
            }
            other => Err(PluginError::NotFound(format!("action {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(args: Vec<LaunchArg>) -> Vec<String> {
        args.into_iter()
            .map(|a| match a {
                LaunchArg::Text(t) => t,
                LaunchArg::Path { .. } => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn help_lists_the_options() {
        let help = "Release, clang-lld, ver = 0.3.0, git = c5016d9, date = 2026.10.22\n\
                    kyty_emulator --game <dir|elf|zar> [options]\n\nOptions:\n\
                    \x20 --game <dir|elf|zar>                 Game directory.\n\
                    \x20 --present-mode <value>               Fifo, Mailbox, or Immediate.\n\
                    \x20 --fullscreen                         Borderless.\n\
                    \x20                                      Implies --vulkan-validation.\n";
        let options = help_options(help);
        assert_eq!(options, ["--fullscreen", "--game", "--present-mode"].map(String::from).into());
    }

    #[test]
    fn a_setting_the_build_does_not_know_is_left_out() {
        // Builds from KytyPS5's main branch have no --video-out-resolution and stop at it.
        let settings = Settings { video_out: VideoOut::FullHd, ..Settings::default() };
        let known: BTreeSet<String> =
            ["--game", "--screen-width", "--screen-height", "--present-mode"].map(String::from).into();
        let args = texts(kyty_args(&settings, "/g/PPSA1".into(), Some(&known)));
        assert_eq!(
            args,
            ["--game", "/g/PPSA1", "--screen-width", "1920", "--screen-height", "1080", "--present-mode", "Mailbox"]
        );
    }

    #[test]
    fn the_command_line_puts_extra_arguments_last() {
        let settings = Settings { amd_cpu: true, extra_args: "--x 'a b'".into(), ..Settings::default() };
        let args: Vec<String> = kyty_args(&settings, "/g/PPSA1".into(), None)
            .into_iter()
            .map(|a| match a {
                LaunchArg::Text(t) => t,
                LaunchArg::Path { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(
            args,
            [
                "--game",
                "/g/PPSA1",
                "--screen-width",
                "1920",
                "--screen-height",
                "1080",
                "--present-mode",
                "Mailbox",
                "--video-out-resolution",
                "Title",
                "--fullscreen",
                "--amd-cpu",
                "--x",
                "a b"
            ]
        );
    }

    #[test]
    fn a_build_s_version_comes_from_its_help_banner() {
        assert_eq!(
            banner_version("KytyPS5 git = 6799ecb, date = 2026.09.29\nusage"),
            Some("2026-09-29 (6799ecb)".into())
        );
        assert_eq!(banner_version("git = 6799ecb\n"), Some("6799ecb".into()));
        assert_eq!(banner_version("usage: kyty_emulator [options]"), None);
        assert_eq!(banner_version(""), None);
    }

    #[test]
    fn assets_and_tags() {
        assert!(asset_matches("KytyPS5-2026-10-12-dad900c-Linux-x86_64.tar.gz", Some(Platform::Linux)));
        assert!(!asset_matches("KytyPS5-2026-10-13-9e41d83-macOS-x86_64.zip", Some(Platform::Linux)));
        assert!(asset_matches("KytyPS5-2026-10-13-9e41d83-macOS-x86_64.zip", Some(Platform::Macos)));
        // The macOS test builds don't carry the tag in their names.
        assert!(asset_matches("KytyPS5-macOS-x86_64.zip", Some(Platform::Macos)));
        assert!(asset_matches("KytyPS5-2026-10-16-d7974dc-Windows-x64.zip", Some(Platform::Windows)));
        assert!(!asset_matches("KytyPS5-2026-10-16-d7974dc-Windows-x64.zip", Some(Platform::Macos)));
        assert_eq!(pretty("KytyPS5-2026-09-29-59a1760"), "2026-09-29 · 59a1760");
    }
}
