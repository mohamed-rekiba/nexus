//! The Catalog's grid: cards, chips, search, sorts, the loading state and hidden games.

use super::*;

/// A catalog's topic of `name` for `platform`, with this title id (empty for none).
fn named(id: u32, name: &str, platform: &str, title_id: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[{platform}] {name} [EUR]"), "platform": platform,
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": "2026-10-01",
        "game_info": { "name": name, "title_id": title_id, "genre": "Action" } })
}

/// The Catalog's cards, as (title, platform, badges), sorted.
fn cards(t: &TestCore) -> Vec<(String, String, Vec<String>)> {
    let mut cards: Vec<_> = t
        .core
        .view()
        .catalog
        .games
        .games
        .iter()
        .map(|c| (c.title.clone(), c.platform.clone(), c.badges.iter().map(|b| b.text.clone()).collect()))
        .collect();
    cards.sort();
    cards
}

#[tokio::test]
async fn releases_of_one_name_for_two_consoles_stay_two_games() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([
            named(1, "Astro", "PS5", "PPSA01234", 'a'),
            named(2, "Astro", "PS4", "CUSA01234", 'b'),
            named(3, "Bolt", "PS5", "", 'c'),
            named(4, "Bolt", "PS4", "", 'd'),
        ]),
        false,
    )
    .await;
    let mut games: Vec<(String, String, usize)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.key.clone(), c.releases)).collect();
    games.sort();
    assert_eq!(games.len(), 4, "{games:?}");
    let keys: std::collections::HashSet<&String> = games.iter().map(|(_, key, _)| key).collect();
    assert_eq!(keys.len(), 4, "each game has its own key: {games:?}");
    assert!(games.iter().all(|(_, _, releases)| *releases == 1), "{games:?}");
}

#[tokio::test]
async fn an_installed_game_matches_its_offer_by_name_on_the_same_console_and_has_a_chip() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS5").games(["Alpha"]).build().console("PS5"))
        .boot_core()
        .await;
    let topics = serde_json::json!([
        named(1, "Alpha", "PS5", "", 'a'),
        named(2, "Alpha", "PS4", "", 'b'),
        named(3, "Bolt", "PS5", "", 'c')
    ]);
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    let installed = |t: &TestCore| {
        cards(t).into_iter().filter(|(_, _, badges)| badges.contains(&"Installed".to_owned())).collect::<Vec<_>>()
    };
    assert_eq!(
        installed(&t),
        [("Alpha".to_owned(), "PS5".to_owned(), vec!["Installed".to_owned()])],
        "the PS4 game of the same name is another game"
    );
    let systems = t.core.view().catalog.games.systems;
    let chip = systems.iter().find(|s| s.id == nexus_core::INSTALLED_FILTER).expect("an Installed chip");
    assert_eq!((chip.label.as_str(), chip.count, chip.on), ("Installed", 1, false));

    t.core.dispatch(Command::CatalogSystem(Some(nexus_core::INSTALLED_FILTER.into()))).await.unwrap();
    let shown: Vec<(String, String)> = cards(&t).into_iter().map(|(title, platform, _)| (title, platform)).collect();
    assert_eq!(shown, [("Alpha".to_owned(), "PS5".to_owned())], "only the installed game");
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    assert_eq!(cards(&t).len(), 3);
}

/// A topic of `name` at `version`, `size` GB, with `seeders`, for the PS5.
fn sized(id: u32, name: &str, version: &str, size: u64, seeders: u64, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] {name} [EUR / {version}]"), "platform": "PS5",
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": format!("2026-10-{id:02}"),
        "size_bytes": size * 1_000_000_000, "seeders": seeders,
        "game_info": { "name": name, "version": version, "genre": "Action" } })
}

#[tokio::test]
async fn a_card_shows_its_best_release_and_the_game_hub_orders_them_by_version() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS5").games(["Alpha"]).build().console("PS5"))
        .boot_core()
        .await;
    let mut art = sized(5, "Quest", "1.00", 5, 1, 'e');
    art["steam"] = steam_block();
    let topics = serde_json::json!([
        // The highest version wins, not the newest upload.
        sized(1, "Racer", "1.05", 3, 1, 'a'),
        sized(2, "Racer", "1.02", 2, 1, 'b'),
        sized(3, "Racer", "1.00", 1, 1, 'c'),
        // A release with art wins over a higher version.
        sized(4, "Quest", "2.00", 6, 1, 'd'),
        art,
        // The version the player has wins over all.
        sized(6, "Alpha", "02.000.000", 8, 9, 'f'),
        sized(7, "Alpha", "01.000.002", 7, 0, '1'),
        // Of one version, the one more people share.
        sized(8, "Bolt", "1.00", 4, 1, '2'),
        sized(9, "Bolt", "1.00", 9, 30, '3'),
    ]);
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let alpha = t.core.view().home.games.first().cloned().expect("the installed Alpha");
    t.core.dispatch(Command::OpenGame(alpha.key.parse().unwrap())).await.unwrap();
    t.core.dispatch(Command::Back).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    let mut sizes: Vec<(String, Option<u64>)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.size_bytes)).collect();
    sizes.sort();
    let gb = |n: u64| Some(n * 1_000_000_000);
    assert_eq!(
        sizes,
        [
            ("Alpha".to_owned(), gb(7)),
            ("Bolt".to_owned(), gb(9)),
            ("Quest".to_owned(), gb(5)),
            ("Racer".to_owned(), gb(3))
        ]
    );

    let racer = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Racer").unwrap();
    t.core.dispatch(Command::OpenGame(racer.key.parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    let releases: Vec<(&str, &str, bool)> =
        detail.releases.iter().map(|r| (r.version.as_str(), r.version_label.as_str(), r.best)).collect();
    assert_eq!(
        releases,
        [
            ("1.05", "Version 3 of 3 (newest)", true),
            ("1.02", "Version 2 of 3", false),
            ("1.00", "Version 1 of 3", false)
        ]
    );
}

#[tokio::test]
async fn a_card_and_the_game_hub_say_how_available_a_release_is() {
    let mut low = sized(2, "Bolt", "1.00", 1, 2, 'b');
    low["leechers"] = 4.into();
    low["peers_observed"] = "2026-10-07T14:07:32Z".into();
    let mut unknown = sized(4, "Drift", "1.00", 1, 0, 'd');
    unknown["seeders"] = serde_json::Value::Null;
    let topics =
        serde_json::json!([sized(1, "Astro", "1.00", 1, 3, 'a'), low, sized(3, "Comet", "1.00", 1, 0, 'c'), unknown]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let mut cards: Vec<(String, String)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.availability.clone())).collect();
    cards.sort();
    let want = [("Astro", "good"), ("Bolt", "low"), ("Comet", "none"), ("Drift", "unknown")];
    assert_eq!(cards, want.map(|(a, b)| (a.to_owned(), b.to_owned())), "3 or more is good, 1 or 2 low");

    let bolt = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Bolt").unwrap();
    t.core.dispatch(Command::OpenGame(bolt.key.parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.availability, "low", "the best release's");
    let r = &detail.releases[0];
    assert_eq!((r.seeders, r.leechers, r.counted_at.as_str()), (Some(2), Some(4), "2026-10-07T14:07:32Z"));
    assert_eq!(r.availability, "low");
}

#[tokio::test]
async fn the_catalog_s_genre_chips_are_the_fixed_genres() {
    let genre = |id: u32, name: &str, genre: &str, hash: char| {
        let mut topic = named(id, name, "PS5", "", hash);
        topic["game_info"]["genre"] = genre.into();
        topic
    };
    let topics = serde_json::json!([
        genre(1, "Astro", "Hack and Slash, Role-Playing", 'a'),
        genre(2, "Bolt", "Action", 'b'),
        genre(3, "Comet", "Card game", 'c'),
        genre(4, "Drift", "", 'd'),
    ]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let chips: Vec<(String, usize)> =
        t.core.view().catalog.games.genres.iter().map(|g| (g.label.clone(), g.count)).collect();
    let want = [("All genres", 4), ("Action", 2), ("Other", 1), ("RPG", 1)];
    assert_eq!(chips, want.map(|(g, n)| (g.to_owned(), n)), "a game with no genre words has no chip");

    t.core.dispatch(Command::CatalogGenre(Some("Action".into()))).await.unwrap();
    let mut titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, ["Astro", "Bolt"]);
}

#[tokio::test]
async fn the_catalog_s_search_matches_each_word_in_the_title_title_id_publisher_region_and_version() {
    let release = |id: u32, name: &str, title_id: &str, publisher: &str, region: &str, version: &str, hash: char| {
        let mut topic = named(id, name, "PS5", title_id, hash);
        topic["game_info"]["publisher"] = publisher.into();
        topic["game_info"]["region"] = region.into();
        topic["game_info"]["version"] = version.into();
        topic
    };
    let topics = serde_json::json!([
        release(1, "Astro Bot", "PPSA01234", "Sony Interactive", "EUR", "v01.05", 'a'),
        release(2, "Astro Bot", "PPSA01234", "Sony Interactive", "USA", "v01.02", 'b'),
        release(3, "Bolt", "PPSA05555", "Ubisoft", "USA", "v2.00", 'c'),
    ]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let found = |query: &'static str| {
        let t = &t;
        async move {
            t.core.dispatch(Command::CatalogSearch(query.into())).await.unwrap();
            let mut titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
            titles.sort();
            titles
        }
    };
    assert_eq!(found("ppsa05555").await, ["Bolt"], "the title id");
    assert_eq!(found("sony eur").await, ["Astro Bot"], "the publisher and the region");
    assert_eq!(found("bot astro").await, ["Astro Bot"], "each word, in any order");
    assert_eq!(found("usa v2").await, ["Bolt"], "the region and the version, of one release");
    assert!(found("ubisoft eur").await.is_empty(), "every word, in one release");
    assert_eq!(found("ast eu").await, ["Astro Bot"], "the start of each word");
    assert!(found("stro eur").await.is_empty(), "not the middle of one");
    assert_eq!(found("stro b").await, ["Astro Bot"], "the whole search inside the title");
    assert_eq!(found("usa").await, ["Astro Bot", "Bolt"]);
}

#[tokio::test]
async fn a_topic_s_first_picture_is_the_cover_when_there_is_no_steam_art() {
    let picture = |id: u32, name: &str, hash: char| {
        let mut topic = named(id, name, "PS5", "", hash);
        topic["game_info"]["image_urls"] =
            serde_json::json!(["https://i1.imageban.ru/out/cover.jpg", "https://i1.imageban.ru/out/shot.jpg"]);
        topic
    };
    let mut steam = picture(2, "Celeste", 'b');
    steam["steam"] = steam_block();
    let t = offering("emu-kyty", "PS5", serde_json::json!([picture(1, "Astro", 'a'), steam]), false).await;
    let cover = |title: &str| t.core.view().catalog.games.games.into_iter().find(|c| c.title == title).unwrap().cover;
    let topic_s = nexus_ui_api::ImageRef::Url {
        plugin: id("release-catalog"),
        url: "https://i1.imageban.ru/out/cover.jpg".into(),
    };
    assert_eq!(cover("Astro"), Some(topic_s.clone()), "the topic's first picture");
    assert_eq!(cover("Celeste"), steam_image("portrait.jpg"), "Steam's art comes first");

    let astro = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Astro").unwrap();
    t.core.dispatch(Command::OpenGame(astro.key.parse().unwrap())).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().cover, Some(topic_s), "the Game Hub's cover too");
}

#[tokio::test]
async fn the_catalog_sorts_by_size_smallest_first_by_compatibility_and_by_the_catalog_s_release_date() {
    let mut astro = sized(1, "Astro", "1.00", 5, 1, 'a');
    astro["game_info"]["release_date"] = "2021-03-04".into();
    let mut bolt = sized(2, "Bolt", "1.00", 1, 1, 'b');
    bolt["game_info"]["release_year"] = "2024".into();
    let mut comet = sized(3, "Comet", "1.00", 3, 1, 'c');
    comet["game_info"]["release_date"] = "2023-11-30".into();
    // No size and no date: last in both.
    let mut drift = sized(4, "Drift", "1.00", 0, 1, 'd');
    drift.as_object_mut().unwrap().remove("size_bytes");
    let t = offering("emu-kyty", "PS5", serde_json::json!([astro, bolt, comet, drift]), false).await;

    t.core.dispatch(Command::CatalogSort("smallest".into())).await.unwrap();
    assert_eq!(titles(&t), ["Bolt", "Comet", "Astro", "Drift"], "smallest first, no size last");
    t.core.dispatch(Command::CatalogSort("size".into())).await.unwrap();
    assert_eq!(titles(&t), ["Astro", "Comet", "Bolt", "Drift"], "largest first");
    t.core.dispatch(Command::CatalogSort("release".into())).await.unwrap();
    assert_eq!(titles(&t), ["Bolt", "Comet", "Astro", "Drift"], "newest release date first, a year counts");
    t.core.dispatch(Command::CatalogSort("compat".into())).await.unwrap();
    assert_eq!(t.core.view().catalog.games.sort, "compat");
}

#[tokio::test]
async fn the_catalog_says_when_its_list_was_updated_and_counts_its_games_and_releases() {
    let mut astro = sized(1, "Astro", "1.00", 1, 3, 'a');
    astro["peers_observed"] = "2026-10-07T14:07:32Z".into();
    let mut newer = sized(2, "Astro", "1.01", 1, 3, 'b');
    newer["peers_observed"] = "2026-10-19T21:07:34.000Z".into();
    let t = offering("emu-kyty", "PS5", serde_json::json!([astro, newer, sized(3, "Bolt", "1.00", 1, 3, 'c')]), false)
        .await;
    let view = t.core.view().catalog;
    assert_eq!(view.updated, "2026-10-19", "the newest count of seeders");
    assert_eq!((view.games.total, view.games.all, view.releases), (2, 2, 3));
    assert!(!view.loading);

    t.core.dispatch(Command::CatalogSearch("bolt".into())).await.unwrap();
    let view = t.core.view().catalog;
    assert_eq!((view.games.total, view.games.all, view.releases), (1, 2, 3), "the search leaves 1 of 2 games");

    // With no count of seeders, the date the list was collected.
    let t = with_offers().await;
    assert_eq!(t.core.view().catalog.updated, "2026-10-18");
}

/// A catalog service whose catalogs come only once `open` has permits.
fn slow_catalogs(open: Arc<tokio::sync::Semaphore>) -> StubPlugin {
    struct Slow(Arc<tokio::sync::Semaphore>);
    #[nexus_api::async_trait]
    impl nexus_api::Releases for Slow {
        async fn releases(&self, _: &nexus_api::Console) -> nexus_api::PluginResult<Option<ReleaseList>> {
            let _permit = self.0.acquire().await;
            Ok(None)
        }
    }
    StubPlugin::from_toml(
        "id = \"slow-catalog\"\nkind = \"service\"\nname = \"Slow\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         capabilities = [\"releases\"]",
    )
    .provides(move |_| Ok(nexus_api::Provided { releases: Some(Arc::new(Slow(open.clone()))), ..Default::default() }))
}

#[tokio::test]
async fn the_catalog_opens_at_once_and_says_it_is_loading_until_the_catalogs_are_read() {
    let open = Arc::new(tokio::sync::Semaphore::new(0));
    let t = FakeHost::new()
        .plugin(slow_catalogs(open.clone()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS5").games([]).build().console("PS5"))
        .boot_core()
        .await;
    let core = t.core.clone();
    let navigate = tokio::spawn(async move { core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await });
    t.until(Area::Navigation, |core| core.view().screen == nexus_ui_api::ScreenId::Catalog).await;
    assert_eq!(t.core.view().screen, nexus_ui_api::ScreenId::Catalog, "the Catalog opens before the read ends");
    assert!(t.core.view().catalog.loading, "the catalogs are not read yet");

    open.add_permits(10);
    navigate.await.unwrap().unwrap();
    assert!(!t.core.view().catalog.loading, "read");
}

#[tokio::test]
async fn the_game_hub_s_facts_come_from_the_best_release() {
    let mut best = sized(1, "Astro", "1.05", 2, 5, 'a');
    best["leechers"] = 2.into();
    best["peers_observed"] = "2026-10-07T14:07:32Z".into();
    best["game_info"] = serde_json::json!({ "name": "Astro", "version": "1.05", "region": "EUR",
        "release_summary": "The best release.", "publisher": "Sony", "developer": "Team Asobi",
        "release_date": "2024-09-06", "multiplayer": "No", "release_update": "v01.05", "format": "fpkg",
        "minimum_firmware": "5.xx", "interface_languages": "English, French", "audio_languages": "English" });
    // A newer upload of a lower version.
    let mut newest = sized(9, "Astro", "1.00", 1, 1, 'b');
    newest["game_info"]["release_summary"] = "The newest upload.".into();
    newest["game_info"]["region"] = "USA".into();
    let t = offering("emu-kyty", "PS5", serde_json::json!([best, newest]), false).await;
    let detail = open_first(&t).await;
    assert_eq!(detail.description, "The best release.");
    assert_eq!(detail.developer, "Team Asobi");
    let facts: Vec<(&str, &str, &str)> =
        detail.facts.iter().map(|f| (f.id.as_str(), f.label.as_str(), f.value.as_str())).collect();
    assert_eq!(
        facts,
        [
            ("region", "Region", "EUR"),
            ("version", "Version", "1.05"),
            ("publisher", "Publisher", "Sony"),
            ("developer", "Developer", "Team Asobi"),
            ("release-date", "Release date", "Sep 6, 2024"),
            ("players", "Players", "Single player"),
            ("update", "Update", "v01.05"),
            ("format", "Format", "fpkg"),
            ("firmware", "Minimum firmware", "5.xx"),
            ("interface-languages", "Interface languages", "English, French"),
            ("audio-languages", "Audio languages", "English"),
            ("seeders", "Sharing it (seeders)", "5"),
            ("leechers", "Getting it (leechers)", "2"),
            ("counted", "Counted", "Oct 7, 2026 · not live"),
        ]
    );
}

#[tokio::test]
async fn with_the_game_catalog_turned_off_there_is_no_catalog_to_open() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
        .boot_core()
        .await;
    assert!(t.core.view().catalog.available);
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Downloads)).await.unwrap();

    let off = Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: false };
    t.core.dispatch(off).await.unwrap();

    assert!(!t.core.view().catalog.available);
    let opened = t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await;
    assert!(matches!(opened, Err(nexus_core::CoreError::Invalid(_))), "{opened:?}");
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, nexus_ui_api::ScreenId::Home, "back past where the Catalog was");

    let on = Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: true };
    t.core.dispatch(on).await.unwrap();
    assert!(t.core.view().catalog.available);
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
}

#[tokio::test]
async fn a_hidden_game_leaves_the_catalog_and_the_hidden_filter_shows_it() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
        .boot_core()
        .await;
    let topics: Vec<serde_json::Value> = ["Astro", "Bolt", "Comet"]
        .iter()
        .enumerate()
        .map(|(n, name)| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] {name}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": name } })
        })
        .collect();
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", serde_json::Value::Array(topics)));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    let titles =
        |t: &TestCore| -> Vec<String> { t.core.view().catalog.games.games.iter().map(|g| g.title.clone()).collect() };
    let hidden_chip =
        |t: &TestCore| t.core.view().catalog.games.systems.into_iter().find(|s| s.id == nexus_core::HIDDEN_FILTER);
    assert_eq!(titles(&t), ["Astro", "Bolt", "Comet"]);
    assert!(hidden_chip(&t).is_none(), "no Hidden chip while nothing is hidden");
    let bolt: nexus_api::GameKey =
        t.core.view().catalog.games.games.iter().find(|g| g.title == "Bolt").unwrap().key.parse().unwrap();

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Hide { game: bolt.clone(), hidden: true })).await.unwrap();

    assert_eq!(titles(&t), ["Astro", "Comet"]);
    let grid = t.core.view().catalog;
    assert_eq!((grid.games.total, grid.games.all, grid.releases), (2, 2, 2), "a hidden game counts nowhere");
    let chip = hidden_chip(&t).expect("a Hidden chip");
    assert_eq!((chip.label.as_str(), chip.count, chip.on), ("Hidden", 1, false));
    assert_eq!(t.core.kernel().config().get(&["app", "hidden_games"]), serde_json::json!([bolt.to_string()]));

    t.core.dispatch(Command::CatalogSystem(Some(nexus_core::HIDDEN_FILTER.into()))).await.unwrap();
    assert_eq!(titles(&t), ["Bolt"], "the Hidden chip shows only the hidden games");
    assert!(t.core.view().catalog.games.games[0].hidden);

    // The Game Catalog's "Show the Hidden filter" off: no chip, and its filter no longer applies.
    let filter = |on: bool| Command::SetConfigField {
        plugin: id("release-catalog"),
        scope: ConfigScope::Emulator,
        key: "show_hidden_filter".into(),
        value: on.to_string(),
    };
    t.core.dispatch(filter(false)).await.unwrap();
    assert!(hidden_chip(&t).is_none(), "the chip is off");
    assert_eq!(titles(&t), ["Astro", "Comet"], "the hidden game stays hidden");
    t.core.dispatch(filter(true)).await.unwrap();
    assert_eq!(hidden_chip(&t).map(|c| c.count), Some(1));

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Hide { game: bolt, hidden: false })).await.unwrap();
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    assert_eq!(titles(&t), ["Astro", "Bolt", "Comet"]);
    assert!(hidden_chip(&t).is_none());
}
