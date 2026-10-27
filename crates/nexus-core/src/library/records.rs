//! The library's saved records: the games the player hid from the Catalog
//! (`app.hidden_games`), the offers' covers kept for installed games (`app.game_art`), and the
//! player's own results (`app.results`, with `app.result_builds` and `app.result_skipped`, the
//! builds each answer was for).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nexus_api::{GameKey, ImageSource, PluginId};
use serde_json::Value;

use crate::config::ConfigService;

const HIDDEN_GAMES: [&str; 2] = ["app", "hidden_games"];
const GAME_ART: [&str; 2] = ["app", "game_art"];
const RESULTS: &str = "results";
const RESULT_BUILDS: &str = "result_builds";
const RESULT_SKIPPED: &str = "result_skipped";

/// The results a player can give a game, worst last.
pub(crate) const RESULT_LEVELS: [&str; 4] = ["ingame", "menus", "boots", "nothing"];

/// A plugin's image: the plugin that has it, and where.
pub(crate) type PluginImage = (PluginId, ImageSource);

/// The subsystem's part of the config. It reads the config each time, so a value set there
/// shows at once.
pub(crate) struct Records {
    config: Arc<ConfigService>,
}

impl Records {
    pub fn new(config: Arc<ConfigService>) -> Self {
        Self { config }
    }

    /// The games the player hid, by their key as text.
    pub fn hidden(&self) -> HashSet<String> {
        let list = self.config.get(&HIDDEN_GAMES);
        list.as_array().map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
    }

    /// Hides `game`, or shows it again. The list is saved sorted, and goes when it is empty.
    pub fn set_hidden(&self, game: &GameKey, hidden: bool) -> std::io::Result<()> {
        let key = game.to_string();
        let mut list: Vec<String> = self.hidden().into_iter().filter(|k| *k != key).collect();
        if hidden {
            list.push(key);
        }
        list.sort();
        if list.is_empty() {
            self.config.remove(&HIDDEN_GAMES)
        } else {
            self.config.set(&HIDDEN_GAMES, serde_json::json!(list))
        }
    }

    /// The offers' covers kept for installed games. One that doesn't read is left out.
    pub fn kept_art(&self) -> HashMap<GameKey, PluginImage> {
        let kept = self.config.get(&GAME_ART);
        let Some(kept) = kept.as_object() else { return HashMap::new() };
        kept.iter()
            .filter_map(|(game, art)| {
                let plugin = PluginId::new(art.get("plugin")?.as_str()?).ok()?;
                let image = serde_json::from_value(art.get("image")?.clone()).ok()?;
                Some((game.parse().ok()?, (plugin, image)))
            })
            .collect()
    }

    /// Keeps these covers, in place of the ones kept before.
    pub fn keep_art<'a>(&self, art: impl IntoIterator<Item = (&'a GameKey, &'a PluginImage)>) {
        let kept: serde_json::Map<String, Value> = art
            .into_iter()
            .map(|(game, (plugin, image))| (game.to_string(), serde_json::json!({ "plugin": plugin, "image": image })))
            .collect();
        if let Err(e) = self.config.set(&GAME_ART, Value::Object(kept)) {
            tracing::warn!(error = %e, "could not keep the installed games' art");
        }
    }

    /// The player's own results, by the game's key as text: `ingame`, `menus`, `boots` or
    /// `nothing`.
    pub fn results(&self) -> HashMap<String, String> {
        let results = self.config.get(&["app", RESULTS]);
        let Some(results) = results.as_object() else { return HashMap::new() };
        results.iter().filter_map(|(game, level)| Some((game.clone(), level.as_str()?.to_owned()))).collect()
    }

    /// Saves the player's answer for `game` on `build`: a result, or `None` for "Not now".
    pub fn answer(&self, game: &GameKey, level: Option<&str>, build: Option<&str>) -> std::io::Result<()> {
        let game = game.to_string();
        let set = |key: &str, value: Value| self.config.set(&["app", key, &game], value);
        let build = build.map(Value::from).unwrap_or(Value::Null);
        match level {
            Some(level) => {
                set(RESULTS, Value::from(level))?;
                set(RESULT_BUILDS, build)
            }
            None => set(RESULT_SKIPPED, build),
        }
    }

    /// Whether the player answered for `game` on `build`, or said "Not now" to it. A game with
    /// no build is always asked.
    pub fn answered(&self, game: &GameKey, build: Option<&str>) -> bool {
        let Some(build) = build else { return false };
        let game = game.to_string();
        [RESULT_BUILDS, RESULT_SKIPPED].iter().any(|key| self.config.get(&["app", key, &game]).as_str() == Some(build))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Records {
        Records::new(Arc::new(ConfigService::in_memory()))
    }

    fn key(text: &str) -> GameKey {
        text.parse().unwrap()
    }

    #[test]
    fn the_hidden_games_save_sorted_and_the_list_goes_when_empty() {
        let r = records();
        r.set_hidden(&key("cat:b"), true).unwrap();
        r.set_hidden(&key("cat:a"), true).unwrap();
        assert_eq!(r.config.get(&HIDDEN_GAMES), serde_json::json!(["cat:a", "cat:b"]));
        assert_eq!(r.hidden(), HashSet::from(["cat:a".to_owned(), "cat:b".to_owned()]));

        r.set_hidden(&key("cat:a"), false).unwrap();
        r.set_hidden(&key("cat:b"), false).unwrap();
        assert_eq!(r.config.get(&HIDDEN_GAMES), Value::Null, "no empty list is left");
    }

    #[test]
    fn kept_art_reads_back_as_it_was_kept() {
        let r = records();
        let art = HashMap::from([(key("emu:g"), (PluginId::new("cat").unwrap(), ImageSource::Url("u".into())))]);
        r.keep_art(&art);
        assert_eq!(
            r.config.get(&GAME_ART),
            serde_json::json!({ "emu:g": { "plugin": "cat", "image": { "type": "url", "value": "u" } } }),
            "the saved format"
        );
        assert_eq!(r.kept_art(), art);
    }

    #[test]
    fn an_answer_is_for_its_build_and_not_now_counts_as_one() {
        let r = records();
        let game = key("emu:g");
        assert!(!r.answered(&game, None), "a game with no build is always asked");
        r.answer(&game, Some("ingame"), Some("1.0")).unwrap();
        assert_eq!(r.results().get("emu:g").map(String::as_str), Some("ingame"));
        assert!(r.answered(&game, Some("1.0")));
        assert!(!r.answered(&game, Some("1.1")), "a new build asks again");

        r.answer(&game, None, Some("1.1")).unwrap();
        assert!(r.answered(&game, Some("1.1")), "\"Not now\" for that build");
        assert_eq!(r.results().get("emu:g").map(String::as_str), Some("ingame"), "the result stays");
    }
}
