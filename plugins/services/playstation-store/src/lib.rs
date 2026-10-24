//! The PlayStation Store's descriptions of PS4 and PS5 games, by title id: the art the
//! console shows (square tile, Game Hub art, portrait cover, logo, screenshots), the star
//! rating, genres, publisher, release date, age rating and description.
//!
//! It asks Sony's public catalog API in English first: the game's own store (from its content
//! id) when that is the US, Europe or Asia store, then the others, and Japan's store last. Answers are kept in `catalog.json` in the plugin's
//! folder: 14 days for a game found, 3 for one the store doesn't have.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

const API: &str =
    "https://m.np.playstation.com/api/catalog/v2/titles/{tid}_00/concepts/?age=99&country={c}&language={l}";
/// How long an answer stays good, in seconds.
const FRESH: u64 = 14 * 86_400;
const MISS_FRESH: u64 = 3 * 86_400;
const CACHE: &str = "catalog.json";

pub struct PlayStationStore;

#[async_trait]
impl Plugin for PlayStationStore {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl ServicePlugin for PlayStationStore {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let cache = match ctx.storage().read(&RelPath::new(CACHE)?).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        Ok(Provided { metadata: Some(Arc::new(Store { ctx, cache: Mutex::new(cache) })), ..Provided::default() })
    }
}

inventory::submit! { PluginFactory::service(|| Box::new(PlayStationStore)) }

#[derive(Clone, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct Entry {
    /// When it was asked, in seconds since 1970.
    at: u64,
    found: Option<GameMetadata>,
    /// The language asked first. An answer asked in another order is asked again.
    #[serde(default)]
    language: String,
    /// What this plugin read from the answer; an older reading is asked again.
    #[serde(default)]
    reading: u32,
}

/// Raised when `read_concept` reads more of an answer, so cached answers are read again.
const READING: u32 = 2;

struct Store {
    ctx: PluginContext,
    cache: Mutex<HashMap<String, Entry>>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A PS5 (`PPSA`) or PS4 (`CUSA`) title id: four letters and five digits.
pub fn valid_title_id(tid: &str) -> bool {
    tid.len() == 9
        && (tid.starts_with("PPSA") || tid.starts_with("CUSA"))
        && tid[4..].bytes().all(|b| b.is_ascii_digit())
}

/// The store regions to ask, as (country, language): the game's own English store first, then
/// the other English ones, then Japan's. The content id gives the game's store, else its
/// region (`EUR`, `ASIA`). A Japanese copy is described in English when the English stores
/// have it.
pub fn regions(content_id: &str, region: &str) -> Vec<(&'static str, &'static str)> {
    let region = region.to_ascii_uppercase();
    let own = match content_id.get(..2) {
        Some("EP") => "GB",
        Some("HP" | "KP") => "HK",
        Some(_) => "US",
        None if region.starts_with("EU") || region == "UK" || region == "PAL" => "GB",
        None if region.starts_with("ASIA") || region == "HK" || region == "KOR" => "HK",
        None => "US",
    };
    let english = [("US", "en-US"), ("GB", "en-GB"), ("HK", "en-HK")];
    let mut order: Vec<_> = english.iter().copied().filter(|(c, _)| *c == own).collect();
    order.extend(english.iter().copied().filter(|(c, _)| *c != own));
    order.push(("JP", "ja-JP"));
    order
}

/// The image service resizes on request: `w` is the width wanted.
fn sized(url: &str, w: u32) -> ImageSource {
    if url.contains("image.api.playstation.com") && !url.contains('?') {
        ImageSource::Url(format!("{url}?w={w}"))
    } else {
        ImageSource::Url(url.to_owned())
    }
}

/// The store's text without its markup: `<br>` as a new line, other tags dropped.
fn plain(text: &str) -> String {
    let text = text.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Japanese text, from a game only Japan's store has: dropped, so the game's own words show
/// instead. The art stays.
fn without_japanese_text(m: &mut GameMetadata) {
    let japanese = |s: &str| s.chars().any(|c| matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x9fff | 0xac00..=0xd7af));
    if m.genres.iter().any(|g| japanese(g)) {
        m.genres.clear();
    }
    for text in [&mut m.description, &mut m.tagline, &mut m.publisher] {
        if japanese(text) {
            text.clear();
        }
    }
}

/// One concept of the catalog's answer, as the old launcher read it.
pub fn read_concept(c: &Value, language: &str) -> GameMetadata {
    let mut images: HashMap<&str, &str> = HashMap::new();
    let mut shots = Vec::new();
    for i in c["media"]["images"].as_array().into_iter().flatten() {
        let (kind, url) = (i["type"].as_str().unwrap_or(""), i["url"].as_str().unwrap_or(""));
        if url.is_empty() {
            continue;
        }
        if kind == "SCREENSHOT" {
            shots.push(sized(url, 1280));
        } else {
            images.entry(kind).or_insert(url);
        }
    }
    let image = |kind: &str, w: u32| images.get(kind).map(|u| sized(u, w));
    let trailer = c["media"]["videos"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|v| v["url"].as_str())
        .filter(|u| u.starts_with("https://"))
        .unwrap_or_default()
        .to_owned();
    let description = |kind: &str| {
        c["descriptions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|d| d["type"] == kind)
            .and_then(|d| d["desc"].as_str())
            .map(plain)
            .unwrap_or_default()
    };
    let number = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64());
    let rating = {
        let sr = &c["starRating"];
        let total = number(&sr["total"]).unwrap_or(0.0) as u64;
        number(&sr["score"]).filter(|_| total > 0).map(|score| Rating { score: score as f32, total })
    };
    let long = description("LONG");
    let short = description("SHORT");
    let mut m = GameMetadata {
        title: c["nameEn"].as_str().or(c["name"].as_str()).unwrap_or("").to_owned(),
        description: if long.is_empty() { short.clone() } else { long },
        // A short text in capitals only, or a few words, is a banner, not a line about the game.
        tagline: if short.chars().count() > 30 && short.chars().any(char::is_lowercase) {
            short
        } else {
            String::new()
        },
        publisher: c["publisherName"].as_str().unwrap_or("").to_owned(),
        genres: c["localizedGenres"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g["value"].as_str().map(str::to_owned))
            .collect(),
        release: c["releaseDate"]["date"].as_str().unwrap_or("").chars().take(10).collect(),
        rating,
        age_rating: c["contentRating"]["description"].as_str().unwrap_or("").to_owned(),
        icon: image("MASTER", 440),
        cover: image("PORTRAIT_BANNER", 760),
        background: image("GAMEHUB_COVER_ART", 1920).or_else(|| image("BACKGROUND_LAYER_ART", 1920)),
        logo: image("LOGO", 900),
        screenshots: shots.into_iter().take(12).collect(),
        trailer,
        store_url: c["id"]
            .as_u64()
            .map(|id| id.to_string())
            .or_else(|| c["id"].as_str().map(str::to_owned))
            .map(|id| format!("https://store.playstation.com/{}/concept/{id}", language.to_lowercase()))
            .unwrap_or_default(),
    };
    without_japanese_text(&mut m);
    m
}

impl Store {
    async fn ask(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>> {
        let mut last_error = None;
        for (country, language) in regions(&game.content_id, &game.region) {
            let url = API.replace("{tid}", &game.title_id).replace("{c}", country).replace("{l}", language);
            let answer = match self.ctx.http().send(HttpRequest::get(url)).await {
                Ok(answer) if answer.status == 200 => answer,
                Ok(_) => continue,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };
            let Ok(Value::Array(concepts)) = serde_json::from_slice::<Value>(&answer.body) else { continue };
            let concept = concepts.iter().find(|c| c["type"] == "GAME").or(concepts.first());
            if let Some(concept) = concept {
                return Ok(Some(read_concept(concept, language)));
            }
        }
        // No region answered at all: the network is down, not the game missing.
        match last_error {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }

    async fn save(&self) {
        let bytes = {
            let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            serde_json::to_vec(&*cache).unwrap_or_default()
        };
        if let Ok(path) = RelPath::new(CACHE) {
            let _ = self.ctx.storage().write(&path, &bytes).await;
        }
    }
}

#[async_trait]
impl Metadata for Store {
    async fn describe(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>> {
        if !valid_title_id(&game.title_id) {
            return Ok(None);
        }
        let cached = self.cache.lock().unwrap_or_else(|p| p.into_inner()).get(&game.title_id).cloned();
        let language = regions(&game.content_id, &game.region)[0].1;
        if let Some(entry) = cached.as_ref().filter(|e| e.language == language && e.reading == READING) {
            let fresh = if entry.found.is_some() { FRESH } else { MISS_FRESH };
            if now().saturating_sub(entry.at) < fresh {
                return Ok(entry.found.clone());
            }
        }
        let found = match self.ask(game).await {
            Ok(found) => found,
            // Offline: what it knew before, if anything.
            Err(e) => return cached.and_then(|c| c.found).map(Some).ok_or(e),
        };
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).insert(
            game.title_id.clone(),
            Entry { at: now(), found: found.clone(), language: language.to_owned(), reading: READING },
        );
        self.save().await;
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_concept_reads_as_the_old_launcher_read_it() {
        let concept = serde_json::json!({
            "id": 10001234, "type": "GAME", "name": "Contra: Operation Galuga", "publisherName": "KONAMI",
            "localizedGenres": [{"value": "Shooter"}, {"value": "Arcade"}],
            "releaseDate": {"date": "2024-03-12T00:00:00Z"},
            "starRating": {"score": "4.44", "total": "3227"},
            "contentRating": {"description": "PEGI 12"},
            "descriptions": [
                {"type": "SHORT", "desc": "Reloaded. Reimagined. Reborn. The run-'n'-gun series is back!"},
                {"type": "LONG", "desc": "The classic returns.<br>Two players."}
            ],
            "media": {"images": [
                {"type": "MASTER", "url": "https://image.api.playstation.com/m.png"},
                {"type": "LOGO", "url": "https://image.api.playstation.com/l.png"},
                {"type": "GAMEHUB_COVER_ART", "url": "https://image.api.playstation.com/h.png"},
                {"type": "SCREENSHOT", "url": "https://image.api.playstation.com/s1.png"}
            ], "videos": [{"type": "TRAILER", "url": "https://vulcan.dl.playstation.net/trailer.mp4"}]}
        });
        let m = read_concept(&concept, "en-GB");
        assert_eq!(m.title, "Contra: Operation Galuga");
        assert_eq!(m.publisher, "KONAMI");
        assert_eq!(m.genres, ["Shooter", "Arcade"]);
        assert_eq!(m.release, "2024-03-12");
        assert_eq!(m.rating, Some(Rating { score: 4.44, total: 3227 }));
        assert_eq!(m.age_rating, "PEGI 12");
        assert_eq!(m.description, "The classic returns.\nTwo players.");
        assert_eq!(m.tagline, "Reloaded. Reimagined. Reborn. The run-'n'-gun series is back!");
        assert_eq!(m.trailer, "https://vulcan.dl.playstation.net/trailer.mp4");
        assert_eq!(m.icon, Some(ImageSource::Url("https://image.api.playstation.com/m.png?w=440".into())));
        assert_eq!(m.background, Some(ImageSource::Url("https://image.api.playstation.com/h.png?w=1920".into())));
        assert_eq!(m.screenshots.len(), 1);
        assert_eq!(m.store_url, "https://store.playstation.com/en-gb/concept/10001234");
    }

    #[test]
    fn a_banner_is_no_tagline_and_japanese_text_is_dropped() {
        let concept = serde_json::json!({
            "name": "Sonic Racing: CrossWorlds", "publisherName": "セガ",
            "localizedGenres": [{"value": "レース"}],
            "descriptions": [{"type": "SHORT", "desc": "Drift Across Dimensions!"}, {"type": "LONG", "desc": "同じレースは二度とない"}],
            "media": {"images": [{"type": "MASTER", "url": "https://image.api.playstation.com/m.png"}]}
        });
        let m = read_concept(&concept, "ja-JP");
        assert_eq!(m.tagline, "", "24 characters: a banner");
        assert_eq!(m.description, "");
        assert_eq!(m.publisher, "");
        assert!(m.genres.is_empty());
        assert!(m.icon.is_some(), "the art stays");
    }

    #[test]
    fn the_game_s_own_english_store_is_asked_first_and_japan_s_last() {
        assert_eq!(regions("EP0002-PPSA02433_00-X", "")[0], ("GB", "en-GB"));
        assert_eq!(regions("UP0001-CUSA00001_00-X", "")[0], ("US", "en-US"));
        assert_eq!(regions("", "")[0], ("US", "en-US"));
        let japanese = regions("JP0001-PPSA00001_00-X", "");
        assert_eq!(japanese, [("US", "en-US"), ("GB", "en-GB"), ("HK", "en-HK"), ("JP", "ja-JP")]);
    }

    #[test]
    fn without_a_content_id_the_region_picks_the_store() {
        assert_eq!(regions("", "EUR")[0], ("GB", "en-GB"));
        assert_eq!(regions("", "asia")[0], ("HK", "en-HK"));
        assert_eq!(regions("", "USA")[0], ("US", "en-US"));
        assert_eq!(regions("UP0001-CUSA00001_00-X", "EUR")[0], ("US", "en-US"), "the content id wins");
    }

    #[test]
    fn only_ps4_and_ps5_title_ids_are_asked_about() {
        assert!(valid_title_id("PPSA15368"));
        assert!(valid_title_id("CUSA00001"));
        assert!(!valid_title_id("PPSA1536"));
        assert!(!valid_title_id("G0"));
    }
}
