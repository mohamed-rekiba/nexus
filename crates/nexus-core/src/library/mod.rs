//! The library subsystem: what the core knows about each game, one [`GameRecord`] per
//! [`GameKey`], and the games the catalogs offer.
//!
//! A game's record holds what its plugin lists (the Library's entry and its badges), what its
//! plugin says when asked (its details, compatibility, slots and settings), what
//! a store says about it (its metadata), and, for an installed game, the catalog's offer it is
//! with that offer's cover. The catalogs' copies and the offers grouped from them sit beside
//! the records.
//!
//! The subsystem follows the pattern of [`crate::torrents`]: its own lock, its own saved
//! records ([`records`]: `app.hidden_games`, `app.game_art`, `app.results`), commands in
//! ([`LibrarySubsystem::handle`]), a snapshot out ([`LibrarySubsystem::snapshot`]), and its
//! changes published as [`Area::Library`] or [`Area::Catalogs`]. It asks no plugin: the core
//! reads the plugins and hands it the answers as plain data, since a read crosses areas (the
//! emulators' builds, notices, the selection).
//!
//! The snapshot shares the records and the offers through `Arc`: a view reads them on every
//! render, and a change copies them only while a snapshot is held.

mod records;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use nexus_api::{CompatInfo, GameDetails, GameKey, GameMetadata, GameSummary, PluginId, SlotItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) use self::records::{PluginImage, Records};
use crate::changes::{Area, Notifier};
use crate::releases::{CatalogCopy, Offer};
use crate::{CoreError, CoreResult};

/// A game as a plugin lists it.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub key: GameKey,
    pub summary: GameSummary,
}

/// What a game's plugin says about it when asked: for an offer, what its catalog says.
#[derive(Clone, Debug)]
pub(crate) struct Detail {
    pub details: GameDetails,
    pub compat: Option<CompatInfo>,
    /// Its `game.action` and `game.detail-section` slot items.
    pub actions: Vec<SlotItem>,
    pub sections: Vec<SlotItem>,
    /// The game's own settings, when its emulator has any.
    pub settings: Option<GameSettings>,
}

impl Detail {
    /// Only the details: an offer's, from its catalog.
    pub fn of(details: GameDetails) -> Self {
        Self { details, compat: None, actions: Vec::new(), sections: Vec::new(), settings: None }
    }
}

/// A game's own settings: its emulator's schema for them, and their values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GameSettings {
    pub schema: Value,
    pub values: Value,
}

/// Whether a game's details are in.
#[derive(Clone, Debug, Default)]
pub(crate) enum DetailState {
    #[default]
    Unread,
    Read(Box<Detail>),
    /// They failed to load on select: they load again on open, or with the next library load.
    Failed,
}

/// Everything the core knows about one game.
#[derive(Clone, Debug, Default)]
pub(crate) struct GameRecord {
    /// The game as its plugin lists it, while it is in the Library.
    pub entry: Option<Entry>,
    /// Its `game.badge` slot items.
    pub badges: Vec<SlotItem>,
    pub detail: DetailState,
    /// What a store's catalog says about it, and the plugin that said it.
    pub meta: Option<(PluginId, GameMetadata)>,
    /// The stores were asked about it this run, so none is asked twice.
    pub asked: bool,
    /// For an installed game, the offer it is: its Game Hub has the store data of the offer's releases.
    pub offer: Option<GameKey>,
    /// The cover of that offer: the game's art when it has none of its own (a PC game). Kept in
    /// `app.game_art`, so it stays with the catalog off.
    pub offer_art: Option<PluginImage>,
}

/// A plugin's image as the theme host fetches it.
pub(crate) fn image_ref(plugin: &PluginId, source: &nexus_api::ImageSource) -> nexus_ui_api::ImageRef {
    use nexus_api::ImageSource;
    use nexus_ui_api::ImageRef;
    match source {
        ImageSource::Url(url) => ImageRef::Url { plugin: plugin.clone(), url: url.clone() },
        ImageSource::Asset(path) => ImageRef::PluginAsset { plugin: plugin.clone(), path: path.clone() },
        ImageSource::Folder(path) => ImageRef::Folder { plugin: plugin.clone(), path: path.clone() },
    }
}

/// Sorts games by title, as the Library shows them.
pub(crate) fn sort_by_title(library: &mut [Entry]) {
    library.sort_by(|a, b| a.summary.title.to_lowercase().cmp(&b.summary.title.to_lowercase()).then(a.key.cmp(&b.key)));
}

/// An installed game, the offer it is, and the offer's cover.
pub(crate) type InstalledOffer = (GameKey, GameKey, Option<PluginImage>);

/// What the player asks of the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LibraryCmd {
    /// Lists every catalog plugin's games again, with their badges, and reads the stores'
    /// metadata and the games' details in the background.
    Refresh,
    /// Hides a game the Catalog offers, or shows it again. Hidden games leave the Catalog, but
    /// for the "Hidden" filter (`HIDDEN_FILTER`). Kept in the config.
    Hide { game: GameKey, hidden: bool },
}

/// A game's art, from the layers of [`LibrarySnapshot::metadata`] and its own details.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GameArt {
    pub cover: Option<PluginImage>,
    pub background: Option<PluginImage>,
    pub logo: Option<PluginImage>,
    pub screenshots: Vec<PluginImage>,
}

/// The library's state, read whole.
#[derive(Clone, Debug, Default)]
pub(crate) struct LibrarySnapshot {
    games: Arc<HashMap<GameKey, GameRecord>>,
    /// The Library's games, by title.
    order: Arc<Vec<GameKey>>,
    /// Each emulator's catalog, from each catalog plugin.
    pub catalogs: Arc<Vec<CatalogCopy>>,
    /// The games the catalogs offer, newest first.
    pub offers: Arc<Vec<Offer>>,
    /// The catalog plugins' Catalog buttons and badges (`catalog.action`), by plugin.
    pub catalog_buttons: Arc<Vec<(PluginId, Vec<SlotItem>)>>,
    /// The catalog plugins answered once: until then the Catalog says it is loading.
    pub catalogs_read: bool,
    /// A library load runs.
    pub loading: bool,
    /// The first library load, with the first game's details, is done.
    pub booted: bool,
    /// The store's descriptions: how many of how many are in. `None` without a store.
    pub describing: Option<(usize, usize)>,
    /// The store has been asked about every game of the library.
    pub described: bool,
    /// Work in the background, as the old launcher's status pill: "Downloading game art 3/40".
    pub task: Option<String>,
    /// The games the player hid from the Catalog, by their key as text.
    pub hidden: HashSet<String>,
    /// The player's own results, by the game's key as text.
    pub results: HashMap<String, String>,
}

impl LibrarySnapshot {
    /// The Library's games, by title.
    pub fn library(&self) -> impl Iterator<Item = &Entry> {
        self.order.iter().filter_map(|key| self.games.get(key)?.entry.as_ref())
    }

    /// The game is in the Library.
    pub fn listed(&self, key: &GameKey) -> Option<&Entry> {
        self.games.get(key)?.entry.as_ref()
    }

    /// Whether `key`'s details are in, failed, or not read yet.
    pub fn detail_state(&self, key: &GameKey) -> &DetailState {
        static UNREAD: DetailState = DetailState::Unread;
        self.games.get(key).map_or(&UNREAD, |r| &r.detail)
    }

    /// `key`'s details, once they are in.
    pub fn detail(&self, key: &GameKey) -> Option<&Detail> {
        match self.detail_state(key) {
            DetailState::Read(detail) => Some(detail),
            _ => None,
        }
    }

    pub fn badges(&self, key: &GameKey) -> &[SlotItem] {
        self.games.get(key).map_or(&[], |r| &r.badges)
    }

    /// What a store's catalog says about `key`.
    pub fn meta(&self, key: &GameKey) -> Option<&(PluginId, GameMetadata)> {
        self.games.get(key)?.meta.as_ref()
    }

    /// What the stores and the catalogs say about `key`, first layer first: a store's catalog,
    /// then a release's store data (an offer's own, or for an installed game, the offer's it
    /// is). A field a layer leaves empty comes from the next.
    pub fn metadata(&self, key: &GameKey) -> Vec<(PluginId, GameMetadata)> {
        let offer_key = self.games.get(key).and_then(|r| r.offer.as_ref()).unwrap_or(key);
        let release = self
            .offer(offer_key)
            .and_then(|offer| crate::releases::metadata_of(offer).map(|m| (offer.entry.key.plugin.clone(), m.clone())));
        self.meta(key).cloned().into_iter().chain(release).collect()
    }

    /// A game's cover, background, logo and screenshots: from the layers of [`Self::metadata`],
    /// then the game's own (the cover from its summary, the background from its details), then
    /// (the cover) its offer's.
    pub fn art(&self, key: &GameKey) -> GameArt {
        let layers = self.metadata(key);
        let pick = |field: fn(&GameMetadata) -> Option<&nexus_api::ImageSource>| {
            layers.iter().find_map(|(plugin, m)| field(m).map(|image| (plugin.clone(), image.clone())))
        };
        let own = |image: Option<&nexus_api::ImageSource>| image.map(|i| (key.plugin.clone(), i.clone()));
        let details = self.detail(key).map(|d| &d.details);
        let summary = details.map(|d| &d.summary).or_else(|| self.listed(key).map(|e| &e.summary));
        GameArt {
            cover: pick(|m| m.cover.as_ref().or(m.icon.as_ref()))
                .or_else(|| own(summary.and_then(|s| s.cover.as_ref())))
                .or_else(|| self.offer_art(key)),
            background: pick(|m| m.background.as_ref()).or_else(|| own(details.and_then(|d| d.background.as_ref()))),
            logo: pick(|m| m.logo.as_ref()),
            screenshots: layers
                .iter()
                .map(|(plugin, m)| m.screenshots.iter().map(|s| (plugin.clone(), s.clone())).collect::<Vec<_>>())
                .find(|shots| !shots.is_empty())
                .unwrap_or_default(),
        }
    }

    /// The square art of a game's card: the store's tile when a catalog describes it, else its
    /// own cover, else the cover of the offer it is. `entry` is a Library game or an offer.
    pub fn tile(&self, entry: &Entry) -> Option<PluginImage> {
        let store = self.meta(&entry.key).and_then(|(plugin, m)| Some((plugin.clone(), m.icon.clone()?)));
        store
            .or_else(|| entry.summary.cover.clone().map(|c| (entry.key.plugin.clone(), c)))
            .or_else(|| self.offer_art(&entry.key))
    }

    fn offer_art(&self, key: &GameKey) -> Option<PluginImage> {
        self.games.get(key)?.offer_art.clone()
    }

    /// The game's title in the Library, else its plugin's id for it.
    pub fn title(&self, key: &GameKey) -> String {
        self.listed(key).map(|e| e.summary.title.clone()).unwrap_or_else(|| key.local.clone())
    }

    /// The offer `key` names, if it is one.
    pub fn offer(&self, key: &GameKey) -> Option<&Offer> {
        self.offers.iter().find(|o| &o.entry.key == key)
    }

    /// The player hid `key` from the Catalog.
    pub fn hidden(&self, key: &GameKey) -> bool {
        self.hidden.contains(&key.to_string())
    }

    /// The player's own result for `key`: `ingame`, `menus`, `boots` or `nothing`.
    pub fn result(&self, key: &GameKey) -> Option<&str> {
        self.results.get(&key.to_string()).map(String::as_str)
    }
}

#[derive(Default)]
struct State {
    games: Arc<HashMap<GameKey, GameRecord>>,
    order: Arc<Vec<GameKey>>,
    catalogs: Arc<Vec<CatalogCopy>>,
    offers: Arc<Vec<Offer>>,
    catalog_buttons: Arc<Vec<(PluginId, Vec<SlotItem>)>>,
    catalogs_read: bool,
    loading: bool,
    booted: bool,
    describing: Option<(usize, usize)>,
    described: bool,
    task: Option<String>,
    /// The art being fetched around the selection; the next select stops it.
    prefetch: Option<tokio::task::AbortHandle>,
    /// The image addresses already fetched ahead this run, so none is fetched twice.
    prefetched: HashSet<String>,
}

impl State {
    fn games(&mut self) -> &mut HashMap<GameKey, GameRecord> {
        Arc::make_mut(&mut self.games)
    }

    fn game(&mut self, key: &GameKey) -> &mut GameRecord {
        self.games().entry(key.clone()).or_default()
    }

    /// The offer each installed game is, and its cover. `true` when a cover changed, to keep.
    fn set_installed(&mut self, installed: Vec<InstalledOffer>) -> bool {
        let found: HashMap<GameKey, GameKey> = installed.iter().map(|(g, o, _)| (g.clone(), o.clone())).collect();
        let games = self.games();
        for (key, record) in games.iter_mut() {
            record.offer = found.get(key).cloned();
        }
        let mut changed = false;
        for (game, offer, art) in installed {
            let record = games.entry(game).or_default();
            record.offer = Some(offer);
            if let Some(art) = art
                && record.offer_art.as_ref() != Some(&art)
            {
                record.offer_art = Some(art);
                changed = true;
            }
        }
        changed
    }
}

struct Shared {
    records: Records,
    notifier: Notifier,
    state: Mutex<State>,
}

/// The games. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct LibrarySubsystem {
    inner: Arc<Shared>,
}

impl LibrarySubsystem {
    /// Starts with the offers' covers the last run kept.
    pub fn new(records: Records, notifier: Notifier) -> Self {
        let mut state = State::default();
        for (game, art) in records.kept_art() {
            state.game(&game).offer_art = Some(art);
        }
        Self { inner: Arc::new(Shared { records, notifier, state: Mutex::new(state) }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Changes the state and publishes the change in `area`.
    fn change<R>(&self, area: Area, change: impl FnOnce(&mut State) -> R) -> R {
        let out = change(&mut self.state());
        self.inner.notifier.changed(area);
        out
    }

    /// The games, the offers and the saved choices, as they are now.
    pub fn snapshot(&self) -> LibrarySnapshot {
        let (hidden, results) = (self.inner.records.hidden(), self.inner.records.results());
        let state = self.state();
        LibrarySnapshot {
            games: state.games.clone(),
            order: state.order.clone(),
            catalogs: state.catalogs.clone(),
            offers: state.offers.clone(),
            catalog_buttons: state.catalog_buttons.clone(),
            catalogs_read: state.catalogs_read,
            loading: state.loading,
            booted: state.booted,
            describing: state.describing,
            described: state.described,
            task: state.task.clone(),
            hidden,
            results,
        }
    }

    /// Runs a command that is the subsystem's alone. The core reads the plugins for
    /// [`LibraryCmd::Refresh`] first.
    pub fn handle(&self, command: LibraryCmd) -> CoreResult<()> {
        match command {
            LibraryCmd::Hide { game, hidden } => {
                self.inner.records.set_hidden(&game, hidden).map_err(|e| CoreError::Io(e.to_string()))?;
                self.inner.notifier.changed(Area::Library);
                Ok(())
            }
            LibraryCmd::Refresh => Err(CoreError::Invalid("the core reads the plugins first".into())),
        }
    }

    /// The first library load of the run is done.
    pub fn booted(&self) -> bool {
        self.state().booted
    }

    pub fn set_booted(&self) {
        self.change(Area::Library, |s| s.booted = true);
    }

    /// A library load starts: the details that failed may load again.
    pub fn start_loading(&self) {
        self.change(Area::Library, |s| {
            s.loading = true;
            for record in s.games().values_mut() {
                if matches!(record.detail, DetailState::Failed) {
                    record.detail = DetailState::Unread;
                }
            }
        });
    }

    /// The Library's games, as their plugins list them, with their badges, and the offer each
    /// installed game is. The games and badges listed before go; the rest of their records
    /// stays.
    pub fn set_library(
        &self,
        mut library: Vec<Entry>,
        mut badges: HashMap<GameKey, Vec<SlotItem>>,
        installed: Vec<InstalledOffer>,
    ) {
        sort_by_title(&mut library);
        let keep = self.change(Area::Library, |s| {
            s.order = Arc::new(library.iter().map(|e| e.key.clone()).collect());
            let games = s.games();
            for (key, record) in games.iter_mut() {
                record.entry = None;
                record.badges = badges.remove(key).unwrap_or_default();
            }
            for (key, items) in badges {
                games.entry(key).or_default().badges = items;
            }
            for entry in library {
                let key = entry.key.clone();
                games.entry(key).or_default().entry = Some(entry);
            }
            s.loading = false;
            s.set_installed(installed).then(|| Self::kept(s))
        });
        if let Some(art) = keep {
            self.inner.records.keep_art(art.iter().map(|(g, a)| (g, a)));
        }
    }

    /// Every offer cover the records hold, to keep.
    fn kept(state: &State) -> Vec<(GameKey, PluginImage)> {
        state.games.iter().filter_map(|(key, r)| Some((key.clone(), r.offer_art.clone()?))).collect()
    }

    /// The details of `key` are in. `badges`, when its plugin gave them, replace its badges.
    pub fn set_detail(&self, key: &GameKey, detail: Detail, badges: Option<Vec<SlotItem>>) {
        self.change(Area::Library, |s| {
            let record = s.game(key);
            record.detail = DetailState::Read(Box::new(detail));
            if let Some(badges) = badges {
                record.badges = badges;
            }
        });
    }

    /// The details of `key` failed to load. Details read before stay.
    pub fn detail_failed(&self, key: &GameKey) {
        self.change(Area::Library, |s| {
            let record = s.game(key);
            if !matches!(record.detail, DetailState::Read(_)) {
                record.detail = DetailState::Failed;
            }
        });
    }

    /// Forgets the details of `key`: the game was uninstalled.
    pub fn forget_detail(&self, key: &GameKey) {
        self.change(Area::Library, |s| s.game(key).detail = DetailState::Unread);
    }

    /// Changes the details of `key`, when they are in.
    fn change_detail(&self, key: &GameKey, change: impl FnOnce(&mut Detail)) {
        self.change(Area::Library, |s| {
            if let Some(GameRecord { detail: DetailState::Read(detail), .. }) = s.games().get_mut(key) {
                change(detail);
            }
        });
    }

    pub fn set_game_settings(&self, key: &GameKey, settings: Option<GameSettings>) {
        self.change_detail(key, |d| d.settings = settings);
    }

    /// Of `keys`, the games no store described or was asked about yet. They count as asked
    /// from now on.
    pub fn to_ask(&self, keys: &[GameKey]) -> Vec<GameKey> {
        let mut state = self.state();
        let todo: Vec<GameKey> = keys
            .iter()
            .filter(|key| state.games.get(*key).is_none_or(|r| r.meta.is_none() && !r.asked))
            .cloned()
            .collect();
        for key in &todo {
            state.game(key).asked = true;
        }
        todo
    }

    /// A store's answer about `key`. `None` with `failed`: the store didn't answer (offline),
    /// so the game is asked again when it shows.
    pub fn described(&self, key: &GameKey, found: Option<(PluginId, GameMetadata)>, failed: bool) {
        self.change(Area::Library, |s| match found {
            Some(found) => s.game(key).meta = Some(found),
            None if failed => s.game(key).asked = false,
            None => {}
        });
    }

    /// The boot's count of the store's descriptions, or `None` for none.
    pub fn set_describing(&self, describing: Option<(usize, usize)>) {
        self.change(Area::Library, |s| s.describing = describing);
    }

    /// The store has been asked about every game. `found_store`: a store answered, so the
    /// boot's count stays.
    pub fn set_described(&self, found_store: bool) {
        self.change(Area::Library, |s| {
            if !found_store {
                s.describing = None;
            }
            s.described = true;
        });
    }

    pub fn set_task(&self, task: Option<String>) {
        self.change(Area::Library, |s| s.task = task);
    }

    /// The catalogs as just read, their buttons, and the offers grouped from them, with the
    /// offer each installed game is. Returns how many releases are new since the last read.
    pub fn set_catalogs(
        &self,
        copies: Vec<CatalogCopy>,
        buttons: Vec<(PluginId, Vec<SlotItem>)>,
        offers: Vec<Offer>,
        installed: Vec<InstalledOffer>,
    ) -> usize {
        let (added, keep) = self.change(Area::Catalogs, |s| {
            let added = crate::releases::added_releases(&s.catalogs, &copies);
            s.catalogs = Arc::new(copies);
            s.catalogs_read = true;
            s.catalog_buttons = Arc::new(buttons);
            s.offers = Arc::new(offers);
            let keep = s.set_installed(installed).then(|| Self::kept(s));
            (added, keep)
        });
        if let Some(art) = keep {
            self.inner.records.keep_art(art.iter().map(|(g, a)| (g, a)));
        }
        added
    }

    /// The catalogs were read, and some plugin gave one.
    pub fn has_catalogs(&self) -> bool {
        !self.state().catalogs.is_empty()
    }

    /// No catalog is turned on: the copies and the offers go, so turning one on reads them
    /// again.
    pub fn clear_catalogs(&self) {
        self.change(Area::Catalogs, |s| {
            s.catalogs = Arc::default();
            s.offers = Arc::default();
        });
    }

    /// Saves the player's answer to "How far did it get?" for `game` on `build`: a result, or
    /// `None` for "Not now".
    pub fn answer(&self, game: &GameKey, level: Option<&str>, build: Option<&str>) -> CoreResult<()> {
        if let Some(level) = level
            && !records::RESULT_LEVELS.contains(&level)
        {
            return Err(CoreError::Invalid(format!("no result {level:?}")));
        }
        self.inner.records.answer(game, level, build).map_err(|e| CoreError::Io(e.to_string()))?;
        self.inner.notifier.changed(Area::Library);
        Ok(())
    }

    /// Whether the player answered "How far did it get?" for `game` on `build`, or said
    /// "Not now" to it.
    pub fn answered(&self, game: &GameKey, build: Option<&str>) -> bool {
        self.inner.records.answered(game, build)
    }

    /// Keeps the fetch of the art around the selection, and returns the last one, to stop.
    pub fn prefetching(&self, task: tokio::task::AbortHandle) -> Option<tokio::task::AbortHandle> {
        self.state().prefetch.replace(task)
    }

    /// Whether `url` wasn't fetched ahead yet this run. It counts as fetched from now on.
    pub fn first_fetch(&self, url: &str) -> bool {
        self.state().prefetched.insert(url.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use nexus_api::{ImageSource, ReleaseList};

    use super::*;
    use crate::changes::Changes;

    fn library() -> (LibrarySubsystem, Arc<crate::ConfigService>) {
        let config = Arc::new(crate::ConfigService::in_memory());
        let notifier = Notifier::new(Changes::new(), crate::EventBus::default());
        (LibrarySubsystem::new(Records::new(config.clone()), notifier), config)
    }

    fn key(text: &str) -> GameKey {
        text.parse().unwrap()
    }

    fn entry(key_text: &str, title: &str) -> Entry {
        let key = key(key_text);
        let summary = GameSummary {
            id: key.local.clone(),
            title: title.into(),
            platform: "PS5".into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: None,
            content_id: None,
        };
        Entry { key, summary }
    }

    fn badge(id: &str) -> SlotItem {
        SlotItem {
            id: id.into(),
            order: Default::default(),
            content: nexus_api::SlotContent::Badge {
                label: id.into(),
                short: String::new(),
                tone: nexus_api::Tone::Neutral,
                icon: None,
            },
        }
    }

    fn image(plugin: &str, url: &str) -> PluginImage {
        (PluginId::new(plugin).unwrap(), ImageSource::Url(url.into()))
    }

    fn meta(plugin: &str, change: impl FnOnce(&mut GameMetadata)) -> (PluginId, GameMetadata) {
        let mut m = GameMetadata::default();
        change(&mut m);
        (PluginId::new(plugin).unwrap(), m)
    }

    fn details(entry: &Entry) -> Detail {
        Detail::of(GameDetails {
            summary: entry.summary.clone(),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: None,
            facts: Vec::new(),
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
        })
    }

    #[test]
    fn the_library_is_in_title_order_and_a_new_list_takes_the_old_one_s_place() {
        let (lib, _) = library();
        lib.set_library(vec![entry("emu:2", "beta"), entry("emu:1", "Alpha")], HashMap::new(), Vec::new());
        let titles = |s: &LibrarySnapshot| s.library().map(|e| e.summary.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&lib.snapshot()), ["Alpha", "beta"]);

        lib.set_library(vec![entry("emu:3", "Gamma")], HashMap::new(), Vec::new());
        let snapshot = lib.snapshot();
        assert_eq!(titles(&snapshot), ["Gamma"]);
        assert!(snapshot.listed(&key("emu:1")).is_none());
        assert_eq!(snapshot.title(&key("emu:1")), "1", "a game no longer listed has its plugin's id");
    }

    #[test]
    fn a_new_list_replaces_the_badges_and_keeps_what_the_stores_said() {
        let (lib, _) = library();
        let game = key("emu:1");
        let badges = HashMap::from([(game.clone(), vec![badge("old")])]);
        lib.set_library(vec![entry("emu:1", "Alpha")], badges, Vec::new());
        lib.described(&game, Some(meta("store", |m| m.tagline = "fast".into())), false);
        assert_eq!(lib.to_ask(std::slice::from_ref(&game)), Vec::<GameKey>::new(), "described: not asked again");

        lib.set_library(vec![entry("emu:1", "Alpha")], HashMap::new(), Vec::new());
        let snapshot = lib.snapshot();
        assert!(snapshot.badges(&game).is_empty(), "the badges the plugin no longer gives go");
        assert_eq!(snapshot.meta(&game).unwrap().1.tagline, "fast");
    }

    #[test]
    fn a_game_is_asked_about_once_unless_the_store_did_not_answer() {
        let (lib, _) = library();
        let game = key("emu:1");
        assert_eq!(lib.to_ask(std::slice::from_ref(&game)), vec![game.clone()]);
        assert!(lib.to_ask(std::slice::from_ref(&game)).is_empty(), "asked once a run");
        lib.described(&game, None, true);
        assert_eq!(lib.to_ask(std::slice::from_ref(&game)), vec![game.clone()], "offline: asked again");
        lib.described(&game, None, false);
        assert!(lib.to_ask(std::slice::from_ref(&game)).is_empty(), "the store doesn't know it");
    }

    #[test]
    fn details_that_failed_load_again_with_the_next_library_load_and_read_ones_stay() {
        let (lib, _) = library();
        let (failing, read) = (key("emu:1"), key("emu:2"));
        lib.detail_failed(&failing);
        assert!(matches!(lib.snapshot().detail_state(&failing), DetailState::Failed));
        lib.set_detail(&read, details(&entry("emu:2", "Beta")), Some(vec![badge("b")]));
        lib.detail_failed(&read);
        assert!(lib.snapshot().detail(&read).is_some(), "a failed reload keeps the details");
        assert_eq!(lib.snapshot().badges(&read).len(), 1);

        lib.start_loading();
        let snapshot = lib.snapshot();
        assert!(snapshot.loading);
        assert!(matches!(snapshot.detail_state(&failing), DetailState::Unread));
        assert!(snapshot.detail(&read).is_some());

        lib.forget_detail(&read);
        assert!(matches!(lib.snapshot().detail_state(&read), DetailState::Unread));
    }

    #[test]
    fn a_card_s_tile_is_the_store_s_then_its_own_cover_then_its_offer_s() {
        let (lib, _) = library();
        let mut game = entry("emu:1", "Alpha");
        let offer = key("cat:emu/1");
        lib.set_library(vec![game.clone()], HashMap::new(), vec![(game.key.clone(), offer, Some(image("cat", "o")))]);
        assert_eq!(lib.snapshot().tile(&game), Some(image("cat", "o")));
        game.summary.cover = Some(ImageSource::Url("own".into()));
        assert_eq!(lib.snapshot().tile(&game), Some(image("emu", "own")));
        lib.described(&game.key, Some(meta("store", |m| m.icon = Some(ImageSource::Url("tile".into())))), false);
        assert_eq!(lib.snapshot().tile(&game), Some(image("store", "tile")));
    }

    #[test]
    fn an_offer_s_cover_is_kept_and_comes_back_with_the_next_run() {
        let (lib, config) = library();
        let game = entry("emu:1", "Alpha");
        let installed = vec![(game.key.clone(), key("cat:emu/1"), Some(image("cat", "o")))];
        lib.set_catalogs(Vec::new(), Vec::new(), Vec::new(), installed);
        assert!(!config.get(&["app", "game_art", "emu:1"]).is_null(), "kept in the config");

        let next = LibrarySubsystem::new(
            Records::new(config.clone()),
            Notifier::new(Changes::new(), crate::EventBus::default()),
        );
        assert_eq!(next.snapshot().art(&game.key).cover, Some(image("cat", "o")), "with no catalog");
    }

    #[test]
    fn the_store_s_metadata_comes_before_the_game_s_own_art() {
        let (lib, _) = library();
        let game = entry("emu:1", "Alpha");
        let mut detail = details(&game);
        detail.details.background = Some(ImageSource::Url("own-bg".into()));
        lib.set_detail(&game.key, detail, None);
        assert_eq!(lib.snapshot().art(&game.key).background, Some(image("emu", "own-bg")));

        let store = meta("store", |m| {
            m.background = Some(ImageSource::Url("bg".into()));
            m.screenshots = vec![ImageSource::Url("s1".into())];
        });
        lib.described(&game.key, Some(store), false);
        let art = lib.snapshot().art(&game.key);
        assert_eq!(art.background, Some(image("store", "bg")));
        assert_eq!(art.screenshots, [image("store", "s1")]);
        assert_eq!(lib.snapshot().metadata(&game.key).len(), 1, "no catalog, no release layer");
    }

    #[test]
    fn hiding_a_game_shows_in_the_next_snapshot_and_a_result_must_be_one_the_player_can_give() {
        let (lib, _) = library();
        let game = key("cat:emu/1");
        lib.handle(LibraryCmd::Hide { game: game.clone(), hidden: true }).unwrap();
        assert!(lib.snapshot().hidden(&game));
        lib.handle(LibraryCmd::Hide { game: game.clone(), hidden: false }).unwrap();
        assert!(!lib.snapshot().hidden(&game));

        assert!(lib.answer(&game, Some("great"), Some("1")).is_err());
        lib.answer(&game, Some("menus"), Some("1")).unwrap();
        assert_eq!(lib.snapshot().result(&game), Some("menus"));
        assert!(lib.answered(&game, Some("1")));
    }

    #[test]
    fn clearing_the_catalogs_drops_the_offers() {
        let (lib, _) = library();
        let list = ReleaseList {
            releases: Vec::new(),
            source: nexus_api::ReleaseSource::Bundled,
            host: String::new(),
            version: String::new(),
            updated: String::new(),
            problem: None,
        };
        let copy =
            CatalogCopy { service: PluginId::new("cat").unwrap(), emulator: PluginId::new("emu").unwrap(), list };
        lib.set_catalogs(vec![copy], Vec::new(), Vec::new(), Vec::new());
        assert!(lib.snapshot().catalogs_read);
        lib.clear_catalogs();
        let snapshot = lib.snapshot();
        assert!(snapshot.catalogs.is_empty() && snapshot.offers.is_empty());
        assert!(snapshot.catalogs_read, "the Catalog has answered once");
    }
}
