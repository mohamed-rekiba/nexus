//! KytyPS5, the PS5 emulator, as a plugin.
//!
//! - Builds: KytyPS5's GitHub releases (Linux `.tar.gz`, macOS `.zip`). The host downloads,
//!   checks the release's SHA-256 and unpacks into `builds/<tag>`; the plugin keeps the
//!   current and the previous build for a rollback. A new build runs once with `--help`
//!   before it becomes current; one that prints no banner is removed. The kit's build manager
//!   runs the flow over [`KytyFlavor`].
//! - Games: PS5 game folders (`sce_sys/param.json`) in the player's game folders.
//! - KytyPS5 writes saves and caches relative to its working folder, which is the build's
//!   folder. When the current build changes, those folders move to the new build, so saves
//!   and shader caches survive updates and rollbacks: see [`data`].
//! - Compatibility: the community list KytyPS5's own launcher uses.
//! - Installs: PS5 packages, exFAT images and unpacked game folders, in [`install`].

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use emulator_kit::builds::{Flavor, OwnBuild, ReleaseSource};
use emulator_kit::console::{CompatList, ConsoleEmulator, ConsoleSpec};
use emulator_kit::games::Console;
use emulator_kit::host::Host;
use emulator_kit::installs::Format;
use emulator_kit::{args, compat};
use nexus_api::schemars::JsonSchema;
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

mod data;
mod install;

pub use data::DATA_DIRS;

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
/// The console: it names its page, and its folder in the games and packages folders.
const CONSOLE: &str = "PS5";
/// Where you keep packages and disc images to install.
pub const PACKAGES: &str = nexus_api::PACKAGES_FOLDER;
/// Where macOS builds keep the emulator binary.
const MAC_BUNDLE: &str = "KytyPS5.app/Contents/MacOS";
/// The folder you map to your own KytyPS5 build.
pub const OWN_BUILD: &str = "kyty-build";
/// Where a build's binary may be in your own build's folder: at its top, in KytyPS5's usual
/// build folders, or in a macOS app bundle.
const OWN_PATHS: [&str; 6] = ["", "_Build/release-linux", "_Build/linux/install", "_Build/linux", "build", MAC_BUNDLE];

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

impl emulator_kit::settings::Settings for Settings {
    type Game = GameSettings;

    fn defaults(platform: Option<Platform>) -> Self {
        default_settings(platform)
    }

    fn own_build(&self) -> bool {
        self.own_build
    }

    fn check(&self, platform: Option<Platform>) -> PluginResult<()> {
        release_source(self, platform).map(|_| ())
    }

    fn migrate(stored: &Value) -> Option<Value> {
        migrate_settings(stored)
    }
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

/// KytyPS5's builds. Its data folders move with the current build: see [`data`].
pub struct KytyFlavor {
    pub platform: Option<Platform>,
}

#[async_trait]
impl Flavor for KytyFlavor {
    type Settings = Settings;
    const NAME: &'static str = "KytyPS5";
    const CONSOLE: &'static str = CONSOLE;
    const OWN_VERSION: bool = true;
    const OFFICIAL_STYLE: ActionStyle = ActionStyle::Primary;

    fn release_source(&self, settings: &Settings) -> PluginResult<ReleaseSource> {
        let (repo, prereleases) = release_source(settings, self.platform)?;
        Ok(ReleaseSource { url: releases_url(&repo), cache: releases_cache(&repo)?, prereleases })
    }

    fn asset_matches(&self, name: &str) -> bool {
        asset_matches(name, self.platform)
    }

    fn archive_format(&self, asset: &str) -> ArchiveFormat {
        if asset.ends_with(".zip") { ArchiveFormat::Zip } else { ArchiveFormat::TarGz }
    }

    fn pretty(&self, tag: &str) -> String {
        pretty(tag)
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        data::binary(storage, tag, self.platform).await
    }

    /// The test run's `--help` must print KytyPS5's banner.
    fn runs(&self, help: &str) -> bool {
        banner_version(help).is_some()
    }

    fn version(&self, help: &str) -> Option<String> {
        banner_version(help)
    }

    fn own_build(&self) -> OwnBuild {
        let name = data::binary_name(self.platform);
        let files = OWN_PATHS.map(|dir| if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") });
        OwnBuild { folder: OWN_BUILD, itself: true, files: files.to_vec(), system: "kyty_emulator" }
    }

    async fn work_dir(&self, storage: &dyn Storage, tag: &str) -> PluginResult<Option<RelPath>> {
        data::work_dir(storage, tag, self.platform).await.map(Some)
    }

    async fn on_switch(&self, host: &Host, from: Option<&str>, to: Option<&str>) -> PluginResult<()> {
        let storage = host.storage.as_ref();
        let from = data::home(storage, from, self.platform).await?;
        let to = data::home(storage, to, self.platform).await?;
        if from != to {
            data::move_data(host, &from, &to).await?;
        }
        Ok(())
    }

    /// Your own build's saves and caches come along, where the official build has none.
    async fn on_leave_own(&self, host: &Host, own: Option<&Location>, to: Option<&str>) -> PluginResult<()> {
        let to = data::home(host.storage.as_ref(), to, self.platform).await?;
        data::adopt_own_data(host, own, &to).await
    }
}

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
        let host = Host::of(&ctx);
        let spec = ConsoleSpec {
            console: Console::Ps5,
            packages: &[Format::Ps5Package, Format::Exfat],
            compat: CompatList { url: COMPATIBILITY, page: LIST_PAGE, parse: compat::kyty },
            keys: KEYS,
            section: "kyty",
            version_row: false,
            own_row: true,
        };
        let emu = Arc::new(ConsoleEmulator::start(KytyFlavor { platform: host.platform }, spec, host).await?);
        let kyty = Arc::new(Kyty { emu: emu.clone(), options: Default::default() });
        Ok(Provided {
            catalog: Some(emu.clone()),
            downloader: Some(emu.builds().clone()),
            installer: Some(emu.builds().clone()),
            configurator: Some(emu.clone()),
            launcher: Some(kyty.clone()),
            compatibility: Some(emu.clone()),
            game_installer: Some(kyty),
            controllers: Some(Arc::new(emu.controls())),
            slots: Some(emu),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::emulator(|| Box::new(KytyPlugin)) }

struct Kyty {
    emu: Arc<ConsoleEmulator<KytyFlavor>>,
    /// The options each downloaded build knows ([`help_options`]), by tag: a build never changes.
    options: std::sync::Mutex<HashMap<String, BTreeSet<String>>>,
}

impl Kyty {
    /// The options a build knows, from its `--help`; `None` when it doesn't answer, and then no
    /// setting is left out. `tag` caches a downloaded build's answer.
    async fn options(&self, binary: &Location, tag: Option<&str>) -> Option<BTreeSet<String>> {
        if let Some(known) = tag.and_then(|t| self.options.lock().ok()?.get(t).cloned()) {
            return Some(known);
        }
        let out = match self.emu.host().programs.output(binary, &["--help".into()], Duration::from_secs(5)).await {
            Ok(out) => out,
            Err(e) => {
                self.emu.host().log.warn(&format!("--help of {binary}: {e}"));
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
                self.emu.host().log.info(&format!("KytyPS5 at {binary} has no {option}: that setting is left out"));
            }
        }
        kyty_args(settings, game, known.as_ref())
    }
}

#[async_trait]
impl Launcher for Kyty {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        // `config` is the emulator settings with the game's settings on top; the game's
        // fields are optional, so null ones fall back to the emulator's.
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let args = self.args_for(&run.program, run.tag.as_deref(), &settings, LaunchArg::path(found.dir)).await;
        Ok(LaunchSpec { program: run.program, args, env: Default::default(), cwd: run.cwd, game_process: None })
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
