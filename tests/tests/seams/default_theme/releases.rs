//! The Game Hub's releases: their rows, facts and sheets.

use super::*;

#[test]
fn a_pc_release_says_its_system_and_when_this_computer_can_t_start_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = composed(|h| {
        h.home(home.path())
            .platform(Some(nexus_api::Platform::Windows))
            .plugin(installing_emulator())
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "Penguins (v1.2) [amd64] [Native]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"), "game_info": {} }] });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    let offer = host.core().view().catalog.games.games.first().cloned().expect("the offer");
    assert_eq!(offer.platform, "PC");
    let badges: Vec<_> = offer.badges.iter().map(|b| b.text.clone()).collect();
    assert!(badges.contains(&"Can't run here".to_owned()), "the Catalog marks it: {badges:?}");
    settle(&mut host);
    assert_eq!(host.count("filter:PC"), 1, "a PC choice in the system filter");
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    settle(&mut host);

    assert_eq!(values(&host, "release-not-here"), ["This is a Linux release: it can't run on Windows"]);
}

#[test]
fn a_click_on_the_game_hub_s_store_button_moves_the_highlight_to_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([{ "id": "1", "title": "Celeste [Wine]", "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": {}, "steam": { "appid": 504230, "short": "Help Madeline survive." } }]),
    );
    assert_eq!(focused(&host, "hub-store"), ["Store"], "the first button");
    key(&mut host, slint::platform::Key::RightArrow);
    assert!(focused(&host, "hub-store").is_empty(), "the keys moved on");
    press(&mut host, "hub-store", "Store");
    assert_eq!(focused(&host, "hub-store"), ["Store"], "one highlight, on what was clicked");
}

#[test]
fn a_game_packed_with_wine_shows_its_protondb_tier_and_its_page() {
    let home = tempfile::tempdir().unwrap();
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([{ "id": "1", "title": "Celeste [Wine]", "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": {}, "steam": { "appid": 504230, "short": "Help Madeline survive.", "protondb": "gold" } }]),
    );
    assert_eq!(values(&host, "protondb"), ["Gold"], "the ProtonDB card");
    assert_eq!(values(&host, "hub-store"), ["Store"], "Steam's page is the store page");

    press(&mut host, "game-options", "Options");
    assert!(values(&host, "game-menu").contains(&"ProtonDB page".to_owned()));
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "game-menu")
        .find(|e| e.accessible_value().as_deref() == Some("ProtonDB page"))
        .expect("the ProtonDB item");
    found.invoke_accessible_default_action();
    let opened = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::OpenUrl(u) if u == "https://www.protondb.com/app/504230"));
    assert!(opened, "the ProtonDB page opens in the browser");
}

#[test]
fn the_game_hub_s_facts_wrap_in_rows_of_three() {
    let home = tempfile::tempdir().unwrap();
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([{ "id": "1", "title": "Celeste [Wine]", "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": {}, "steam": { "appid": 504230, "short": "Help Madeline survive.", "protondb": "gold" } }]),
    );
    let place = |host: &Composed, label: &str| {
        let fact = host.first(label);
        (fact.absolute_position().x, fact.absolute_position().y)
    };
    let facts = if host.count("on-this-pc") > 0 { 5 } else { 4 };
    let (played_x, played_y) = place(&host, "played");
    let (proton_x, proton_y) = place(&host, "protondb");
    assert!(proton_y > played_y, "{facts} facts: the last one starts a second row");
    assert!(proton_x < played_x, "the second row starts on the left");

    // Right to left: the same rows, each in the other order.
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    let (played_x, played_y) = place(&host, "played");
    let (proton_x, proton_y) = place(&host, "protondb");
    assert!(proton_y > played_y);
    assert!(proton_x > played_x, "the second row starts on the right");
}

#[test]
fn as_the_whole_screen_the_game_menu_opens_nothing_outside_nexus() {
    let home = tempfile::tempdir().unwrap();
    let whole_screen = nexus_core::system::Place { whole_screen: true, nexus_os: false, stick: false };
    let mut host = first_offer_on(
        home.path(),
        installing_emulator(),
        |h| h.place(whole_screen),
        serde_json::json!([{ "id": "1", "title": "Celeste [Wine]", "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": {}, "steam": { "appid": 504230, "short": "Help Madeline survive.", "protondb": "gold" } }]),
    );
    assert_eq!(values(&host, "protondb"), ["Gold"], "the tier still shows");
    press(&mut host, "game-options", "Options");
    let items = values(&host, "game-menu");
    for outside in ["Open game folder", "View emulator log", "Compatibility list", "ProtonDB page"] {
        assert!(!items.contains(&outside.to_owned()), "no file manager or browser is on screen: {items:?}");
    }
}

#[test]
fn a_game_without_a_protondb_tier_has_no_card_or_menu_item() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    assert!(values(&host, "protondb").is_empty());
    press(&mut host, "game-options", "Options");
    assert!(!values(&host, "game-menu").contains(&"ProtonDB page".to_owned()));
}

#[test]
fn a_ps5_package_says_on_its_row_and_its_sheet_that_it_may_not_install() {
    let home = tempfile::tempdir().unwrap();
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([{ "id": "1", "title": "[PS5] Astro [EUR / v01.05 / fpkg]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }]),
    );
    let warning = "Retail PS5 packages can't be installed; pick a folder, an archive or an exFAT image";
    assert_eq!(values(&host, "release-warning"), [warning], "the row");
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(values(&host, "release-warning"), [warning, warning], "the row and the sheet");
    assert_eq!(host.count("download-agree"), 1, "it still downloads");
}

#[test]
fn every_release_s_download_button_ends_at_the_same_place_inside_its_row() {
    use slint::platform::Key;
    let home = tempfile::tempdir().unwrap();
    let region = "EU² / US² / JP² / AS² / INT(GLO)² / EU¹ / US¹ / JP¹ / AS¹ / INT(GLO)¹ / EUR / USA / JPN / ASIA";
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([
            { "id": "1", "title": "[PS5] Astro [long / fpkg]", "size": "1 GB",
              "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
              "game_info": { "name": "Astro", "title_id": "PPSA01234",
                "version": "1.000.010 (dated 30.03.2026) and a long note on what this version is", "region": region } },
            { "id": "2", "title": "[PS5] Astro [EUR / v01.00]", "size": "2 GB",
              "magnet": "magnet:?xt=urn:btih:d23fe1c06bba254a9dc9f519b335aa7c1367a88b",
              "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.00", "region": "EUR" } },
        ]),
    );
    let releases = host.core().view().detail.expect("a Game Hub").releases;
    assert_eq!(releases.len(), 2);
    let end = |e: &i_slint_backend_testing::ElementHandle| e.absolute_position().x + e.size().width;
    let mut ends = Vec::new();
    for release in &releases {
        let row = host.first(&format!("release-{}", release.id));
        let button =
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "release-download")
                .find(|e| e.accessible_value().as_deref() == Some(release.id.as_str()))
                .unwrap_or_else(|| {
                    panic!("a download button for {}: {:?}", release.id, values(&host, "release-download"))
                });
        assert!(end(&button) <= end(&row), "release {}'s button ends in its row", release.id);
        ends.push(end(&button));
    }
    assert!((ends[0] - ends[1]).abs() < 1.0, "the buttons end at the same place: {ends:?}");

    // One highlight: the focused row, not its button too.
    key(&mut host, Key::DownArrow);
    let row = host.first(&format!("release-{}", releases[0].id));
    assert_eq!(row.accessible_item_selected(), Some(true), "the row has the keys");
    let button = host.first("release-download");
    assert_eq!(button.accessible_item_selected(), Some(false), "its button isn't drawn as focused too");
}

#[test]
fn the_game_hub_scrolls_to_the_focused_release() {
    let home = tempfile::tempdir().unwrap();
    // Eight versions of one game: more rows than the window shows.
    let topics: Vec<serde_json::Value> = (1..=8u32)
        .map(|n| {
            serde_json::json!({ "id": n.to_string(), "title": format!("[PS5] Astro [EUR / v01.0{n}]"), "size": "1 GB",
                "magnet": format!("magnet:?xt=urn:btih:{n:040x}"),
                "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": format!("v01.0{n}"), "region": "EUR" } })
        })
        .collect();
    let mut host = first_offer_with(home.path(), serde_json::Value::Array(topics));
    let releases = host.core().view().detail.expect("a Game Hub").releases;
    assert_eq!(releases.len(), 8);
    for release in &releases {
        key(&mut host, slint::platform::Key::DownArrow);
        let row = host.first(&format!("release-{}", release.id));
        let bottom = row.absolute_position().y + row.size().height;
        assert!(bottom <= 900.0, "release {} is in view: its bottom at {bottom}", release.version);
    }
}

#[test]
fn a_game_hub_opened_after_another_shows_its_own_releases() {
    let home = tempfile::tempdir().unwrap();
    let other = "d23fe1c06bba254a9dc9f519b335aa7c1367a88b";
    let mut host = first_offer_with(
        home.path(),
        serde_json::json!([
            { "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
                "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
                "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } },
            { "id": "2", "title": "[PS5] Bolt [USA / v02.00]", "size": "2 GB",
                "magnet": format!("magnet:?xt=urn:btih:{other}"),
                "game_info": { "name": "Bolt", "title_id": "PPSA05678", "version": "v02.00", "region": "USA" } },
        ]),
    );
    let hub = host.core().view().detail.expect("a Game Hub");
    let first = hub.releases[0].id.clone();
    let second = if first == ASTRO_HASH { other } else { ASTRO_HASH };
    // The first game's release row, focused, and its download sheet, closed.
    key(&mut host, slint::platform::Key::DownArrow);
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.count("download-agree"), 1, "the sheet is open");
    key(&mut host, slint::platform::Key::Escape);
    // The second game opens while the first game's Game Hub shows.
    let offers = host.core().view().catalog.games.games;
    let next = offers.iter().find(|g| g.key != hub.key).expect("the second game").key.clone();
    host.dispatch(Command::OpenGame(next.parse().unwrap()));
    settle(&mut host);

    let core_release = host.core().view().detail.unwrap().releases[0].id.clone();
    assert_eq!(core_release, second, "the core has the second game's release");
    assert_eq!(host.count(&format!("release-{first}")), 0, "the first game's row is gone");
    assert_eq!(host.count(&format!("release-{second}")), 1, "the second game's row shows");
}

/// A strong PC, or one whose driver is too old for Vulkan 1.3.
fn this_pc(vulkan: (u32, u32)) -> nexus_core::Specs {
    let gpu = nexus_core::Gpu {
        name: "Test GPU".into(),
        memory_gb: Some(24.0),
        discrete: true,
        unified: false,
        vulkan: Some(vulkan),
    };
    nexus_core::Specs {
        cpu: "Test CPU".into(),
        cores: 16,
        threads: 32,
        memory_gb: 64.0,
        avx2: Some(true),
        gpus: Some(vec![gpu]),
    }
}

/// Astro's 50 GB release, for an emulator that needs Vulkan 1.3, open on its Game Hub.
fn astro_on(
    home: &std::path::Path,
    build: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost,
) -> Composed {
    first_offer_on(
        home,
        FakeCatalog::kyty().playable().requires("vulkan = \"1.3\"").build(),
        build,
        serde_json::json!([{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "50 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }]),
    )
}

#[test]
fn the_game_hub_says_how_the_game_runs_on_this_pc() {
    let home = tempfile::tempdir().unwrap();
    let host = astro_on(home.path(), |h| h.hardware(this_pc((1, 3))));
    assert_eq!(values(&host, "on-this-pc"), ["Should run well"]);
}

#[test]
fn a_game_that_won_t_start_here_says_why_on_its_card_and_its_sheet() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on(home.path(), |h| h.hardware(this_pc((1, 2))));
    assert_eq!(values(&host, "on-this-pc"), ["Won't start"]);
    assert!(values(&host, "download-pc-warning").is_empty(), "only on the sheet");
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(values(&host, "download-pc-warning"), ["This PC can't run PS5 games: No Vulkan 1.3"]);
    assert_eq!(host.count("download-agree"), 1, "it still downloads");
}

#[test]
fn a_game_bigger_than_the_free_space_says_so_on_its_card_and_its_sheet() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on(home.path(), |h| h.hardware(this_pc((1, 3))).free_space(10_000_000_000));
    assert_eq!(values(&host, "on-this-pc"), ["Not enough space"]);
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(
        values(&host, "release-no-room"),
        ["Not enough free space in the download folder: 53.7 GB needed, 10.0 GB free"]
    );
}

#[test]
fn the_game_hub_shows_the_best_release_s_facts_and_each_release_s_place_and_availability() {
    let home = tempfile::tempdir().unwrap();
    let topic = |id: u32, version: &str, seeders: u64, hash: char| {
        serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] Astro [EUR / {version}]"), "size": "2 GB",
            "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "seeders": seeders,
            "leechers": 1, "peers_observed": "2026-10-07T14:07:32Z", "published_at": format!("2026-10-0{id}"),
            "game_info": { "name": "Astro", "version": version, "publisher": "Sony" } })
    };
    let mut host =
        first_offer_with(home.path(), serde_json::json!([topic(2, "1.00", 1, 'b'), topic(1, "1.05", 5, 'a')]));
    assert_eq!(values(&host, "hub-availability"), ["Good"], "the best release's");
    assert_eq!(values(&host, "release-version"), ["Version 2 of 2 (newest)", "Version 1 of 2"]);
    assert_eq!(values(&host, "release-availability"), ["Good", "Low"]);
    assert_eq!(values(&host, "release-peers"), ["5 seeders · 1 leecher", "1 seeder · 1 leecher"]);

    press(&mut host, "technical-details", "closed");
    assert_eq!(values(&host, "fact:publisher"), ["Sony"]);
    assert_eq!(values(&host, "fact:seeders"), ["5"]);
    assert!(x_of(&host, "fact-name:publisher") < x_of(&host, "fact:publisher"), "the name, then the value");
    assert!(x_of(&host, "release-availability") < x_of(&host, "release-peers"));

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    assert!(x_of(&host, "fact-name:publisher") > x_of(&host, "fact:publisher"), "right to left, the name on the right");
    assert!(x_of(&host, "release-availability") > x_of(&host, "release-peers"), "right to left, mirrored");
}
