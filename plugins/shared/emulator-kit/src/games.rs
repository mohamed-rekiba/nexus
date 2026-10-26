//! Installed console games in a named host folder: folders with `sce_sys/param.json` (PS5)
//! or `sce_sys/param.sfo` (PS4).
//!
//! Each of the player's folders is searched three folders down, for layouts like
//! `Games/PS5/<Title>/<PPSAxxxxx-app0>`.

use std::sync::Mutex;

use nexus_api::serde_json::{self, Value};
use nexus_api::{Folders, HostPath, PluginError, PluginResult, RelPath};

use crate::sfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Console {
    Ps5,
    Ps4,
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
    pub dir: HostPath,
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
                icon: file_in(folders, &sce, &["icon0.png"]).await,
                background: file_in(folders, &sce, &["pic1.png", "pic0.png"]).await,
            })
        }
    }
}

/// How far below a game folder a game is looked for: a console folder (`PS5`), a folder of
/// the player's, then the game.
const SCAN_DEPTH: usize = 3;

/// Every game for `console` in the folder named `folder`, sorted by title. A game folder may
/// hold games of other consoles too: each console's games are told apart by their files.
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
                if depth == SCAN_DEPTH {
                    continue;
                }
                // A root that can't be listed (not there yet) holds no games.
                for child in folders.list(&dir).await.unwrap_or_default() {
                    if child.is_dir && !child.name.starts_with('.') {
                        next.push(dir.join(&child.name)?);
                    }
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
        facts.push(GameFact::new("folder", "Folder", self.dir.path.as_str()));
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
