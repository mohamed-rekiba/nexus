//! The manifest file an install leaves in a game's folder: the game's name and every file in it
//! that can start it, best first. It holds what the install found, and doesn't change after;
//! what Play starts is the player's Starts setting (see [`GameManifest::start`]).

use nexus_api::Platform;
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};

use crate::probe::{self, Kind};

/// The manifest file in an installed game's folder.
pub const GAME_FILE: &str = ".nexus-game.json";
/// The most a manifest file may be.
pub const MAX_GAME_FILE: u64 = 64 * 1024;

/// A file that can start the game: its path from the game's folder, and what it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Start {
    pub path: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct GameManifest {
    pub version: u32,
    pub name: String,
    /// Every file that can start the game, best first (see [`probe::rank`]).
    pub starts: Vec<Start>,
}

/// The first manifest: what Play started, and up to 8 other choices.
#[derive(Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct V1 {
    name: String,
    launch: String,
    kind: Kind,
    #[serde(default)]
    alternatives: Vec<V1Start>,
}

#[derive(Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct V1Start {
    launch: String,
    kind: Kind,
}

impl GameManifest {
    pub const VERSION: u32 = 2;

    pub fn new(name: String, starts: Vec<Start>) -> Self {
        GameManifest { version: Self::VERSION, name, starts }
    }

    /// The manifest in `bytes`: this version, or the first one read as a list of its files (its
    /// `version` stays 1). `None` when it isn't one, or nothing in it is a safe path in the
    /// game's folder.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let mut manifest = match value.get("version").and_then(Value::as_u64)? {
            1 => {
                let v1: V1 = serde_json::from_value(value).ok()?;
                let first = Start { path: v1.launch, kind: v1.kind };
                let others = v1.alternatives.into_iter().map(|a| Start { path: a.launch, kind: a.kind });
                GameManifest { version: 1, name: v1.name, starts: std::iter::once(first).chain(others).collect() }
            }
            2 => serde_json::from_value(value).ok()?,
            _ => return None,
        };
        manifest.starts.retain(|s| safe_relative(&s.path));
        (!manifest.starts.is_empty()).then_some(manifest)
    }

    /// What Play starts on `platform` without a Starts setting: the first start script by its
    /// name (`start.sh`) that runs here, else the first file that runs here, else the first.
    pub fn default_start(&self, platform: Platform) -> &Start {
        let runs = |s: &&Start| s.kind.runs_on(platform);
        self.starts
            .iter()
            .filter(runs)
            .find(|s| s.kind == Kind::Script && probe::start_script(&s.path))
            .or_else(|| self.starts.iter().find(runs))
            .unwrap_or(&self.starts[0])
    }

    /// Whether `path` is one of its files.
    pub fn has(&self, path: &str) -> bool {
        self.starts.iter().any(|s| s.path == path)
    }

    /// What Play starts: `chosen` (the Starts setting) when it is one of its files, else the
    /// default.
    pub fn start(&self, chosen: Option<&str>, platform: Platform) -> &Start {
        chosen.and_then(|c| self.starts.iter().find(|s| s.path == c)).unwrap_or_else(|| self.default_start(platform))
    }
}

/// A path inside its folder: no root, no `..`, no empty part.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(list: &[(&str, Kind)]) -> GameManifest {
        GameManifest::new("Game".into(), list.iter().map(|(p, k)| Start { path: (*p).into(), kind: *k }).collect())
    }

    #[test]
    fn the_default_is_the_start_script_that_runs_here_else_the_first_file_that_runs_here() {
        let game = starts(&[("Game.exe", Kind::Windows), ("start.sh", Kind::Script), ("Game.app/x", Kind::MacApp)]);
        assert_eq!(game.default_start(Platform::Linux).path, "start.sh");
        assert_eq!(game.default_start(Platform::Macos).path, "Game.exe", "a script can't run on a Mac");
        assert_eq!(game.default_start(Platform::Windows).path, "Game.exe");
        let linux_only = starts(&[("game.x86_64", Kind::Native)]);
        assert_eq!(linux_only.default_start(Platform::Windows).path, "game.x86_64", "nothing runs here: the first");
        assert_eq!(game.start(Some("Game.app/x"), Platform::Linux).path, "Game.app/x", "the setting, as it is");
        assert_eq!(game.start(Some("gone.sh"), Platform::Linux).path, "start.sh", "a file it doesn't have");
    }

    #[test]
    fn the_first_manifest_reads_as_its_files_in_order_and_unsafe_paths_go() {
        let v1 = br#"{ "version": 1, "name": "Game", "launch": "Game.exe", "kind": "windows",
            "alternatives": [{ "launch": "start.sh", "kind": "script" }, { "launch": "../x", "kind": "native" }] }"#;
        let read = GameManifest::read(v1).unwrap();
        assert_eq!(read.version, 1);
        assert_eq!(read.starts, starts(&[("Game.exe", Kind::Windows), ("start.sh", Kind::Script)]).starts);
        let now = starts(&[("start.sh", Kind::Script)]);
        let written = serde_json::to_vec(&now).unwrap();
        assert_eq!(GameManifest::read(&written), Some(now), "this version reads back");
        assert_eq!(GameManifest::read(br#"{ "version": 3, "name": "Game", "starts": [] }"#), None);
        assert_eq!(GameManifest::read(br#"{ "version": 2, "name": "Game", "starts": [] }"#), None, "nothing starts");
    }
}
