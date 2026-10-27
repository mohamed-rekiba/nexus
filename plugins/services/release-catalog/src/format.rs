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
//!
//! The file reads into the typed model of [`crate::topic`], and [`release`] turns each topic
//! into a release. What a topic is for comes from its own fields, else from the forum it or
//! its file names, which the catalog sources describe (see [`Forums`]), else from its title.

use std::collections::HashSet;

use nexus_api::serde_json;
use nexus_api::{Fact, FactKind, GameMetadata, ImageSource, Release, ReleaseList};

use crate::info_hash;
use crate::sources::{ForumFacts, Forums};
use crate::topic::{CatalogFile, GameInfo, Scalar, Topic};

/// A text field, trimmed. Empty when it is missing.
fn text(value: &Option<String>) -> String {
    value.as_deref().unwrap_or("").trim().to_owned()
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
/// a build. A version may come without its "v" ("Amnesia: Rebirth (1.41)"), and a date is
/// written as one ("(15.01.2021)").
pub(crate) fn pc_name(name: &str) -> String {
    let name = name.split(" + ").next().unwrap_or(name).trim();
    // Digits, a dot, a digit: "1.11", "15.01.2021".
    let dotted = |s: &str| {
        let rest = s.trim_start_matches(|c: char| c.is_ascii_digit());
        rest.len() < s.len() && rest.strip_prefix('.').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
    };
    let is_tail = |group: &str| {
        let lower = group.to_ascii_lowercase();
        let year = lower.len() >= 5 && lower[..4].bytes().all(|b| b.is_ascii_digit()) && lower[4..].starts_with(')');
        let version = dotted(&lower)
            || lower
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
fn system(info: &GameInfo, title: &str) -> &'static str {
    let named = text(&info.platform).to_ascii_lowercase();
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

/// A fact written as text or as a number, as text. Empty when it is missing.
fn scalar(value: &Option<Scalar>) -> String {
    value.as_ref().map(Scalar::text).unwrap_or_default()
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

/// An `https://` address, or empty.
fn address(value: &Option<String>) -> String {
    Some(text(value)).filter(|url| url.starts_with("https://")).unwrap_or_default()
}

/// The items of a list of texts, trimmed, without the empty ones.
fn texts(items: &Option<Vec<String>>) -> Vec<String> {
    let items = items.iter().flatten();
    items.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// A topic's Steam data as a store's catalog gives it: Steam's words, the library portrait as
/// the cover, the hero art (else the header) as the background, and the store page. `None`
/// when it has no app id: Steam didn't have the game.
fn steam(topic: &Topic) -> Option<GameMetadata> {
    let steam = topic.steam.as_ref()?;
    let app_id = steam.appid?;
    let assets = steam.assets.clone().unwrap_or_default();
    let url = |value: &Option<String>| Some(address(value)).filter(|u| !u.is_empty()).map(ImageSource::Url);
    let (developers, publishers) = (texts(&steam.developers), texts(&steam.publishers));
    Some(GameMetadata {
        description: text(&steam.short),
        publisher: developers.first().or(publishers.first()).cloned().unwrap_or_default(),
        genres: texts(&steam.genres),
        release: steam_date(&text(&steam.release)).unwrap_or_default(),
        cover: url(&assets.portrait).or_else(|| url(&steam.header)),
        background: url(&assets.hero).or_else(|| url(&steam.header)),
        logo: url(&assets.logo),
        screenshots: texts(&steam.shots)
            .into_iter()
            .filter(|url| url.starts_with("https://"))
            .map(ImageSource::Url)
            .collect(),
        store_url: format!("https://store.steampowered.com/app/{app_id}/"),
        ..GameMetadata::default()
    })
}

/// ProtonDB's tier for a Windows game packed with Wine that Steam has, with its ProtonDB page.
/// `None` for any other game, and when ProtonDB has no tier.
fn protondb(topic: &Topic, wine_packed: bool) -> Option<Fact> {
    let steam = topic.steam.as_ref().filter(|_| wine_packed)?;
    let app_id = steam.appid?;
    let tier = Some(text(&steam.protondb).to_ascii_lowercase()).filter(|t| !t.is_empty())?;
    let url = format!("https://www.protondb.com/app/{app_id}");
    Some(Fact { url, ..Fact::new(FactKind::Compatibility, "ProtonDB", tier) })
}

/// Whether a topic is a Windows game packed with Wine: its own field says so, else its forum,
/// else a `[Wine]` tag in its title.
pub(crate) fn packed_with_wine(topic: &Topic, forum: Option<&ForumFacts>) -> bool {
    let title = text(&topic.title);
    topic.wine_packed.unwrap_or_else(|| forum.is_some_and(|f| f.wine) || tags(&title).iter().any(|t| t == "wine"))
}

/// Why a release may not install: a PS5 package, which only the console can install (KytyPS5
/// installs debug packages). `None` for any other.
fn install_warning(info: &GameInfo, title: &str, platform: &str) -> Option<String> {
    (platform == "PS5" && is_package(info, title))
        .then(|| "Retail PS5 packages can't be installed; pick a folder, an archive or an exFAT image".to_owned())
}

/// The topic's game facts, in the order the Game Hub lists them, and its genre words and
/// ProtonDB tier, which it shows elsewhere. Only the facts the topic has.
fn facts(topic: &Topic, info: &GameInfo, genre: String, wine_packed: bool) -> Vec<Fact> {
    let released =
        Some(scalar(&info.release_date)).filter(|d| !d.is_empty()).unwrap_or_else(|| scalar(&info.release_year));
    let facts = [
        (FactKind::Publisher, "Publisher", text(&info.publisher)),
        (FactKind::Developer, "Developer", text(&info.developer)),
        (FactKind::ReleaseDate, "Release date", released),
        (FactKind::Players, "Players", players(&text(&info.multiplayer))),
        (FactKind::Update, "Update", text(&info.release_update)),
        (FactKind::Format, "Format", text(&info.format)),
        (FactKind::Firmware, "Minimum firmware", text(&info.minimum_firmware)),
        (FactKind::InterfaceLanguages, "Interface languages", text(&info.interface_languages)),
        (FactKind::AudioLanguages, "Audio languages", text(&info.audio_languages)),
        (FactKind::Genre, "Genre", genre),
    ];
    let facts = facts.into_iter().filter(|(_, _, value)| !value.is_empty());
    facts.map(|(kind, label, value)| Fact::new(kind, label, value)).chain(protondb(topic, wine_packed)).collect()
}

/// A PKG release: its game info's format, or a tag in its title, says `pkg` or `fpkg`.
fn is_package(info: &GameInfo, title: &str) -> bool {
    let format = text(&info.format).to_ascii_lowercase();
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
fn platform(info: &GameInfo, title: &str, title_id: &str) -> String {
    let named = text(&info.platform).to_ascii_uppercase();
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
/// them: its console (`platform`: `PS5`, `PS4` or `PC`) and the system a PC release is for
/// (`system`). `None` for a field it doesn't have. Its `wine_packed` is read by
/// [`packed_with_wine`].
fn declared(topic: &Topic) -> (Option<String>, Option<String>) {
    let platform = text(&topic.platform).to_ascii_uppercase();
    let platform = ["PS5", "PS4", "PC"].contains(&platform.as_str()).then_some(platform);
    let system = Some(text(&topic.system).to_ascii_lowercase()).filter(|s| !s.is_empty());
    (platform, system)
}

/// Reads a catalog file. Any part that doesn't read fails the whole file. `forums` says what
/// the topics of each forum are for.
pub fn read(bytes: &[u8], forums: &Forums) -> Result<ReleaseList, String> {
    let file: CatalogFile = serde_json::from_slice(bytes).map_err(|e| format!("not a catalog file: {e}"))?;
    releases(&file, forums)
}

/// The releases of a catalog file. It fails when any part of the file doesn't read.
pub fn releases(file: &CatalogFile, forums: &Forums) -> Result<ReleaseList, String> {
    if file.complete != Some(true) {
        return Err("the catalog isn't complete: partial listings aren't read".into());
    }
    if file.translation.as_ref().and_then(|t| t.language.as_deref()) != Some("en") {
        return Err("the catalog isn't in English".into());
    }
    let topics = file.topics.as_ref().ok_or("the catalog has no topics list")?;
    if file.topic_count != Some(topics.len() as u64) {
        return Err("topic_count doesn't match the topics".into());
    }
    let file_forum = forums.named(file.source.as_deref(), file.platform.as_deref());
    let mut ids = HashSet::new();
    let mut hashes = HashSet::new();
    let mut releases = Vec::with_capacity(topics.len());
    for topic in topics {
        let id = topic.id.parse::<i64>().ok().filter(|id| *id > 0).ok_or("a topic has no valid id")?;
        if !ids.insert(id) {
            return Err(format!("topic {id} is listed twice"));
        }
        // A file of several forums names each topic's forum.
        let forum = forums.named(topic.source.as_deref(), topic.platform.as_deref()).or(file_forum);
        let Some(release) = release(id, topic, forum)? else { continue };
        // The same torrent in two topics: the first one stands.
        if hashes.insert(release.id.clone()) {
            releases.push(release);
        }
    }
    let updated = Some(text(&file.refreshed_at)).filter(|t| !t.is_empty()).unwrap_or_else(|| text(&file.collected_at));
    Ok(ReleaseList { releases, version: text(&file.version), updated, ..ReleaseList::default() })
}

/// The release of topic number `id`, from the forum `forum`, if the file or the topic names
/// one. `None` for a topic that isn't a game, or has nothing to download. It fails for a
/// topic with no game name.
///
/// What the topic is for comes from its own fields first, then from its forum, then from its
/// title's tags.
pub fn release(id: i64, topic: &Topic, forum: Option<&ForumFacts>) -> Result<Option<Release>, String> {
    let none = GameInfo::default();
    let info = topic.game_info.as_ref().unwrap_or(&none);
    let (own_platform, own_system) = declared(topic);
    let title = text(&topic.title);
    let named = text(&info.name);
    let name = clean_name(if named.is_empty() { &title } else { &named });
    if name.is_empty() {
        return Err(format!("topic {id} has no game name"));
    }
    let raw_id = text(&info.title_id);
    let mut all_ids = title_ids(&raw_id);
    if all_ids.is_empty() {
        all_ids = title_ids(&title).into_iter().take(1).collect();
    }
    let genre = text(&info.genre);
    let magnet = text(&topic.magnet);
    if !is_game(&title, &genre, &raw_id, &magnet) {
        return Ok(None);
    }
    // Nothing to download without a magnet link with an info hash.
    let Some(hash) = magnet.starts_with("magnet:?").then(|| info_hash(&magnet)).flatten() else { return Ok(None) };
    let title_id = all_ids.first().cloned().unwrap_or_default();
    let pc_forum = forum.filter(|f| f.console.as_str() == "PC");
    // A PC forum says what all its topics are for; a console forum fills in a topic that
    // doesn't say.
    let platform = own_platform.unwrap_or_else(|| match forum {
        Some(forum) if forum.console.as_str() == "PC" => "PC".to_owned(),
        Some(forum) => Some(platform(info, &title, &title_id))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| forum.console.as_str().to_owned()),
        None => platform(info, &title, &title_id),
    });
    let system = match (own_system, pc_forum) {
        _ if platform != "PC" => String::new(),
        (Some(own), _) => own,
        (None, Some(forum)) => forum.system.clone().unwrap_or_default(),
        (None, None) => system(info, &title).to_owned(),
    };
    let name = if platform == "PC" { pc_name(&name) } else { name };
    let added = [&topic.published_at, &topic.added_at, &topic.registered_at]
        .into_iter()
        .map(text)
        .find(|date| !date.is_empty())
        .unwrap_or_default();
    let wine_packed = packed_with_wine(topic, forum);
    Ok(Some(Release {
        id: hash,
        topic: id.to_string(),
        title_ids: all_ids.iter().skip(1).cloned().collect(),
        title_id,
        title: name,
        version: text(&info.version),
        region: text(&info.region),
        size_bytes: topic.size_bytes.or_else(|| size_bytes(&text(&topic.size))),
        added,
        description: text(&info.release_summary),
        torrent: magnet,
        metadata: steam(topic),
        install_warning: install_warning(info, &title, &platform),
        facts: facts(topic, info, genre, wine_packed),
        system,
        platform,
        seeders: topic.seeders,
        leechers: topic.leechers,
        counted_at: text(&topic.peers_observed),
        image_urls: texts(&info.image_urls).into_iter().filter(|url| url.starts_with("https://")).collect(),
        ..Release::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_api::serde_json::Value;

    /// Reads a catalog file with the forums of the bundled catalog sources.
    fn read(bytes: &[u8]) -> Result<ReleaseList, String> {
        super::read(bytes, &forums())
    }

    fn forums() -> Forums {
        crate::sources::read(crate::BUNDLED_SOURCES).unwrap().forums()
    }

    /// Whether each topic of a catalog file is packed with Wine, its forum found as [`releases`]
    /// finds it.
    fn wine(bytes: &[u8]) -> Vec<bool> {
        let file: CatalogFile = serde_json::from_slice(bytes).unwrap();
        let forums = forums();
        let file_forum = forums.named(file.source.as_deref(), file.platform.as_deref());
        let topics = file.topics.unwrap_or_default();
        let forum = |t: &Topic| forums.named(t.source.as_deref(), t.platform.as_deref()).or(file_forum);
        topics.iter().map(|t| packed_with_wine(t, forum(t))).collect()
    }

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
        let facts: Vec<(FactKind, &str, &str)> =
            astro.facts.iter().map(|f| (f.kind, f.label.as_str(), f.value.as_str())).collect();
        assert_eq!(
            facts,
            [
                (FactKind::Publisher, "Publisher", "Sony"),
                (FactKind::Developer, "Developer", "Team Asobi"),
                (FactKind::ReleaseDate, "Release date", "2024-09-06"),
                (FactKind::Players, "Players", "Single player"),
                (FactKind::Update, "Update", "v01.05"),
                (FactKind::Format, "Format", "exfat"),
                (FactKind::Firmware, "Minimum firmware", "5.xx"),
                (FactKind::InterfaceLanguages, "Interface languages", "English, Russian"),
                (FactKind::AudioLanguages, "Audio languages", "English"),
                (FactKind::Genre, "Genre", "Action, Platformer"),
            ],
            "in the Game Hub's order, a date before a year"
        );
        assert_eq!(astro.image_urls, ["https://i1.imageban.ru/out/cover.jpg"], "only https:// addresses");
        let bolt = &list.releases[1];
        assert_eq!((bolt.seeders, bolt.leechers, bolt.counted_at.as_str()), (None, None, ""), "no count");
        assert_eq!(
            (bolt.fact_text(FactKind::Players), bolt.fact_text(FactKind::ReleaseDate)),
            ("Multiplayer", "2020"),
            "a year as a number, when there is no date"
        );
        assert_eq!(list.releases[2].fact_text(FactKind::Players), "2x", "the catalog's own words");
    }

    #[test]
    fn a_release_written_before_the_game_facts_still_reads() {
        let old = serde_json::json!({ "id": "abc", "title": "Astro", "platform": "PS5", "torrent": MAGNET });
        let release: Release = serde_json::from_value(old).unwrap();
        assert_eq!((release.seeders, release.facts.as_slice()), (None, &[][..]));
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
        let bytes = file(serde_json::json!([
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
        ]));
        let list = read(&bytes).unwrap();
        assert_eq!(wine(&bytes), [true, true, false], "a native Linux game isn't packed with Wine");
        let celeste = &list.releases[0];
        let steam = celeste.metadata.as_ref().expect("the Steam data");
        let image = |file: &str| Some(ImageSource::Url(format!("{art}/{file}")));
        assert_eq!(steam.description, "Help Madeline survive.");
        assert_eq!(steam.genres, ["Action", "Indie"]);
        assert_eq!(steam.release, "2018-01-25", "Steam's date as YYYY-MM-DD");
        assert_eq!(steam.publisher, "Maddy Makes Games", "the developer first");
        assert_eq!(steam.screenshots, [image("ss_1.jpg").unwrap()], "only https:// addresses");
        assert_eq!(
            (steam.cover.clone(), steam.background.clone(), steam.logo.clone()),
            (image("library_600x900_2x.jpg"), image("library_hero.jpg"), None)
        );
        assert_eq!(steam.store_url, "https://store.steampowered.com/app/504230/");
        let tier = celeste.fact(FactKind::Compatibility).expect("ProtonDB's tier for a game packed with Wine");
        assert_eq!(
            (tier.label.as_str(), tier.value.as_str(), tier.url.as_str()),
            ("ProtonDB", "platinum", "https://www.protondb.com/app/504230")
        );
        assert_eq!(list.releases[1].metadata, None, "Steam has no such game");
        assert_eq!(list.releases[1].fact(FactKind::Compatibility), None);
        assert_eq!(list.releases[2].metadata, None);
        assert_eq!(steam_date("25 Jan, 2018").as_deref(), Some("2018-01-25"));
        assert_eq!(steam_date("Coming soon"), None);
    }

    /// A game Steam has that isn't packed with Wine has no ProtonDB tier; one with only a
    /// header has it as its cover and background.
    #[test]
    fn a_native_game_on_steam_has_no_protondb_tier() {
        let list = read(&file(serde_json::json!([
            { "id": "1", "title": "Tux Racer [Native]", "magnet": MAGNET, "game_info": {},
              "steam": { "appid": 7, "header": "https://x.example/header.jpg", "protondb": "gold" } },
        ])))
        .unwrap();
        let tux = &list.releases[0];
        assert_eq!(tux.fact(FactKind::Compatibility), None);
        let header = Some(ImageSource::Url("https://x.example/header.jpg".into()));
        let steam = tux.metadata.as_ref().unwrap();
        assert_eq!((steam.cover.clone(), steam.background.clone()), (header.clone(), header));
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
        let wine_file = forum("https://xyz.net/forum/viewforum.php?f=2059", "wine", topic("added_at"));
        let wine_list = read(&wine_file).unwrap();
        let r = &wine_list.releases[0];
        assert_eq!((r.platform.as_str(), r.system.as_str()), ("PC", "linux"));
        assert_eq!(wine(&wine_file), [true], "every release of the Wine forum is packed with Wine");
        assert_eq!(r.title, "Warcraft I: Remastered");
        assert_eq!(r.added, "2026-10-19T11:06:33+00:00");
        assert_eq!(r.size_bytes, Some(695_312_589), "the exact size, not the title's 663.1 MB");

        let native_file = forum("https://xyz.net/forum/viewforum.php?f=1992", "linux", topic("registered_at"));
        let native = read(&native_file).unwrap();
        let r = &native.releases[0];
        assert_eq!((r.platform.as_str(), r.system.as_str()), ("PC", "linux"));
        assert_eq!(wine(&native_file), [false], "a native Linux game");
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
        let bytes = file(serde_json::json!([topic("1", 'a', "1992"), topic("2", 'b', "2059")]));
        let list = read(&bytes).unwrap();
        let systems: Vec<_> = list.releases.iter().map(|r| (r.platform.as_str(), r.system.as_str())).collect();
        assert_eq!(systems, [("PC", "linux"), ("PC", "linux")]);
        assert_eq!(wine(&bytes), [false, true], "native Linux, then Wine");
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
        let bytes = file(serde_json::json!([
            // The fields win over the topic's forum (the PS5 forum) and over the title's tags.
            topic("1", 'a', "[PS4] Racer", serde_json::json!({ "platform": "PS4" })),
            topic("2", 'b', "Celeste [Wine]", pc(false)),
            topic("3", 'c', "Quake", pc(true)),
            // A field it doesn't have comes from the forum.
            topic("4", 'd', "Astro", serde_json::json!({})),
        ]));
        let list = read(&bytes).unwrap();
        let read: Vec<_> = list.releases.iter().map(|r| (r.platform.as_str(), r.system.as_str())).collect();
        assert_eq!(read, [("PS4", ""), ("PC", "linux"), ("PC", "linux"), ("PS5", "")]);
        assert_eq!(wine(&bytes), [false, false, true, false]);
    }

    #[test]
    fn a_ps5_pkg_release_says_it_can_t_install() {
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
            topic(5, "[PS4] Racer [EUR / pkg]", ""),
        ])))
        .unwrap();
        let warned: Vec<_> =
            list.releases.iter().map(|r| (r.title.as_str(), r.install_warning.as_deref().unwrap_or(""))).collect();
        let why = "Retail PS5 packages can't be installed; pick a folder, an archive or an exFAT image";
        assert_eq!(
            warned,
            [("Astro", why), ("Bolt", why), ("Comet", ""), ("Drift", ""), ("Racer", "")],
            "a PS5 package, by its format or its title; a PS4 package installs"
        );
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
        assert_eq!(pc_name("Trine 3: The Artifacts of Power (1.11 (build 3091))"), "Trine 3: The Artifacts of Power");
        assert_eq!(pc_name("Amnesia: Rebirth (1.41)"), "Amnesia: Rebirth");
        assert_eq!(pc_name("WRATH: Aeon of Ruin (15.01.2021)"), "WRATH: Aeon of Ruin", "a date");
        assert_eq!(pc_name("Worms 2 (2)"), "Worms 2 (2)", "a number alone is part of the name");
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
