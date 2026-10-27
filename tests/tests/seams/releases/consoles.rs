//! Which emulator gets which console's catalog, and the console switches.

use super::*;

/// A PS4 catalog with one game, Racer.
fn ps4_catalog() -> String {
    catalog_of("2026-10-18T00:00:00Z", serde_json::json!([topic(1, "CUSA00001", "Racer", "1.00", "2026-10-01", 'a')]))
}

#[tokio::test]
async fn an_emulator_gets_the_catalog_of_the_console_it_plays_and_keeps_its_own_id_in_the_keys() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-a").system("PS4").games([]).build().console("PS5"))
        .plugin(FakeCatalog::new("emu-b").system("PS4").games([]).build().console("PS4"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    t.http.serve("https://xyz.dev/catalog/ps4.json", ps4_catalog());
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    assert_eq!(offer_keys(&t), ["release-catalog:emu-a/PPSA01234", "release-catalog:emu-b/CUSA00001"]);
    let mut sent = t.http.sent();
    sent.sort();
    assert_eq!(sent, ["https://xyz.dev/catalog/ps4.json", URL], "one file for each console");
}

#[tokio::test]
async fn a_console_turned_off_in_the_catalog_s_settings_offers_nothing_and_its_games_stay() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-a").system("PS5").games(["Alpha"]).build().console("PS5"))
        .plugin(FakeCatalog::new("emu-b").system("PS4").games([]).build().console("PS4"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    t.http.serve("https://xyz.dev/catalog/ps4.json", ps4_catalog());
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    // A toggle for each console an emulator plays, on.
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let toggles = |t: &TestCore| -> Vec<(String, String, String)> {
        let view = t.core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
        panel
            .fields
            .iter()
            .filter(|f| f.key.starts_with("show_p"))
            .map(|f| (f.key.clone(), f.label.clone(), f.value.clone()))
            .collect()
    };
    let shown = |on: &str| -> Vec<(String, String, String)> {
        vec![
            ("show_ps4".into(), "Show PS4 games".into(), "true".into()),
            ("show_ps5".into(), "Show PS5 games".into(), on.into()),
        ]
    };
    assert_eq!(toggles(&t), shown("true"));

    let ps5 = |on: bool| Command::SetConfigField {
        plugin: id("release-catalog"),
        scope: ConfigScope::Emulator,
        key: "show_ps5".into(),
        value: on.to_string(),
    };
    t.core.dispatch(ps5(false)).await.unwrap();
    assert_eq!(toggles(&t), shown("false"));
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    assert_eq!(offer_keys(&t), ["release-catalog:emu-b/CUSA00001"], "no PS5 game to download");
    let offer = "release-catalog:emu-a/PPSA01234".parse().unwrap();
    let download = Command::Torrent(nexus_core::TorrentCmd::Download { offer, release: HASH.into(), folder: None });
    assert!(t.core.dispatch(download).await.is_err(), "nor a PS5 release");
    let library: Vec<String> = t.core.view().home.games.iter().map(|g| g.title.clone()).collect();
    assert_eq!(library, ["Alpha"], "the installed PS5 game stays in the library");

    t.core.dispatch(ps5(true)).await.unwrap();
    assert_eq!(offer_keys(&t), ["release-catalog:emu-a/PPSA01234", "release-catalog:emu-b/CUSA00001"]);
}

#[tokio::test]
async fn a_console_whose_emulator_lists_its_own_releases_has_no_other_catalog() {
    // Steam's library comes from the Steam plugin, which lists its own console's releases: the
    // console catalog is asked nothing for it.
    let t = FakeHost::new()
        .platform(Some(nexus_api::Platform::Linux))
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .built_in(nexus_api::PluginBox::code(emu_steam::SteamPlugin))
        .plugin(FakeCatalog::new("emu-a").system("PS4").games([]).build().console("PS5"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let view = t.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.clone()).collect();
    assert!(keys.contains(&"show_ps5".to_owned()) && !keys.contains(&"show_steam".to_owned()), "{keys:?}");
    assert!(t.http.sent().is_empty(), "nothing fetched for Steam: {:?}", t.http.sent());
}

#[tokio::test]
async fn the_catalog_s_console_toggles_follow_the_emulators_turned_on() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-a").system("PS4").games([]).build().console("PS5"))
        .plugin(FakeCatalog::new("emu-b").system("PS4").games([]).build().console("PS4"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let toggles = |t: &TestCore| -> Vec<String> {
        let view = t.core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
        panel.fields.iter().filter(|f| f.key.starts_with("show_p")).map(|f| f.key.clone()).collect()
    };
    assert_eq!(toggles(&t), ["show_ps4", "show_ps5"]);

    let emu_b = |enabled: bool| Command::SetPluginEnabled { plugin: id("emu-b"), enabled };
    t.core.dispatch(emu_b(false)).await.unwrap();
    assert_eq!(toggles(&t), ["show_ps5"], "no PS4 emulator, no PS4 toggle");
    t.core.dispatch(emu_b(true)).await.unwrap();
    assert_eq!(toggles(&t), ["show_ps4", "show_ps5"]);
}

#[tokio::test]
async fn of_two_emulators_for_one_console_the_first_loaded_gets_the_catalog() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-a").system("PS4").games([]).build().console("PS5"))
        .plugin(FakeCatalog::new("emu-c").system("PS4").games([]).build().console("PS5"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;

    let order = t.core.kernel().order();
    let first = order.iter().find(|p| ["emu-a", "emu-c"].contains(&p.as_str())).unwrap();
    assert_eq!(offer_keys(&t), [format!("release-catalog:{first}/PPSA01234")], "the other gets no offers");
    assert_eq!(t.core.view().catalog.sources.len(), 1);
    assert_eq!(t.http.sent(), [URL], "the PS5 catalog is fetched once");
}

#[tokio::test]
async fn an_emulator_with_no_console_gets_no_catalog() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-a").system("PS4").games([]).build())
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();

    assert_eq!(offer_keys(&t), Vec::<String>::new());
    assert!(t.core.view().catalog.sources.is_empty());
    assert!(t.http.sent().is_empty(), "the catalog plugin is not asked");
}
