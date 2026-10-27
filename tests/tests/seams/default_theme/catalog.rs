//! The Catalog: the Library's view of what the game catalogs offer.

use super::*;

#[test]
fn with_the_game_catalog_turned_off_the_ways_to_the_catalog_go() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Astro"]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    settle(&mut host);
    assert_eq!(host.count("Search"), 1, "with a catalog, the top bar's Search");
    assert_eq!(key_asks(&mut host, Key::Tab), [Command::Navigate(nexus_core::Route::Catalog)]);
    host.dispatch(Command::Navigate(nexus_core::Route::Home));

    host.dispatch(Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: false });
    settle(&mut host);

    assert_eq!(host.count("Search"), 0, "no Search in the top bar");
    let hints = values(&host, "hint");
    assert!(hints.contains(&"Downloads".to_owned()) && !hints.contains(&"Search".to_owned()), "R1's hint: {hints:?}");
    assert_eq!(key_asks(&mut host, Key::Tab), [Command::Navigate(nexus_core::Route::Downloads)], "R1 skips it");
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(key_asks(&mut host, Key::Backtab), [Command::Navigate(nexus_core::Route::Home)], "and L1");
}

#[test]
fn the_catalog_is_the_library_s_view_and_loads_a_page_at_a_time() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Alpha"]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // 130 games in the catalog.
    let topics: Vec<serde_json::Value> = (0..130u32)
        .map(|n| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}") } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 130,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    let shown = |host: &Composed| host.core().view().catalog.games.games.len();
    assert_eq!(shown(&host), nexus_core::GRID_PAGE, "the first page");
    assert!(host.count("library-sort") == 1 && host.count("filter:All") == 1, "the Library's bar and filters");
    assert_eq!(values(&host, "default-count"), ["130 games · 130 releases"], "the games and the releases");

    // Down the grid with Page Down: near the end of each page, the next one comes, to the last.
    key(&mut host, Key::PageDown);
    assert_eq!(shown(&host), nexus_core::GRID_PAGE, "not yet near the end");
    for _ in 0..10 {
        key(&mut host, Key::PageDown);
    }
    assert_eq!(shown(&host), 130);

    // Escape leaves the Catalog, and Home keeps its own selection.
    key(&mut host, Key::Escape);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Home);
    assert!(host.core().view().home.selected.is_some(), "Home's game, not the Catalog's");
}

/// The default theme on the Catalog, with three games a catalog offers: Astro, Bolt, Comet.
fn three_on_the_catalog() -> Composed {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let topics: Vec<serde_json::Value> = ["Astro", "Bolt", "Comet"]
        .iter()
        .enumerate()
        .map(|(n, name)| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] {name}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": name } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 3,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    // The boot screen is over everything until the library and Home's art are in.
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    host
}

#[test]
fn in_the_catalog_s_header_rows_the_hints_say_how_the_keys_move() {
    use slint::platform::Key;
    let mut host = three_on_the_catalog();
    assert!(values(&host, "hint").is_empty(), "no pill over the cards");
    key(&mut host, Key::UpArrow);
    assert_eq!(values(&host, "hint"), ["Move", "Choose", "Rows", "Back"], "on the systems");
    key(&mut host, Key::UpArrow);
    assert_eq!(values(&host, "hint"), ["Move", "Choose", "Rows", "Back"], "on the bar");
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::DownArrow);
    assert!(values(&host, "hint").is_empty(), "back on the cards");
}

#[test]
fn the_eye_on_a_catalog_card_hides_the_game_and_the_hidden_filter_shows_it_again() {
    let mut host = three_on_the_catalog();
    assert_eq!(values(&host, "default-game"), ["Astro", "Bolt", "Comet"]);
    assert_eq!(values(&host, "card-hide"), ["Hide Astro", "Hide Bolt", "Hide Comet"], "each card's eye");
    // The pad's way: the game's details, then Options: its menu hides it too. The details
    // have the eye as well.
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail);
    assert_eq!(values(&host, "hub-hide"), ["Hide"]);
    key(&mut host, slint::platform::Key::Menu);
    assert!(
        values(&host, "game-menu").contains(&"Hide from the Catalog".to_owned()),
        "{:?}",
        values(&host, "game-menu")
    );
    key(&mut host, slint::platform::Key::Escape);
    key(&mut host, slint::platform::Key::Escape);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Catalog);

    press(&mut host, "card-hide", "Hide Bolt");
    assert_eq!(values(&host, "default-game"), ["Astro", "Comet"]);

    press_filter(&mut host, "Hidden");
    assert_eq!(values(&host, "default-game"), ["Bolt"], "the Hidden filter: only the hidden games");
    assert_eq!(values(&host, "card-hide"), ["Show Bolt"]);
    press(&mut host, "card-hide", "Show Bolt");
    press_filter(&mut host, "Hidden");
    assert_eq!(values(&host, "default-game"), ["Astro", "Bolt", "Comet"]);
}

#[test]
fn the_bar_under_the_catalog_s_header_shows_how_far_down_all_its_games_the_grid_is() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let topics: Vec<serde_json::Value> = (0..200u32)
        .map(|n| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}") } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 200,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    let progress = |host: &Composed| -> u32 {
        let value = values(host, "grid-progress");
        assert_eq!(value.len(), 1, "one bar: {value:?}");
        value[0].trim_end_matches('%').parse().unwrap()
    };
    let top = progress(&host);
    assert!(top > 0 && top < 30, "at the top, against all 200 games, not the 60 loaded: {top}%");
    for _ in 0..10 {
        key(&mut host, Key::DownArrow);
    }
    rest(&mut host);
    settle(&mut host);
    let lower = progress(&host);
    assert!(lower > top && lower < 100, "further down: {top}% then {lower}%");
    for _ in 0..40 {
        key(&mut host, Key::DownArrow);
        host.pump();
    }
    rest(&mut host);
    settle(&mut host);
    assert_eq!(progress(&host), 100, "at the end");
}

#[test]
fn back_from_a_game_s_details_the_catalog_is_where_it_was() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let topics: Vec<serde_json::Value> = (0..60u32)
        .map(|n| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}") } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 60,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    // The boot screen is over everything until the library and Home's art are in.
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    for _ in 0..5 {
        key(&mut host, Key::DownArrow);
    }
    rest(&mut host);
    settle(&mut host);
    let selected = host.core().view().catalog.games.games.into_iter().find(|g| g.selected).expect("a selection").title;
    let place = |host: &Composed| {
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "default-game")
            .find(|g| g.accessible_value().as_deref() == Some(selected.as_str()))
            .unwrap_or_else(|| panic!("no card for {selected}"))
            .absolute_position()
            .y
    };
    let before = place(&host);
    assert!(before > 0.0 && before < 900.0, "the selected card is on screen: {before}");

    key(&mut host, Key::Return);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail);
    key(&mut host, Key::Escape);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Catalog);
    settle(&mut host);
    assert_eq!(place(&host), before, "{selected} where it was");
}

#[test]
fn the_catalog_loads_only_the_covers_near_the_screen() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
            .plugin(FakeStore::new("store").covers().build())
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let topics: Vec<serde_json::Value> = (0..60u32)
        .map(|n| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}"), "title_id": format!("PPSA{:05}", n + 1) } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 60,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    // The store describes the page in the background.
    host.advance(std::time::Duration::from_millis(100));
    settle(&mut host);
    host.pump();
    host.ui.render().unwrap();
    let catalog = host.core().view().catalog.games;
    assert_eq!(catalog.games.len(), 60);
    assert!(catalog.games.iter().all(|g| g.cover.is_some()), "every game has a cover to load");

    let wanted = host.ui.wanted_images();
    assert!(!wanted.is_empty() && wanted.len() < 40, "{} of 60 covers asked for", wanted.len());
    assert!(wanted.iter().all(|(_, size, _)| *size == nexus_ui_host::ImageSize::Thumb), "card-sized");
}

#[test]
fn the_catalog_s_header_says_when_the_list_was_updated_and_how_many_games_of_all_show() {
    let home = tempfile::tempdir().unwrap();
    let topic = |id: u32, name: &str, hash: char| {
        serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] {name} [EUR / v1.0{id}]"),
            "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "seeders": 4,
            "peers_observed": "2026-10-19T21:07:34.000Z", "game_info": { "name": name, "version": format!("v1.0{id}") } })
    };
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([topic(1, "Astro", 'a'), topic(2, "Astro", 'b'), topic(3, "Bolt", 'c')]),
    );
    key(&mut host, slint::platform::Key::Escape);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Catalog);
    assert_eq!(values(&host, "default-count"), ["2 games · 3 releases"]);
    assert_eq!(values(&host, "catalog-updated"), ["List updated Oct 19, 2026"]);
    assert_eq!(host.count("catalog-source"), 0, "the copies in use are on the Game Catalog's page");

    host.dispatch(Command::CatalogSearch("bolt".into()));
    settle(&mut host);
    assert_eq!(values(&host, "default-count"), ["1 of 2 games · 3 releases"]);
}

#[test]
fn the_catalog_says_loading_catalog_until_the_catalogs_are_read() {
    struct Slow(std::sync::Arc<tokio::sync::Semaphore>);
    #[nexus_api::async_trait]
    impl nexus_api::Releases for Slow {
        async fn releases(&self, _: &nexus_api::Console) -> nexus_api::PluginResult<Option<nexus_api::ReleaseList>> {
            let _permit = self.0.acquire().await;
            Ok(None)
        }
    }
    let open = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let slow = open.clone();
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5")).plugin(
            nexus_testkit::StubPlugin::from_toml(
                "id = \"slow-catalog\"\nkind = \"service\"\nname = \"Slow\"\nversion = \"1.0.0\"\n\
                 host_api = \"^0.2\"\ncapabilities = [\"releases\"]",
            )
            .provides(move |_| {
                Ok(nexus_api::Provided {
                    releases: Some(std::sync::Arc::new(Slow(slow.clone()))),
                    ..Default::default()
                })
            }),
        )
    });
    host.ui.start().unwrap();
    let core = host.core().clone();
    let navigate = host.rt.spawn(async move { core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await });
    host.advance(std::time::Duration::from_millis(50));
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "grid-empty"), ["Loading catalog…"]);

    open.add_permits(10);
    host.rt.block_on(navigate).unwrap().unwrap();
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "grid-empty"), ["No catalog lists any game yet"]);
}

/// The Catalog with Alpha, which the emulator has, in two releases, and Bolt in one.
fn alpha_and_bolt_on_the_catalog() -> Composed {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS5").games(["Alpha"]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let topic = |id: u32, name: &str, seeders: u64, hash: char| {
        serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] {name} [EUR / v1.0{id}]"), "size": "2 GB",
            "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "seeders": seeders,
            "game_info": { "name": name, "version": format!("v1.0{id}") } })
    };
    let topics = serde_json::json!([topic(1, "Alpha", 5, 'a'), topic(2, "Alpha", 4, 'b'), topic(3, "Bolt", 1, 'c')]);
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 3,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    host.dispatch(Command::CatalogSort("title".into()));
    settle(&mut host);
    host
}

#[test]
fn a_catalog_card_says_its_releases_and_how_available_it_is() {
    let mut host = alpha_and_bolt_on_the_catalog();
    assert_eq!(values(&host, "default-game"), ["Alpha", "Bolt"]);
    assert_eq!(values(&host, "card-releases"), ["2 releases", "1 release"]);
    assert_eq!(values(&host, "card-availability"), ["Good", "Low"]);
    assert_eq!(values(&host, "card-console"), ["PS5", "PS5"], "the console badge");
    assert!(x_of(&host, "card-size") < x_of(&host, "card-releases"), "the size, then the releases");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    assert!(x_of(&host, "card-size") > x_of(&host, "card-releases"), "right to left, the size on the right");

    // The Library's cards have neither.
    host.dispatch(Command::Back);
    press_tab(&mut host, "Library");
    assert_eq!(values(&host, "default-game"), ["Alpha"]);
    assert_eq!((host.count("card-releases"), host.count("card-availability")), (0, 0));
}

#[test]
fn the_catalog_s_selected_card_gets_the_ring() {
    use slint::platform::Key;
    let mut host = alpha_and_bolt_on_the_catalog();
    // The card itself says it is selected, as its ring and its size show it: not only the grid.
    for _ in 0..2 {
        key(&mut host, Key::RightArrow);
        let [selected] = focused(&host, "default-game").try_into().expect("one selected card");
        let other = if selected == "Alpha" { "Bolt" } else { "Alpha" };
        assert!(lit(&host, &selected) && !lit(&host, other), "the ring is on {selected}");
    }
}

#[test]
fn the_catalog_s_chip_row_has_installed_for_the_games_the_player_has() {
    let mut host = alpha_and_bolt_on_the_catalog();
    assert_eq!(values(&host, "filter:Installed"), ["off"]);
    press_filter(&mut host, "Installed");
    assert_eq!(values(&host, "default-game"), ["Alpha"], "only the game the player has");
    assert_eq!(values(&host, "filter:Installed"), ["on"]);
}

/// The Catalog of one PS5 game, with Refresh from the forums on or off.
fn catalog_with_refresh(refresh: bool) -> Composed {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS5").games([]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05" } }] });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c", "refresh_from_forums": refresh }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    host
}

#[test]
fn with_the_refresh_off_the_catalog_s_button_opens_settings() {
    let mut host = catalog_with_refresh(false);
    assert_eq!(values(&host, "release-catalog:catalog-refresh"), ["Turn on Refresh from the forums in Settings"]);
    assert_eq!(host.count("release-catalog:catalog-refresh-state"), 0);
    press(&mut host, "release-catalog:catalog-refresh", "Turn on Refresh from the forums in Settings");
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Settings);
}

#[test]
fn the_catalog_s_refresh_button_is_on_its_bar_and_the_keys_reach_it() {
    use slint::platform::Key;
    let mut host = catalog_with_refresh(true);
    assert_eq!(values(&host, "release-catalog:catalog-refresh"), ["Refresh"]);
    assert_eq!(host.count("release-catalog:catalog-refresh-state"), 1, "what it says of the refresh");
    assert!(x_of(&host, "library-sort") < x_of(&host, "release-catalog:catalog-refresh"), "after the sort");

    // Up from the grid to the bar, then right past the sort and the card sizes.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::UpArrow);
    for _ in 0..4 {
        key(&mut host, Key::RightArrow);
    }
    let asked = key_asks(&mut host, Key::Return);
    let refresh = asked.iter().any(|c| matches!(c, Command::RunAction { action, .. } if action == "refresh-now"));
    assert!(refresh, "Enter refreshes: {asked:?}");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    assert!(x_of(&host, "library-sort") > x_of(&host, "release-catalog:catalog-refresh"), "right to left, mirrored");
}
