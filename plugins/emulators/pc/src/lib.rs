//! PC games on Linux, Windows and macOS.
//!
//! - Install ([`install`]): from a download folder the host has unpacked. The manifest file in
//!   the game's folder lists every file that can start it ([`manifest`]).
//! - Library ([`library`]): the games in `PC/` of the player's game folders: installed ones,
//!   and folders copied in by hand, which get their manifest file the first time.
//! - Settings ([`settings`]): Starts, the file Play starts; on Linux a Resolution.
//! - Play ([`launch`]): the file Starts chose, as it is; a Windows program off Windows through
//!   umu-launcher (Proton) or Wine ([`runner`]). A release this system can't start installs
//!   anyway, and its Game Hub says why.

mod install;
mod launch;
mod library;
pub mod manifest;
pub mod probe;
mod runner;
mod settings;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nexus_api::*;

pub use crate::manifest::{GAME_FILE, GameManifest, Start};

const MANIFEST: &str = include_str!("../plugin.toml");
pub const GAMES: &str = nexus_api::GAMES_FOLDER;
pub const PACKAGES: &str = nexus_api::PACKAGES_FOLDER;
/// Where installs go in the games folder.
pub const CONSOLE_FOLDER: &str = "PC";
/// Where a GOG installer's game says its name, on its first line.
const GOG_INFO: &str = "data/noarch/gameinfo";
/// Where a portable Wine wrapper (Kron4ek's) says its game's name, on its sixth line: beside
/// its start script. Only the game's name comes from these files.
const WRAPPER_INFO: &str = "game_info/game_info.txt";
const WRAPPER_NAME_LINE: usize = 5;

pub struct PcPlugin;

#[async_trait]
impl Plugin for PcPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for PcPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let host = ctx.services().get::<HostInfo>();
        let platform = host.as_ref().and_then(|h| h.platform).or_else(Platform::current).unwrap_or(Platform::Linux);
        let runs_32bit = host.is_some_and(|h| h.runs_32bit);
        let pc = Arc::new(Pc { ctx, platform, runs_32bit, not_games: Mutex::default() });
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

inventory::submit! { PluginFactory::code(|| Box::new(PcPlugin)) }

struct Pc {
    ctx: PluginContext,
    platform: Platform,
    /// The host runs 32-bit Linux programs.
    runs_32bit: bool,
    /// Folders in `PC/` with nothing that starts a game (`<root>/<folder>`): not probed again
    /// this run.
    not_games: Mutex<HashSet<String>>,
}
