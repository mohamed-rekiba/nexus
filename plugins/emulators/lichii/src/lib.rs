//! Lichii, the Switch and Switch 2 emulator (a fork of Eden and yuzu), as a plugin.
//!
//! - Builds: Lichii's GitHub releases, all of them pre-releases. On Linux, the bare AppImage
//!   (`lichii-linux-<arch>.AppImage`, not the `-pgo` one): the host checks its SHA-256 and runs
//!   its `--appimage-extract`, so `builds/<tag>/squashfs-root/AppRun`, the Lichii program, runs
//!   without FUSE. The release's tar.gz needs FFmpeg libraries from the system, so it isn't used.
//!   A new build gets no test run: Lichii's `--help` opens its window and never ends.
//! - Games: Switch game files (`.nsp`, `.nsz`, `.xci`, `.xcz`, `.nup`) in the player's game
//!   folders, named by their file names (see [`emulator_kit::games::switch_file`]).
//! - Lichii keeps keys, firmware, saves and settings in its own folder (`~/.local/share/lichii`),
//!   which every build shares. Builds before build-150 used Eden's (`~/.local/share/eden`).
//!   Nexus updates firmware and configured keys when the player asks, in [`firmware`]; they
//!   can also be installed by hand.
//! - Installs: Switch game files from a download, in [`install`].
//! - No compatibility list: Lichii's community reports have no list to download.

use std::sync::Arc;

use emulator_kit::builds::{Flavor, OwnBuild, ReleaseSource};
use emulator_kit::console::{ConsoleEmulator, ConsoleSpec, Packages};
use emulator_kit::games::Console;
use emulator_kit::host::Host;
use emulator_kit::versions;
use nexus_api::schemars::JsonSchema;
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::Value;
use nexus_api::*;

mod config;
mod firmware;
mod install;
mod nsz;

pub use firmware::release_url;

const MANIFEST: &str = include_str!("../plugin.toml");
/// Every release is a pre-release; a build comes out several times a day.
pub const RELEASES: &str = "https://api.github.com/repos/zortos293/Lichii/releases?per_page=30";
/// The console, as its games' platform and the catalog name it: the manifest's `[system]` id.
const CONSOLE: &str = "SWITCH";
/// The folder of Switch games in the games folder.
pub const FOLDER: &str = "Switch";
/// Lichii's settings folder, where its window saves `qt-config.ini`.
pub const CONFIG: &str = "eden-config";
/// Lichii's data folder, which every Lichii build shares: keys, firmware, saves and mods.
pub const DATA: &str = "eden-data";
/// The exit code eden-cli asks to be started again with (`kRestartExitCode` in Lichii).
const RESTART_CODE: i32 = 75;

/// Lichii settings. Its other settings live in its own config file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars", extend("x-order" = ["own_build", "fullscreen", "gpu_split", "airplane_mode"]))]
pub struct Settings {
    #[serde(default = "yes")]
    #[schemars(title = "Full screen")]
    pub fullscreen: bool,
    /// How Lichii splits a Switch 2 game's GPU work between threads (EDEN_NX2_SPLIT). It wins
    /// over Lichii's nx2_switches.txt files.
    #[serde(default)]
    #[schemars(title = "Split the GPU work")]
    pub gpu_split: GpuSplit,
    /// Turns off the internet for games, as on a Switch in airplane mode. Off by default, so
    /// online games can connect. For the games Nexus starts with an official build: your own
    /// build and Lichii's window use the airplane mode set in Lichii, which is on by default.
    #[serde(default)]
    #[schemars(title = "Airplane mode")]
    pub airplane_mode: bool,
    /// Runs the Lichii you built or installed: from the folder you map as lichii-build, or
    /// from your PATH. Installing an official build turns this off. Saves are shared: Lichii
    /// keeps them in its data folder.
    #[serde(default)]
    #[schemars(title = "Use your own build")]
    pub own_build: bool,
    /// The firmware settings of Nexus 0.16.4 and older: Automatic updates and the firmware
    /// release. Read once, so that the Firmware block takes them over
    /// ([`firmware::Firmware::adopt`]), and not used after.
    #[serde(default, rename = "firmware_updates", skip_serializing)]
    #[schemars(skip)]
    pub old_firmware_updates: Option<bool>,
    #[serde(default, rename = "firmware_release", skip_serializing)]
    #[schemars(skip)]
    pub old_firmware_release: Option<String>,
    /// The keys version of Nexus 0.16 and older, which the firmware didn't follow. Read so
    /// that saved settings still load, and not used: the firmware release replaces it.
    #[serde(default, rename = "keys_version", skip_serializing)]
    #[schemars(skip)]
    pub old_keys_version: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen: true,
            gpu_split: GpuSplit::default(),
            airplane_mode: false,
            own_build: false,
            old_firmware_updates: None,
            old_firmware_release: None,
            old_keys_version: None,
        }
    }
}

impl emulator_kit::settings::Settings for Settings {
    type Game = GameSettings;

    fn own_build(&self) -> bool {
        self.own_build
    }
}

/// Per-game settings: anything set here wins over the Lichii settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Full screen")]
    pub fullscreen: Option<bool>,
    #[serde(default)]
    #[schemars(title = "Split the GPU work")]
    pub gpu_split: Option<GpuSplit>,
}

/// Lichii's split of a Switch 2 game's GPU work (EDEN_NX2_SPLIT, NX2_GPU_THREAD_SPLIT.md in
/// Lichii). Lichii's own default, 2, leaves some games black on some PCs (Mario Kart World on
/// AMD with Mesa's radv, build 94), so Nexus starts from one thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub enum GpuSplit {
    /// One thread (works in every game)
    #[default]
    #[serde(rename = "0")]
    Off,
    /// Engine work on a second thread
    #[serde(rename = "1")]
    Methods,
    /// Engine work and macros on a second thread (fastest; black screen in some games)
    #[serde(rename = "2")]
    Macros,
}

impl GpuSplit {
    /// The value of EDEN_NX2_SPLIT.
    pub fn value(self) -> &'static str {
        match self {
            Self::Off => "0",
            Self::Methods => "1",
            Self::Macros => "2",
        }
    }
}

/// The official build for this system and processor: the Linux AppImage, not the `-pgo` one.
pub fn asset_matches(name: &str, platform: Option<Platform>, arch: &str) -> bool {
    match platform {
        Some(Platform::Linux) | None => name == format!("lichii-linux-{arch}.AppImage"),
        _ => false,
    }
}

/// The program a build starts: the AppImage's own program.
fn program(tag: &str) -> PluginResult<RelPath> {
    Ok(versions::folder(tag)?.join("squashfs-root")?.join("AppRun")?)
}

/// The folder you map to your own Lichii build.
pub const OWN_BUILD: &str = "lichii-build";
/// What your own build's program may be called, in its folder.
const OWN_PROGRAMS: [&str; 3] = ["lichii", "lichii.AppImage", "build/bin/lichii"];

/// Lichii's builds. Every build shares Lichii's data folder, so it needs no hooks.
pub struct LichiiFlavor {
    pub platform: Option<Platform>,
}

#[async_trait]
impl Flavor for LichiiFlavor {
    type Settings = Settings;
    const NAME: &'static str = "Lichii";
    const CONSOLE: &'static str = CONSOLE;
    const FOLDER: &'static str = FOLDER;
    const TEST_RUN: bool = false;
    const HAS_WINDOW: bool = true;

    fn release_source(&self, _settings: &Settings) -> PluginResult<ReleaseSource> {
        Ok(ReleaseSource { url: RELEASES.into(), cache: RelPath::new("cache/releases.json")?, prereleases: true })
    }

    fn asset_matches(&self, name: &str) -> bool {
        asset_matches(name, self.platform, std::env::consts::ARCH)
    }

    fn archive_format(&self, _asset: &str) -> ArchiveFormat {
        ArchiveFormat::AppImage
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        let program = program(tag)?;
        if !storage.exists(&program).await? {
            return Err(PluginError::Other(format!("Lichii {tag} has no {program}")));
        }
        Ok(program)
    }

    /// Not used: Lichii's builds get no test run ([`Flavor::TEST_RUN`]).
    fn runs(&self, _help: &str) -> bool {
        true
    }

    fn own_build(&self) -> OwnBuild {
        OwnBuild { folder: OWN_BUILD, itself: false, files: OWN_PROGRAMS.map(String::from).to_vec(), system: "lichii" }
    }
}

pub struct LichiiPlugin;

#[async_trait]
impl Plugin for LichiiPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for LichiiPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let host = Host::of(&ctx);
        let spec = ConsoleSpec {
            console: Console::Switch,
            // A Switch game file, not an update or DLC.
            packages: Packages::Named(|name| emulator_kit::games::switch_file(name).is_some()),
            compat: None,
            // Lichii's keyboard keys aren't listed here: its own window sets them.
            keys: &[],
            section: "lichii",
            version_row: true,
            own_row: false,
        };
        let emu = Arc::new(ConsoleEmulator::start(LichiiFlavor { platform: host.platform }, spec, host).await?);
        let controllers = ctx.services().get::<Controllers>().map(|c| (*c).clone());
        let firmware = Arc::new(firmware::Firmware::new(emu.host().clone(), ctx.events().clone()));
        let saved = emu.builds().settings().load().await;
        if let Err(e) = firmware.adopt(saved.old_firmware_updates, saved.old_firmware_release).await {
            emu.host().log.warn(&format!("the firmware settings of an older Nexus didn't move: {e}"));
        }
        let lichii = Arc::new(Lichii { emu: emu.clone(), controllers, firmware });
        Ok(Provided {
            catalog: Some(emu.clone()),
            downloader: Some(emu.builds().clone()),
            installer: Some(emu.builds().clone()),
            configurator: Some(emu.clone()),
            launcher: Some(lichii.clone()),
            game_installer: Some(lichii.clone()),
            slots: Some(lichii.clone()),
            backup: Some(backup_items()),
            assets: Some(nexus_api::system_art!()),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(LichiiPlugin)) }

/// What a backup holds of Lichii: from Lichii's data folder, the keys, the NAND (firmware and
/// saves), the SD card, amiibo and mods; the settings its window saves; and the firmware
/// settings of the Firmware block.
fn backup_items() -> std::sync::Arc<dyn Backup> {
    let data = |path: &str| BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::new(path).expect("a plain path")));
    emulator_kit::backup::Items::provided(vec![
        BackupItem::new("keys", "Switch keys", data("keys")),
        BackupItem::new("nand", "Switch firmware and saves", data("nand")),
        BackupItem::new("sdmc", "Switch SD card", data("sdmc")),
        BackupItem::new("amiibo", "amiibo", data("amiibo")),
        BackupItem::new("load", "Switch mods", data("load")),
        BackupItem::new(
            "settings",
            "Lichii settings",
            BackupPlace::Folder(HostPath::new(CONFIG, 0, RelPath::new(config::QT_CONFIG).expect("a plain path"))),
        ),
        BackupItem::new(
            "firmware-settings",
            "Switch firmware settings",
            BackupPlace::Data(RelPath::new(firmware::SETTINGS).expect("a plain path")),
        ),
    ])
}

struct Lichii {
    emu: Arc<ConsoleEmulator<LichiiFlavor>>,
    /// The host's controllers; `None` on a host without them.
    controllers: Option<Controllers>,
    firmware: Arc<firmware::Firmware>,
}

/// The console emulator's slots, and the Firmware block on Lichii's panel.
#[async_trait]
impl Slots for Lichii {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let mut items = self.emu.items(request).await?;
        if request.slot == SlotId::EmulatorPanel && request.game.is_none() {
            items.extend(self.firmware.items().await);
        }
        Ok(items)
    }

    async fn run_action(&self, action: &str, game: Option<&str>, args: &Value) -> PluginResult<()> {
        match self.firmware.run_action(action, args).await {
            Some(done) => done,
            None => self.emu.run_action(action, game, args).await,
        }
    }
}

impl Lichii {
    /// The command-line program of the official build `tag`, when it has one.
    async fn cli(&self, tag: &str) -> Option<RelPath> {
        let cli = versions::folder(tag).ok()?.join("squashfs-root/usr/bin/eden-cli").ok()?;
        self.emu.host().storage.exists(&cli).await.ok()?.then_some(cli)
    }
}

#[async_trait]
impl Launcher for Lichii {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        // `config` is the emulator settings with the game's settings on top.
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        // An official build starts the game in its command-line program: no Lichii window
        // before or after, and the controllers from Nexus's settings file. Your own build
        // starts Lichii as it is.
        let cli = match &run.tag {
            Some(tag) => self.cli(tag).await,
            None => None,
        };
        let mut args = Vec::new();
        // eden-cli exits with 75 when a game starts another of its programs (a collection
        // of games, a restart after a setting): it is started again with
        // EDEN_CLI_RESTART_HANDOFF set, and reads where the game was from Lichii's folder.
        let relaunch = cli.is_some().then(|| Relaunch {
            code: RESTART_CODE,
            env: [("EDEN_CLI_RESTART_HANDOFF".to_owned(), "1".to_owned())].into(),
        });
        let program = match cli {
            Some(cli) => {
                let connected = match &self.controllers {
                    Some(controllers) => controllers.connected().await,
                    None => Vec::new(),
                };
                let file = config::write(self.emu.host(), &connected, &settings).await?;
                args.extend(["-c".into(), LaunchArg::path(Location::Data(file))]);
                Location::Data(cli)
            }
            None => run.program,
        };
        // Lichii takes the game file with -g; -f starts it in full screen.
        if settings.fullscreen {
            args.push("-f".into());
        }
        args.extend(["-g".into(), LaunchArg::path(found.file.unwrap_or(found.dir))]);
        let env = [("EDEN_NX2_SPLIT".to_owned(), settings.gpu_split.value().into())].into();
        Ok(LaunchSpec { program, args, env, cwd: run.cwd, game_process: None, relaunch, sandbox: None })
    }

    /// Lichii's window: its controls, keys, firmware and graphics settings. A game started from
    /// it gets the same GPU split as one Nexus starts.
    async fn open_emulator(&self, config: &Value) -> PluginResult<Option<LaunchSpec>> {
        let builds = self.emu.builds();
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let env = [("EDEN_NX2_SPLIT".to_owned(), settings.gpu_split.value().into())].into();
        Ok(Some(LaunchSpec {
            program: run.program,
            args: Vec::new(),
            env,
            cwd: run.cwd,
            game_process: None,
            relaunch: None,
            sandbox: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_appimage_for_this_processor_and_no_other_asset() {
        let linux = Some(Platform::Linux);
        assert!(asset_matches("lichii-linux-x86_64.AppImage", linux, "x86_64"));
        assert!(asset_matches("lichii-linux-aarch64.AppImage", linux, "aarch64"));
        assert!(!asset_matches("lichii-linux-x86_64-pgo.AppImage", linux, "x86_64"));
        assert!(!asset_matches("lichii-linux-x86_64.tar.gz", linux, "x86_64"));
        assert!(!asset_matches("lichii-linux-aarch64.AppImage", linux, "x86_64"));
        assert!(!asset_matches("eglantine-nx2-build-34-linux-x86_64.AppImage", linux, "x86_64"));
        assert!(!asset_matches("lichii-windows-x86_64.zip", Some(Platform::Windows), "x86_64"));
    }
}
