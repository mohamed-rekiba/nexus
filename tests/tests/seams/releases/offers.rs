//! Offers: one game per title id, its releases, Steam's words and art, and release warnings.

use super::*;

#[tokio::test]
async fn the_catalog_lists_one_game_per_title_id_newest_first_with_its_badges() {
    let t = with_offers().await;
    let view = t.core.view().catalog;
    assert_eq!(view.games.sort, "added", "newest first");
    let shown: Vec<(String, Vec<String>)> =
        view.games.games.iter().map(|c| (c.title.clone(), c.badges.iter().map(|b| b.text.clone()).collect())).collect();
    assert_eq!(
        shown,
        [
            ("Alpha".to_owned(), vec!["Installed".to_owned()]),
            ("Racer".to_owned(), vec!["New".to_owned()]),
            ("Quest".to_owned(), vec![]),
        ],
        "the emulator's own game is Installed, a release from this week is New"
    );
    assert_eq!(view.sources.len(), 1);
    assert!(
        view.sources[0].starts_with("emu-kyty: fetched from xyz.dev · updated 2026-10-18 · 4 releases"),
        "{}",
        view.sources[0]
    );

    t.core.dispatch(Command::CatalogSearch("rac".into())).await.unwrap();
    let titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    assert_eq!(titles, ["Racer"], "the search narrows the offers");
    assert_eq!(t.core.view().home.library.query, "", "the Library keeps its own search");

    t.core.dispatch(Command::CatalogSearch(String::new())).await.unwrap();
    t.core.dispatch(Command::CatalogSort("title".into())).await.unwrap();
    let titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    assert_eq!(titles, ["Alpha", "Quest", "Racer"]);
}

#[tokio::test]
async fn an_offer_opens_on_its_releases_newest_first() {
    let t = with_offers().await;
    let racer = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Racer").unwrap();
    t.core.dispatch(Command::OpenGame(racer.key.parse().unwrap())).await.unwrap();

    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.title, "Racer");
    assert_eq!(detail.emulator, "emu-kyty");
    assert_eq!(detail.description, "Racer 1.05.", "the newest release's words");
    let versions: Vec<(&str, bool)> = detail.releases.iter().map(|r| (r.version.as_str(), r.new)).collect();
    assert_eq!(versions, [("1.05", true), ("1.00", false)]);
    assert!(!detail.can.play, "a game to download doesn't play");
    assert_eq!((detail.can.primary.as_str(), detail.can.get.as_str()), ("get", "Download"), "it downloads");
    assert!(detail.facts.iter().any(|f| f.id == "region" && f.value == "EUR"));
}

/// A store that answers every title id, and says which it was asked about.
fn counting_store(asked: Arc<Mutex<Vec<String>>>) -> StubPlugin {
    struct Counting(Arc<Mutex<Vec<String>>>);
    #[nexus_api::async_trait]
    impl nexus_api::Metadata for Counting {
        async fn describe(
            &self,
            game: &nexus_api::GameRef,
        ) -> nexus_api::PluginResult<Option<nexus_api::GameMetadata>> {
            self.0.lock().unwrap().push(game.title_id.clone());
            Ok(Some(nexus_api::GameMetadata { title: game.title.clone(), ..Default::default() }))
        }
    }
    StubPlugin::from_toml(
        "id = \"store\"\nkind = \"service\"\nname = \"store\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         capabilities = [\"metadata\"]",
    )
    .provides(move |_| {
        Ok(nexus_api::Provided { metadata: Some(Arc::new(Counting(asked.clone()))), ..Default::default() })
    })
}

#[tokio::test]
async fn the_catalog_shows_a_page_at_a_time_and_describes_only_what_it_shows() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
        .plugin(counting_store(asked.clone()))
        .boot_core()
        .await;
    // 130 games, one release each, a hash apiece.
    let topics: Vec<serde_json::Value> = (0..130u32)
        .map(|n| serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS4] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}"), "title_id": format!("CUSA{:05}", n + 1), "genre": "Action" } }))
        .collect();
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", serde_json::Value::Array(topics)));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    let page = |t: &TestCore| {
        let g = t.core.view().catalog.games;
        (g.games.len(), g.total)
    };
    assert_eq!(page(&t), (nexus_core::GRID_PAGE, 130));
    // Until the fetch is in, the Catalog shows the bundled catalog, and the store is asked
    // about its first page too: only this catalog's games (CUSA…) count.
    let described = || async {
        t.clock.advance(std::time::Duration::from_secs(1)).await;
        asked.lock().unwrap().iter().filter(|id| id.starts_with("CUSA")).count()
    };
    assert_eq!(described().await, nexus_core::GRID_PAGE, "the store is asked about the shown page only");

    t.core.dispatch(Command::CatalogMore).await.unwrap();
    assert_eq!(page(&t), (2 * nexus_core::GRID_PAGE, 130));
    t.core.dispatch(Command::CatalogMore).await.unwrap();
    assert_eq!(page(&t), (130, 130), "the last page is the rest");
    assert_eq!(described().await, 130, "each game once");

    t.core.dispatch(Command::CatalogSearch("Game 1".into())).await.unwrap();
    assert_eq!(page(&t), (30, 30), "Game 100 to Game 129");
    t.core.dispatch(Command::CatalogSearch("Game".into())).await.unwrap();
    assert_eq!(page(&t), (nexus_core::GRID_PAGE, 130), "a search starts from the first page");
}

#[tokio::test]
async fn a_pc_offer_shows_steam_s_words_and_art_and_its_protondb_tier() {
    let t = offering(
        "emu-pc",
        "PC",
        serde_json::json!([{ "id": "1", "title": "Celeste (v1.4.0.0) [amd64] [Wine]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": {}, "steam": steam_block() }]),
        false,
    )
    .await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert_eq!(card.cover, steam_image("portrait.jpg"), "the card has Steam's portrait");

    let detail = open_first(&t).await;
    assert_eq!(detail.description, "Help Madeline survive.");
    assert_eq!(detail.developer, "Maddy Makes Games");
    assert_eq!(detail.genres, ["Action", "Indie"]);
    assert_eq!(detail.release, "2018-01-25");
    assert_eq!(detail.cover, steam_image("portrait.jpg"));
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.logo, steam_image("logo.png"));
    assert_eq!(detail.screenshots, [steam_image("ss_1.jpg").unwrap()]);
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
    assert_eq!(detail.protondb, "gold", "a game packed with Wine has ProtonDB's tier");
    assert_eq!(detail.protondb_url, "https://www.protondb.com/app/504230");
}

#[tokio::test]
async fn an_installed_game_with_no_art_of_its_own_has_its_offer_s_and_keeps_it_with_the_catalog_off() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-pc").system("PC").games(["Celeste"]).build().console("PC"))
        .boot_core()
        .await;
    let topics = serde_json::json!([{ "id": "1", "title": "Celeste (v1.4.0.0) [amd64] [Wine]",
        "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Celeste" }, "steam": steam_block() }]);
    t.http.serve("https://xyz.dev/catalog/pc.json", catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert!(card.badges.iter().any(|b| b.text == "Installed"), "the offer is the installed game");

    let cover = |t: &TestCore| t.core.view().home.games.first().and_then(|g| g.cover.clone());
    assert_eq!(cover(&t), steam_image("portrait.jpg"), "the library card has the offer's art");
    // Its Game Hub has the offer's Steam art, and no releases: it is installed.
    let game: nexus_api::GameKey = t.core.view().home.games[0].key.parse().unwrap();
    t.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.cover, steam_image("portrait.jpg"));
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.logo, steam_image("logo.png"));
    assert!(detail.releases.is_empty(), "the offer's releases are the Catalog's");

    t.core.dispatch(Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: false }).await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    assert_eq!(cover(&t), steam_image("portrait.jpg"), "and keeps its cover with no catalog");
    t.core.dispatch(Command::OpenGame(game)).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().cover, steam_image("portrait.jpg"), "in its Game Hub too");
}

#[tokio::test]
async fn a_native_pc_offer_has_no_protondb_tier() {
    let t = offering(
        "emu-pc",
        "PC",
        serde_json::json!([{ "id": "1", "title": "Celeste [Native]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": {}, "steam": steam_block() }]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
    assert_eq!((detail.protondb.as_str(), detail.protondb_url.as_str()), ("", ""));
}

#[tokio::test]
async fn a_ps_offer_the_store_has_no_art_for_gets_steam_s_art() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([{ "id": "1", "title": "[PS5] Celeste [EUR]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Celeste" },
            "steam": steam_block() }]),
        true,
    )
    .await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert_eq!(card.cover, steam_image("portrait.jpg"));
    let detail = open_first(&t).await;
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.protondb, "", "a console game has no ProtonDB tier");
}

#[tokio::test]
async fn the_store_s_data_comes_first_and_steam_fills_what_it_leaves_out() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([{ "id": "1", "title": "[PS5] Celeste [EUR]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
            "game_info": { "name": "Celeste", "title_id": "PPSA07777" }, "steam": steam_block() }]),
        true,
    )
    .await;
    let offer = t.core.view().catalog.games.games.first().cloned().unwrap();
    t.core.dispatch(Command::OpenGame(offer.key.parse().unwrap())).await.unwrap();
    t.until(Area::Library, |core| core.view().detail.is_some_and(|d| d.developer == "MOJANG AB")).await;
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.developer, "MOJANG AB", "the store's publisher");
    assert_eq!(detail.genres, ["Adventure", "Arcade"], "the store's genres");
    assert_eq!(detail.screenshots.len(), 2, "the store's screenshots");
    assert_eq!(detail.description, "Help Madeline survive.", "the store has no words: Steam's");
    assert_eq!(detail.background, steam_image("hero.jpg"), "the store has no background: Steam's");
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
}

#[tokio::test]
async fn a_ps5_package_release_says_it_can_t_be_installed() {
    let package = "dddddddddddddddddddddddddddddddddddddddd";
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([
            { "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
              "published_at": "2026-10-01", "game_info": { "name": "Astro", "title_id": "PPSA01234", "format": "exfat" } },
            { "id": "2", "title": "[PS5] Astro [EUR / v01.04 / fpkg]", "magnet": format!("magnet:?xt=urn:btih:{package}"),
              "published_at": "2026-09-01", "game_info": { "name": "Astro", "title_id": "PPSA01234" } },
        ]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    let warnings: Vec<(&str, &str)> = detail.releases.iter().map(|r| (r.id.as_str(), r.warning.as_str())).collect();
    assert_eq!(
        warnings,
        [(HASH, ""), (package, "Retail PS5 packages can't be installed; pick a folder, an archive or an exFAT image"),],
        "the package stays listed, with why"
    );
}

#[tokio::test]
async fn a_ps4_package_release_has_no_warning() {
    let t = offering(
        "emu-shadps4",
        "PS4",
        serde_json::json!([{ "id": "1", "title": "[PS4] Racer [EUR / v1.00 / pkg]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Racer", "title_id": "CUSA00001" } }]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    assert_eq!(detail.releases[0].warning, "", "shadPS4 unpacks PS4 packages");
}

#[tokio::test]
async fn an_edition_with_its_own_title_id_joins_its_base_game() {
    let t = offering(
        "emu-shadps4",
        "PS4",
        serde_json::json!([
            topic(1, "CUSA00001", "Racer", "1.00", "2026-09-01", 'a'),
            topic(2, "CUSA00009", "Racer Digital Deluxe Edition", "1.05", "2026-10-01", 'b'),
            topic(3, "CUSA00008", "Racer: Collector's Edition", "1.02", "2026-09-15", 'c'),
            topic(4, "CUSA00002", "Racer 2 Deluxe Edition", "1.00", "2026-08-01", 'd'),
        ]),
        false,
    )
    .await;
    let mut offers: Vec<(String, usize)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.releases)).collect();
    offers.sort();
    assert_eq!(
        offers,
        [("Racer 2 Deluxe Edition".to_owned(), 1), ("Racer Digital Deluxe Edition".to_owned(), 3)],
        "the editions join the base game; the sequel stays apart"
    );
}
