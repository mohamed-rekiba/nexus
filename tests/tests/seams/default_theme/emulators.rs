//! The emulator pages: the Emulators list, each plugin's page, and the install prompt.

use super::*;

#[test]
fn a_service_is_listed_after_the_emulators_with_nothing_to_install() {
    let mut host = composed(|h| {
        h.plugin(FakeStore::new("store").build()).plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.settle();
    let rows: Vec<_> =
        host.core().view().emulators.plugins.iter().map(|p| (p.id.to_string(), p.kind.clone())).collect();
    assert!(rows.contains(&("store".into(), "service".into())), "{rows:?}");
    assert!(rows.contains(&("emu-a".into(), "emulator".into())), "{rows:?}");
    let listed: Vec<String> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "plugin:store")
            .chain(i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "plugin:emu-a"))
            .map(|e| e.accessible_label().unwrap_or_default().to_string())
            .collect();
    assert_eq!(listed.len(), 2, "both in the Emulators sheet");
}

#[test]
fn a_first_play_asks_before_the_emulator_downloads_and_enter_agrees() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();

    assert_eq!(host.values("install-prompt"), ["Install Emu?"]);
    let says = host.values("install-prompt-text").concat();
    assert!(says.contains("Astro") && says.contains("1.0") && says.contains("300 MB"), "{says}");

    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let agreed =
        host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Core(Command::ConfirmInstall)));
    assert!(agreed, "Enter installs and plays");
}

#[test]
fn an_emulator_s_download_says_where_it_is_in_words() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    // The build isn't there: the download fails, and its card stays to say so.
    let core = host.core().clone();
    assert!(host.rt.block_on(core.dispatch(Command::ConfirmInstall)).is_err());
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    host.settle();
    assert_eq!(host.values("download-state"), ["Failed"], "a word for the player, not the core's \"failed\"");
}

#[test]
fn not_now_closes_the_install_prompt() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();
    host.navigate(Key::Escape);
    assert!(host.values("install-prompt").is_empty(), "Escape says not now");
    assert!(host.core().view().install_prompt.is_none());
}

/// The Emulators list, then KytyPS5's sheet.
fn kyty_page() -> Composed {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.settle();
    host.first("plugin:emu-kyty").invoke_accessible_default_action();
    host.pump();
    host.settle();
    host
}

#[test]
fn a_plugin_s_page_opens_from_plugins_and_runs_its_actions() {
    let host = kyty_page();
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Plugin);
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("on"));
    // Slot widgets carry their plugin and slot id as the label: "emu-kyty:check" is "Check for new builds".
    host.first("emu-kyty:check").invoke_accessible_default_action();
    let asked =
        host.requests.borrow().iter().any(
            |r| matches!(r, nexus_ui_host::UiRequest::Core(Command::RunAction { action, .. }) if action == "check"),
        );
    assert!(asked, "the row runs the plugin's action");
}

#[test]
fn a_plugin_s_page_changes_its_settings() {
    let mut host = kyty_page();
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("false"));
    host.first("setting:own_build").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"));
}

#[test]
fn a_plugin_turned_off_on_its_page_shows_only_the_switch() {
    let mut host = kyty_page();
    host.first("plugin-enabled").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("off"));
    assert_eq!(host.count("setting:fullscreen"), 0, "its settings are hidden");
    assert_eq!(host.count("build"), 0);
}

#[test]
fn the_emulators_list_names_each_console_and_its_program_s_build() {
    let mut host = composed(|h| {
        h.plugin(
            FakeCatalog::new("emu-k")
                .named("PS5")
                .emulator("KytyPS5")
                .platform("PS5")
                .without_slots()
                .playable()
                .installed("KytyPS5-2026-10-21-e2dd48a")
                .games([])
                .build(),
        )
        .plugin(
            FakeCatalog::new("emu-s")
                .named("PS4")
                .emulator("shadPS4")
                .platform("PS5")
                .without_slots()
                .playable()
                .installed("v.0.20.0")
                .games([])
                .build(),
        )
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.settle();
    let hint = |plugin: &str| host.first(plugin).accessible_description().unwrap_or_default().to_string();
    assert_eq!(hint("plugin:emu-k"), "Installed · KytyPS5-2026-10-21-e2dd48a", "the build names its program once");
    assert_eq!(hint("plugin:emu-s"), "Installed · shadPS4 v.0.20.0");
    assert_eq!((host.count("PS5"), host.count("PS4")), (1, 1), "each row names its console");
    assert_eq!((host.count("KytyPS5"), host.count("shadPS4")), (0, 0), "and not its program");
}

#[test]
fn an_emulator_with_no_builds_is_not_called_not_installed() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.settle();
    let hint = host.first("plugin:emu-pc").accessible_description().unwrap_or_default().to_string();
    assert_eq!(hint, "No install needed", "PC games need no build");
}

#[test]
fn an_emulator_with_no_builds_has_no_build_row() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin("emu-pc".parse().unwrap())));
    host.settle();
    assert!(host.core().view().plugin_page.is_some(), "PC's page");
    assert_eq!(host.count("build"), 0, "PC games need no build, so there is nothing to install");
    assert_eq!(host.count("uninstall"), 0);
}

#[test]
fn the_game_catalog_page_shows_its_sources_its_refresh_and_refresh_now() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Alpha"]).build().console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // Sources with one PS5 forum, so the test doesn't depend on the project's own file, and
    // its feed and forum list.
    let sources = serde_json::json!({ "ps5": { "platform": "PS5",
        "forums": [{ "key": "ps5", "url": "https://xyz.net/forum/viewforum.php?f=546", "tag": "PS5" }] } });
    host.test.http.serve("https://xyz.dev/catalog-sources.json", sources.to_string());
    let feed = include_str!("../../../../plugins/services/release-catalog/fixtures/forum-546.atom");
    host.test.http.serve(&release_catalog::refresh::feed_url("546"), feed);
    let list = include_str!("../../../../plugins/services/release-catalog/fixtures/pvc-546.json");
    host.test.http.serve(&release_catalog::refresh::list_url("546"), list);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "sources_url": "https://xyz.dev/catalog-sources.json",
            "refresh_from_forums": true }),
    });
    // Small enough for the blocks to fit under the settings, a "Show PS5 games" among them.
    host.dispatch(Command::Pref(Pref::Scale, 0.8.into()));
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin(id("release-catalog"))));
    host.settle();
    // The blocks: the theme's section cards carry no label a test can find, so the blocks are
    // checked in the view, and their place on the page by its label.
    assert_eq!(host.count("plugin-blocks"), 1, "the page draws the service's blocks");
    let page = host.core().view().plugin_page.expect("the plugin's page");
    let labels: Vec<String> = page.sections.iter().map(|s| s.label.clone()).collect();
    for label in ["release-catalog:sources", "release-catalog:refresh"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    assert_eq!(host.count("release-catalog:refresh-now"), 1, "the Refresh now row");
    assert_eq!(host.count("build"), 0, "a service has no build");

    // Refresh now asks the forums again. The refresh runs in the background: the runtime runs
    // it while the test waits.
    let feed = release_catalog::refresh::feed_url("546");
    let asked = |host: &Composed| host.test.http.sent().iter().filter(|u| **u == feed).count();
    // The refresh runs in the plugin's own task: the clock moves until it asked.
    let wait_for = |host: &Composed, count: usize| {
        for _ in 0..100 {
            if asked(host) >= count {
                break;
            }
            host.advance(std::time::Duration::from_millis(10));
        }
    };
    wait_for(&host, 1);
    let before = asked(&host);
    assert_eq!(before, 1, "the core's read refreshed the catalog");
    host.first("release-catalog:refresh-now").invoke_accessible_default_action();
    host.pump();
    host.settle();
    wait_for(&host, before + 1);
    // Once for the action. The core's read of the catalogs after it finds the copy fresh.
    host.advance(std::time::Duration::from_millis(100));
    assert_eq!(asked(&host), before + 1, "Refresh now asked the forums");
}

#[test]
fn a_section_card_grows_with_values_that_wrap() {
    let long = "Refreshed Oct 23, 2026 at 06:06 UTC: 16 new, 0 removed, and a few words more to wrap";
    let rows = [("PC", long), ("PS4", long), ("PS5", long)];
    let facts = FakeService::new("facts", "Facts").section(
        nexus_api::SlotId::EmulatorPanel,
        "refresh",
        "Catalog refresh",
        &rows,
    );
    let mut host = composed(|h| h.plugin(facts.build()));
    host.ui.start().unwrap();
    host.resize(700.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin(id("facts"))));
    host.settle();

    let cards: Vec<(f32, f32)> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "section-card")
            .map(|c| (c.absolute_position().y, c.absolute_position().y + c.size().height))
            .collect();
    let values: Vec<i_slint_backend_testing::ElementHandle> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "section-value").collect();
    assert_eq!((cards.len(), values.len()), (1, 3), "one card, its three values");
    assert!(values.iter().all(|v| v.size().height > 30.0), "each value wraps onto two lines or more");
    for value in &values {
        let bottom = value.absolute_position().y + value.size().height;
        assert!(bottom <= cards[0].1 + 0.5, "a value ends at {bottom}, below its card's {}", cards[0].1);
    }
}

#[test]
fn a_plugin_s_page_leaves_the_shared_folders_to_settings() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin("emu-kyty".parse().unwrap())));
    host.settle();
    let page = host.core().view().plugin_page.expect("KytyPS5's page");
    let names: Vec<_> = page.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["kyty-build"], "its own folder only: the shared ones are in Settings");
    assert_eq!(host.count("folder:packages"), 0);
    assert_eq!(host.count("folder:games"), 0);
}

#[test]
fn a_theme_s_sheet_in_themes_has_its_options_while_it_is_in_use() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Themes));
    host.settle();
    assert_eq!(host.count("plugin:emu-a"), 0, "Themes lists themes only");
    host.first("plugin:default").invoke_accessible_default_action();
    host.pump();
    host.settle();

    assert_eq!(host.first("theme-current").accessible_value().as_deref(), Some("on"));
    assert_eq!(host.first("option:direction").accessible_value().as_deref(), Some("ltr"));
    host.first("option:direction").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("option:direction").accessible_value().as_deref(), Some("rtl"));
}

#[test]
fn emulators_lists_emulators_only() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.settle();
    assert_eq!(host.count("plugin:emu-a"), 1);
    assert_eq!(host.count("plugin:default"), 0, "themes are in Themes");
}

#[test]
fn right_to_left_the_install_prompt_has_install_and_play_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    host.settle();
    assert!(x_of(&host, "Install and play") > x_of(&host, "Not now"), "Install and play on the right");
    host.navigate(Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(asked.iter().any(|c| matches!(c, Command::ConfirmInstall)), "Right stays on it: {asked:?}");
}
