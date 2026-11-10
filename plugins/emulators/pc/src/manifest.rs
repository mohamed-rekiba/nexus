//! The PC plugin's part of the game file in a game's folder (`nexus_game.json`, which every
//! game of every console has): the game's title and every file in it that can start it, best
//! first (`starts`). The rest of the file is the core's: the plugin keeps it as it is when it
//! writes. What the install found doesn't change after; what Play starts is the player's
//! Starts setting (see [`GameManifest::start`]).
//!
//! A folder may hold the plugin's own `.nexus-game.json` instead (`version` 1 or 2): the
//! library reads it once, moves it into the game file, and deletes it.

use nexus_api::Platform;
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};

use crate::probe::{self, Kind};

/// The game file in an installed game's folder.
pub const GAME_FILE: &str = "nexus_game.json";
/// The plugin's own file, `version` 1 or 2, that the game file takes the place of.
pub const OLD_GAME_FILE: &str = ".nexus-game.json";
/// The most a game file may be.
pub const MAX_GAME_FILE: u64 = 256 * 1024;
/// The game file's format, when the plugin writes the file alone.
const FILE_FORMAT: u64 = 1;

/// A file that can start the game: its path from the game's folder, and what it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Start {
    pub kind: Kind,
    pub path: String,
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
    /// Read from the game file. 1 and 2 are the plugin's own files.
    pub const VERSION: u32 = 3;

    pub fn new(name: String, starts: Vec<Start>) -> Self {
        GameManifest { version: Self::VERSION, name, starts }
    }

    /// The plugin's part in `bytes`: a game file's `title` and `starts`, else the plugin's own
    /// file, `version` 2, or 1 read as a list of its files. Its `version` says which. `None`
    /// when it is none of these, or nothing in it is a safe path in the game's folder: a game
    /// file the core wrote alone has no `starts`.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        // The plugin's own files have a `version`; the game file has none.
        if let Some(starts) = value.get("starts").filter(|_| value.get("version").is_none()) {
            let title = value.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
            let starts: Vec<Start> = serde_json::from_value(starts.clone()).ok()?;
            let mut manifest = GameManifest { version: Self::VERSION, name: title, starts };
            manifest.starts.retain(|s| safe_relative(&s.path));
            return (!manifest.starts.is_empty()).then_some(manifest);
        }
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

    /// The game file with the plugin's part: `existing`, the file in the folder, with every key
    /// it has kept, its `starts` set, and a `title` and a `format` when it has none. A file that
    /// doesn't read is written anew.
    pub fn game_file(&self, existing: Option<&[u8]>) -> Vec<u8> {
        let mut file = existing
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        file.entry("format").or_insert(Value::from(FILE_FORMAT));
        let titled = file.get("title").and_then(Value::as_str).is_some_and(|t| !t.is_empty());
        if !titled {
            file.insert("title".into(), Value::from(self.name.clone()));
        }
        file.insert("starts".into(), serde_json::to_value(&self.starts).unwrap_or_default());
        let mut bytes = serde_json::to_vec_pretty(&Value::Object(file)).unwrap_or_default();
        bytes.push(b'\n');
        bytes
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
    fn the_game_file_gives_its_title_and_its_starts() {
        let file = br#"{ "format": 1, "title": "Cuphead", "description": "Run and gun.",
            "starts": [{ "kind": "native", "path": "Cuphead.x86_64" }, { "kind": "script", "path": "../x" }] }"#;
        let read = GameManifest::read(file).unwrap();
        assert_eq!(read.name, "Cuphead");
        assert_eq!(read.starts, starts(&[("Cuphead.x86_64", Kind::Native)]).starts, "an unsafe path goes");
        assert_eq!(GameManifest::read(br#"{ "format": 1, "title": "A PS5 game" }"#), None, "no starts: no PC game");
    }

    #[test]
    fn writing_the_game_file_keeps_what_the_rest_of_nexus_wrote_in_it() {
        let core = br#"{ "format": 1, "title": "Cuphead (Store)", "description": "Run and gun.",
            "art": { "cover": "nexus_art/cover.jpg" }, "starts": [{ "kind": "windows", "path": "old.exe" }] }"#;
        let ours = starts(&[("Cuphead.x86_64", Kind::Native)]);

        let written: Value = serde_json::from_slice(&ours.game_file(Some(core))).unwrap();

        assert_eq!(written["starts"], serde_json::json!([{ "kind": "native", "path": "Cuphead.x86_64" }]));
        assert_eq!(written["title"], "Cuphead (Store)", "a title already there stays");
        assert_eq!(written["description"], "Run and gun.");
        assert_eq!(written["art"]["cover"], "nexus_art/cover.jpg");
        assert_eq!(written["format"], 1);

        let alone: Value = serde_json::from_slice(&ours.game_file(None)).unwrap();
        assert_eq!(
            alone,
            serde_json::json!({
                "format": 1,
                "title": "Game",
                "starts": [{ "kind": "native", "path": "Cuphead.x86_64" }]
            })
        );
        let broken: Value = serde_json::from_slice(&ours.game_file(Some(b"{not json"))).unwrap();
        assert_eq!(broken["title"], "Game", "a file that doesn't read is written anew");
    }

    #[test]
    fn the_first_manifest_reads_as_its_files_in_order_and_unsafe_paths_go() {
        let v1 = br#"{ "version": 1, "name": "Game", "launch": "Game.exe", "kind": "windows",
            "alternatives": [{ "launch": "start.sh", "kind": "script" }, { "launch": "../x", "kind": "native" }] }"#;
        let read = GameManifest::read(v1).unwrap();
        assert_eq!(read.version, 1);
        assert_eq!(read.starts, starts(&[("Game.exe", Kind::Windows), ("start.sh", Kind::Script)]).starts);
        let now = starts(&[("start.sh", Kind::Script)]);
        assert_eq!(GameManifest::read(&now.game_file(None)), Some(now), "the game file reads back");
        let v2 = br#"{ "version": 2, "name": "Game", "starts": [{ "kind": "script", "path": "start.sh" }] }"#;
        assert_eq!(GameManifest::read(v2).map(|m| (m.version, m.name)), Some((2, "Game".to_owned())));
        assert_eq!(GameManifest::read(br#"{ "version": 9, "name": "Game", "starts": [] }"#), None);
        assert_eq!(GameManifest::read(br#"{ "version": 2, "name": "Game", "starts": [] }"#), None, "nothing starts");
    }
}
