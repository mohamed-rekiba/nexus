//! Steam's store, for a Steam game's details: its words, genres, developer, release date, age
//! rating, screenshots and trailer, from Steam's public game details
//! (`store.steampowered.com/api/appdetails`), which need no key. Each game's answer is kept a
//! week in the plugin's storage, and an older one is used when Steam can't be reached: Steam
//! limits how often it is asked. The art stays Steam's cached files first (see
//! `Steam::art`).

use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::serde_json::{self, Value};
use nexus_api::*;

use crate::{PLATFORM, Steam};

/// Steam's public game details.
const DETAILS: &str = "https://store.steampowered.com/api/appdetails";
/// How long an answer is kept before Steam is asked again.
const FRESH: u64 = 7 * 24 * 60 * 60;
/// The most screenshots a Game Hub gets.
const MAX_SCREENSHOTS: usize = 12;

/// A kept answer: when it was read, and what it said (`None`: Steam has no such game).
#[derive(Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct Kept {
    at: u64,
    found: Option<GameMetadata>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Steam's date, in English as the store writes it ("Oct 10, 2007" or "10 Oct, 2007"), as
/// `YYYY-MM-DD`; empty when it isn't one ("Coming soon", "Q4 2026").
fn date(text: &str) -> String {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let words: Vec<&str> = text.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let [a, b, year] = words[..] else { return String::new() };
    let month = |w: &str| MONTHS.iter().position(|m| w.to_ascii_lowercase().starts_with(m)).map(|i| i + 1);
    let (month, day) = match (month(a), month(b)) {
        (Some(m), None) => (m, b),
        (None, Some(m)) => (m, a),
        _ => return String::new(),
    };
    match (day.parse::<u32>(), year.parse::<u32>()) {
        (Ok(day), Ok(year)) if (1..=31).contains(&day) && year > 1970 => format!("{year:04}-{month:02}-{day:02}"),
        _ => String::new(),
    }
}

/// Text from Steam's store with its few HTML entities read back.
fn plain(text: &str) -> String {
    text.replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&").trim().to_owned()
}

/// What Steam's game details (`data` of one app) say, as a game's metadata.
pub(crate) fn metadata(app: u32, data: &Value) -> GameMetadata {
    let text = |key: &str| data.get(key).and_then(Value::as_str).map(plain).unwrap_or_default();
    let names = |key: &str| -> Vec<String> {
        let list = data.get(key).and_then(Value::as_array).into_iter().flatten();
        list.filter_map(|v| v.as_str().or_else(|| v.get("description").and_then(Value::as_str)).map(plain)).collect()
    };
    let urls = |key: &str, field: &str| -> Vec<String> {
        let list = data.get(key).and_then(Value::as_array).into_iter().flatten();
        list.filter_map(|v| v.get(field).and_then(Value::as_str))
            .filter(|u| u.starts_with("https://"))
            .map(str::to_owned)
            .collect()
    };
    let age = data.get("required_age").and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(0);
    let release = data.get("release_date").and_then(|r| r.get("date")).and_then(Value::as_str).map(date);
    GameMetadata {
        title: text("name"),
        description: text("short_description"),
        publisher: names("developers")
            .into_iter()
            .next()
            .or_else(|| names("publishers").into_iter().next())
            .unwrap_or_default(),
        genres: names("genres"),
        release: release.unwrap_or_default(),
        age_rating: if age > 0 { format!("{age}+") } else { String::new() },
        screenshots: urls("screenshots", "path_full").into_iter().take(MAX_SCREENSHOTS).map(ImageSource::Url).collect(),
        // Steam's trailers are streams (HLS), which the trailer player plays.
        trailer: urls("movies", "hls_h264").into_iter().next().unwrap_or_default(),
        store_url: format!("https://store.steampowered.com/app/{app}/"),
        ..GameMetadata::default()
    }
}

impl Steam {
    /// Steam's store's details of `app`: kept a week, else asked; the kept one when Steam
    /// can't be reached.
    pub(crate) async fn store_details(&self, app: u32) -> PluginResult<Option<GameMetadata>> {
        let file = RelPath::new(format!("store/{app}.json"))?;
        let storage = self.ctx.storage();
        let kept: Option<Kept> = storage.read(&file).await.ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if let Some(kept) = kept.as_ref().filter(|k| now().saturating_sub(k.at) < FRESH) {
            return Ok(kept.found.clone());
        }
        let found = match self.ask_store(app).await {
            Ok(found) => found,
            Err(e) => return kept.map(|k| k.found).ok_or(e),
        };
        let bytes = serde_json::to_vec(&Kept { at: now(), found: found.clone() })?;
        if let Err(e) = storage.write(&file, &bytes).await {
            self.ctx.log().warn(&format!("Steam's details of {app} weren't kept: {e}"));
        }
        Ok(found)
    }

    async fn ask_store(&self, app: u32) -> PluginResult<Option<GameMetadata>> {
        let url = format!("{DETAILS}?appids={app}&l=english");
        let answer = self.ctx.http().send(HttpRequest::get(url)).await?;
        let body: Value = answer.json()?;
        let entry = body.get(app.to_string()).cloned().unwrap_or(Value::Null);
        if entry.get("success").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
        Ok(entry.get("data").map(|data| metadata(app, data)))
    }
}

#[async_trait]
impl Metadata for Steam {
    /// A Steam game, by its app id (its title id): Steam's store's details.
    async fn describe(&self, game: &GameRef) -> PluginResult<Option<GameMetadata>> {
        if game.platform != PLATFORM {
            return Ok(None);
        }
        let Ok(app) = game.title_id.parse::<u32>() else { return Ok(None) };
        self.store_details(app).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_s_dates_read_either_way_round() {
        assert_eq!(date("Oct 10, 2007"), "2007-10-10");
        assert_eq!(date("10 Oct, 2007"), "2007-10-10");
        assert_eq!(date("Coming soon"), "");
        assert_eq!(date("Q4 2026"), "");
    }

    /// Team Fortress 2's details, trimmed from Steam's answer.
    #[test]
    fn steam_s_details_become_the_game_s_metadata() {
        let data = serde_json::json!({
            "name": "Team Fortress 2",
            "short_description": "Nine distinct classes &amp; more.",
            "developers": ["Valve"], "publishers": ["Valve"],
            "genres": [{"id": "1", "description": "Action"}, {"id": "37", "description": "Free To Play"}],
            "release_date": {"coming_soon": false, "date": "Oct 10, 2007"},
            "required_age": 0,
            "screenshots": [{"id": 0, "path_thumbnail": "https://x/t.jpg", "path_full": "https://x/full.jpg"}],
            "movies": [{"id": 1, "name": "Trailer", "hls_h264": "https://x/hls.m3u8", "dash_h264": "https://x/d.mpd"}],
        });
        let meta = metadata(440, &data);
        assert_eq!(meta.title, "Team Fortress 2");
        assert_eq!(meta.description, "Nine distinct classes & more.");
        assert_eq!((meta.publisher.as_str(), meta.release.as_str()), ("Valve", "2007-10-10"));
        assert_eq!(meta.genres, ["Action", "Free To Play"]);
        assert_eq!(meta.screenshots, [ImageSource::Url("https://x/full.jpg".into())]);
        assert_eq!(meta.trailer, "https://x/hls.m3u8");
        assert_eq!(meta.store_url, "https://store.steampowered.com/app/440/");
        assert_eq!(meta.age_rating, "", "no age given");
    }
}
