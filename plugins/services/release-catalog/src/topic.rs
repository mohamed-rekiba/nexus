//! The catalog file as one typed model: the file, its topics, and each topic's game info and
//! Steam data. The catalog refresh changes it (see [`crate::refresh`]), and [`crate::format`]
//! turns it into releases.
//!
//! A field with a type the model doesn't expect reads as missing, as a JSON lookup of it would:
//! one odd field never fails a whole catalog file. A missing field and a `null` one are the
//! same, and a missing field is not written. Each struct keeps the fields it doesn't name in
//! `extra`, so a refresh writes them back as they were.

use nexus_api::serde::de::DeserializeOwned;
use nexus_api::serde::{Deserialize, Deserializer, Serialize};
use nexus_api::serde_json::{Map, Number, Value};

/// A catalog file, as the catalog collector writes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct CatalogFile {
    /// `false` or missing for a partial listing.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub translation: Option<Translation>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub topic_count: Option<u64>,
    /// `None` when the file has no topics list. Unlike the other fields, a list that doesn't
    /// read fails the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<Topic>>,
    /// The forum the file is a listing of: its first listing page.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// For a listing of one forum, the forum's key in the catalog sources (`ps5`, `wine`).
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// The catalog's own version.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub collected_at: Option<String>,
    /// When the plugin's refresh or the collector's last refreshed it.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
    /// When the last forum list of a refresh was made.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub api_observed: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The language a catalog file's titles and game info are in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Translation {
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One release topic of a forum.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Topic {
    /// The topic's number, as text. A number in the file reads as its digits. Empty when the
    /// topic has none.
    #[serde(default, deserialize_with = "topic_id", skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The forum's title of the topic, tags and all.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The topic's page.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The forum the topic is in: its first listing page.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The console the topic is for, when the topic says: `PS5`, `PS4` or `PC`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// For a PC release, the system it is for: `linux`, `windows`, `macos`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub wine_packed: Option<bool>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub magnet: Option<String>,
    /// Whether the magnet link was found: `available`, `not_found`, `not_checked`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub magnet_status: Option<String>,
    /// The size as the forum writes it: `79.64 GB`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub seeders: Option<u64>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub leechers: Option<u64>,
    /// When the counts of peers were taken.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub peers_observed: Option<String>,
    /// When the topic was posted, as the collector read it from the topic's page.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// When a refresh added the topic, from the feed.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub added_at: Option<String>,
    /// When the torrent was uploaded, from the forum list.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub game_info: Option<GameInfo>,
    /// Where the game info came from: [`BORROWED`] when a refresh took it from another
    /// release of the game.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub game_info_status: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub steam: Option<Steam>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The [`Topic::game_info_status`] of game info that a refresh took from another release.
pub const BORROWED: &str = "borrowed";

/// What the collector read from a topic's page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct GameInfo {
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// One title id or several: `PPSA05666 / PPSA05667`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub developer: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub release_date: Option<Scalar>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub release_year: Option<Scalar>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub multiplayer: Option<String>,
    #[serde(default, deserialize_with = "texts", skip_serializing_if = "Option::is_none")]
    pub image_urls: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub release_summary: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub release_update: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub minimum_firmware: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub interface_languages: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub audio_languages: Option<String>,
    /// How the release is packed: `exfat`, `fpkg`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The system a PC release is for, or the console: `Linux`, `Windows`, `PS5`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// For game info a refresh borrowed, the page of the topic that took it.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// For game info a refresh borrowed, the topic it came from.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub borrowed_from: Option<String>,
    // Facts about the game that the plugin doesn't read, only borrows: kept as the file has
    // them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_rating: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_multiplayer: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_multiplayer: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ps5_pro_enhanced: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pssr_support: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psvr2: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl GameInfo {
    /// The facts about the game, the same in every release of it, with nothing else. The other
    /// facts (region, version, format, firmware, languages) are about one upload.
    pub fn game_facts(&self) -> GameInfo {
        GameInfo {
            name: self.name.clone(),
            title_id: self.title_id.clone(),
            genre: self.genre.clone(),
            developer: self.developer.clone(),
            publisher: self.publisher.clone(),
            release_date: self.release_date.clone(),
            release_year: self.release_year.clone(),
            age_rating: self.age_rating.clone(),
            multiplayer: self.multiplayer.clone(),
            local_multiplayer: self.local_multiplayer.clone(),
            online_multiplayer: self.online_multiplayer.clone(),
            ps5_pro_enhanced: self.ps5_pro_enhanced.clone(),
            pssr_support: self.pssr_support.clone(),
            psvr2: self.psvr2.clone(),
            image_urls: self.image_urls.clone(),
            release_summary: self.release_summary.clone(),
            ..GameInfo::default()
        }
    }
}

/// A topic's Steam data, as the collector writes it. With no `appid`, Steam doesn't have the
/// game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Steam {
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub appid: Option<u64>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(default, deserialize_with = "texts", skip_serializing_if = "Option::is_none")]
    pub genres: Option<Vec<String>>,
    /// Steam's date: `Jan 25, 2018`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(default, deserialize_with = "texts", skip_serializing_if = "Option::is_none")]
    pub developers: Option<Vec<String>>,
    #[serde(default, deserialize_with = "texts", skip_serializing_if = "Option::is_none")]
    pub publishers: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default, deserialize_with = "texts", skip_serializing_if = "Option::is_none")]
    pub shots: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub assets: Option<SteamAssets>,
    /// The ProtonDB tier: `gold`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub protondb: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The art of a game on Steam.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct SteamAssets {
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A fact that the file writes as text or as a number: a year can be either.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde", untagged)]
pub enum Scalar {
    Text(String),
    Number(Number),
}

impl Scalar {
    /// The fact as text, trimmed.
    pub fn text(&self) -> String {
        match self {
            Scalar::Text(text) => text.trim().to_owned(),
            Scalar::Number(number) => number.to_string(),
        }
    }
}

/// A field's value when it has the type `T`, else `None`: as `null`.
pub(crate) fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    Ok(T::deserialize(Value::deserialize(d)?).ok())
}

/// The text items of a list, in order. `None` when the field is not a list.
fn texts<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => Some(
            items
                .into_iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    })
}

/// A topic's number, as text or as a number. Empty for any other value.
fn topic_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(id) => id,
        Value::Number(id) => id.to_string(),
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_api::serde_json::{self, json};

    /// `value` with each `null` field taken out, at any depth: a missing field and a `null` one
    /// are the same.
    fn without_nulls(value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, without_nulls(v))).collect(),
            ),
            Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
            other => other,
        }
    }

    /// The file `bytes`, read into the model and written back, is the same file but for its
    /// `null` fields.
    fn round_trips(name: &str, bytes: &[u8]) {
        let before: Value = serde_json::from_slice(bytes).unwrap();
        let file: CatalogFile = serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let after = serde_json::to_value(&file).unwrap();
        assert!(without_nulls(before) == without_nulls(after), "{name} changed when written back");
    }

    #[test]
    fn every_sample_and_bundled_file_writes_back_as_it_was() {
        round_trips("samples/ps5.json", include_bytes!("../samples/ps5.json"));
        round_trips("samples/ps4.json", include_bytes!("../samples/ps4.json"));
        round_trips("samples/pc.json", include_bytes!("../samples/pc.json"));
        for (name, bytes) in crate::BUNDLED {
            round_trips(name, bytes);
        }
    }

    #[test]
    fn a_topic_reads_its_fields_and_keeps_the_ones_it_does_not_name() {
        let topic: Topic = serde_json::from_value(json!({ "id": 6912182, "title": "[PS5] Astro", "seeders": 7,
            "comments": 3, "game_info": { "name": "Astro", "release_year": 2024, "psvr2": "Yes", "rating": 9 },
            "steam": { "appid": 504230, "assets": { "hero": "https://x/h.jpg" }, "metacritic": 92 } }))
        .unwrap();
        assert_eq!(
            (topic.id.as_str(), topic.title.as_deref(), topic.seeders),
            ("6912182", Some("[PS5] Astro"), Some(7))
        );
        assert_eq!(topic.extra["comments"], 3, "a field it doesn't name");
        let info = topic.game_info.as_ref().unwrap();
        assert_eq!(info.release_year.as_ref().map(Scalar::text).as_deref(), Some("2024"), "a year as a number");
        assert_eq!((info.psvr2.as_ref(), &info.extra["rating"]), (Some(&json!("Yes")), &json!(9)));
        let steam = topic.steam.as_ref().unwrap();
        assert_eq!(steam.appid, Some(504_230));
        assert_eq!(steam.assets.as_ref().unwrap().hero.as_deref(), Some("https://x/h.jpg"));
        assert_eq!(steam.extra["metacritic"], 92);
    }

    #[test]
    fn a_field_of_another_type_reads_as_missing() {
        let topic: Topic = serde_json::from_value(json!({ "id": [1], "title": 5, "seeders": "7", "leechers": -1,
            "wine_packed": "yes", "game_info": "none", "steam": { "appid": null, "shots": ["https://x/1.jpg", 2] } }))
        .unwrap();
        assert_eq!(topic.id, "", "a list is no topic number");
        assert_eq!((topic.title, topic.seeders, topic.leechers, topic.wine_packed), (None, None, None, None));
        assert_eq!(topic.game_info, None, "text is no game info");
        let steam = topic.steam.unwrap();
        assert_eq!(steam.appid, None, "Steam doesn't have the game");
        assert_eq!(steam.shots, Some(vec!["https://x/1.jpg".to_owned()]), "only the text items");
        let file: CatalogFile = serde_json::from_value(json!({ "complete": "yes", "topic_count": "1" })).unwrap();
        assert_eq!((file.complete, file.topic_count, file.topics), (None, None, None));
        assert!(serde_json::from_value::<CatalogFile>(json!({ "topics": ["not a topic"] })).is_err());
    }

    #[test]
    fn the_game_facts_leave_out_what_is_about_one_upload() {
        let info: GameInfo = serde_json::from_value(json!({ "name": "Astro", "title_id": "PPSA01234",
            "version": "v01.05", "region": "EUR", "format": "exfat", "psvr2": false, "notes": "x" }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(info.game_facts()).unwrap(),
            json!({ "name": "Astro", "title_id": "PPSA01234", "psvr2": false })
        );
    }
}
