//! Which of the player's games a Catalog offer is, and the offer's release that counts: pure
//! reads over the library, the downloads and the game installs.
//!
//! An offer and an installed game are two records with two keys: the catalog plugin's and the
//! emulator's. [`Ownership`] links them, by the download that installed the game, by title id,
//! by name on the same console, or by a game's whole name that starts the offer's: a game
//! named from its file, "Mario Strikers Battle League", is the offer "Mario Strikers: Battle
//! League Football". The view asks it about every offer, so it builds its maps once.

use std::collections::{BTreeMap, HashMap};

use nexus_api::{GameKey, PluginId, Release};

use crate::game_installs::{GamePhase, InstallsSnapshot};
use crate::library::{Entry, InstalledOffer, LibrarySnapshot};
use crate::releases::Offer;
use crate::torrents::TorrentJob;

/// The emulators' installed games by title id and by name, with the downloads and installs
/// that say which game a download became.
pub(crate) struct Ownership<'a> {
    library: &'a LibrarySnapshot,
    /// The downloads, whose installs say which game an offer is.
    jobs: &'a BTreeMap<String, TorrentJob>,
    /// The game installs, which say which game a download installed.
    installs: &'a InstallsSnapshot,
    /// The installed games.
    games: Vec<&'a Entry>,
    by_id: HashMap<(&'a PluginId, &'a str), &'a Entry>,
    /// By the emulator, and the console and the title as letters and digits (`ps5/astrobot`).
    by_name: HashMap<(&'a PluginId, String), &'a Entry>,
    /// The games of [`MIN_PREFIX_WORDS`] words or more, with their console and their title's
    /// words, for a name that starts an offer's.
    by_words: Vec<(&'a Entry, String, Vec<String>)>,
}

/// The fewest words a game's name has for it to link to an offer whose name it starts: one
/// word ("Doom") starts too many names.
const MIN_PREFIX_WORDS: usize = 2;

impl<'a> Ownership<'a> {
    /// Over the Library's games.
    pub fn new(
        library: &'a LibrarySnapshot,
        jobs: &'a BTreeMap<String, TorrentJob>,
        installs: &'a InstallsSnapshot,
    ) -> Self {
        Self::over(library.library(), library, jobs, installs)
    }

    /// Over `games`, a list the library doesn't hold yet: a library load's.
    pub fn over(
        games: impl IntoIterator<Item = &'a Entry>,
        library: &'a LibrarySnapshot,
        jobs: &'a BTreeMap<String, TorrentJob>,
        installs: &'a InstallsSnapshot,
    ) -> Self {
        let games: Vec<&Entry> = games.into_iter().collect();
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        let mut by_words = Vec::new();
        for e in &games {
            let title = words(&e.summary.title);
            if title.len() >= MIN_PREFIX_WORDS {
                by_words.push((*e, e.summary.platform.to_ascii_lowercase(), title));
            }
            if let Some(id) = e.summary.title_id.as_deref().filter(|t| !t.is_empty()) {
                by_id.entry((&e.key.plugin, id)).or_insert(*e);
            }
            if let Some(name) = name(&e.summary.platform, &e.summary.title) {
                by_name.entry((&e.key.plugin, name)).or_insert(*e);
            }
        }
        Self { library, jobs, installs, games, by_id, by_name, by_words }
    }

    /// The game of the emulator's library that the download `job` installed: the one with the
    /// title its install reported. A PC game has no title id, so this is how its download
    /// finds it.
    pub fn installed_from(&self, job: &TorrentJob) -> Option<&'a Entry> {
        let install = self.installs.latest(&job.emulator, &job.source)?;
        let title = &install.installed.as_ref().filter(|_| install.phase == GamePhase::Done)?.title;
        self.games.iter().copied().find(|e| e.key.plugin == job.emulator && &e.summary.title == title)
    }

    /// The installed game an offer is, from the emulator's library: the game a download of one
    /// of its releases installed, else the game with one of its title ids, else the game of its
    /// name on the same console. A PS4 and a PS5 game of one name are two games.
    pub fn installed_game(&self, offer: &Offer) -> Option<&'a Entry> {
        self.linked(offer).or_else(|| self.by_prefix(offer))
    }

    /// The installed game the offer is, by its download, its title id or its name.
    fn linked(&self, offer: &Offer) -> Option<&'a Entry> {
        let downloaded = self.jobs.values().filter(|j| j.offer == offer.entry.key).find_map(|j| self.installed_from(j));
        let emulator = &offer.emulator;
        let by_id = || {
            let mut ids = offer.releases.iter().flat_map(|r| std::iter::once(&r.title_id).chain(&r.title_ids));
            ids.find_map(|t| self.by_id.get(&(emulator, t.as_str())).copied())
        };
        let by_name = || {
            let console = &offer.entry.summary.platform;
            let mut names = offer.releases.iter().filter_map(|r| name(console, &r.title));
            names.find_map(|name| self.by_name.get(&(emulator, name)).copied())
        };
        downloaded.or_else(by_id).or_else(by_name)
    }

    /// The one installed game, of the offer's emulator and console, whose whole name starts the
    /// name of one of the offer's releases, word by word. `None` when no game or more than one
    /// fits.
    fn by_prefix(&self, offer: &Offer) -> Option<&'a Entry> {
        let console = offer.entry.summary.platform.to_ascii_lowercase();
        let names: Vec<Vec<String>> = offer.releases.iter().map(|r| words(&r.title)).collect();
        let mut found: Option<&'a Entry> = None;
        for (game, on, title) in &self.by_words {
            if game.key.plugin != offer.emulator || *on != console {
                continue;
            }
            if names.iter().any(|name| name.len() > title.len() && name.starts_with(title)) {
                match found {
                    Some(other) if other.key != game.key => return None,
                    _ => found = Some(game),
                }
            }
        }
        found
    }

    /// The release an offer's card shows: the version the player has, then a release whose
    /// download installed the game, then one with a magnet link, then one with art, then the
    /// highest version, then the one most people share. Of equals, the newest upload.
    pub fn best_release<'o>(&self, offer: &'o Offer) -> &'o Release {
        use crate::releases::version_key;
        let has = self
            .installed_game(offer)
            .and_then(|e| self.library.detail(&e.key))
            .and_then(|d| d.details.facts.iter().find(|f| f.id == "version"))
            .map(|f| version_key(&f.value))
            .filter(|v| !v.is_empty());
        let rank = |r: &Release| {
            let downloaded = self.jobs.get(&r.id).and_then(|j| self.installed_from(j)).is_some();
            (
                has.as_ref().is_some_and(|v| *v == version_key(&r.version)),
                downloaded,
                !r.torrent.is_empty(),
                r.metadata.is_some() || !r.image_urls.is_empty(),
                version_key(&r.version),
                r.seeders.unwrap_or(0),
            )
        };
        // The releases are newest upload first, and `max_by_key` keeps the last of equals.
        offer.releases.iter().rev().max_by_key(|r| rank(r)).unwrap_or(&offer.releases[0])
    }

    /// The game Play starts for `key`: for a Catalog offer the player has installed, the
    /// installed game it is; else `key` itself.
    pub fn played_game(&self, key: &GameKey) -> GameKey {
        let installed = self.library.offer(key).and_then(|o| self.installed_game(o));
        installed.map(|g| g.key.clone()).unwrap_or_else(|| key.clone())
    }

    /// The offer each installed game is, by the game, with the offer's cover.
    ///
    /// A game whose name starts the names of two offers or more is none of them: which one it is
    /// can't be told.
    pub fn installed_offers(&self, offers: &[Offer]) -> Vec<InstalledOffer> {
        let links: Vec<(&Offer, &Entry, bool)> = offers
            .iter()
            .filter_map(|offer| match self.linked(offer) {
                Some(game) => Some((offer, game, false)),
                None => self.by_prefix(offer).map(|game| (offer, game, true)),
            })
            .collect();
        let mut by_prefix: HashMap<&GameKey, usize> = HashMap::new();
        for (_, game, prefix) in &links {
            if *prefix {
                *by_prefix.entry(&game.key).or_default() += 1;
            }
        }
        links
            .into_iter()
            .filter(|(_, game, prefix)| !prefix || by_prefix.get(&game.key) == Some(&1))
            .map(|(offer, game, _)| {
                let cover = offer.entry.summary.cover.clone().map(|c| (offer.entry.key.plugin.clone(), c));
                (game.key.clone(), offer.entry.key.clone(), cover)
            })
            .collect()
    }
}

/// A title's words, in lower case: letters and digits. An apostrophe joins its word
/// ("Assassin's" is "assassins").
fn words(title: &str) -> Vec<String> {
    let title = title.to_lowercase().replace(['\'', '\u{2019}'], "");
    title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// A game's name to find it by: its console and its title, as letters and digits.
fn name(console: &str, title: &str) -> Option<String> {
    let title = crate::releases::plain(title);
    (!title.is_empty()).then(|| format!("{}/{title}", console.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_api::{GameDetails, GameFact, GameSummary};

    use super::*;
    use crate::changes::{Changes, Notifier};
    use crate::library::{Detail, LibrarySubsystem, Records};

    fn key(text: &str) -> GameKey {
        text.parse().unwrap()
    }

    fn summary(id: &str, title: &str, platform: &str, title_id: Option<&str>) -> GameSummary {
        GameSummary {
            id: id.into(),
            title: title.into(),
            platform: platform.into(),
            cover: None,
            tags: Vec::new(),
            release_year: None,
            title_id: title_id.map(str::to_owned),
            content_id: None,
        }
    }

    fn game(key_text: &str, title: &str, platform: &str, title_id: Option<&str>) -> Entry {
        let key = key(key_text);
        Entry { summary: summary(&key.local, title, platform, title_id), key }
    }

    fn release(id: &str, title: &str, title_id: &str, version: &str) -> Release {
        Release {
            id: id.into(),
            title: title.into(),
            title_id: title_id.into(),
            version: version.into(),
            platform: "PS5".into(),
            ..Release::default()
        }
    }

    fn offer(key_text: &str, platform: &str, releases: Vec<Release>) -> Offer {
        let key = key(key_text);
        let entry = Entry { summary: summary(&key.local, &releases[0].title, platform, None), key };
        Offer { entry, emulator: PluginId::new("emu").unwrap(), releases }
    }

    /// A library of `games`, with these details, and these offers.
    fn library(games: Vec<Entry>, details: Vec<(GameKey, Detail)>, offers: Vec<Offer>) -> LibrarySnapshot {
        let config = Arc::new(crate::ConfigService::in_memory());
        let notifier = Notifier::new(Changes::new(), crate::EventBus::default());
        let library = LibrarySubsystem::new(Records::new(config), notifier);
        library.set_library(games, HashMap::new(), Vec::new());
        for (key, detail) in details {
            library.set_detail(&key, detail, None);
        }
        let mut snapshot = library.snapshot();
        snapshot.offers = Arc::new(offers);
        snapshot
    }

    fn owned<T>(library: &LibrarySnapshot, read: impl FnOnce(&Ownership) -> T) -> T {
        let (jobs, installs) = (BTreeMap::new(), InstallsSnapshot::default());
        read(&Ownership::new(library, &jobs, &installs))
    }

    #[test]
    fn an_offer_is_the_installed_game_with_one_of_its_title_ids() {
        let astro = game("emu:G1", "Astro Bot (Japan)", "PS5", Some("PPSA01234"));
        let mut other_region = release("a", "Astro", "PPSA00001", "1.0");
        other_region.title_ids = vec!["PPSA01234".into()];
        let library = library(vec![astro], Vec::new(), vec![offer("cat:o1", "PS5", vec![other_region])]);
        let found = owned(&library, |o| o.installed_game(&library.offers[0]).map(|e| e.key.clone()));
        assert_eq!(found, Some(key("emu:G1")), "a title id the release also carries");
    }

    #[test]
    fn an_offer_is_the_installed_game_of_its_name_on_the_same_console_only() {
        let library = library(
            vec![game("emu:G1", "Astro's Playroom", "PS5", None)],
            Vec::new(),
            vec![
                offer("cat:ps5", "PS5", vec![release("a", "ASTRO'S PLAYROOM", "", "1.0")]),
                offer("cat:ps4", "PS4", vec![release("b", "Astro's Playroom", "", "1.0")]),
            ],
        );
        owned(&library, |o| {
            assert_eq!(o.installed_game(&library.offers[0]).map(|e| e.key.clone()), Some(key("emu:G1")));
            assert!(o.installed_game(&library.offers[1]).is_none(), "a PS4 game of one name is another game");
        });
    }

    #[test]
    fn an_offer_is_the_installed_game_whose_whole_name_starts_its_name() {
        let library = library(
            vec![game("emu:G1", "Mario Strikers Battle League", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Strikers: Battle League Football", "", "1.3.2")])],
        );
        let found = owned(&library, |o| o.installed_game(&library.offers[0]).map(|e| e.key.clone()));
        assert_eq!(found, Some(key("emu:G1")), "the EU name says more than the file's");
        let linked = owned(&library, |o| o.installed_offers(&library.offers));
        assert_eq!(
            linked.iter().map(|(g, o, _)| (g.clone(), o.clone())).collect::<Vec<_>>(),
            [(key("emu:G1"), key("cat:o1"))]
        );
    }

    #[test]
    fn a_name_that_starts_another_links_only_when_it_is_sure() {
        // One word is too little: "Doom" starts many names.
        let one_word = library(
            vec![game("emu:G1", "Doom", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Doom Eternal", "", "1.0")])],
        );
        assert!(owned(&one_word, |o| o.installed_game(&one_word.offers[0]).is_none()), "one word");

        // Two games whose names both start the offer's: neither is it.
        let two_games = library(
            vec![game("emu:G1", "Mario Kart", "Switch", None), game("emu:G2", "Mario Kart 8", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Kart 8 Deluxe", "", "1.0")])],
        );
        assert!(owned(&two_games, |o| o.installed_game(&two_games.offers[0]).is_none()), "two games fit");

        // A game whose name starts two offers' names is neither, for art and game files.
        let two_offers = library(
            vec![game("emu:G1", "Mario Kart", "Switch", None)],
            Vec::new(),
            vec![
                offer("cat:o1", "Switch", vec![release("a", "Mario Kart 8 Deluxe", "", "1.0")]),
                offer("cat:o2", "Switch", vec![release("b", "Mario Kart World", "", "1.0")]),
            ],
        );
        assert!(owned(&two_offers, |o| o.installed_offers(&two_offers.offers)).is_empty(), "two offers fit");

        // Words, not letters: "Mario Strike" doesn't start "Mario Strikers".
        let half_word = library(
            vec![game("emu:G1", "Mario Strike", "Switch", None)],
            Vec::new(),
            vec![offer("cat:o1", "Switch", vec![release("a", "Mario Strikers Battle League", "", "1.0")])],
        );
        assert!(owned(&half_word, |o| o.installed_game(&half_word.offers[0]).is_none()), "half a word");
    }

    #[test]
    fn an_offer_of_another_emulator_is_not_the_game() {
        let library = library(
            vec![game("other:G1", "Astro", "PS5", Some("PPSA01234"))],
            Vec::new(),
            vec![offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")])],
        );
        assert!(owned(&library, |o| o.installed_game(&library.offers[0]).is_none()));
    }

    #[test]
    fn play_on_an_installed_offer_starts_the_installed_game() {
        let library = library(
            vec![game("emu:G1", "Astro", "PS5", Some("PPSA01234"))],
            Vec::new(),
            vec![
                offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")]),
                offer("cat:o2", "PS5", vec![release("b", "Other", "PPSA09999", "1.0")]),
            ],
        );
        owned(&library, |o| {
            assert_eq!(o.played_game(&key("cat:o1")), key("emu:G1"));
            assert_eq!(o.played_game(&key("cat:o2")), key("cat:o2"), "not installed: the offer itself");
            assert_eq!(o.played_game(&key("emu:G1")), key("emu:G1"), "a Library game");
        });
    }

    #[test]
    fn the_best_release_is_the_version_the_player_has_then_the_highest() {
        let releases = vec![
            release("new", "Astro", "PPSA01234", "1.00"),
            release("high", "Astro", "PPSA01234", "1.05"),
            release("mine", "Astro", "PPSA01234", "1.02"),
        ];
        let not_installed = library(Vec::new(), Vec::new(), vec![offer("cat:o1", "PS5", releases.clone())]);
        assert_eq!(owned(&not_installed, |o| o.best_release(&not_installed.offers[0]).id.clone()), "high");

        let astro = game("emu:G1", "Astro", "PS5", Some("PPSA01234"));
        let details = GameDetails {
            summary: astro.summary.clone(),
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: None,
            facts: vec![GameFact::new("version", "Version", "01.02")],
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
        };
        let installed =
            library(vec![astro], vec![(key("emu:G1"), Detail::of(details))], vec![offer("cat:o1", "PS5", releases)]);
        assert_eq!(owned(&installed, |o| o.best_release(&installed.offers[0]).id.clone()), "mine");
    }

    #[test]
    fn each_installed_game_names_the_offer_it_is() {
        let library = library(
            vec![game("emu:G1", "Astro", "PS5", Some("PPSA01234"))],
            Vec::new(),
            vec![
                offer("cat:o1", "PS5", vec![release("a", "Astro", "PPSA01234", "1.0")]),
                offer("cat:o2", "PS5", vec![release("b", "Other", "PPSA09999", "1.0")]),
            ],
        );
        let found = owned(&library, |o| o.installed_offers(&library.offers));
        assert_eq!(found, vec![(key("emu:G1"), key("cat:o1"), None)]);
    }
}
