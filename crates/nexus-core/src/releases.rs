//! The games the catalogs offer to download. A [`nexus_api::Releases`] plugin lists each
//! console's releases, and the first emulator that plays the console gets them. The core
//! groups them into one game (an offer) per title id or title without its edition, marks new
//! and installed ones, and shows an offer's Game Hub with its releases to download.

use std::collections::HashMap;

use nexus_api::{FactKind, GameKey, GameMetadata, GameSummary, ImageSource, PluginId, Release, ReleaseList};

use crate::library::Entry;

/// A release added this many days ago, or fewer, is new.
pub const NEW_DAYS: i64 = 14;

/// The catalog of the console an emulator plays, from one plugin.
#[derive(Clone, Debug)]
pub(crate) struct CatalogCopy {
    pub service: PluginId,
    pub emulator: PluginId,
    pub list: ReleaseList,
}

/// One game a catalog offers: its releases, newest first.
#[derive(Clone, Debug)]
pub(crate) struct Offer {
    pub entry: Entry,
    pub emulator: PluginId,
    pub releases: Vec<Release>,
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date (more after it is ignored).
pub(crate) fn days_of(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-');
    let (y, m, d): (i64, i64, i64) =
        (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// A date to sort by: `YYYY-MM-DD` for text that starts with a date, `YYYY` for text that holds
/// only a year (1900 to 2099). `None` for neither. Compared as text, a later date is greater.
pub(crate) fn date_key(text: &str) -> Option<String> {
    let text = text.trim();
    if days_of(text).is_some() {
        return text.get(..10).map(str::to_owned);
    }
    let year = |w: &&str| w.len() == 4 && (w.starts_with("19") || w.starts_with("20"));
    text.split(|c: char| !c.is_ascii_digit()).find(year).map(str::to_owned)
}

/// The day a release's game came out, to sort by (see [`date_key`]): the catalog's release
/// date or year, else the date in the release's store data.
pub(crate) fn release_day(release: &Release) -> Option<String> {
    date_key(release.fact_text(FactKind::ReleaseDate))
        .or_else(|| release.metadata.as_ref().and_then(|m| date_key(&m.release)))
}

/// A `YYYY-MM-DD` date in words, "Sep 6, 2024"; other text stays as it is.
pub(crate) fn date_words(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    if days_of(date).is_none() || date.len() != 10 {
        return date.to_owned();
    }
    let (year, month, day) =
        (&date[..4], date[5..7].parse::<usize>().unwrap_or(1), date[8..10].parse::<u32>().unwrap_or(1));
    format!("{} {day}, {year}", MONTHS[month - 1])
}

/// Today, in days since 1970-01-01.
pub(crate) fn today() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (secs / 86_400) as i64
}

pub(crate) fn is_new(added: &str, today: i64) -> bool {
    days_of(added).is_some_and(|day| (0..=NEW_DAYS).contains(&(today - day)))
}

/// How easy a release is to download, from how many people share it: `good` (3 or more),
/// `low` (1 or 2), `none` (0), or `unknown` when the catalog has no count.
pub(crate) fn availability(seeders: Option<u64>) -> &'static str {
    match seeders {
        None => "unknown",
        Some(0) => "none",
        Some(1..=2) => "low",
        Some(_) => "good",
    }
}

/// The genres the Catalog's chips name, with the words that put a game in each. A `.` is any
/// one character, `.?` one or none. "Other" is for a game whose genre words name none of them.
pub(crate) const GENRES: [(&str, &[&str]); 16] = [
    ("Action", &["action", "hack", "slash", "beat.?.?em", "brawler", "souls"]),
    ("Adventure", &["adventure", "narrative", "exploration", "open.world", "story"]),
    ("RPG", &["rpg", "role"]),
    ("Shooter", &["shoot", "fps", "gun"]),
    ("Horror", &["horror"]),
    ("Survival", &["survival"]),
    ("Racing", &["racing", "driving", "kart", "motor"]),
    (
        "Sports",
        &[
            "sport",
            "football",
            "soccer",
            "basketball",
            "golf",
            "tennis",
            "wrestling",
            "boxing",
            "skate",
            "baseball",
            "hockey",
            "cricket",
            "ufc",
        ],
    ),
    ("Fighting", &["fight", "martial"]),
    ("Platformer", &["platform", "metroidvania"]),
    ("Puzzle", &["puzzle"]),
    ("Simulation", &["simulat", "management", "farming", "sandbox", "builder", "life"]),
    ("Strategy", &["strateg", "tactic", "tower", "4x"]),
    ("Stealth", &["stealth"]),
    ("Family", &["party", "family", "casual", "kids", "music", "rhythm"]),
    // "vr" only as a word of its own: see `genres_in`.
    ("VR", &["virtual reality"]),
];

/// Whether `pattern` (see [`GENRES`]) matches the start of `text`.
fn starts_with_pattern(text: &[char], pattern: &[char]) -> bool {
    match pattern {
        [] => true,
        ['.', '?', rest @ ..] => {
            starts_with_pattern(text, rest) || (!text.is_empty() && starts_with_pattern(&text[1..], rest))
        }
        ['.', rest @ ..] => !text.is_empty() && starts_with_pattern(&text[1..], rest),
        [c, rest @ ..] => text.first() == Some(c) && starts_with_pattern(&text[1..], rest),
    }
}

/// The fixed genres that genre words name, such as a catalog's "Hack and Slash, RPG" and a
/// store's "Action": in the order of [`GENRES`], or "Other" when the words name none. Empty
/// when there are no words.
///
/// Each view asks for every offer's genres, and a catalog has few different genre texts, so
/// each text is matched once.
pub(crate) fn genres_in(texts: &[&str]) -> Vec<&'static str> {
    /// More texts than any catalog has; past it the answers start over.
    const MOST: usize = 16_384;
    static MATCHED: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Vec<&'static str>>>> =
        std::sync::LazyLock::new(Default::default);
    let text = texts.join(" | ").to_lowercase();
    let mut matched = MATCHED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(genres) = matched.get(&text) {
        return genres.clone();
    }
    let genres = match_genres(&text);
    if matched.len() >= MOST {
        matched.clear();
    }
    matched.insert(text, genres.clone());
    genres
}

/// [`genres_in`] of one lower-case text.
fn match_genres(text: &str) -> Vec<&'static str> {
    let text: Vec<char> = text.chars().collect();
    if !text.iter().any(|c| c.is_alphanumeric()) {
        return Vec::new();
    }
    let has = |pattern: &str| {
        let pattern: Vec<char> = pattern.chars().collect();
        (0..text.len()).any(|at| starts_with_pattern(&text[at..], &pattern))
    };
    let words: String = text.iter().collect();
    let vr = words.split(|c: char| !c.is_alphanumeric()).any(|w| w == "vr");
    let mut out: Vec<&'static str> = GENRES
        .iter()
        .filter(|(name, patterns)| patterns.iter().any(|p| has(p)) || (*name == "VR" && vr))
        .map(|(name, _)| *name)
        .collect();
    if out.is_empty() {
        out.push("Other");
    }
    out
}

/// A version as numbers to compare: its runs of digits, with the zeros at the end taken off.
/// `v01.05` is `[1, 5]`, and `1.05.000` is the same version.
pub(crate) fn version_key(version: &str) -> Vec<u64> {
    let mut parts: Vec<u64> = version.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect();
    while parts.last() == Some(&0) {
        parts.pop();
    }
    parts
}

/// The title as a grouping key: lowercase letters and digits only.
pub(crate) fn plain(title: &str) -> String {
    title.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// The words before `edition` that name an edition of the same game, as the old launcher's
/// grouping has them. `digital` may come before each.
const EDITIONS: [&[&str]; 10] = [
    &["deluxe"],
    &["ultimate"],
    &["standard"],
    &["premium"],
    &["gold"],
    &["complete"],
    &["collectors"],
    &["game", "of", "the", "year"],
    &["goty"],
    &["definitive"],
];

/// The title with its edition taken off, as a grouping key: "Racer: Digital Deluxe Edition"
/// and "Racer" are one game, "Racer 2" is not. "All DLC" goes too. An apostrophe joins words
/// ("collector's").
fn edition_free(title: &str) -> String {
    let words: Vec<String> = title
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2019}'))
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let has = |at: usize, seq: &[&str]| words.get(at..at + seq.len()).is_some_and(|w| w.iter().eq(seq));
    let mut kept = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let start = if words[i] == "digital" { i + 1 } else { i };
        let edition = EDITIONS.iter().find(|e| has(start, e) && has(start + e.len(), &["edition"]));
        if let Some(e) = edition {
            i = start + e.len() + 1;
        } else if has(i, &["all", "dlc"]) {
            i += 2;
        } else {
            kept.push(words[i].as_str());
            i += 1;
        }
    }
    plain(&kept.concat())
}

/// Groups one catalog's releases into offers: releases that share a title id, or a title
/// with its edition taken off on the same console, are one game. Newest release first in each,
/// newest offer first. A catalog file can hold releases for another console: a PS4 and a PS5
/// release of one name stay two games.
pub(crate) fn offers(copy: &CatalogCopy) -> Vec<Offer> {
    let releases = &copy.list.releases;
    // Union-find over the keys each release has.
    let mut parent: Vec<usize> = (0..releases.len()).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = root(parent, parent[i]);
        }
        parent[i]
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, r) in releases.iter().enumerate() {
        let ids = std::iter::once(&r.title_id).chain(&r.title_ids).filter(|t| !t.is_empty());
        let mut keys: Vec<String> = ids.map(|t| format!("id:{t}")).collect();
        let name = edition_free(&r.title);
        if !name.is_empty() {
            keys.push(format!("title:{}:{name}", r.platform.to_ascii_lowercase()));
        }
        for key in keys {
            match seen.get(&key) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    seen.insert(key, i);
                }
            }
        }
    }
    let mut groups: Vec<Vec<&Release>> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for (i, r) in releases.iter().enumerate() {
        let g = root(&mut parent, i);
        let at = *slot.entry(g).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[at].push(r);
    }
    let mut out: Vec<Offer> = groups
        .into_iter()
        .map(|mut group| {
            group.sort_by(|a, b| b.added.cmp(&a.added).then(a.title.cmp(&b.title)));
            let newest = group[0];
            let title_id = group.iter().map(|r| &r.title_id).find(|t| !t.is_empty()).cloned();
            let local = match &title_id {
                Some(t) => format!("{}/{t}", copy.emulator),
                None => format!("{}/title-{}", copy.emulator, plain(&newest.title)),
            };
            // The cover of the release's store data, else a release's first picture. A store's
            // art comes first.
            let store = group.iter().find_map(|r| r.metadata.as_ref()).and_then(|m| m.cover.clone());
            let picture = group.iter().find_map(|r| r.image_urls.first()).map(|url| ImageSource::Url(url.clone()));
            let cover = store.or(picture);
            Offer {
                entry: Entry {
                    key: GameKey::new(copy.service.clone(), local),
                    summary: GameSummary {
                        id: String::new(),
                        title: newest.title.clone(),
                        platform: newest.platform.clone(),
                        cover,
                        tags: Vec::new(),
                        release_year: None,
                        title_id,
                        content_id: None,
                    },
                },
                emulator: copy.emulator.clone(),
                releases: group.into_iter().cloned().collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.releases[0].added.cmp(&a.releases[0].added).then(a.entry.summary.title.cmp(&b.entry.summary.title))
    });
    // Two games of one title for two consoles: the second key names its console.
    let mut locals = std::collections::HashSet::new();
    for offer in &mut out {
        if !locals.insert(offer.entry.key.local.clone()) {
            let console = plain(&offer.entry.summary.platform);
            offer.entry.key.local = format!("{}-{console}", offer.entry.key.local);
            locals.insert(offer.entry.key.local.clone());
        }
        offer.entry.summary.id = offer.entry.key.local.clone();
    }
    out
}

/// How many releases the catalogs `now` have that the same plugin's catalog for the same
/// emulator `before` didn't, by topic (a torrent uploaded again is no new release), else by
/// id. A catalog that took the place of the bundled copy, or of none, adds none: only a
/// catalog read before from its address or a refresh counts.
pub(crate) fn added_releases(before: &[CatalogCopy], now: &[CatalogCopy]) -> usize {
    let name = |r: &Release| if r.topic.is_empty() { format!("id:{}", r.id) } else { format!("topic:{}", r.topic) };
    now.iter()
        .filter_map(|copy| {
            let old = before.iter().find(|b| b.service == copy.service && b.emulator == copy.emulator)?;
            if old.list.source == nexus_api::ReleaseSource::Bundled {
                return None;
            }
            let known: std::collections::HashSet<String> = old.list.releases.iter().map(name).collect();
            Some(copy.list.releases.iter().filter(|r| !known.contains(&name(r))).count())
        })
        .sum()
}

/// The store data of the offer's newest release that has some.
pub(crate) fn metadata_of(offer: &Offer) -> Option<&GameMetadata> {
    offer.releases.iter().find_map(|r| r.metadata.as_ref())
}

/// The compatibility tier of the offer's newest release that has one, and its page: empty for
/// none.
pub(crate) fn compatibility(offer: &Offer) -> (String, String) {
    match offer.releases.iter().find_map(|r| r.fact(FactKind::Compatibility)) {
        Some(tier) => (tier.value.clone(), tier.url.clone()),
        None => (String::new(), String::new()),
    }
}

/// A PC release's system as people read it: `Linux`, `Windows`, `macOS`; empty for none.
pub(crate) fn system_label(system: &str) -> &'static str {
    match system {
        "linux" => "Linux",
        "windows" => "Windows",
        "macos" => "macOS",
        _ => "",
    }
}

/// Why a release for `system` can't start on `here`, or empty when it can (or nobody says).
/// A Windows release runs everywhere: through Proton or Wine off Windows.
pub(crate) fn not_here(system: &str, here: Option<nexus_api::Platform>) -> String {
    use nexus_api::Platform;
    let (Some(here), label) = (here, system_label(system)) else { return String::new() };
    let fits = match system {
        "linux" => here == Platform::Linux,
        "macos" => here == Platform::Macos,
        _ => true,
    };
    if fits {
        return String::new();
    }
    let on = match here {
        Platform::Linux => "Linux",
        Platform::Macos => "a Mac",
        Platform::Windows => "Windows",
    };
    format!("This is a {label} release: it can't run on {on}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genre_words_map_to_the_fixed_genres() {
        assert_eq!(genres_in(&["Hack and Slash, Role-Playing"]), ["Action", "RPG"]);
        assert_eq!(genres_in(&["Beat 'em up", "Open World"]), ["Action", "Adventure"]);
        assert_eq!(genres_in(&["FPS / Survival Horror"]), ["Shooter", "Horror", "Survival"]);
        assert_eq!(genres_in(&["Kart Racing", "Party"]), ["Racing", "Family"]);
        assert_eq!(genres_in(&["PS VR"]), ["VR"], "a word on its own");
        assert_eq!(genres_in(&["Lavrov"]), ["Other"], "not part of a word");
        assert_eq!(genres_in(&["Card game"]), ["Other"], "words that name no genre");
        assert!(genres_in(&["", " "]).is_empty(), "no words, no genre");
        assert_eq!(GENRES.len(), 16);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert_eq!(version_key("v01.05"), [1, 5]);
        assert_eq!(version_key("1.05.000"), version_key("v1.5"));
        assert!(version_key("1.10") > version_key("1.9"));
        assert!(version_key("").is_empty());
    }

    #[test]
    fn a_date_reads_in_words() {
        assert_eq!(date_words("2024-09-06"), "Sep 6, 2024");
        assert_eq!(date_words("2024"), "2024");
        assert_eq!(date_words("2024-13-01"), "2024-13-01");
    }

    #[test]
    fn a_date_or_a_year_sorts_as_text() {
        assert_eq!(date_key("2024-09-06").as_deref(), Some("2024-09-06"));
        assert_eq!(date_key("2024-09-06T10:00:00Z").as_deref(), Some("2024-09-06"));
        assert_eq!(date_key("2020").as_deref(), Some("2020"));
        assert_eq!(date_key("Released in 1998, again in 2005").as_deref(), Some("1998"));
        assert_eq!(date_key("soon"), None);
        assert_eq!(date_key(""), None);
    }

    #[test]
    fn dates_count_days_from_1970() {
        assert_eq!(days_of("1970-01-01"), Some(0));
        assert_eq!(days_of("2026-10-18"), Some(20_744));
        assert_eq!(days_of("2026-10-18T10:00:00Z"), Some(20_744));
        assert_eq!(days_of("2026-13-01"), None);
        assert_eq!(days_of(""), None);
        assert!(is_new("2026-10-04", 20_744), "14 days ago");
        assert!(!is_new("2026-10-03", 20_744), "15 days ago");
        assert!(!is_new("", 20_744));
    }
}
