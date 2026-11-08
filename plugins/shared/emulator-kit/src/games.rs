//! Installed console games in a named host folder: folders with `sce_sys/param.json` (PS5)
//! or `sce_sys/param.sfo` (PS4), and Switch game files (see [`switch_file`]).
//!
//! Each of the player's folders is searched three folders down, for layouts like
//! `Games/PS5/<Title>/<PPSAxxxxx-app0>`.

use std::sync::Mutex;

use nexus_api::serde_json::{self, Value};
use nexus_api::{DirEntry, Folders, HostPath, PluginError, PluginResult, RelPath};

use crate::sfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Console {
    Ps5,
    Ps4,
    /// Switch and Switch 2 games, each one file.
    Switch,
}

/// The extensions of a Switch game file: a package (`.nsp`), a cartridge image (`.xci`),
/// their compressed forms (`.nsz`, `.xcz`), and a Switch 2 package (`.nup`).
pub const SWITCH_FILES: [&str; 5] = ["nsp", "nsz", "xci", "xcz", "nup"];

/// Whether `name` is a Switch game file of any kind: a game, an update or DLC.
pub fn is_switch_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| SWITCH_FILES.iter().any(|s| e.eq_ignore_ascii_case(s)))
}

/// The Switch game in the file `name`, from the name alone: its files are encrypted, and their
/// keys are the player's. A name like `Title [0100ABCDEF012000][v131072].nsp` gives the title,
/// the title id and the version. `None` for another kind of file, and for an update or DLC: a
/// base game's title id ends in `000`, a tag that says "DLC" is never the game (a standalone app
/// such as an art book has a game's title id), and a name without a title id says "update",
/// "upd" or "dlc".
pub fn switch_file(name: &str) -> Option<(String, String, String)> {
    if !is_switch_file(name) {
        return None;
    }
    let (stem, _) = name.rsplit_once('.')?;
    let groups: Vec<&str> = stem.split('[').skip(1).filter_map(|g| g.split_once(']').map(|(g, _)| g.trim())).collect();
    let title_id = groups
        .iter()
        .find(|g| g.len() == 16 && g.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|g| g.to_ascii_uppercase())
        .unwrap_or_default();
    if !title_id.is_empty() && !title_id.ends_with("000") {
        return None;
    }
    let words = |text: &str| -> Vec<String> {
        text.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).map(str::to_owned).collect()
    };
    // A tag that says DLC: a release's standalone apps (an art book) have a game's title id.
    if groups.iter().any(|g| words(g).iter().any(|w| w == "dlc")) {
        return None;
    }
    if title_id.is_empty() && words(stem).iter().any(|w| matches!(w.as_str(), "update" | "upd" | "dlc")) {
        return None;
    }
    let version = groups
        .iter()
        .find(|g| g.len() > 1 && g[..1].eq_ignore_ascii_case("v") && g[1..].chars().all(|c| c.is_ascii_digit()))
        .map(|g| g.to_ascii_lowercase())
        .unwrap_or_default();
    let title = stem.split('[').next().unwrap_or(stem).trim();
    let title = if title.is_empty() { stem.trim() } else { title };
    Some((title.to_owned(), title_id, version))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundGame {
    /// The title id, with `-2`, `-3`… for more copies of the same game.
    pub id: String,
    pub title: String,
    pub title_id: String,
    pub content_id: String,
    pub version: String,
    /// The age the game is rated for, from `ageLevel.default` (PS5 only).
    pub age: Option<u8>,
    /// The game's folder: what its size counts and an uninstall deletes. For a Switch game, the
    /// folder that holds it alone with its update and DLC (an install's `Switch/<title>`), else
    /// its file.
    pub dir: HostPath,
    /// The file a Switch game starts. `None` for a game that is a folder.
    pub file: Option<HostPath>,
    pub icon: Option<HostPath>,
    pub background: Option<HostPath>,
}

/// The most a metadata file may be.
const MAX_PARAM: u64 = 1024 * 1024;

async fn read(folders: &dyn Folders, path: &HostPath) -> Option<Vec<u8>> {
    let bytes = folders.read_range(path, 0, MAX_PARAM + 1).await.ok()?;
    (bytes.len() as u64 <= MAX_PARAM).then_some(bytes)
}

async fn file_in(folders: &dyn Folders, dir: &HostPath, names: &[&str]) -> Option<HostPath> {
    for name in names {
        let path = dir.join(name).ok()?;
        if folders.exists(&path).await.unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

fn folder_name(dir: &HostPath) -> String {
    dir.path.file_name().unwrap_or("game").to_owned()
}

/// A PS5 game's title id and name, from its `param.json`. The title id must be letters and
/// digits, since it names the game's folder.
pub fn ps5_param(bytes: &[u8]) -> Option<(String, String)> {
    let p: Value = serde_json::from_slice(bytes).ok()?;
    let title_id = p["titleId"].as_str().filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))?;
    let localized = &p["localizedParameters"];
    let language = localized["defaultLanguage"].as_str().unwrap_or("en-US");
    let title =
        localized[language]["titleName"].as_str().or(localized["en-US"]["titleName"].as_str()).unwrap_or(title_id);
    Some((title_id.to_owned(), title.to_owned()))
}

/// The game in `dir`, if it holds one for `console`.
pub async fn read_game(folders: &dyn Folders, console: Console, dir: &HostPath) -> Option<FoundGame> {
    let sce = dir.join("sce_sys").ok()?;
    match console {
        // A Switch game is a file: see `scan`.
        Console::Switch => None,
        Console::Ps5 => {
            let p: Value = serde_json::from_slice(&read(folders, &sce.join("param.json").ok()?).await?).ok()?;
            let localized = &p["localizedParameters"];
            let language = localized["defaultLanguage"].as_str().unwrap_or("en-US");
            let title_id = p["titleId"].as_str().unwrap_or_default().to_owned();
            let title = localized[language]["titleName"]
                .as_str()
                .or(localized["en-US"]["titleName"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| if title_id.is_empty() { folder_name(dir) } else { title_id.clone() });
            Some(FoundGame {
                id: String::new(),
                title,
                content_id: p["contentId"].as_str().unwrap_or_default().to_owned(),
                version: p["contentVersion"].as_str().unwrap_or_default().to_owned(),
                age: p["ageLevel"]["default"].as_u64().and_then(|a| u8::try_from(a).ok()).filter(|a| *a > 0),
                title_id,
                dir: dir.clone(),
                file: None,
                icon: file_in(folders, &sce, &["icon0.png"]).await,
                background: file_in(folders, &sce, &["pic0.png"]).await,
            })
        }
        Console::Ps4 => {
            let p = sfo::parse(&read(folders, &sce.join("param.sfo").ok()?).await?)?;
            let get = |k: &str| p.get(k).map(|v| v.text().to_owned()).unwrap_or_default();
            // Only full games: updates ("gp") and add-ons have a param.sfo too.
            if !get("CATEGORY").starts_with("gd") {
                return None;
            }
            let title_id = get("TITLE_ID");
            let title = Some(get("TITLE"))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| if title_id.is_empty() { folder_name(dir) } else { title_id.clone() });
            Some(FoundGame {
                id: String::new(),
                title,
                content_id: get("CONTENT_ID"),
                version: get("APP_VER"),
                age: None,
                title_id,
                dir: dir.clone(),
                file: None,
                icon: file_in(folders, &sce, &["icon0.png"]).await,
                background: file_in(folders, &sce, &["pic1.png", "pic0.png"]).await,
            })
        }
    }
}

/// How far below a game folder a game is looked for: a console folder (`PS5`), a folder of
/// the player's, then the game.
const SCAN_DEPTH: usize = 3;

/// The Switch games among the files of `dir`, at `depth` below the root. When `dir` is at least
/// two folders down (`Switch/<title>`) and holds one game, it is that game's folder; else each
/// game is its own file.
fn switch_games(dir: &HostPath, depth: usize, files: &[DirEntry]) -> PluginResult<Vec<FoundGame>> {
    let games: Vec<(&DirEntry, (String, String, String))> =
        files.iter().filter_map(|f| switch_file(&f.name).map(|g| (f, g))).collect();
    let own_folder = depth >= 2 && games.len() == 1;
    let mut out = Vec::new();
    for (entry, (title, title_id, version)) in games {
        let file = dir.join(&entry.name)?;
        out.push(FoundGame {
            id: String::new(),
            title,
            title_id,
            content_id: String::new(),
            version,
            age: None,
            dir: if own_folder { dir.clone() } else { file.clone() },
            file: Some(file),
            icon: None,
            background: None,
        });
    }
    Ok(out)
}

/// Every game for `console` in the folder named `folder`, sorted by title. A game folder may
/// hold games of other consoles too: each console's games are told apart by their files. An
/// install's work folder (`<name>.part`) is never read.
pub async fn scan(folders: &dyn Folders, folder: &str, console: Console) -> PluginResult<Vec<FoundGame>> {
    let mut found: Vec<FoundGame> = Vec::new();
    for root in folders.roots(folder).await? {
        let mut level = vec![HostPath::new(folder, root.root, RelPath::root())];
        for depth in 0..=SCAN_DEPTH {
            let mut next = Vec::new();
            for dir in level {
                if let Some(game) = read_game(folders, console, &dir).await {
                    found.push(game);
                    continue;
                }
                // A Switch game is a file, so the deepest folders are listed too.
                if depth == SCAN_DEPTH && console != Console::Switch {
                    continue;
                }
                // A root that can't be listed (not there yet) holds no games.
                let entries = folders.list(&dir).await.unwrap_or_default();
                let entries: Vec<DirEntry> = entries.into_iter().filter(|e| !e.name.starts_with('.')).collect();
                if depth < SCAN_DEPTH {
                    for child in entries.iter().filter(|e| e.is_dir && !e.name.ends_with(".part")) {
                        next.push(dir.join(&child.name)?);
                    }
                }
                if console == Console::Switch {
                    let files: Vec<DirEntry> = entries.into_iter().filter(|e| !e.is_dir).collect();
                    found.extend(switch_games(&dir, depth, &files)?);
                }
            }
            level = next;
        }
    }
    found.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.dir.cmp(&b.dir)));
    let mut taken = std::collections::HashSet::new();
    for game in &mut found {
        let base = if game.title_id.is_empty() { game.title.clone() } else { game.title_id.clone() };
        let mut id = base.clone();
        let mut n = 1;
        while !taken.insert(id.clone()) {
            n += 1;
            id = format!("{base}-{n}");
        }
        game.id = id;
    }
    Ok(found)
}

/// The games last found, by id. A miss scans again: the host may ask about a game before it
/// lists them (after a restart, for example).
pub struct GameIndex {
    pub folder: &'static str,
    pub console: Console,
    games: Mutex<Vec<FoundGame>>,
    /// Each game's size on disk, by folder, worked out once.
    sizes: Mutex<std::collections::HashMap<HostPath, u64>>,
}

impl GameIndex {
    pub fn new(folder: &'static str, console: Console) -> Self {
        Self { folder, console, games: Mutex::new(Vec::new()), sizes: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<FoundGame>> {
        self.games.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Scans again and returns every game.
    pub async fn refresh(&self, folders: &dyn Folders) -> PluginResult<Vec<FoundGame>> {
        let games = scan(folders, self.folder, self.console).await?;
        *self.lock() = games.clone();
        Ok(games)
    }

    pub async fn find(&self, folders: &dyn Folders, id: &str) -> PluginResult<FoundGame> {
        if let Some(game) = self.lock().iter().find(|g| g.id == id).cloned() {
            return Ok(game);
        }
        self.refresh(folders).await?.into_iter().find(|g| g.id == id).ok_or_else(|| PluginError::NotFound(id.into()))
    }

    /// The game's details, with its size on disk.
    pub async fn details(
        &self,
        folders: &dyn Folders,
        id: &str,
        platform: &str,
    ) -> PluginResult<nexus_api::GameDetails> {
        let game = self.find(folders, id).await?;
        let cached = self.sizes.lock().unwrap_or_else(|p| p.into_inner()).get(&game.dir).copied();
        let size = match cached {
            Some(size) => size,
            None => {
                let size = folders.size(&game.dir).await.unwrap_or(0);
                self.sizes.lock().unwrap_or_else(|p| p.into_inner()).insert(game.dir.clone(), size);
                size
            }
        };
        let mut details = game.details(platform);
        details.size_bytes = (size > 0).then_some(size);
        Ok(details)
    }
}

/// The region a content id was published for, from its first two letters.
pub fn region(content_id: &str) -> Option<&'static str> {
    Some(match content_id.get(..2)? {
        "UP" => "North America",
        "EP" => "Europe",
        "JP" => "Japan",
        "HP" => "Asia",
        "KP" => "Korea",
        _ => return None,
    })
}

impl FoundGame {
    /// The catalog card, with the game's own icon as its cover.
    pub fn summary(&self, platform: &str) -> nexus_api::GameSummary {
        nexus_api::GameSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            platform: platform.to_owned(),
            cover: self.icon.clone().map(nexus_api::ImageSource::Folder),
            tags: Vec::new(),
            release_year: None,
            title_id: (!self.title_id.is_empty()).then(|| self.title_id.clone()),
            content_id: (!self.content_id.is_empty()).then(|| self.content_id.clone()),
        }
    }

    /// The details the game's files hold. Its size is [`GameIndex::details`]'s.
    pub fn details(&self, platform: &str) -> nexus_api::GameDetails {
        use nexus_api::GameFact;
        let mut facts = Vec::new();
        if let Some(region) = region(&self.content_id) {
            facts.push(GameFact::new("region", "Region", region));
        }
        if let Some(age) = self.age {
            facts.push(GameFact::new("age", "Age rating", format!("{age}+")));
        }
        if !self.version.is_empty() {
            facts.push(GameFact::new("version", "Version", &self.version));
        }
        if !self.title_id.is_empty() {
            facts.push(GameFact::new("title-id", "Title ID", &self.title_id));
        }
        if !self.content_id.is_empty() {
            facts.push(GameFact::new("content-id", "Content ID", &self.content_id));
        }
        if let Some(file) = &self.file {
            facts.push(GameFact::new("file", "File", file.path.as_str()));
        }
        if self.file.as_ref() != Some(&self.dir) {
            facts.push(GameFact::new("folder", "Folder", self.dir.path.as_str()));
        }
        nexus_api::GameDetails {
            folder: Some(self.dir.clone()),
            also: Vec::new(),
            uninstall_link: None,
            summary: self.summary(platform),
            // A game's files have no description.
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: self.background.clone().map(nexus_api::ImageSource::Folder),
            facts,
        }
    }
}

/// Games whose title or title id contains `query`, title matches first, at most `limit`.
pub fn search(games: Vec<FoundGame>, query: &nexus_api::SearchQuery) -> Vec<FoundGame> {
    let needle = query.text.to_lowercase();
    let mut hits: Vec<FoundGame> = games
        .into_iter()
        .filter(|g| g.title.to_lowercase().contains(&needle) || g.title_id.to_lowercase().contains(&needle))
        .collect();
    hits.sort_by_key(|g| !g.title.to_lowercase().starts_with(&needle));
    hits.truncate(query.limit as usize);
    hits
}

/// A badge for a result on `platform`: "In-game on macOS", or "In-game on Linux · untested on
/// macOS" when the result comes from another system. Its short form, for cards: "In-game", or
/// "Linux · In-game".
pub fn compat_badge(info: &nexus_api::CompatInfo, platform: Option<nexus_api::Platform>) -> nexus_api::SlotContent {
    let (level, tone) = level_badge(info.level);
    let here = crate::compat::os_name(platform);
    let (label, short) = match info.source.as_deref() {
        _ if info.level == nexus_api::CompatLevel::Unknown => (level.to_owned(), level.to_owned()),
        Some(from) if from == here => (format!("{level} on {here}"), level.to_owned()),
        Some(from) => {
            let os = match from {
                "Windows" => "Win",
                "macOS" => "Mac",
                other => other,
            };
            (format!("{level} on {from} · untested on {here}"), format!("{os} · {level}"))
        }
        None => (level.to_owned(), level.to_owned()),
    };
    nexus_api::SlotContent::Badge { label, short, tone, icon: None }
}

/// A badge for a compatibility level.
fn level_badge(level: nexus_api::CompatLevel) -> (&'static str, nexus_api::Tone) {
    use nexus_api::{CompatLevel, Tone};
    match level {
        CompatLevel::Playable => ("Playable", Tone::Success),
        CompatLevel::Ingame => ("In-game", Tone::Success),
        CompatLevel::Menus => ("Menus", Tone::Warning),
        CompatLevel::Boots => ("Boots", Tone::Warning),
        CompatLevel::Nothing => ("Doesn't start", Tone::Danger),
        CompatLevel::Unknown => ("Not tested", Tone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_file_s_name_gives_its_title_id_and_version_and_updates_are_not_games() {
        let game = switch_file;
        let owned = |t: &str, id: &str, v: &str| Some((t.to_owned(), id.to_owned(), v.to_owned()));
        assert_eq!(game("Celeste [01002b30028f6000][v0].nsp"), owned("Celeste", "01002B30028F6000", "v0"));
        assert_eq!(game("Kirby Air Riders [NUP].NUP"), owned("Kirby Air Riders", "", ""));
        assert_eq!(game("Detention.xcz"), owned("Detention", "", ""));
        assert_eq!(
            game("Dark Souls II (2) [0100ABCDEF012000][US][v65536].xci"),
            owned("Dark Souls II (2)", "0100ABCDEF012000", "v65536")
        );
        assert_eq!(game("Celeste [01002B30028F6800][v262144].nsp"), None, "an update");
        assert_eq!(game("Celeste DLC [01002B30028F7001][v0].nsp"), None, "an add-on");
        let art_book =
            "Sonic x Shadow Generation [Standalone DLC Digital Art Book] [010021301E9A4000][v0] (0.48 GB).nsz";
        assert_eq!(game(art_book), None, "a standalone app the release calls DLC");
        assert_eq!(game("Kirby Air Riders Update 1.1.nsp"), None, "an update by its name");
        assert_eq!(game("readme.txt"), None);
        assert_eq!(game("nsp"), None);
    }
}
