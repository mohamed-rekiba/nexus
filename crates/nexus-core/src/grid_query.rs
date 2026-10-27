//! The Library's and the Catalog's grids: what the player asked for (a [`GridQuery`]: the
//! search, the filters, the sort and how many pages show), and the games that answer it, read
//! from the library, the downloads and the play times. The reads are pure: the view, the
//! prefetch and the store's descriptions of the Catalog's page all ask the same question the
//! same way.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use nexus_api::{CompatLevel, GameKey};
use nexus_ui_api::{FilterVm, LibraryVm};

use crate::app::Grid;
use crate::library::{Entry, LibrarySnapshot};
use crate::ownership::Ownership;
use crate::releases::Offer;
use crate::sessions::PlayTimes;
use crate::{CoreError, CoreResult};

/// How many more games a Library or Catalog page shows each time.
pub const GRID_PAGE: usize = 60;

/// The Catalog's "In-game" filter, among the systems: `Command::CatalogSystem` with this id.
pub const INGAME_FILTER: &str = "ingame";

/// The Catalog's "Installed" filter, among the systems: `Command::CatalogSystem` with this id.
pub const INSTALLED_FILTER: &str = "installed";

/// The Catalog's "Hidden" filter, among the systems: only the games the player hid.
pub const HIDDEN_FILTER: &str = "hidden";

/// How a grid is sorted. Its text, for a theme, is its name in lower case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Sort {
    #[default]
    Title,
    /// Last played first.
    Played,
    /// Largest first.
    Size,
    Smallest,
    Rating,
    /// Newest first.
    Release,
    /// Furthest it runs first.
    Compat,
    /// Newest in the catalog first.
    Added,
}

impl Sort {
    const ALL: [Sort; 8] =
        [Sort::Title, Sort::Played, Sort::Size, Sort::Smallest, Sort::Rating, Sort::Release, Sort::Compat, Sort::Added];

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Title => "title",
            Sort::Played => "played",
            Sort::Size => "size",
            Sort::Smallest => "smallest",
            Sort::Rating => "rating",
            Sort::Release => "release",
            Sort::Compat => "compat",
            Sort::Added => "added",
        }
    }
}

impl FromStr for Sort {
    type Err = CoreError;

    fn from_str(text: &str) -> CoreResult<Self> {
        Sort::ALL
            .into_iter()
            .find(|s| s.as_str() == text)
            .ok_or_else(|| CoreError::Invalid(format!("no sort {text:?}")))
    }
}

/// A grid's search, filters, sort and pages.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridQuery {
    pub grid: Grid,
    pub query: String,
    pub system: Option<String>,
    /// Only games that reach gameplay: the community's result or the player's own.
    pub ingame: bool,
    /// Only the games the player has: in the Catalog, the offers of installed games.
    pub installed: bool,
    /// Only the games the player hid; else the Catalog leaves them out.
    pub hidden: bool,
    pub genre: Option<String>,
    pub sort: Sort,
    /// How many games show: pages of [`GRID_PAGE`], more as the player scrolls.
    pub shown: usize,
}

impl GridQuery {
    /// The Library's: by title.
    pub fn library() -> Self {
        Self {
            grid: Grid::Library,
            query: String::new(),
            system: None,
            ingame: false,
            installed: false,
            hidden: false,
            genre: None,
            sort: Sort::Title,
            shown: GRID_PAGE,
        }
    }

    /// The Catalog's: newest added first.
    pub fn catalog() -> Self {
        Self { grid: Grid::Catalog, sort: Sort::Added, ..Self::library() }
    }

    /// A system, or `INGAME_FILTER`, `INSTALLED_FILTER` or `HIDDEN_FILTER` to turn "In-game",
    /// "Installed" or "Hidden" on or off; `None` clears them all.
    pub fn set_system(&mut self, system: Option<String>) {
        match system.as_deref() {
            Some(INGAME_FILTER) => self.ingame = !self.ingame,
            Some(INSTALLED_FILTER) => self.installed = !self.installed,
            Some(HIDDEN_FILTER) => self.hidden = !self.hidden,
            None => {
                self.system = None;
                self.ingame = false;
                self.installed = false;
                self.hidden = false;
            }
            Some(_) => self.system = system,
        }
    }

    /// The grid's games, searched, filtered and sorted, with the filters' chips.
    pub fn games<'a>(&self, data: &GridData<'a>) -> GridGames<'a> {
        games(self, data)
    }

    /// The games on the shown pages, in order.
    pub fn page(&self, data: &GridData) -> Vec<GameKey> {
        self.games(data).shown.into_iter().take(self.shown).map(|e| e.key.clone()).collect()
    }
}

/// What a grid reads.
pub(crate) struct GridData<'a> {
    pub library: &'a LibrarySnapshot,
    pub ownership: &'a Ownership<'a>,
    pub plays: &'a PlayTimes,
    /// The Catalog shows its "Hidden" filter: no catalog plugin turned it off.
    pub hidden_filter: bool,
}

/// A grid's games, searched, filtered and sorted, before paging, with the filters' chips.
pub(crate) struct GridGames<'a> {
    pub shown: Vec<&'a Entry>,
    /// How many games there are before the search and the filters.
    pub all: usize,
    pub systems: Vec<FilterVm>,
    pub genres: Vec<FilterVm>,
}

/// The games beside `game`, at most `most`: in Home's row, one each side; in the Library's or
/// the Catalog's grid, one each side and the ones above and below, when the grid said how many
/// columns it has.
pub(crate) fn neighbours(
    data: &GridData,
    library: &GridQuery,
    catalog: &GridQuery,
    columns: &HashMap<Grid, usize>,
    game: &GameKey,
    most: usize,
) -> Vec<GameKey> {
    let mut found: Vec<GameKey> = Vec::new();
    let mut around = |list: Vec<&GameKey>, steps: &[isize]| {
        let Some(at) = list.iter().position(|k| *k == game) else { return };
        for step in steps {
            if let Some(key) = at.checked_add_signed(*step).and_then(|i| list.get(i))
                && *key != game
                && !found.contains(key)
            {
                found.push((*key).clone());
            }
        }
    };
    // An offer is in the Catalog's grid; a game of the Library is in Home's row too.
    let query = if data.library.offer(game).is_some() {
        catalog
    } else {
        around(data.library.library().map(|e| &e.key).collect(), &[1, -1]);
        library
    };
    let games = query.games(data);
    let keys: Vec<&GameKey> = games.shown.iter().take(query.shown).map(|e| &e.key).collect();
    match columns.get(&query.grid).copied().filter(|c| *c > 1) {
        Some(c) => around(keys, &[1, -1, c as isize, -(c as isize)]),
        None => around(keys, &[1, -1]),
    }
    found.truncate(most);
    found
}

/// The Catalog's grid as it was last shown. Only the Catalog shows its grid: on another screen
/// the last one stands, so the view doesn't search and sort every offer again on each render.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShownGrid(Option<std::sync::Arc<LibraryVm>>);

impl ShownGrid {
    /// The grid to show: the last one, while another screen is on; `None` to build it now.
    pub fn get(&self, on_screen: bool) -> Option<LibraryVm> {
        self.0.as_deref().filter(|_| !on_screen).cloned()
    }

    /// Keeps `grid` when it is on screen, or when there is none yet.
    pub fn keep(&mut self, on_screen: bool, grid: &LibraryVm) {
        if on_screen || self.0.is_none() {
            self.0 = Some(std::sync::Arc::new(grid.clone()));
        }
    }

    /// Forgets the last grid: the Catalog went.
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// The genres a store's catalog gives the game.
fn genres_of(library: &LibrarySnapshot, key: &GameKey) -> Vec<String> {
    library.meta(key).map(|(_, m)| m.genres.clone()).unwrap_or_default()
}

/// The fixed genres of a game a catalog offers (see [`crate::releases::GENRES`]), from its
/// releases' genre words, a store's genres and its releases' store data.
fn offer_genres(library: &LibrarySnapshot, offer: &Offer) -> Vec<String> {
    let store = library.meta(&offer.entry.key).map(|(_, m)| &m.genres);
    let release = crate::releases::metadata_of(offer).map(|m| &m.genres);
    let words: Vec<&str> = offer
        .releases
        .iter()
        .map(|r| r.fact_text(nexus_api::FactKind::Genre))
        .chain(store.into_iter().chain(release).flatten().map(String::as_str))
        .collect();
    crate::releases::genres_in(&words).into_iter().map(str::to_owned).collect()
}

/// The words of a search or of a field, in lower case: letters and digits. An apostrophe joins
/// its word ("Assassin's" is "assassins").
fn search_words(text: &str) -> Vec<String> {
    let text = text.to_lowercase().replace(['\'', '\u{2019}'], "");
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// Whether each of the search's `words` starts a word of `fields`.
fn has_words<'a>(words: &[String], fields: impl IntoIterator<Item = &'a str>) -> bool {
    let known: Vec<String> = fields.into_iter().flat_map(search_words).collect();
    words.iter().all(|w| known.iter().any(|k| k.starts_with(w.as_str())))
}

/// How far a game gets, for the "In-game" filter and the sort: the player's own result first,
/// then the community's. 0 for unknown, 5 for playable.
fn reach(library: &LibrarySnapshot, key: &GameKey) -> u8 {
    let rank = |result: &str| match result {
        "playable" => 5,
        "ingame" => 4,
        "menus" => 3,
        "boots" => 2,
        "nothing" => 1,
        _ => 0,
    };
    if let Some(own) = library.result(key) {
        return rank(own);
    }
    match library.detail(key).and_then(|d| d.compat.as_ref()).map(|c| c.level) {
        Some(CompatLevel::Playable) => 5,
        Some(CompatLevel::Ingame) => 4,
        Some(CompatLevel::Menus) => 3,
        Some(CompatLevel::Boots) => 2,
        Some(CompatLevel::Nothing) => 1,
        Some(CompatLevel::Unknown) | None => 0,
    }
}

fn games<'a>(query: &GridQuery, data: &GridData<'a>) -> GridGames<'a> {
    let library = data.library;
    let owned = data.ownership;
    let catalog = query.grid == Grid::Catalog;
    let offers: HashMap<&GameKey, &Offer> = library.offers.iter().map(|o| (&o.entry.key, o)).collect();
    let entries: Vec<&Entry> =
        if catalog { library.offers.iter().map(|o| &o.entry).collect() } else { library.library().collect() };
    // The Catalog's hidden games: out, but for the "Hidden" filter, which shows only them. With
    // the filter turned off in the catalog's settings, its chip goes and it doesn't apply.
    let hidden_filter = catalog && data.hidden_filter;
    let only_hidden = hidden_filter && query.hidden;
    // Hidden by the player in Nexus, or in the game's store (Steam's Hidden collection).
    let store_hidden =
        |e: &Entry| offers.get(&e.key).is_some_and(|o| !o.releases.is_empty() && o.releases.iter().all(|r| r.hidden));
    let is_hidden = |e: &Entry| catalog && (library.hidden(&e.key) || store_hidden(e));
    let hidden_count = entries.iter().filter(|e| is_hidden(e)).count();
    let entries: Vec<&Entry> = entries.into_iter().filter(|e| is_hidden(e) == only_hidden).collect();
    // How many games there are before the search and the filters: hidden ones count nowhere.
    let all = entries.len();
    let needle = query.query.trim().to_lowercase();
    let words = search_words(&query.query);
    let found = |e: &Entry| {
        if e.summary.title.to_lowercase().contains(&needle) {
            return true;
        }
        let publisher = library.meta(&e.key).map(|(_, m)| m.publisher.as_str()).unwrap_or_default();
        let title_id = e.summary.title_id.as_deref().unwrap_or_default();
        match offers.get(&e.key) {
            // Every word in one release: its title, title ids, publisher, region and version.
            Some(o) => o.releases.iter().any(|r| {
                let ids = std::iter::once(r.title_id.as_str()).chain(r.title_ids.iter().map(String::as_str));
                let own = r.fact_text(nexus_api::FactKind::Publisher);
                let fields = [e.summary.title.as_str(), &r.title, own, publisher, &r.region, &r.version];
                has_words(&words, fields.into_iter().chain(ids))
            }),
            None => has_words(&words, [e.summary.title.as_str(), title_id, publisher]),
        }
    };
    let searched: Vec<&Entry> = entries.into_iter().filter(|e| needle.is_empty() || found(e)).collect();
    // Systems count what the search leaves; genres count what the system leaves too.
    let mut systems =
        vec![FilterVm { id: String::new(), label: "All".into(), count: searched.len(), on: query.system.is_none() }];
    for e in &searched {
        let platform = &e.summary.platform;
        match systems.iter_mut().find(|s| &s.id == platform) {
            Some(s) => s.count += 1,
            None => systems.push(FilterVm {
                id: platform.clone(),
                label: platform.clone(),
                count: 1,
                on: query.system.as_ref() == Some(platform),
            }),
        }
    }
    let level = |e: &Entry| reach(library, &e.key);
    let ingame = searched.iter().filter(|e| level(e) >= 4).count();
    // The Catalog's only: the Library's games are the player's own. Shown once a game has a
    // result to filter by.
    let only_ingame = catalog && query.ingame;
    if catalog && (ingame > 0 || query.ingame) {
        systems.push(FilterVm { id: INGAME_FILTER.into(), label: "In-game".into(), count: ingame, on: query.ingame });
    }
    // The Catalog's offers of the games the player has.
    let installed = |e: &Entry| offers.get(&e.key).is_some_and(|o| owned.installed_game(o).is_some());
    let installed_count = if catalog { searched.iter().filter(|e| installed(e)).count() } else { 0 };
    if installed_count > 0 || (catalog && query.installed) {
        systems.push(FilterVm {
            id: INSTALLED_FILTER.into(),
            label: "Installed".into(),
            count: installed_count,
            on: query.installed,
        });
    }
    if hidden_filter && (hidden_count > 0 || query.hidden) {
        systems.push(FilterVm {
            id: HIDDEN_FILTER.into(),
            label: "Hidden".into(),
            count: hidden_count,
            on: query.hidden,
        });
    }
    let in_system: Vec<&Entry> = searched
        .into_iter()
        .filter(|e| query.system.as_ref().is_none_or(|s| &e.summary.platform == s))
        .filter(|e| !only_ingame || level(e) >= 4)
        .filter(|e| !catalog || !query.installed || installed(e))
        .collect();
    // The Library's chips are the store's genres; the Catalog's are the fixed genres.
    let chips: HashMap<&GameKey, Vec<String>> = in_system
        .iter()
        .map(|e| {
            let genres = match offers.get(&e.key) {
                Some(o) if catalog => offer_genres(library, o),
                _ => genres_of(library, &e.key),
            };
            (&e.key, genres)
        })
        .collect();
    let chips_of = |e: &Entry| chips.get(&e.key).map(Vec::as_slice).unwrap_or_default();
    let mut genre_counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &in_system {
        for g in chips_of(e).iter().cloned() {
            *genre_counts.entry(g).or_default() += 1;
        }
    }
    let mut genres: Vec<FilterVm> = genre_counts
        .into_iter()
        .map(|(g, count)| FilterVm { on: query.genre.as_ref() == Some(&g), label: g.clone(), id: g, count })
        .collect();
    genres.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
    genres.insert(
        0,
        FilterVm { id: String::new(), label: "All genres".into(), count: in_system.len(), on: query.genre.is_none() },
    );
    let mut shown: Vec<&Entry> =
        in_system.into_iter().filter(|e| query.genre.as_ref().is_none_or(|g| chips_of(e).contains(g))).collect();
    let size = |e: &Entry| -> Option<u64> {
        match offers.get(&e.key) {
            Some(o) => owned.best_release(o).size_bytes,
            None => library.detail(&e.key).and_then(|d| d.details.size_bytes),
        }
    };
    let added = |e: &Entry| offers.get(&e.key).map(|o| o.releases[0].added.clone()).unwrap_or_default();
    // The catalog's own date of its best release first, then the store's, then the year.
    let released = |e: &Entry| {
        let catalog = offers.get(&e.key).and_then(|o| crate::releases::release_day(owned.best_release(o)));
        let store = || library.meta(&e.key).and_then(|(_, m)| crate::releases::date_key(&m.release));
        catalog.or_else(store).or_else(|| e.summary.release_year.map(|y| y.to_string())).unwrap_or_default()
    };
    match query.sort {
        Sort::Played => shown.sort_by_key(|e| std::cmp::Reverse(data.plays.of(&e.key).last)),
        Sort::Size => shown.sort_by_key(|e| std::cmp::Reverse(size(e))),
        // Smallest first; a game whose size isn't known goes last.
        Sort::Smallest => shown.sort_by_key(|e| size(e).unwrap_or(u64::MAX)),
        // Top rated, newest, and best running first; a game without one goes last.
        Sort::Rating => shown.sort_by(|a, b| {
            let score = |e: &Entry| library.meta(&e.key).and_then(|(_, m)| m.rating).map_or(-1.0, |r| r.score);
            score(b).total_cmp(&score(a))
        }),
        Sort::Release => shown.sort_by_cached_key(|e| std::cmp::Reverse(released(e))),
        Sort::Compat => shown.sort_by_key(|e| std::cmp::Reverse(level(e))),
        Sort::Added => shown.sort_by_cached_key(|e| std::cmp::Reverse(added(e))),
        Sort::Title => shown.sort_by_cached_key(|e| e.summary.title.to_lowercase()),
    }
    GridGames { shown, all, systems, genres }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_api::{GameSummary, PluginId, Release};

    use super::*;
    use crate::changes::{Changes, Notifier};
    use crate::game_installs::InstallsSnapshot;
    use crate::library::{LibrarySubsystem, Records};

    fn key(text: &str) -> GameKey {
        text.parse().unwrap()
    }

    fn entry(key_text: &str, title: &str, platform: &str) -> Entry {
        let key = key(key_text);
        let summary = GameSummary {
            id: key.local.clone(),
            title: title.into(),
            platform: platform.into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: None,
            content_id: None,
        };
        Entry { key, summary }
    }

    fn offer(key_text: &str, title: &str, added: &str, size: u64) -> Offer {
        let release = Release {
            id: format!("{key_text}-r"),
            title: title.into(),
            platform: "PS5".into(),
            added: added.into(),
            size_bytes: Some(size),
            ..Release::default()
        };
        Offer { entry: entry(key_text, title, "PS5"), emulator: PluginId::new("emu").unwrap(), releases: vec![release] }
    }

    /// A library of `games` with `offers`, and the player's hidden games.
    fn library(games: Vec<Entry>, offers: Vec<Offer>, hidden: &[&str]) -> LibrarySnapshot {
        let config = Arc::new(crate::ConfigService::in_memory());
        let notifier = Notifier::new(Changes::new(), crate::EventBus::default());
        let library = LibrarySubsystem::new(Records::new(config), notifier);
        library.set_library(games, HashMap::new(), Vec::new());
        let mut snapshot = library.snapshot();
        snapshot.offers = Arc::new(offers);
        snapshot.hidden = hidden.iter().map(|k| k.to_string()).collect();
        snapshot
    }

    /// `read` over `library`, with no downloads and no play time.
    fn over<T>(library: &LibrarySnapshot, plays: &PlayTimes, read: impl FnOnce(&GridData) -> T) -> T {
        let (jobs, installs) = (BTreeMap::new(), InstallsSnapshot::default());
        let ownership = Ownership::new(library, &jobs, &installs);
        read(&GridData { library, ownership: &ownership, plays, hidden_filter: true })
    }

    fn keys(library: &LibrarySnapshot, query: &GridQuery) -> Vec<String> {
        over(library, &PlayTimes::default(), |data| query.page(data).iter().map(|k| k.local.clone()).collect())
    }

    #[test]
    fn a_sort_is_one_of_its_names() {
        for sort in Sort::ALL {
            assert_eq!(sort.as_str().parse::<Sort>().unwrap(), sort);
            let json = serde_json::to_value(sort).unwrap();
            assert_eq!(json, sort.as_str(), "a theme sends the same name");
        }
        assert_eq!("newest".parse::<Sort>().unwrap_err().to_string(), "no sort \"newest\"");
    }

    #[test]
    fn the_filter_chips_turn_on_and_off_and_none_clears_them() {
        let mut query = GridQuery::catalog();
        query.set_system(Some("PS5".into()));
        query.set_system(Some(INGAME_FILTER.into()));
        query.set_system(Some(HIDDEN_FILTER.into()));
        assert_eq!((query.system.as_deref(), query.ingame, query.hidden), (Some("PS5"), true, true));
        query.set_system(Some(INGAME_FILTER.into()));
        assert!(!query.ingame, "a second press turns it off");
        query.set_system(None);
        assert_eq!(query, GridQuery::catalog());
    }

    #[test]
    fn a_search_matches_the_starts_of_words_and_an_apostrophe_joins_its_word() {
        let library = library(
            vec![entry("emu:a", "Assassin's Creed", "PS4"), entry("emu:b", "Bloodborne", "PS4")],
            Vec::new(),
            &[],
        );
        let mut query = GridQuery::library();
        query.query = "assassins cre".into();
        assert_eq!(keys(&library, &query), ["a"]);
        query.query = "borne".into();
        assert_eq!(keys(&library, &query), ["b"], "inside the title");
        query.query = "creed bl".into();
        assert!(keys(&library, &query).is_empty(), "every word, in one game");
    }

    #[test]
    fn the_catalog_leaves_hidden_games_out_but_for_the_hidden_filter() {
        let library = library(
            Vec::new(),
            vec![offer("cat:a", "Astro", "2026-10-01", 5), offer("cat:b", "Bloodborne", "2026-10-02", 9)],
            &["cat:b"],
        );
        let mut query = GridQuery::catalog();
        assert_eq!(keys(&library, &query), ["a"]);
        query.set_system(Some(HIDDEN_FILTER.into()));
        assert_eq!(keys(&library, &query), ["b"]);
        let chips = over(&library, &PlayTimes::default(), |data| query.games(data).systems);
        let hidden = chips.iter().find(|c| c.id == HIDDEN_FILTER).expect("the Hidden chip");
        assert_eq!((hidden.count, hidden.on), (1, true));
    }

    #[test]
    fn the_sorts_order_the_games() {
        let library = library(
            Vec::new(),
            vec![
                offer("cat:a", "Astro", "2026-10-02", 50),
                offer("cat:b", "bloodborne", "2026-10-03", 5),
                offer("cat:c", "Celeste", "2026-10-01", 20),
            ],
            &[],
        );
        let sorted = |sort: Sort| keys(&library, &GridQuery { sort, ..GridQuery::catalog() });
        assert_eq!(sorted(Sort::Added), ["b", "a", "c"], "newest first");
        assert_eq!(sorted(Sort::Title), ["a", "b", "c"], "in any case");
        assert_eq!(sorted(Sort::Size), ["a", "c", "b"]);
        assert_eq!(sorted(Sort::Smallest), ["b", "c", "a"]);
    }

    #[test]
    fn a_page_shows_as_many_games_as_the_query_asks() {
        let games: Vec<Entry> = (0..5).map(|i| entry(&format!("emu:g{i}"), &format!("Game {i}"), "PS4")).collect();
        let library = library(games, Vec::new(), &[]);
        let query = GridQuery { shown: 2, ..GridQuery::library() };
        assert_eq!(keys(&library, &query), ["g0", "g1"]);
    }

    #[test]
    fn the_neighbours_are_beside_above_and_below_when_the_grid_has_columns() {
        let games: Vec<Entry> = (0..9).map(|i| entry(&format!("emu:g{i}"), &format!("Game {i}"), "PS4")).collect();
        let library = library(games, Vec::new(), &[]);
        let (lib, cat) = (GridQuery::library(), GridQuery::catalog());
        let near = |columns: &HashMap<Grid, usize>| {
            over(&library, &PlayTimes::default(), |data| {
                neighbours(data, &lib, &cat, columns, &key("emu:g4"), 6)
                    .iter()
                    .map(|k| k.local.clone())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(near(&HashMap::new()), ["g5", "g3"]);
        assert_eq!(near(&HashMap::from([(Grid::Library, 3)])), ["g5", "g3", "g7", "g1"]);
    }

    #[test]
    fn the_shown_catalog_grid_stands_while_another_screen_is_on() {
        let grid = |total| LibraryVm { total, ..LibraryVm::default() };
        let mut shown = ShownGrid::default();
        assert_eq!(shown.get(false), None, "none yet: build it");
        shown.keep(false, &grid(1));
        assert_eq!(shown.get(false).map(|g| g.total), Some(1), "the first one stands");
        shown.keep(false, &grid(2));
        assert_eq!(shown.get(false).map(|g| g.total), Some(1), "off screen, a new one isn't kept");
        assert_eq!(shown.get(true), None, "on screen, it is built");
        shown.keep(true, &grid(3));
        assert_eq!(shown.get(false).map(|g| g.total), Some(3));
        shown.clear();
        assert_eq!(shown.get(false), None);
    }
}
