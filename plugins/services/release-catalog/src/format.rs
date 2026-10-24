//! The catalog file format, as the catalog collector writes it: a listing of release
//! topics, each with its title, size, magnet link and game info.
//!
//! ```json
//! { "source": "…", "complete": true, "collected_at": "2026-09-30T00:56:18Z",
//!   "refreshed_at": "…", "translation": { "language": "en" }, "topic_count": 1,
//!   "topics": [{ "id": "6912182", "title": "[PS5] Name [EUR / v01.004]", "size": "79.64 GB",
//!     "magnet": "magnet:?xt=urn:btih:…", "published_at": "2026-09-22",
//!     "game_info": { "name": "Name", "title_id": "PPSA05666", "region": "EUR (Europe)",
//!       "version": "v01.004.011", "genre": "Action", "release_summary": "…",
//!       "platform": "PS5" } }] }
//! ```
//!
//! A topic's `seeders`, `leechers` and `peers_observed` (when the counts were taken) are on the
//! topic. Its `game_info` can also have `developer`, `publisher`, `release_date`, `release_year`,
//! `release_update`, `multiplayer`, `minimum_firmware`, `interface_languages`, `audio_languages`,
//! `format` and `image_urls`.
//!
//! A topic can also have a `steam` object, as the catalog collector writes it:
//! `{ "appid": 504230, "short": "…", "genres": […], "release": "Jan 25, 2018", "developers": […],
//! "publishers": […], "header": "https://…", "shots": […], "assets": { "portrait": …, "hero": …,
//! "logo": … }, "protondb": "gold" }`. `{ "appid": null }` means Steam doesn't have the game.
//!
//! A file reads only when all of it does: it is `complete`, in English, `topic_count` matches
//! its topics, and every topic has a unique positive id and a name. Apps, system software and
//! forum threads aren't games and are left out, and so is a topic without a magnet link: there
//! is nothing to download.

use std::collections::HashSet;

use nexus_api::serde_json::{self, Value};
use nexus_api::{Release, ReleaseList, SteamInfo};

use crate::info_hash;

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").trim().to_owned()
}

/// The PS5 (`PPSA12345`) and PS4 (`CUSA12345`) title ids in `value`, in order, once each.
/// Spaces inside an id are allowed (`P P S A 12345`).
pub fn title_ids(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_uppercase()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // The id may not run on from a word: no letter before it.
        let starts = i == 0 || !chars[i - 1].is_ascii_alphabetic();
        let prefix = if !starts {
            None
        } else if chars[i..].starts_with(&['C', 'U', 'S', 'A']) {
            Some(("CUSA", 4))
        } else if chars[i] == 'P' {
            let ps = chars[i..].iter().take_while(|&&c| c == 'P').count();
            chars[i + ps..].starts_with(&['S', 'A']).then_some(("PPSA", ps + 2))
        } else {
            None
        };
        if let Some((name, len)) = prefix {
            let digits = &chars[i + len..];
            let five = digits.len() >= 5 && digits[..5].iter().all(char::is_ascii_digit);
            let sixth = digits.get(5).is_some_and(char::is_ascii_digit);
            if five && !sixth {
                let id = format!("{name}{}", digits[..5].iter().collect::<String>());
                if !out.contains(&id) {
                    out.push(id);
                }
                i += len + 5;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `79.64 GB` (or TB, MB; a comma for the point) in bytes.
pub fn size_bytes(value: &str) -> Option<u64> {
    let upper = value.to_ascii_uppercase();
    for (unit, scale) in
        [("TB", 1024.0 * 1024.0 * 1024.0 * 1024.0), ("GB", 1024.0 * 1024.0 * 1024.0), ("MB", 1024.0 * 1024.0)]
    {
        let Some(at) = upper.find(unit) else { continue };
        let number: String = upper[..at]
            .trim_end()
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let size: f64 = number.replace(',', ".").parse().ok()?;
        return Some((size * scale) as u64);
    }
    None
}

/// The game's name without the listing's tags: leading `[…]` groups, everything from ` [`,
/// and a `PSVR2 only` / `PSVR only` prefix.
pub fn clean_name(value: &str) -> String {
    let mut name = value.trim();
    while let Some(rest) = name.strip_prefix('[') {
        match rest.split_once(']') {
            Some((_, after)) => name = after.trim_start(),
            None => break,
        }
    }
    let name = name.split(" [").next().unwrap_or("").trim();
    name.strip_prefix("PSVR2 only ").or_else(|| name.strip_prefix("PSVR only ")).unwrap_or(name).to_owned()
}

/// A PC title's name without its add-ons and its year and version groups: "Celeste (2018)
/// (v1.4.0.0) + 2 DLC" is "Celeste". It ends at the first group that is a year, a version or
/// a build.
pub(crate) fn pc_name(name: &str) -> String {
    let name = name.split(" + ").next().unwrap_or(name).trim();
    let is_tail = |group: &str| {
        let lower = group.to_ascii_lowercase();
        let year = lower.len() >= 5 && lower[..4].bytes().all(|b| b.is_ascii_digit()) && lower[4..].starts_with(')');
        let version = lower
            .strip_prefix('v')
            .is_some_and(|v| v.trim_start_matches('.').starts_with(|c: char| c.is_ascii_digit()));
        year || version || lower.starts_with("v.build") || lower.starts_with("build")
    };
    let cut = name.match_indices(" (").map(|(at, _)| at).find(|&at| is_tail(&name[at + 2..]));
    cut.map_or(name, |at| name[..at].trim()).to_owned()
}

/// The tags in `[…]` groups of a title, lower case, split at `,` and `/`.
fn tags(title: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = title;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else { break };
        for tag in rest[open + 1..open + close].split([',', '/']) {
            out.push(tag.trim().to_ascii_lowercase());
        }
        rest = &rest[open + close + 1..];
    }
    out
}

/// The system a PC release is for, from its game info's platform, else its title's tags:
/// `linux`, `windows`, `macos`, or `""` when neither says.
fn system(info: &Value, title: &str) -> &'static str {
    let named = text(info, "platform").to_ascii_lowercase();
    let from = |word: &str| match word {
        "linux" | "native" | "wine" | "amd64" | "x86_64" | "x64" => "linux",
        "windows" | "win" | "win64" | "win32" => "windows",
        "macos" | "mac" | "osx" | "mac os" => "macos",
        _ => "",
    };
    match from(&named) {
        "" => tags(title).iter().map(|t| from(t)).find(|s| !s.is_empty()).unwrap_or(""),
        system => system,
    }
}

/// Steam's "Jan 25, 2018" or "25 Jan, 2018" as "2018-01-25".
fn steam_date(date: &str) -> Option<String> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let parts: Vec<&str> = date.split([' ', ',']).filter(|p| !p.is_empty()).collect();
    let [a, b, year] = parts[..] else { return None };
    let (month, day) = if a.starts_with(|c: char| c.is_ascii_digit()) { (b, a) } else { (a, b) };
    let month = MONTHS.iter().position(|m| month.to_ascii_lowercase().starts_with(m))? + 1;
    let (day, year): (u32, u32) = (day.parse().ok()?, year.parse().ok()?);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// The text or the number at `key`, as text: a year can be either. Empty when it is neither.
fn scalar(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::Number(n) => n.to_string(),
        _ => text(value, key),
    }
}

/// The catalog's `multiplayer` words in plain words: `No` is `Single player`, `Yes` is
/// `Multiplayer`. Other words, such as `2x`, stay as they are.
fn players(multiplayer: &str) -> String {
    match multiplayer {
        "No" | "None" | "None (Single player)" => "Single player".into(),
        "Yes" => "Multiplayer".into(),
        other => other.to_owned(),
    }
}

/// The `https://` address at `key`, or empty.
fn address(value: &Value, key: &str) -> String {
    Some(text(value, key)).filter(|url| url.starts_with("https://")).unwrap_or_default()
}

fn texts(value: &Value, key: &str) -> Vec<String> {
    let items = value[key].as_array().into_iter().flatten().filter_map(Value::as_str);
    items.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// A topic's `steam` object, as the catalog collector writes it. `None` when it
/// has no app id: Steam didn't have the game.
fn steam(topic: &Value) -> Option<SteamInfo> {
    let steam = &topic["steam"];
    let app_id = steam["appid"].as_u64()?;
    Some(SteamInfo {
        app_id,
        short: text(steam, "short"),
        genres: texts(steam, "genres"),
        release: steam_date(&text(steam, "release")).unwrap_or_default(),
        developers: texts(steam, "developers"),
        publishers: texts(steam, "publishers"),
        header: address(steam, "header"),
        screenshots: texts(steam, "shots").into_iter().filter(|url| url.starts_with("https://")).collect(),
        portrait: address(&steam["assets"], "portrait"),
        hero: address(&steam["assets"], "hero"),
        logo: address(&steam["assets"], "logo"),
        protondb: text(steam, "protondb").to_ascii_lowercase(),
    })
}

/// A PKG release: its game info's format, or a tag in its title, says `pkg` or `fpkg`.
fn is_package(info: &Value, title: &str) -> bool {
    let format = text(info, "format").to_ascii_lowercase();
    let words = format.split(|c: char| !c.is_ascii_alphanumeric()).map(str::to_owned);
    words.chain(tags(title)).any(|word| word == "pkg" || word == "fpkg")
}

/// Apps (YouTube, Media Player), system software and forum threads (rules, chat) aren't games.
fn is_game(title: &str, genre: &str, title_id: &str, magnet: &str) -> bool {
    const APPS: [&str; 10] = [
        "app",
        "apps",
        "application",
        "applications",
        "program",
        "programs",
        "software",
        "media",
        "utility",
        "utilities",
    ];
    const NOT_GAME: [&str; 8] =
        ["firmware", "exploit", "exploits", "ps4hen", "theme pack", "psnow", "ps now", "shadps4"];
    let app = APPS.iter().any(|g| genre.eq_ignore_ascii_case(g));
    let thread = magnet.is_empty() && title_id.is_empty() && genre.is_empty();
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let named = NOT_GAME.iter().any(|n| {
        let parts: Vec<&str> = n.split(' ').collect();
        words.windows(parts.len()).any(|w| w == parts.as_slice())
    });
    !app && !thread && !named
}

/// The console a topic is for: its game info's, else the `[PS5]` tag in its title, else its
/// title id's.
fn platform(info: &Value, title: &str, title_id: &str) -> String {
    let named = text(info, "platform").to_ascii_uppercase();
    if named == "PS5" || named == "PS4" || named == "PC" {
        return named;
    }
    let upper = title.to_ascii_uppercase();
    if upper.contains("[PS5]") {
        "PS5".into()
    } else if upper.contains("[PS4]") {
        "PS4".into()
    } else if title_id.starts_with("PPSA") {
        "PS5".into()
    } else if title_id.starts_with("CUSA") {
        "PS4".into()
    } else if upper.contains("[PC]") || upper.contains("[GOG]") || !system(info, title).is_empty() {
        "PC".into()
    } else {
        String::new()
    }
}

/// What a topic says it is for in its own fields, as the collector's published files write
/// them: its console (`platform`: `PS5`, `PS4` or `PC`), the system a PC release is for
/// (`system`), and whether it is packed with Wine (`wine_packed`). `None` for a field it doesn't
/// have.
fn declared(topic: &Value) -> (Option<String>, Option<String>, Option<bool>) {
    let platform = text(topic, "platform").to_ascii_uppercase();
    let platform = ["PS5", "PS4", "PC"].contains(&platform.as_str()).then_some(platform);
    let system = Some(text(topic, "system").to_ascii_lowercase()).filter(|s| !s.is_empty());
    (platform, system, topic["wine_packed"].as_bool())
}

/// What a catalog or a topic of one forum is for, from its `source` forum, else its `platform`: the
/// console, the system a PC release is for, and whether its releases are packed with Wine.
/// `None` when the file doesn't say.
///
/// The collector's published files say this in each topic's own fields (see [`declared`]). This
/// table of the forums' numbers serves the collector's raw listings, copied in by hand.
fn forum(file: &Value) -> Option<(&'static str, &'static str, bool)> {
    let source = text(file, "source");
    let id = source.split_once("viewforum.php?f=").map(|(_, id)| id.split('&').next().unwrap_or_default());
    let named = text(file, "platform").to_ascii_lowercase();
    match (id, named.as_str()) {
        (Some("546"), _) | (None, "ps5") => Some(("PS5", "", false)),
        (Some("973"), _) | (None, "ps4") => Some(("PS4", "", false)),
        (Some("1992"), _) | (None, "linux") => Some(("PC", "linux", false)),
        (Some("2059"), _) | (None, "wine") => Some(("PC", "linux", true)),
        _ => None,
    }
}

/// Reads a catalog file. Any part that doesn't read fails the whole file.
pub fn read(bytes: &[u8]) -> Result<ReleaseList, String> {
    let file: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a catalog file: {e}"))?;
    if file["complete"].as_bool() != Some(true) {
        return Err("the catalog isn't complete: partial listings aren't read".into());
    }
    if file["translation"]["language"].as_str() != Some("en") {
        return Err("the catalog isn't in English".into());
    }
    let topics = file["topics"].as_array().ok_or("the catalog has no topics list")?;
    if file["topic_count"].as_u64() != Some(topics.len() as u64) {
        return Err("topic_count doesn't match the topics".into());
    }
    let file_forum = forum(&file);
    let mut ids = HashSet::new();
    let mut hashes = HashSet::new();
    let mut releases = Vec::with_capacity(topics.len());
    for topic in topics {
        let id = topic["id"]
            .as_str()
            .and_then(|id| id.parse::<i64>().ok())
            .or_else(|| topic["id"].as_i64())
            .filter(|id| *id > 0)
            .ok_or("a topic has no valid id")?;
        if !ids.insert(id) {
            return Err(format!("topic {id} is listed twice"));
        }
        let info = &topic["game_info"];
        // A topic's own fields win, then its forum: a file of several forums names each topic's
        // forum. Then the file's forum, then the title's tags.
        let (own_platform, own_system, own_wine) = declared(topic);
        let forum = forum(topic).or(file_forum);
        let title = text(topic, "title");
        let named = text(info, "name");
        let name = clean_name(if named.is_empty() { &title } else { &named });
        if name.is_empty() {
            return Err(format!("topic {id} has no game name"));
        }
        let raw_id = text(info, "title_id");
        let mut all_ids = title_ids(&raw_id);
        if all_ids.is_empty() {
            all_ids = title_ids(&title).into_iter().take(1).collect();
        }
        let genre = text(info, "genre");
        let magnet = text(topic, "magnet");
        if !is_game(&title, &genre, &raw_id, &magnet) {
            continue;
        }
        // Nothing to download without a magnet link with an info hash.
        let Some(hash) = magnet.starts_with("magnet:?").then(|| info_hash(&magnet)).flatten() else { continue };
        // The same torrent in two topics: the first one stands.
        if !hashes.insert(hash.clone()) {
            continue;
        }
        let title_id = all_ids.first().cloned().unwrap_or_default();
        // A PC forum says what all its topics are for; a console forum fills in a topic that
        // doesn't say.
        let platform = own_platform.unwrap_or_else(|| match forum {
            Some(("PC", ..)) => "PC".to_owned(),
            Some((console, ..)) => {
                Some(platform(info, &title, &title_id)).filter(|p| !p.is_empty()).unwrap_or_else(|| console.to_owned())
            }
            None => platform(info, &title, &title_id),
        });
        let system = match (own_system, forum) {
            _ if platform != "PC" => String::new(),
            (Some(own), _) => own,
            (None, Some(("PC", system, _))) => system.to_owned(),
            (None, _) => system(info, &title).to_owned(),
        };
        let name = if platform == "PC" { pc_name(&name) } else { name };
        let added = ["published_at", "added_at", "registered_at"]
            .iter()
            .map(|key| text(topic, key))
            .find(|date| !date.is_empty())
            .unwrap_or_default();
        releases.push(Release {
            id: hash,
            topic: id.to_string(),
            system,
            platform,
            title_ids: all_ids.iter().skip(1).cloned().collect(),
            title_id,
            title: name,
            version: text(info, "version"),
            region: text(info, "region"),
            size_bytes: topic["size_bytes"].as_u64().or_else(|| size_bytes(&text(topic, "size"))),
            added,
            description: text(info, "release_summary"),
            torrent: magnet,
            steam: steam(topic),
            wine_packed: own_wine
                .unwrap_or_else(|| matches!(forum, Some((_, _, true))) || tags(&title).iter().any(|t| t == "wine")),
            package: is_package(info, &title),
            seeders: topic["seeders"].as_u64(),
            leechers: topic["leechers"].as_u64(),
            counted_at: text(topic, "peers_observed"),
            genre,
            developer: text(info, "developer"),
            publisher: text(info, "publisher"),
            release_date: scalar(info, "release_date"),
            release_year: scalar(info, "release_year"),
            update: text(info, "release_update"),
            players: players(&text(info, "multiplayer")),
            minimum_firmware: text(info, "minimum_firmware"),
            interface_languages: text(info, "interface_languages"),
            audio_languages: text(info, "audio_languages"),
            format: text(info, "format"),
            image_urls: texts(info, "image_urls").into_iter().filter(|url| url.starts_with("https://")).collect(),
        });
    }
    let updated =
        Some(text(&file, "refreshed_at")).filter(|t| !t.is_empty()).unwrap_or_else(|| text(&file, "collected_at"));
    Ok(ReleaseList { releases, version: text(&file, "version"), updated, ..ReleaseList::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(topics: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "source": "https://xyz.dev", "complete": true,
            "translation": { "language": "en" }, "collected_at": "2026-09-30T00:56:18Z",
            "topic_count": topics.as_array().unwrap().len(), "topics": topics }))
        .unwrap()
    }

    const MAGNET: &str = "magnet:?xt=urn:btih:8C2F2153B5F6DF67B10BFB053173277A878F83D4&tr=x";

    #[test]
    fn a_topic_reads_as_the_old_launcher_read_it() {
        let list = read(&file(serde_json::json!([{
            "id": "6912182", "title": "[PS5] [PSVR2 only] Horizon [Region Free / v01.004.011 / exfat]",
            "size": "79.64 GB", "magnet": MAGNET, "published_at": "2026-09-22",
            "game_info": { "name": "PSVR2 only Horizon", "title_id": "PPSA05666 / PPSA05667", "region": "Region Free",
                "version": "v01.004.011", "genre": "Action", "release_summary": "Horizon · Action" } }])))
        .unwrap();
        assert_eq!(list.updated, "2026-09-30T00:56:18Z");
        let r = &list.releases[0];
        assert_eq!(r.id, "8c2f2153b5f6df67b10bfb053173277a878f83d4");
        assert_eq!(r.topic, "6912182");
        assert_eq!((r.title.as_str(), r.platform.as_str()), ("Horizon", "PS5"));
        assert_eq!((r.title_id.as_str(), r.title_ids.as_slice()), ("PPSA05666", ["PPSA05667".to_owned()].as_slice()));
        assert_eq!(r.size_bytes, Some((79.64 * 1024.0 * 1024.0 * 1024.0) as u64));
        assert_eq!(
            (r.version.as_str(), r.region.as_str(), r.added.as_str()),
            ("v01.004.011", "Region Free", "2026-09-22")
        );
        assert_eq!(r.description, "Horizon · Action");
    }

    #[test]
    fn a_topic_keeps_its_peers_and_its_game_facts() {
        let list = read(&file(serde_json::json!([
            { "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "magnet": MAGNET, "seeders": 7, "leechers": 2,
              "peers_observed": "2026-10-07T14:07:32Z",
              "game_info": { "name": "Astro", "genre": "Action, Platformer", "developer": "Team Asobi",
                "publisher": "Sony", "release_date": "2024-09-06", "release_year": "2024", "release_update": "v01.05",
                "multiplayer": "No", "minimum_firmware": "5.xx", "interface_languages": "English, Russian",
                "audio_languages": "English", "format": "exfat",
                "image_urls": ["https://i1.imageban.ru/out/cover.jpg", "http://plain.example/a.jpg", "not an address"] } },
            { "id": "2", "title": "[PS5] Bolt", "magnet": "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "game_info": { "multiplayer": "Yes", "release_year": 2020 } },
            { "id": "3", "title": "[PS5] Comet", "magnet": "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "game_info": { "multiplayer": "2x" } },
        ])))
        .unwrap();
        let astro = &list.releases[0];
        assert_eq!(
            (astro.seeders, astro.leechers, astro.counted_at.as_str()),
            (Some(7), Some(2), "2026-10-07T14:07:32Z")
        );
        assert_eq!(astro.genre, "Action, Platformer");
        assert_eq!((astro.developer.as_str(), astro.publisher.as_str()), ("Team Asobi", "Sony"));
        assert_eq!((astro.release_date.as_str(), astro.release_year.as_str()), ("2024-09-06", "2024"));
        assert_eq!((astro.update.as_str(), astro.players.as_str()), ("v01.05", "Single player"));
        assert_eq!(astro.minimum_firmware, "5.xx");
        assert_eq!(
            (astro.interface_languages.as_str(), astro.audio_languages.as_str()),
            ("English, Russian", "English")
        );
        assert_eq!(astro.format, "exfat");
        assert_eq!(astro.image_urls, ["https://i1.imageban.ru/out/cover.jpg"], "only https:// addresses");
        let bolt = &list.releases[1];
        assert_eq!((bolt.seeders, bolt.leechers, bolt.counted_at.as_str()), (None, None, ""), "no count");
        assert_eq!((bolt.players.as_str(), bolt.release_year.as_str()), ("Multiplayer", "2020"), "a year as a number");
        assert_eq!(list.releases[2].players, "2x", "the catalog's own words");
    }

    #[test]
    fn a_release_written_before_the_game_facts_still_reads() {
        let old = serde_json::json!({ "id": "abc", "title": "Astro", "platform": "PS5", "torrent": MAGNET });
        let release: Release = serde_json::from_value(old).unwrap();
        assert_eq!((release.seeders, release.genre.as_str()), (None, ""));
        assert!(release.image_urls.is_empty());
    }

    #[test]
    fn a_pc_topic_is_a_pc_release_with_the_system_it_is_for() {
        let list = read(&file(serde_json::json!([
            { "id": "1", "title": "Celeste (v1.4.0.0) + 2 DLC [amd64] [Multi] [Wine]", "magnet": MAGNET,
                "game_info": { "genre": "Platformer" } },
            { "id": "2", "title": "[GOG] Hollow Knight [Windows]", "magnet": "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "game_info": {} },
            { "id": "3", "title": "Tux Racer", "magnet": "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "game_info": { "platform": "macOS" } },
            { "id": "4", "title": "[PC] Quake", "magnet": "magnet:?xt=urn:btih:cccccccccccccccccccccccccccccccccccccccc", "game_info": {} },
        ])))
        .unwrap();
        let read: Vec<_> =
            list.releases.iter().map(|r| (r.title.as_str(), r.platform.as_str(), r.system.as_str())).collect();
        assert_eq!(
            read,
            [
                ("Celeste", "PC", "linux"),
                ("Hollow Knight", "PC", "windows"),
                ("Tux Racer", "PC", "macos"),
                ("Quake", "PC", ""),
            ]
        );
    }

    #[test]
    fn a_topic_keeps_its_steam_data_and_says_when_it_is_packed_with_wine() {
        let art = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/504230";
        let list = read(&file(serde_json::json!([
            { "id": "1", "title": "Celeste (v1.4.0.0) [amd64] [Wine]", "magnet": MAGNET, "game_info": {},
              "steam": { "appid": 504230, "name": "Celeste", "short": "Help Madeline survive.", "genres": ["Action", "Indie"],
                "release": "Jan 25, 2018", "developers": ["Maddy Makes Games"], "publishers": ["Maddy Makes Games Inc."],
                "linux": true, "metacritic": 92, "header": format!("{art}/header.jpg"),
                "shots": [format!("{art}/ss_1.jpg"), "not an address"], "video": null,
                "assets": { "portrait": format!("{art}/library_600x900_2x.jpg"), "hero": format!("{art}/library_hero.jpg"), "logo": null },
                "protondb": "platinum", "checked_at": "2026-10-14T00:00:00Z" } },
            { "id": "2", "title": "Unknown Game [Wine]", "magnet": "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "game_info": {}, "steam": { "appid": null, "checked_at": "2026-10-14T00:00:00Z" } },
            { "id": "3", "title": "Tux Racer [Native]", "magnet": "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "game_info": {} },
        ])))
        .unwrap();
        let celeste = &list.releases[0];
        assert!(celeste.wine_packed);
        let steam = celeste.steam.as_ref().expect("the Steam data");
        assert_eq!(steam.app_id, 504_230);
        assert_eq!(steam.short, "Help Madeline survive.");
        assert_eq!(steam.genres, ["Action", "Indie"]);
        assert_eq!(steam.release, "2018-01-25", "Steam's date as YYYY-MM-DD");
        assert_eq!(
            (steam.developers.as_slice(), steam.publishers.as_slice()),
            (["Maddy Makes Games".to_owned()].as_slice(), ["Maddy Makes Games Inc.".to_owned()].as_slice())
        );
        assert_eq!(steam.header, format!("{art}/header.jpg"));
        assert_eq!(steam.screenshots, [format!("{art}/ss_1.jpg")], "only https:// addresses");
        assert_eq!(
            (steam.portrait.as_str(), steam.hero.as_str(), steam.logo.as_str()),
            (format!("{art}/library_600x900_2x.jpg").as_str(), format!("{art}/library_hero.jpg").as_str(), "")
        );
        assert_eq!(steam.protondb, "platinum");
        assert!(list.releases[1].wine_packed);
        assert_eq!(list.releases[1].steam, None, "Steam has no such game");
        assert!(!list.releases[2].wine_packed, "a native Linux game");
        assert_eq!(list.releases[2].steam, None);
        assert_eq!(steam_date("25 Jan, 2018").as_deref(), Some("2018-01-25"));
        assert_eq!(steam_date("Coming soon"), None);
    }

    /// A catalog of one forum, as its collector writes it: `source` and `platform` say what
    /// every topic is for, and the titles have no system tag.
    fn forum(source: &str, platform: &str, topics: Value) -> Vec<u8> {
        let mut file: Value = serde_json::from_slice(&file(topics)).unwrap();
        file["source"] = source.into();
        file["platform"] = platform.into();
        serde_json::to_vec(&file).unwrap()
    }

    #[test]
    fn a_forum_s_catalog_says_what_its_topics_are_for() {
        let topic = |date: &str| {
            serde_json::json!([{ "id": "1", "title": "Warcraft I: Remastered (2024) (v.1.0.0.2762) [RUS + ENG + 9 / ENG] [663.1 MB]",
                "magnet": MAGNET, "game_info": null, date: "2026-10-19T11:06:33+00:00", "size_bytes": 695_312_589 }])
        };
        let wine = read(&forum("https://xyz.net/forum/viewforum.php?f=2059", "wine", topic("added_at"))).unwrap();
        let r = &wine.releases[0];
        assert_eq!((r.platform.as_str(), r.system.as_str()), ("PC", "linux"));
        assert!(r.wine_packed, "every release of the Wine forum is packed with Wine");
        assert_eq!(r.title, "Warcraft I: Remastered");
        assert_eq!(r.added, "2026-10-19T11:06:33+00:00");
        assert_eq!(r.size_bytes, Some(695_312_589), "the exact size, not the title's 663.1 MB");

        let native =
            read(&forum("https://xyz.net/forum/viewforum.php?f=1992", "linux", topic("registered_at"))).unwrap();
        let r = &native.releases[0];
        assert_eq!((r.platform.as_str(), r.system.as_str()), ("PC", "linux"));
        assert!(!r.wine_packed, "a native Linux game");
        assert_eq!(r.added, "2026-10-19T11:06:33+00:00");

        let ps4 = read(&forum("https://xyz.net/forum/viewforum.php?f=973", "ps4", topic("added_at"))).unwrap();
        assert_eq!(ps4.releases[0].platform, "PS4");
    }

    #[test]
    fn a_topic_s_own_forum_wins_over_the_file_s() {
        let topic = |id: &str, hash: char, forum: &str| {
            serde_json::json!({ "id": id, "title": format!("Game {id}"), "game_info": null,
                "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)),
                "source": format!("https://xyz.net/forum/viewforum.php?f={forum}") })
        };
        let list = read(&file(serde_json::json!([topic("1", 'a', "1992"), topic("2", 'b', "2059")]))).unwrap();
        let systems: Vec<_> =
            list.releases.iter().map(|r| (r.platform.as_str(), r.system.as_str(), r.wine_packed)).collect();
        assert_eq!(systems, [("PC", "linux", false), ("PC", "linux", true)], "native Linux, then Wine");
    }

    #[test]
    fn a_topic_s_own_platform_system_and_packing_win() {
        let topic = |id: &str, hash: char, title: &str, own: Value| {
            let mut topic = serde_json::json!({ "id": id, "title": title, "game_info": null,
                "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)),
                "source": "https://xyz.net/forum/viewforum.php?f=546" });
            topic.as_object_mut().unwrap().extend(own.as_object().unwrap().clone());
            topic
        };
        let pc = |wine: bool| serde_json::json!({ "platform": "PC", "system": "linux", "wine_packed": wine });
        let list = read(&file(serde_json::json!([
            // The fields win over the topic's forum (the PS5 forum) and over the title's tags.
            topic("1", 'a', "[PS4] Racer", serde_json::json!({ "platform": "PS4" })),
            topic("2", 'b', "Celeste [Wine]", pc(false)),
            topic("3", 'c', "Quake", pc(true)),
            // A field it doesn't have comes from the forum.
            topic("4", 'd', "Astro", serde_json::json!({})),
        ])))
        .unwrap();
        let read: Vec<_> =
            list.releases.iter().map(|r| (r.platform.as_str(), r.system.as_str(), r.wine_packed)).collect();
        assert_eq!(read, [("PS4", "", false), ("PC", "linux", false), ("PC", "linux", true), ("PS5", "", false)]);
    }

    #[test]
    fn a_pkg_release_says_it_is_a_package() {
        let topic = |id: u32, title: &str, format: &str| {
            serde_json::json!({ "id": id.to_string(), "title": title,
                "magnet": format!("magnet:?xt=urn:btih:{}", id.to_string().repeat(40)),
                "game_info": { "format": format } })
        };
        let list = read(&file(serde_json::json!([
            topic(1, "[PS5] Astro [EUR]", "exfat \\ fpkg"),
            topic(2, "[PS5] Bolt [EUR / v1.00 / PKG]", ""),
            topic(3, "[PS5] Comet [EUR]", "exfat"),
            topic(4, "[PS5] Drift [EUR]", "compressed folder(7z)"),
        ])))
        .unwrap();
        let packages: Vec<_> = list.releases.iter().map(|r| (r.title.as_str(), r.package)).collect();
        assert_eq!(packages, [("Astro", true), ("Bolt", true), ("Comet", false), ("Drift", false)]);
    }

    #[test]
    fn apps_threads_and_topics_without_a_magnet_are_left_out() {
        let list = read(&file(serde_json::json!([
            { "id": "1", "title": "[PS4] YouTube", "magnet": MAGNET, "game_info": { "genre": "App" } },
            { "id": "2", "title": "Forum rules", "game_info": {} },
            { "id": "3", "title": "[PS4] Firmware 11.00", "magnet": MAGNET, "game_info": { "genre": "Other" } },
            { "id": "4", "title": "[PS4] Firewatch [CUSA04118]", "game_info": { "genre": "Adventure" } },
        ])))
        .unwrap();
        assert!(list.releases.is_empty());
    }

    #[test]
    fn a_file_reads_only_when_all_of_it_does() {
        let topic = serde_json::json!({ "id": "1", "title": "[PS4] Firewatch", "magnet": MAGNET, "game_info": {} });
        assert!(read(&file(serde_json::json!([topic, topic]))).is_err(), "a topic twice");
        let mut partial: Value = serde_json::from_slice(&file(serde_json::json!([topic]))).unwrap();
        partial["complete"] = false.into();
        assert!(read(&serde_json::to_vec(&partial).unwrap()).is_err(), "a partial listing");
        partial["complete"] = true.into();
        partial["topic_count"] = 2.into();
        assert!(read(&serde_json::to_vec(&partial).unwrap()).is_err(), "a count that doesn't match");
        assert!(read(&file(serde_json::json!([]))).is_ok(), "an empty catalog");
    }

    #[test]
    fn title_ids_sizes_and_names_read_as_before() {
        assert_eq!(title_ids("PPSA12345/PPSA54321 CUSA00001"), ["PPSA12345", "PPSA54321", "CUSA00001"]);
        assert_eq!(title_ids("P P S A 12345"), ["PPSA12345"]);
        assert!(title_ids("PPSA123456").is_empty(), "six digits");
        assert!(title_ids("XCUSA12345").is_empty(), "part of a word");
        assert_eq!(size_bytes("1,5 TB"), Some((1.5 * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64));
        assert_eq!(size_bytes("512 MB"), Some(512 * 1024 * 1024));
        assert_eq!(size_bytes("big"), None);
        assert_eq!(clean_name("[PS5] [PSVR2 only] Horizon [EUR]"), "Horizon");
        assert_eq!(clean_name("PSVR only Astro"), "Astro");
        assert_eq!(clean_name("[PS4] Game (v1.02) + DLC [EUR]"), "Game (v1.02) + DLC", "console names keep these");
        assert_eq!(pc_name("Celeste (v1.4.0.0) + 2 DLC"), "Celeste");
        assert_eq!(pc_name("Warcraft III: Reforged (2020) (v.2.0.4.23745)"), "Warcraft III: Reforged");
        assert_eq!(
            pc_name("Planet Coaster 2: Premium Edition (2024) (v.build 25677695 (14.10.2026)) + 7 DLC"),
            "Planet Coaster 2: Premium Edition"
        );
        assert_eq!(pc_name("Fernbus Simulator (v.1.35.102872) +34 DLC"), "Fernbus Simulator");
        assert_eq!(pc_name("Silver Pines (2026) (v.629 175052)"), "Silver Pines");
        assert_eq!(
            pc_name("Half-Life (Steam Edition)"),
            "Half-Life (Steam Edition)",
            "a group that is part of the name"
        );
    }

    /// Opt-in: `NEXUS_CATALOG_FILES=<file>[:<file>…]` reads catalog files from disk, such as
    /// the catalog collector's, and prints what each gives.
    #[test]
    fn catalog_files_from_disk_read() {
        let Some(files) = std::env::var_os("NEXUS_CATALOG_FILES") else { return };
        for path in std::env::split_paths(&files) {
            let list = read(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let titled = list.releases.iter().filter(|r| !r.title_id.is_empty()).count();
            println!(
                "{}: {} releases, {titled} with a title id, updated {}",
                path.display(),
                list.releases.len(),
                list.updated
            );
            assert!(!list.releases.is_empty());
        }
    }
}
