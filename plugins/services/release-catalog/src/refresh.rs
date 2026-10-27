//! The catalog refresh: brings a console's catalog file up to date from its forums, with no
//! topic pages read. The catalog collector's `refresh.mjs` and `api.mjs` do the same.
//!
//! Each forum gives two answers, at fixed addresses with the forum's `f=` number (see
//! [`feed_url`] and [`list_url`]):
//! - its Atom feed ([`FEED`], then `<forum>.atom`): the titles of the releases of the last 1 to
//!   4 weeks;
//! - its forum list ([`API`], then `<forum>`, the `pvc` file): every topic with a torrent, with
//!   its hash, size, peers and time of upload, and no title.
//!
//! ```json
//! { "format": { "topic_id": ["tor_status", "seeders", "reg_time", "tor_size_bytes", "…",
//!     "info_hash", "…", "leechers"] },
//!   "result": { "6912345": [2, 5, 1791381312, 13249974886, "…", "AB12…", "…", 9] },
//!   "update_time": 1791382052 }
//! ```
//!
//! The refresh of one forum:
//! - A feed release that the forum list has and the catalog doesn't is added. It has no game
//!   info: it takes the game's facts from another release of the same game (see
//!   [`borrow_details`]). Releases the launcher can't play are not added (see [`unplayable`]).
//! - Each release the forum list has gets its peers, size, time of upload and magnet link from
//!   it. A magnet link with another hash is replaced: the torrent was uploaded again.
//! - A release the forum list doesn't have was deleted or moved, and is dropped. When the list
//!   has under 80% of the forum's releases, the answer is bad and nothing is dropped. A release
//!   that another forum of the catalog lists moved there: it is kept, under that forum.
//!
//! A catalog of several forums (`pc.json`) names each topic's forum in its `source`. Each
//! forum's answers change only its own topics. A topic that names no forum of the catalog is
//! not changed. A forum that doesn't answer leaves its topics as they are.
//!
//! The refresh changes the typed catalog file of [`crate::topic`]; [`crate::format`] reads the
//! same model.

use std::collections::{HashMap, HashSet};

use nexus_api::Console;
use nexus_api::serde::Deserialize;
use nexus_api::serde_json::{self, Value};

use crate::format::{clean_name, pc_name};
use crate::topic::{BORROWED, CatalogFile, Topic};

/// The base address of the forum list API. The forum's number goes after it.
pub const API: &str = "https://api.rutracker.cc/v1/static/pvc/f/";

/// The base address of the Atom feed. The forum's number and `.atom` go after it.
pub const FEED: &str = "https://feed.rutracker.cc/atom/f/";

/// The address of the forum list of the forum numbered `number`.
pub fn list_url(number: &str) -> String {
    format!("{API}{number}")
}

/// The address of the Atom feed of the forum numbered `number`.
pub fn feed_url(number: &str) -> String {
    format!("{FEED}{number}.atom")
}

/// The tracker the forum's own magnet links announce to.
const TRACKER: &str = "http://bt3.t-ru.org/ann?magnet";

/// One release in a forum's feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The topic's number.
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    /// When it was posted or changed, as the feed says.
    pub updated: String,
}

/// A forum's list of topics with a torrent.
#[derive(Clone, Debug)]
pub struct ForumList {
    /// Each topic's row, by its number.
    rows: HashMap<String, Row>,
    /// When the list was made, as an RFC 3339 time.
    observed: String,
}

/// What a forum list says of one topic. A field is `None` when the row has it as `null`, or
/// the list has no such field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Row {
    /// The torrent's info hash, as the list writes it.
    hash: Option<String>,
    seeders: Option<u64>,
    leechers: Option<u64>,
    size_bytes: Option<u64>,
    /// When the torrent was uploaded, in seconds since 1970. `None` for 0.
    registered: Option<i64>,
}

impl ForumList {
    /// The row of topic `id`. `None` when the list doesn't have it.
    fn row(&self, id: &str) -> Option<&Row> {
        self.rows.get(id)
    }

    fn has(&self, id: &str) -> bool {
        self.row(id).is_some()
    }
}

/// One forum of a catalog, with what its topics are for.
#[derive(Clone, Debug)]
pub struct ForumRef {
    /// Its `f` number.
    pub number: String,
    /// Its first listing page. New topics name it in their `source`.
    pub url: String,
    /// The console of the catalog (`PS5`). New topics name it in their `platform`.
    pub console: Console,
    /// For a PC forum, the system its games are for.
    pub system: Option<String>,
    /// Its games are packed with Wine.
    pub wine: bool,
}

/// What a refresh changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub new: usize,
    pub removed: usize,
    /// New releases that took the game's facts from another release.
    pub borrowed: usize,
}

/// `text` with each character reference decoded once: `&amp;`, `&lt;`, `&gt;`, `&quot;`,
/// `&apos;`, and decimal (`&#38;`) or hex (`&#x26;`) ones. A reference it doesn't know stays as
/// it is.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let named = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let c = match &after[..end] {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                name => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16),
                        None => number.parse(),
                    };
                    code.ok().and_then(char::from_u32)
                }
            };
            c.map(|c| (c, end))
        });
        match named {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of an element's content: its CDATA sections as they are, the rest decoded once.
fn xml_text(content: &str) -> String {
    let mut out = String::new();
    let mut rest = content;
    while let Some(at) = rest.find("<![CDATA[") {
        out.push_str(&decode(&rest[..at]));
        let inner = &rest[at + 9..];
        let end = inner.find("]]>").unwrap_or(inner.len());
        out.push_str(&inner[..end]);
        rest = inner.get(end + 3..).unwrap_or_default();
    }
    out.push_str(&decode(rest));
    out
}

/// `html` with its tags taken out and its character references decoded.
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = rest[at..].find('>').map_or("", |end| &rest[at + end + 1..]);
    }
    out.push_str(rest);
    decode(&out)
}

/// The first `<tag …>…</tag>` or `<tag …/>` in `xml`, by its whole name: `<name>` and not
/// `<namespace>`. Its attributes, its content (empty for `<tag/>`), and the rest of `xml` after
/// it.
fn element<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let at = from + xml[from..].find(&open)?;
        let after = &xml[at + open.len()..];
        from = at + open.len();
        if !after.starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace()) {
            continue;
        }
        let end = after.find('>')?;
        let attributes = &after[..end];
        if let Some(attributes) = attributes.strip_suffix('/') {
            return Some((attributes, "", &after[end + 1..]));
        }
        let body = &after[end + 1..];
        let close = format!("</{tag}>");
        let stop = body.find(&close)?;
        return Some((attributes, &body[..stop], &body[stop + close.len()..]));
    }
}

/// The value of attribute `name` in an element's attributes, in single or double quotes,
/// decoded.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=').map(str::trim_start) else { continue };
        let Some(quote) = value.chars().next().filter(|q| *q == '"' || *q == '\'') else { continue };
        let value = &value[1..];
        return value.find(quote).map(|end| decode(&value[..end]));
    }
    None
}

/// The text of the first `<tag>` in `xml`, trimmed: an `html` one without its tags. Empty when
/// there is none.
fn pick(xml: &str, tag: &str) -> String {
    let Some((attributes, content, _)) = element(xml, tag) else { return String::new() };
    let text = xml_text(content);
    let text = if attribute(attributes, "type").as_deref() == Some("html") { html_text(&text) } else { text };
    text.trim().to_owned()
}

/// The `href` of the first `<link>` in `entry` that has one.
fn link(entry: &str) -> Option<String> {
    let mut rest = entry;
    while let Some((attributes, _, after)) = element(rest, "link") {
        if let Some(href) = attribute(attributes, "href").filter(|h| !h.is_empty()) {
            return Some(href);
        }
        rest = after;
    }
    None
}

/// The topic number in a topic address: the digits after `?t=` or `&t=`.
fn topic_number(url: &str) -> Option<String> {
    let at = url.match_indices("t=").find(|(at, _)| *at > 0 && matches!(url.as_bytes()[at - 1], b'?' | b'&'))?.0;
    let digits: String = url[at + 2..].chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// The releases of an Atom feed, in its order (the newest first). Fails only when the answer
/// is not a feed: a feed with no entries reads.
pub fn parse_feed(xml: &str) -> Result<Vec<Entry>, String> {
    if !xml.contains("<feed") {
        return Err("not an Atom feed".into());
    }
    let mut entries = Vec::new();
    let mut rest = xml;
    while let Some((_, entry, after)) = element(rest, "entry") {
        rest = after;
        let Some(id) = link(entry).and_then(|l| topic_number(&l)) else { continue };
        let title = pick(entry, "title").split_whitespace().collect::<Vec<_>>().join(" ");
        // "[Обновлено]" marks a release uploaded again. It is not part of the title.
        const UPDATED: &str = "[обновлено]";
        let title = match title.get(..UPDATED.len()) {
            Some(head) if head.to_lowercase() == UPDATED => title[UPDATED.len()..].trim_start().to_owned(),
            _ => title,
        };
        if title.is_empty() {
            continue;
        }
        let author = Some(pick(entry, "name")).filter(|a| !a.is_empty());
        entries.push(Entry { id, title, author, updated: pick(entry, "updated") });
    }
    Ok(entries)
}

/// A forum list answer, as the API writes it. A part with another type reads as missing.
#[derive(Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct Answer {
    #[serde(default, deserialize_with = "crate::topic::lenient")]
    format: Option<AnswerFormat>,
    /// Each topic's row: its fields in the order [`AnswerFormat::topic_id`] names them.
    #[serde(default, deserialize_with = "crate::topic::lenient")]
    result: Option<HashMap<String, Value>>,
    #[serde(default, deserialize_with = "crate::topic::lenient")]
    update_time: Option<i64>,
}

#[derive(Deserialize)]
#[serde(crate = "nexus_api::serde")]
struct AnswerFormat {
    /// The name of each field of a row, in order.
    #[serde(default, deserialize_with = "crate::topic::lenient")]
    topic_id: Option<Vec<Value>>,
}

/// Reads a forum list answer.
pub fn parse_forum_list(bytes: &[u8]) -> Result<ForumList, String> {
    let answer: Answer = serde_json::from_slice(bytes).map_err(|e| format!("not a forum list: {e}"))?;
    let fields = answer.format.and_then(|f| f.topic_id).ok_or("not a forum list: no format.topic_id")?;
    let fields = fields.iter().map(|f| f.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>();
    let fields = fields.ok_or("not a forum list: a field name is not text")?;
    let result = answer.result.ok_or("not a forum list: no result")?;
    let time = answer.update_time.ok_or("not a forum list: no update_time")?;
    let at = |name: &str| fields.iter().position(|f| f == name);
    let (hash, seeders, leechers, size, registered) =
        (at("info_hash"), at("seeders"), at("leechers"), at("tor_size_bytes"), at("reg_time"));
    let rows = result
        .into_iter()
        .filter_map(|(id, row)| {
            let Value::Array(row) = row else { return None };
            let field = |at: Option<usize>| at.and_then(|at| row.get(at)).filter(|v| !v.is_null());
            let row = Row {
                hash: field(hash).and_then(Value::as_str).map(str::to_owned),
                seeders: field(seeders).and_then(Value::as_u64),
                leechers: field(leechers).and_then(Value::as_u64),
                size_bytes: field(size).and_then(Value::as_u64),
                registered: field(registered).and_then(Value::as_i64).filter(|t| *t != 0),
            };
            Some((id, row))
        })
        .collect();
    Ok(ForumList { rows, observed: iso_time(time) })
}

/// Console releases the launcher can't play, by the first `[…]` group of their title: games
/// that need a VR headset ("[PS VR Only]", "[PS4 VR]", "[PS VR+PS Move]"), PS2, PS1, PSP and
/// other old re-releases ("[PS4 PS2 Classics]", "[PSP-PS4]"), and homebrew apps. The catalog
/// collector has the same rule (`UNPLAYABLE` in `platforms.mjs`).
pub fn unplayable(title: &str) -> bool {
    const WORDS: [&str; 12] =
        ["vr", "move", "homebrew", "classic", "classics", "classis", "psp", "psx", "ps2", "ps1", "snes", "saturn"];
    let Some(group) = title.trim_start().strip_prefix('[') else { return false };
    let Some((group, _)) = group.split_once(']') else { return false };
    group
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| WORDS.iter().any(|w| word.eq_ignore_ascii_case(w)))
}

/// A name in a form to compare: lowercase, letters and digits only, roman numbers as digits,
/// and no edition words. The catalog collector's `normalize` in `steam.mjs` does the same.
pub fn normalize(name: &str) -> String {
    const SKIP: [&str; 16] = [
        "the",
        "a",
        "an",
        "edition",
        "definitive",
        "deluxe",
        "complete",
        "goty",
        "remastered",
        "enhanced",
        "collection",
        "directors",
        "cut",
        "game",
        "of",
        "year",
    ];
    let roman = |w: &str| {
        match w {
            "ii" => "2",
            "iii" => "3",
            "iv" => "4",
            "v" => "5",
            "vi" => "6",
            "vii" => "7",
            "viii" => "8",
            "ix" => "9",
            "x" => "10",
            other => other,
        }
        .to_owned()
    };
    // Apostrophes join their word: "Director's" is one word.
    let text = name.to_lowercase().replace(['™', '®', '©', '\'', '’'], "").replace('&', " and ");
    let words = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(roman);
    words.filter(|w| !SKIP.contains(&w.as_str())).collect::<Vec<_>>().join(" ")
}

/// Releases with no game info (new ones) take the game's facts from another release of the
/// same game that has them: its title ID, genre, developer and cover (see
/// [`crate::topic::GameInfo::game_facts`]). They are marked [`BORROWED`], so the catalog
/// collector still reads their own details. The number of releases that took facts.
pub fn borrow_details(topics: &mut [Topic]) -> usize {
    let key = |topic: &Topic| normalize(&pc_name(&clean_name(topic.title.as_deref().unwrap_or(""))));
    let mut donors = HashMap::new();
    for topic in topics.iter() {
        let Some(info) = &topic.game_info else { continue };
        if info.title_id.as_deref().is_some_and(|id| !id.is_empty())
            && topic.game_info_status.as_deref() != Some(BORROWED)
        {
            donors.entry(key(topic)).or_insert_with(|| (topic.id.clone(), info.game_facts()));
        }
    }
    let mut borrowed = 0;
    for topic in topics.iter_mut() {
        if topic.game_info.is_some() {
            continue;
        }
        let Some((donor, facts)) = donors.get(&key(topic)) else { continue };
        let mut info = facts.clone();
        info.source_url = topic.url.clone();
        info.borrowed_from = Some(donor.clone());
        topic.game_info = Some(info);
        topic.game_info_status = Some(BORROWED.to_owned());
        borrowed += 1;
    }
    borrowed
}

/// A size as the forum writes it: `6.17 GB` (a GB is 2^30 bytes).
fn size_label(bytes: f64) -> Option<String> {
    const TB: f64 = 1_099_511_627_776.0;
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    if !bytes.is_finite() || bytes <= 0.0 {
        return None;
    }
    let (unit, scale) = if bytes >= TB {
        ("TB", TB)
    } else if bytes >= GB {
        ("GB", GB)
    } else {
        ("MB", MB)
    };
    Some(format!("{:.2} {unit}", bytes / scale))
}

/// The bytes in a size label (`11.88 GB`, or TB, MB, KB; a comma for the point).
fn label_bytes(label: &str) -> Option<f64> {
    let label = label.replace('\u{a0}', " ");
    let bytes = label.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let number = |b: u8| b.is_ascii_digit() || b == b'.' || b == b',';
        if !number(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && number(bytes[i]) {
            i += 1;
        }
        let unit = label[i..].trim_start();
        let power = match unit.get(..2).map(str::to_ascii_uppercase).as_deref() {
            Some("KB") => 10,
            Some("MB") => 20,
            Some("GB") => 30,
            Some("TB") => 40,
            _ => continue,
        };
        let value: f64 = label[start..i].replacen(',', ".", 1).parse().ok()?;
        return Some(value * 2f64.powi(power));
    }
    None
}

/// `secs` since 1970 as an RFC 3339 time in UTC: `2026-10-07T14:07:32Z`.
pub fn iso_time(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest / 60 % 60, rest % 60)
}

/// A time in seconds since 1970 in words, in UTC: "Oct 21, 2026 at 14:05 UTC".
pub fn time_words(secs: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let iso = iso_time(secs);
    let month = iso[5..7].parse::<usize>().unwrap_or(1).clamp(1, 12);
    let day = iso[8..10].parse::<u32>().unwrap_or(1);
    format!("{} {day}, {} at {} UTC", MONTHS[month - 1], &iso[..4], &iso[11..16])
}

/// The `f` number in a forum address. `None` when there is none, or it is not all digits.
pub fn forum_number(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or_default();
    let number = query.split('&').find_map(|part| part.strip_prefix("f="))?;
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then(|| number.to_owned())
}

/// `https://host[:port]` of an address.
fn origin(url: &str) -> String {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("https://{authority}")
}

/// The info hash in a magnet link, uppercase, as it is written: the letters and digits after
/// `urn:btih:`.
fn magnet_hash(magnet: &str) -> Option<String> {
    let at = magnet.to_ascii_lowercase().find("urn:btih:")?;
    let hash: String = magnet[at + 9..].chars().take_while(char::is_ascii_alphanumeric).collect();
    (!hash.is_empty()).then(|| hash.to_ascii_uppercase())
}

/// Fills a topic's peers, size, time of upload and magnet link from its row in the list. A
/// magnet link with another hash is replaced: the torrent was uploaded again. A field the row
/// doesn't have changes nothing.
fn apply_row(topic: &mut Topic, row: &Row, observed: &str) {
    let hash = row.hash.as_deref().unwrap_or("").to_ascii_uppercase();
    let valid = hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    let own = topic.magnet.take().filter(|m| !m.is_empty());
    let replaced = valid && own.as_deref().and_then(magnet_hash).as_deref() != Some(hash.as_str());
    let magnet = match own {
        Some(own) if !replaced => Some(own),
        _ if valid => Some(format!("magnet:?xt=urn:btih:{hash}&tr={}", encode(TRACKER))),
        _ => None,
    };
    topic.seeders = row.seeders.or(topic.seeders);
    topic.leechers = row.leechers.or(topic.leechers);
    topic.size_bytes = row.size_bytes.or(topic.size_bytes);
    if let Some(time) = row.registered {
        topic.registered_at = Some(iso_time(time));
    }
    topic.peers_observed = Some(observed.to_owned());
    topic.magnet_status = match (&magnet, topic.magnet_status.take()) {
        (Some(_), _) => Some("available".to_owned()),
        (None, None) => Some("not_found".to_owned()),
        (None, status) => status,
    };
    topic.magnet = magnet;
}

/// `text` as a URI component, as JavaScript's `encodeURIComponent` writes it.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The forum number a topic names in its `source`, else the one the file names.
fn named_forum(topic: &Topic, file_forum: Option<&str>) -> Option<String> {
    topic.source.as_deref().and_then(forum_number).or_else(|| file_forum.map(str::to_owned))
}

/// Refreshes one forum's topics in `topics`. `single` is true when the catalog has only this
/// forum: then a topic that names no forum is its topic.
fn refresh_forum(
    topics: &mut Vec<Topic>,
    file_forum: Option<&str>,
    single: bool,
    forum: &ForumRef,
    feed: &[Entry],
    list: &ForumList,
    now: &str,
) -> Counts {
    let mine = |topic: &Topic| match named_forum(topic, file_forum) {
        Some(number) => number == forum.number,
        None => single,
    };
    let mut known: HashSet<String> = topics.iter().map(|t| t.id.clone()).collect();
    let mut counts = Counts::default();
    for entry in feed.iter().filter(|e| !unplayable(&e.title)) {
        if known.contains(&entry.id) || !list.has(&entry.id) {
            continue;
        }
        known.insert(entry.id.clone());
        let pc = forum.system.is_some();
        topics.push(Topic {
            id: entry.id.clone(),
            title: Some(entry.title.clone()),
            url: Some(format!("{}/forum/viewtopic.php?t={}", origin(&forum.url), entry.id)),
            author: entry.author.clone(),
            magnet_status: Some("not_checked".to_owned()),
            game_info_status: Some("not_collected".to_owned()),
            added_at: Some(if entry.updated.is_empty() { now } else { &entry.updated }.to_owned()),
            platform: Some(forum.console.as_str().to_owned()),
            source: Some(forum.url.clone()),
            system: forum.system.clone(),
            wine_packed: pc.then_some(forum.wine),
            ..Topic::default()
        });
        counts.new += 1;
    }
    let total = topics.iter().filter(|t| mine(t)).count();
    let listed = topics.iter().filter(|t| mine(t) && list.has(&t.id)).count();
    if listed as f64 >= total as f64 * 0.8 {
        topics.retain(|t| !mine(t) || list.has(&t.id));
        counts.removed = total - listed;
    }
    for topic in topics.iter_mut().filter(|t| mine(t)) {
        let Some(row) = list.row(&topic.id) else { continue };
        apply_row(topic, row, &list.observed);
        // The list's byte count wins: a label that doesn't match it (an upload of another size)
        // is replaced. A label that matches keeps the forum's own words.
        let bytes = topic.size_bytes.unwrap_or(0) as f64;
        let shown = topic.size.as_deref().and_then(label_bytes);
        if bytes > 0.0 && !shown.is_some_and(|s| s > 0.0 && (s - bytes).abs() <= bytes * 0.01) {
            topic.size = size_label(bytes);
        }
    }
    counts
}

/// Moves each topic that its own forum's list doesn't have, and another forum's list has, to
/// that forum: its `source`, and its `system` and `wine_packed` for a PC forum. So a topic that
/// moved from one forum of the catalog to another is kept, under its new forum.
fn move_topics(topics: &mut [Topic], file_forum: Option<&str>, forums: &[(ForumRef, Vec<Entry>, ForumList)]) {
    for topic in topics.iter_mut() {
        let Some(own) = named_forum(topic, file_forum) else { continue };
        let Some((_, _, own_list)) = forums.iter().find(|(f, ..)| f.number == own) else { continue };
        if own_list.has(&topic.id) {
            continue;
        }
        let Some((to, ..)) = forums.iter().find(|(f, _, list)| f.number != own && list.has(&topic.id)) else {
            continue;
        };
        topic.source = Some(to.url.clone());
        if let Some(system) = &to.system {
            topic.system = Some(system.clone());
            topic.wine_packed = Some(to.wine);
        }
    }
}

/// Refreshes a catalog file with the feed and forum list of each of its forums that answered.
/// `forum_count` is the number of forums the catalog has, answered or not: a forum that didn't
/// answer leaves its topics as they are. `now` is in seconds since 1970. Sets the file's
/// `topic_count` and `refreshed_at`.
pub fn refresh(
    file: &mut CatalogFile,
    forums: &[(ForumRef, Vec<Entry>, ForumList)],
    forum_count: usize,
    now: i64,
) -> Result<Counts, String> {
    let now = iso_time(now);
    let file_forum = file.source.as_deref().and_then(forum_number);
    let mut all = file.topics.take().ok_or("the catalog has no topics list")?;
    move_topics(&mut all, file_forum.as_deref(), forums);
    let mut counts = Counts::default();
    for (forum, feed, list) in forums {
        let one = refresh_forum(&mut all, file_forum.as_deref(), forum_count == 1, forum, feed, list, &now);
        counts.new += one.new;
        counts.removed += one.removed;
    }
    counts.borrowed = borrow_details(&mut all);
    if let Some(list) = forums.last().map(|(_, _, l)| l) {
        file.api_observed = Some(list.observed.clone());
    }
    file.topic_count = Some(all.len() as u64);
    file.topics = Some(all);
    file.refreshed_at = Some(now);
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = include_str!("../fixtures/forum-546.atom");
    const PVC: &str = include_str!("../fixtures/pvc-546.json");

    fn forum() -> ForumRef {
        ForumRef {
            number: "546".into(),
            url: "https://xyz.net/forum/viewforum.php?f=546".into(),
            console: Console::new("PS5").unwrap(),
            system: None,
            wine: false,
        }
    }

    fn topic(id: &str, title: &str, hash: char) -> Value {
        serde_json::json!({ "id": id, "title": title, "size": "1.00 GB",
            "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "magnet_status": "available",
            "game_info": { "genre": "Action" } })
    }

    fn file(topics: Vec<Value>) -> Value {
        serde_json::json!({ "source": "https://xyz.net/forum/viewforum.php?f=546", "complete": true,
            "translation": { "language": "en" }, "topic_count": topics.len(), "topics": topics })
    }

    fn by_id(file: &Value, id: &str) -> Value {
        file["topics"].as_array().unwrap().iter().find(|t| t["id"] == id).cloned().unwrap_or(Value::Null)
    }

    /// [`refresh`] of the catalog file `file`, written as JSON: the file is read into the model,
    /// refreshed, and written back.
    fn refresh_json(
        file: &mut Value,
        forums: &[(ForumRef, Vec<Entry>, ForumList)],
        forum_count: usize,
        now: i64,
    ) -> Result<Counts, String> {
        let mut typed: CatalogFile = serde_json::from_value(file.clone()).unwrap();
        let counts = refresh(&mut typed, forums, forum_count, now)?;
        *file = serde_json::to_value(&typed).unwrap();
        Ok(counts)
    }

    /// [`borrow_details`] of topics written as JSON.
    fn borrow_json(topics: &mut Vec<Value>) -> usize {
        let mut typed: Vec<Topic> = serde_json::from_value(Value::from(topics.clone())).unwrap();
        let borrowed = borrow_details(&mut typed);
        *topics = typed.iter().map(|t| serde_json::to_value(t).unwrap()).collect();
        borrowed
    }

    #[test]
    fn the_feed_reads_its_releases_in_order() {
        let entries = parse_feed(FEED).unwrap();
        let read: Vec<(&str, &str)> = entries.iter().map(|e| (e.id.as_str(), e.title.as_str())).collect();
        assert_eq!(
            read,
            [
                ("6912345", "[PS5] Astro [EUR / RUS / ENG / v01.10 / Лицензия] [12.34 GB]"),
                ("6912346", "[PS5 VR] Moss & Friends [EUR]"),
                ("6900002", "[PS5] Bolt [EUR / v1.02]"),
            ]
        );
        assert_eq!(entries[0].author.as_deref(), Some("uploader"));
        assert_eq!(entries[0].updated, "2026-10-07T13:55:12+00:00");
        assert!(parse_feed("<feed></feed>").unwrap().is_empty(), "a feed with no entries reads");
        assert!(parse_feed("<html>Temporarily disabled</html>").is_err());
    }

    #[test]
    fn the_feed_decodes_its_text_and_skips_entries_without_a_topic() {
        let xml = r#"<feed><entry><link href="https://xyz.net/forum/viewtopic.php?f=1&amp;t=7"/>
            <title>  A &#38; B &#x26;   C &lt;D&gt;</title></entry>
            <entry><link href="https://xyz.net/forum/viewforum.php?f=7"/><title>Forum</title></entry>
            <entry><link href="https://xyz.net/forum/viewtopic.php?t=8"/><title></title></entry></feed>"#;
        let entries = parse_feed(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].id.as_str(), entries[0].title.as_str()), ("7", "A & B & C <D>"));
        assert_eq!(entries[0].author, None);
    }

    /// The (id, title, author) of each entry of `xml`.
    fn read_feed(xml: &str) -> Vec<(String, String, Option<String>)> {
        parse_feed(xml).unwrap().into_iter().map(|e| (e.id, e.title, e.author)).collect()
    }

    #[test]
    fn the_feed_decodes_each_character_reference_once() {
        let xml = r#"<feed><entry><link href="https://xyz.net/forum/viewtopic.php?t=7"/>
            <title>A &amp;#38;lt; B &#38;lt; C</title></entry></feed>"#;
        assert_eq!(read_feed(xml)[0].1, "A &#38;lt; B &lt; C");
    }

    #[test]
    fn the_feed_reads_an_entry_with_attributes_and_a_link_in_single_quotes() {
        let xml = r#"<feed><entry xml:lang="ru"><link rel='alternate' href='https://xyz.net/forum/viewtopic.php?t=8&amp;x=1'/>
            <title>Game</title></entry><entryx><link href="https://xyz.net/forum/viewtopic.php?t=9"/>
            <title>Not an entry</title></entryx></feed>"#;
        assert_eq!(read_feed(xml), [("8".to_owned(), "Game".to_owned(), None)]);
    }

    #[test]
    fn the_feed_matches_tag_names_whole() {
        let xml = r#"<feed><entry><link href="https://xyz.net/forum/viewtopic.php?t=7"/>
            <titlex>Not the title</titlex><title>Game</title>
            <author><namespace>x</namespace><name>uploader</name></author></entry></feed>"#;
        assert_eq!(read_feed(xml), [("7".to_owned(), "Game".to_owned(), Some("uploader".to_owned()))]);
    }

    #[test]
    fn an_html_title_loses_its_tags() {
        let xml = r#"<feed><entry><link href="https://xyz.net/forum/viewtopic.php?t=7"/>
            <title type="html">&lt;b&gt;Bold&lt;/b&gt; Game &amp;amp; Co</title></entry>
            <entry><link href="https://xyz.net/forum/viewtopic.php?t=8"/>
            <title type='html'><![CDATA[<i>Ital</i> &amp; More]]></title></entry></feed>"#;
        let titles: Vec<String> = read_feed(xml).into_iter().map(|(_, title, _)| title).collect();
        assert_eq!(titles, ["Bold Game & Co", "Ital & More"]);
    }

    #[test]
    fn the_forum_list_reads_its_rows_by_field_name() {
        let list = parse_forum_list(PVC.as_bytes()).unwrap();
        let row = list.row("6900001").unwrap();
        assert_eq!(row.seeders, Some(42));
        assert_eq!(row.hash.as_deref(), Some("C12FE1C06BBA254A9DC9F519B335AA7C1367A88A"));
        assert_eq!(list.row("6900004").unwrap().leechers, None, "null is not a value");
        assert_eq!(list.observed, "2026-10-07T14:07:32Z");
        assert!(!list.has("6900005"));
        assert!(parse_forum_list(b"{ not json").is_err());
        assert!(parse_forum_list(br#"{ "result": {} }"#).is_err(), "no format");
    }

    #[test]
    fn a_refresh_adds_updates_and_drops_releases() {
        let mut catalog = file(vec![
            topic("6900001", "[PS5] Astro [EUR]", 'x'),
            topic("6900002", "[PS5] Bolt [EUR]", '2'),
            topic("6900003", "[PS5] Comet [EUR]", '3'),
            topic("6900004", "[PS5] Drift [EUR]", '4'),
            topic("6900005", "[PS5] Echo [EUR]", '5'),
        ]);
        catalog["topics"][0]["magnet"] = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&tr=x".into();
        let answers = [(forum(), parse_feed(FEED).unwrap(), parse_forum_list(PVC.as_bytes()).unwrap())];
        let counts = refresh_json(&mut catalog, &answers, 1, 1_791_400_000).unwrap();
        // 6912346 is VR only, 6900002 is known, 6900005 is gone from the list.
        assert_eq!(counts, Counts { new: 1, removed: 1, borrowed: 0 });
        assert_eq!(catalog["topic_count"], 5);
        assert_eq!(catalog["refreshed_at"], "2026-10-07T19:06:40Z");
        assert_eq!(by_id(&catalog, "6900005"), Value::Null);
        assert_eq!(by_id(&catalog, "6912346"), Value::Null, "a VR game is left out");

        let astro = by_id(&catalog, "6900001");
        assert_eq!(astro["magnet"], "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&tr=x", "same hash");
        assert_eq!((&astro["seeders"], &astro["leechers"]), (&42.into(), &3.into()));
        assert_eq!(astro["size"], "1.00 GB", "a label that matches the bytes stays");
        assert_eq!(astro["registered_at"], iso_time(1_790_000_000));
        let bolt = by_id(&catalog, "6900002");
        assert_eq!(
            bolt["magnet"],
            format!("magnet:?xt=urn:btih:{}&tr=http%3A%2F%2Fbt3.t-ru.org%2Fann%3Fmagnet", "D2".repeat(20))
        );
        assert_eq!(bolt["size"], "2.00 GB");
        assert_eq!(bolt["title"], "[PS5] Bolt [EUR]", "the catalog's title stays");
        assert_eq!(by_id(&catalog, "6900004")["leechers"], Value::Null, "a null in the list changes nothing");

        let new = by_id(&catalog, "6912345");
        assert_eq!(new["title"], "[PS5] Astro [EUR / RUS / ENG / v01.10 / Лицензия] [12.34 GB]");
        assert_eq!(new["size"], "12.34 GB");
        assert_eq!(
            (&new["magnet_status"], &new["seeders"], &new["leechers"]),
            (&"available".into(), &5.into(), &9.into())
        );
        assert_eq!(new["url"], "https://xyz.net/forum/viewtopic.php?t=6912345");
        assert_eq!((&new["author"], &new["added_at"]), (&"uploader".into(), &"2026-10-07T13:55:12+00:00".into()));
        assert_eq!(new["game_info"], Value::Null, "Astro here has no title id to borrow");
    }

    #[test]
    fn a_list_that_has_under_80_percent_of_the_releases_drops_nothing() {
        let topics: Vec<Value> = (1..=5).map(|i| topic(&format!("690000{i}"), "[PS5] Game", 'a')).collect();
        let mut catalog = file(topics);
        let mut answer: Value = serde_json::from_str(PVC).unwrap();
        answer["result"].as_object_mut().unwrap().retain(|id, _| id == "6900001");
        let answers = [(forum(), Vec::new(), parse_forum_list(answer.to_string().as_bytes()).unwrap())];
        assert_eq!(refresh_json(&mut catalog, &answers, 1, 0).unwrap().removed, 0);
        assert_eq!(catalog["topic_count"], 5);
    }

    #[test]
    fn each_forum_changes_only_its_own_topics() {
        let linux = "https://xyz.net/forum/viewforum.php?f=1992";
        let wine = "https://xyz.net/forum/viewforum.php?f=2059";
        let mut native = topic("1", "Celeste", 'a');
        native["source"] = linux.into();
        let mut packed = topic("2", "Quake [Wine]", 'b');
        packed["source"] = wine.into();
        let mut catalog = file(vec![native, packed]);
        catalog["source"] = Value::Null;
        let list = format!(
            r#"{{ "format": {{ "topic_id": ["seeders", "info_hash"] }}, "update_time": 0,
               "result": {{ "1": [4, "{}"], "3": [1, "{}"] }} }}"#,
            "A".repeat(40),
            "C".repeat(40)
        );
        let feed = r#"<feed><entry><link href="https://xyz.net/forum/viewtopic.php?t=3"/><title>Tux Racer</title></entry></feed>"#;
        let forum = ForumRef {
            number: "1992".into(),
            url: linux.into(),
            console: Console::new("PC").unwrap(),
            system: Some("linux".into()),
            wine: false,
        };
        let answers = [(forum, parse_feed(feed).unwrap(), parse_forum_list(list.as_bytes()).unwrap())];
        let counts = refresh_json(&mut catalog, &answers, 2, 0).unwrap();
        assert_eq!(counts, Counts { new: 1, removed: 0, borrowed: 0 });
        assert_eq!(by_id(&catalog, "1")["seeders"], 4);
        assert_eq!(by_id(&catalog, "2")["seeders"], Value::Null, "the Wine forum's topic is not changed");
        let tux = by_id(&catalog, "3");
        assert_eq!(
            (&tux["platform"], &tux["system"], &tux["wine_packed"]),
            (&"PC".into(), &"linux".into(), &false.into())
        );
        assert_eq!(tux["source"], linux);
    }

    #[test]
    fn with_a_forum_that_did_not_answer_a_topic_that_names_no_forum_is_not_changed() {
        let linux = "https://xyz.net/forum/viewforum.php?f=1992";
        let mut topics: Vec<Value> = (1..=5)
            .map(|i| {
                let mut native = topic(&i.to_string(), "Celeste", 'a');
                native["source"] = linux.into();
                native
            })
            .collect();
        // The file names no forum, and neither does this topic: it may be the other forum's.
        topics.push(topic("9", "Quake [Wine]", 'b'));
        let mut catalog = file(topics);
        catalog["source"] = Value::Null;
        let rows: Vec<String> = (1..=5).map(|i| format!(r#""{i}": [4, "{}"]"#, "A".repeat(40))).collect();
        let list = format!(
            r#"{{ "format": {{ "topic_id": ["seeders", "info_hash"] }}, "update_time": 0, "result": {{ {} }} }}"#,
            rows.join(", ")
        );
        let forum = ForumRef {
            number: "1992".into(),
            url: linux.into(),
            console: Console::new("PC").unwrap(),
            system: Some("linux".into()),
            wine: false,
        };
        // The Wine forum of the catalog failed: only the Linux forum answered.
        let answers = [(forum, Vec::new(), parse_forum_list(list.as_bytes()).unwrap())];
        let counts = refresh_json(&mut catalog, &answers, 2, 0).unwrap();
        assert_eq!(counts.removed, 0, "not the Linux forum's topic, so not dropped");
        assert_eq!(by_id(&catalog, "1")["seeders"], 4);
        assert_eq!(by_id(&catalog, "9")["title"], "Quake [Wine]");
    }

    #[test]
    fn a_topic_that_moved_to_another_forum_of_the_catalog_is_kept_under_it() {
        let linux = "https://xyz.net/forum/viewforum.php?f=1992";
        let wine = "https://xyz.net/forum/viewforum.php?f=2059";
        let topics: Vec<Value> = (1..=6)
            .map(|i| {
                let mut native = topic(&i.to_string(), "Celeste", 'a');
                native["source"] = linux.into();
                native["system"] = "linux".into();
                native["wine_packed"] = false.into();
                native
            })
            .collect();
        let mut catalog = file(topics);
        catalog["source"] = Value::Null;
        let list = |rows: &[(u32, u32)]| {
            let rows: Vec<String> = rows.iter().map(|(id, seeders)| format!(r#""{id}": [{seeders}, null]"#)).collect();
            let list = format!(
                r#"{{ "format": {{ "topic_id": ["seeders", "info_hash"] }}, "update_time": 0, "result": {{ {} }} }}"#,
                rows.join(", ")
            );
            parse_forum_list(list.as_bytes()).unwrap()
        };
        let forum = |number: &str, url: &str, packed: bool| ForumRef {
            number: number.into(),
            url: url.into(),
            console: Console::new("PC").unwrap(),
            system: Some("linux".into()),
            wine: packed,
        };
        // Topic 2 moved from the Linux forum to the Wine forum. The Linux forum comes first.
        let answers = [
            (forum("1992", linux, false), Vec::new(), list(&[(1, 4), (3, 4), (4, 4), (5, 4), (6, 4)])),
            (forum("2059", wine, true), Vec::new(), list(&[(2, 9)])),
        ];
        let counts = refresh_json(&mut catalog, &answers, 2, 0).unwrap();
        assert_eq!(counts, Counts { new: 0, removed: 0, borrowed: 0 });
        let moved = by_id(&catalog, "2");
        assert_eq!((&moved["source"], &moved["wine_packed"]), (&wine.into(), &true.into()), "{moved}");
        assert_eq!(moved["seeders"], 9, "the Wine forum's list applies to it");
    }

    #[test]
    fn a_release_without_details_borrows_the_game_facts_of_another_release_of_the_same_game() {
        let donor = serde_json::json!({ "id": "6910307",
            "title": "[PS5] Little Nightmares Enhanced Complete Edition [EUR / RUS / ENG / v01.004.000 / Retail / FPKG]",
            "game_info_status": "available",
            "game_info": { "title_id": "PPSA10737", "genre": "Horror, Puzzle", "developer": "Tarsier Studios",
                "image_urls": ["https://example.org/cover.jpg"], "version": "v01.004.000", "region": "EUR" } });
        let fresh = serde_json::json!({ "id": "6921989",
            "title": "[PS5] Little Nightmares Enhanced Complete Edition [RUS / EUR / 01.004 / Retail / exfat] [10.1 GB]",
            "url": "https://xyz.net/forum/viewtopic.php?t=6921989", "game_info": null,
            "game_info_status": "not_collected" });
        let other = serde_json::json!({ "id": "1", "title": "[PS5] Little Nightmares II [EUR]", "game_info": null });
        let mut topics = vec![donor, fresh, other];
        assert_eq!(borrow_json(&mut topics), 1);
        assert_eq!(
            topics[1]["game_info"],
            serde_json::json!({ "source_url": "https://xyz.net/forum/viewtopic.php?t=6921989", "borrowed_from": "6910307",
                "title_id": "PPSA10737", "genre": "Horror, Puzzle", "developer": "Tarsier Studios",
                "image_urls": ["https://example.org/cover.jpg"] })
        );
        assert_eq!(topics[1]["game_info_status"], "borrowed");
        assert_eq!(topics[2]["game_info"], Value::Null, "another game");
        assert_eq!(borrow_json(&mut topics), 0, "only once");
    }

    #[test]
    fn a_refreshed_release_reads_with_its_peers_and_the_game_facts_it_borrowed() {
        // Astro's release from before has the game's facts; the feed's new Astro release has none.
        let mut donor = topic("6900001", "[PS5] Astro [EUR]", 'x');
        donor["magnet"] = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&tr=x".into();
        donor["game_info"] = serde_json::json!({ "name": "Astro", "title_id": "PPSA01234", "genre": "Action, Platformer",
            "developer": "Team Asobi", "publisher": "Sony", "release_year": "2024", "multiplayer": "No",
            "image_urls": ["https://i1.imageban.ru/out/astro.jpg"], "version": "v01.05", "format": "exfat" });
        let mut catalog: CatalogFile = serde_json::from_value(file(vec![
            donor,
            topic("6900002", "[PS5] Bolt [EUR]", '2'),
            topic("6900003", "[PS5] Comet [EUR]", '3'),
            topic("6900004", "[PS5] Drift [EUR]", '4'),
        ]))
        .unwrap();
        let answers = [(forum(), parse_feed(FEED).unwrap(), parse_forum_list(PVC.as_bytes()).unwrap())];
        assert_eq!(refresh(&mut catalog, &answers, 1, 1_791_400_000).unwrap().borrowed, 1);

        // The refreshed model reads as it is, with no file written and read again.
        let forums = crate::sources::read(crate::BUNDLED_SOURCES).unwrap().forums();
        let list = crate::format::releases(&catalog, &forums).unwrap();
        let fresh = list.releases.iter().find(|r| r.version.is_empty() && r.title == "Astro").expect("the new release");
        assert_eq!((fresh.seeders, fresh.leechers), (Some(5), Some(9)), "the forum list's counts");
        assert_eq!(fresh.counted_at, "2026-10-07T14:07:32Z");
        use nexus_api::FactKind;
        assert_eq!((fresh.title_id.as_str(), fresh.fact_text(FactKind::Genre)), ("PPSA01234", "Action, Platformer"));
        assert_eq!(
            (fresh.fact_text(FactKind::Developer), fresh.fact_text(FactKind::Publisher)),
            ("Team Asobi", "Sony")
        );
        assert_eq!(
            (fresh.fact_text(FactKind::ReleaseDate), fresh.fact_text(FactKind::Players)),
            ("2024", "Single player")
        );
        assert_eq!(fresh.image_urls, ["https://i1.imageban.ru/out/astro.jpg"]);
        assert_eq!(fresh.fact(FactKind::Format), None, "the format is about one upload: not borrowed");
        let astro = list.releases.iter().find(|r| r.version == "v01.05").unwrap();
        assert_eq!((astro.seeders, astro.leechers), (Some(42), Some(3)));
    }

    #[test]
    fn releases_the_launcher_cannot_play_are_left_out_by_their_tag() {
        for title in [
            "[PS VR Only] GORN [USA]",
            "[PS4 VR] Dreams",
            "[PS VR+PS Move] Job Simulator",
            "[PS4 Exclusive VR Only] Blood & Truth",
            "[PS4 PS2 Classics] God of War [NTSC-U]",
            "[PS4 PS2 Classis] Star Wars",
            "[PSP-PS4] LocoRoco",
            "[PS4 PSX Classics] Ape Escape",
            "[PS4 homebrew] Store",
        ] {
            assert!(unplayable(title), "{title}");
        }
        for title in [
            "[PS4] Firewatch [EUR]",
            "[PS4 Exclusive] God of War III (3) Remastered [EUR]",
            "[PS4 Exclusive PlayLink] That's You!",
            "My Friend Peppa Pig [EUR/RUS]",
            "[PS5] Ghost of Tsushima",
            "[PS4] Vampyr [EUR]",
            "[PS4] Moving Out [EUR]",
            "[PS5] [PSVR2 only] Horizon",
        ] {
            assert!(!unplayable(title), "{title}");
        }
    }

    #[test]
    fn names_sizes_and_times_read_as_the_collector_reads_them() {
        assert_eq!(normalize("Age of Empires II: Definitive Edition™"), "age empires 2");
        assert_eq!(normalize("Director's Cut & More"), "and more");
        assert_eq!(label_bytes("11.88\u{a0}GB"), Some(11.88 * 1_073_741_824.0));
        assert_eq!(label_bytes("1,5 TB"), Some(1.5 * 1_099_511_627_776.0));
        assert_eq!(label_bytes("512 KB"), Some(512.0 * 1024.0));
        assert_eq!(label_bytes("v1.02 big"), None);
        assert_eq!(size_label(6_624_000_000.0).as_deref(), Some("6.17 GB"));
        assert_eq!(size_label(0.0), None);
        assert_eq!(iso_time(1_791_382_052), "2026-10-07T14:07:32Z");
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(time_words(1_791_382_052), "Oct 7, 2026 at 14:07 UTC");
        assert_eq!(forum_number("https://xyz.net/forum/viewforum.php?f=546").as_deref(), Some("546"));
        assert_eq!(forum_number("https://xyz.net/forum/viewforum.php?x=1&f=2059#top").as_deref(), Some("2059"));
        assert_eq!(forum_number("https://xyz.net/forum/"), None);
        assert_eq!(forum_number("https://xyz.net/forum/viewforum.php?f=1/../x"), None, "digits only");
        assert_eq!(list_url("546"), "https://api.rutracker.cc/v1/static/pvc/f/546");
        assert_eq!(feed_url("546"), "https://feed.rutracker.cc/atom/f/546.atom");
        assert_eq!(encode("http://bt3.t-ru.org/ann?magnet"), "http%3A%2F%2Fbt3.t-ru.org%2Fann%3Fmagnet");
    }
}
