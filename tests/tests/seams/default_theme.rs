//! Seam 3: the default theme's home: Home and Library in the top bar, the tile row and hero,
//! the Library's filters and grid, and the theme's options.

use crate::common::*;
use nexus_core::{Area, Command, Pref, SessionCmd};
use slint_interpreter::ComponentHandle;

fn default_theme() -> Composed {
    default_theme_with(|h| h)
}

/// [`default_theme`], with more plugins.
fn default_theme_with(more: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    let mut host = composed(|h| {
        more(h.plugin(games_on("emu-a", "PS4", vec!["Alpha", "Beta"])).plugin(games_on("emu-b", "PS5", vec!["Gamma"])))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    settle(&mut host);
    host
}

fn values(host: &Composed, label: &str) -> Vec<String> {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label);
    found.map(|e| e.accessible_value().unwrap_or_default().to_string()).collect()
}

/// The element labelled `label` whose value is `value`, invoked.
fn press(host: &mut Composed, label: &str, value: &str) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label);
    let target = found
        .into_iter()
        .find(|e| e.accessible_value().as_deref() == Some(value))
        .unwrap_or_else(|| panic!("no {label} {value}"));
    target.invoke_accessible_default_action();
    host.pump();
    settle(host);
}

/// Down in Settings until the row labelled `label` is in view: rows below the sheet's edge are
/// not drawn.
fn down_to(host: &mut Composed, label: &str) {
    for _ in 0..40 {
        if host.count(label) > 0 {
            return;
        }
        key(host, slint::platform::Key::DownArrow);
    }
    panic!("no {label} in Settings");
}

/// A top bar tab, invoked: Home or Library.
fn press_tab(host: &mut Composed, name: &str) {
    press(host, "default-system", name);
}

/// A Library filter, invoked.
fn press_filter(host: &mut Composed, name: &str) {
    let label = format!("filter:{name}");
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), &label);
    found.into_iter().next().unwrap_or_else(|| panic!("no filter {name}")).invoke_accessible_default_action();
    host.pump();
    settle(host);
}

fn key(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
    settle(host);
}

#[test]
fn with_the_game_catalog_turned_off_the_ways_to_the_catalog_go() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-kyty", vec!["Astro"]).console("PS5"))
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
fn the_boot_screen_shows_until_the_library_is_in() {
    let mut host = composed(|h| h.plugin(games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "boot"), ["Loading..."], "from the first frame");

    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    settle(&mut host);
    assert!(values(&host, "boot").is_empty(), "it fades out over Home");
}

/// Home's line for the first game, once its details are in.
fn home_line(host: &mut Composed) -> String {
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.until(Area::Library, |core| core.view().detail.is_some());
    host.ui.render().unwrap();
    settle(host);
    let shown = values(host, "default-description");
    let [text] = shown.as_slice() else { panic!("one line, got {shown:?}") };
    text.clone()
}

#[test]
fn home_shows_the_store_s_tagline() {
    let tagline = "Build whatever you can imagine, in a world that is yours.";
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, tagline)],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    assert_eq!(home_line(&mut host), tagline);
}

#[test]
fn without_a_tagline_home_shows_the_game_s_facts_on_one_line() {
    let mut host = composed(|h| {
        h.plugin(games_without_description("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    // The stub's version fact is 01.000.002. Past the summary's length, the same line is cut.
    let full = "Alpha · Adventure, Arcade · Developer: MOJANG AB · Version: 01.000.002";
    let text = home_line(&mut host);
    match text.strip_suffix('…') {
        Some(kept) => assert!(full.starts_with(kept) && full[kept.len()..].starts_with(' '), "{text}"),
        None => assert_eq!(text, full),
    }
}

#[test]
fn home_cuts_a_long_line_after_a_word() {
    // 30 × 18 characters: longer than any line Home shows.
    const LONG: &str = "Build and explore Build and explore Build and explore Build and explore Build and explore \
                        Build and explore Build and explore Build and explore Build and explore Build and explore \
                        Build and explore Build and explore Build and explore Build and explore Build and explore \
                        Build and explore Build and explore Build and explore Build and explore Build and explore \
                        Build and explore Build and explore Build and explore Build and explore Build and explore \
                        Build and explore Build and explore Build and explore Build and explore Build and explore";
    let mut host = composed(|h| h.plugin(games_without_description("emu-a", vec![LONG])));
    host.ui.start().unwrap();
    let text = home_line(&mut host);
    let kept = text.strip_suffix('…').unwrap_or_else(|| panic!("ends with …: {text}"));
    assert!(text.chars().count() <= nexus_ui_api::SUMMARY_CHARS + 1, "{} characters", text.chars().count());
    assert!(LONG.starts_with(kept) && LONG[kept.len()..].starts_with(' '), "cut after a whole word: {text}");
}

#[test]
fn a_rating_counts_its_players_in_thousands() {
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.44, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    let detail = host.instance().get_global_property("Vm", "detail").unwrap();
    let slint_interpreter::Value::Struct(detail) = detail else { panic!("a struct") };
    assert_eq!(detail.get_field("rating-text"), Some(&slint_interpreter::Value::String("4.4 · 3.2K ratings".into())));
}

#[test]
fn the_boot_screen_waits_for_homes_art() {
    let mut host = composed(|h| {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(8, 8).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let art = h.http().serve("https://art.example/alpha.png", png.into_inner());
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            Some(art),
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    settle(&mut host);
    assert_eq!(values(&host, "boot"), ["All set · 1 installed game found"], "the library is in, its art isn't");

    host.load_images();
    settle(&mut host);
    assert!(values(&host, "boot").is_empty(), "it goes once the art is in");
}

#[test]
fn a_tile_shows_a_light_while_its_cover_loads_and_the_name_only_without_art() {
    // Only Alpha (CUSA00000) has a cover.
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha", "Beta"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            Some("https://art.example/alpha.png".into()),
            std::time::Duration::ZERO,
        ))
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(64, 96, image::Rgb([200, 60, 40])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    host.test.http.serve("https://art.example/alpha.png", png.into_inner());
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    settle(&mut host);
    assert_eq!(host.count("cover-loading"), 1, "Alpha's cover is on its way");
    assert_eq!(values(&host, "cover-name"), ["Beta"], "only the game without art shows its name");

    host.load_images();
    settle(&mut host);
    assert_eq!(host.count("cover-loading"), 0, "the cover is in");
    assert_eq!(values(&host, "cover-name"), ["Beta"]);
}

#[test]
fn the_first_run_welcomes_the_player_and_enter_starts() {
    use slint::platform::Key;
    let mut host = composed(|h| h.first_run().plugin(games("emu-a", vec!["Alpha", "Beta"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    settle(&mut host);
    assert_eq!(values(&host, "welcome"), ["All set · 2 installed games found"]);
    let steps = values(&host, "welcome-step");
    assert_eq!(steps.first().map(String::as_str), Some("Game folders: 2 games"));
    assert_eq!(steps.last().map(String::as_str), Some("Covers & Home screen: Ready"));

    key(&mut host, Key::Return);

    assert!(host.core().prefs().welcomed, "saved: it doesn't show again");
    assert!(values(&host, "welcome").is_empty());
}

#[test]
fn the_top_bar_has_home_and_library_and_home_shows_every_game() {
    let host = default_theme();
    assert_eq!(values(&host, "default-system"), ["Home", "Library"]);
    let mut tiles = values(&host, "default-tile");
    tiles.sort();
    assert_eq!(tiles, ["Alpha", "Beta", "Gamma"], "Home's row has every game");
}

#[test]
fn the_library_shows_every_game_and_a_filter_shows_one_system() {
    let mut host = default_theme();
    press_tab(&mut host, "Library");
    assert_eq!(values(&host, "default-count"), ["3 games"]);
    assert!(values(&host, "default-tile").is_empty(), "the tile row is hidden behind the Library");

    press_filter(&mut host, "PS4");
    assert_eq!(values(&host, "default-count"), ["2 games"]);
    let mut cards = values(&host, "default-game");
    cards.sort();
    assert_eq!(cards, ["Alpha", "Beta"]);

    press_tab(&mut host, "Home");
    assert_eq!(values(&host, "default-tile").len(), 3, "Home still has every game");
}

#[test]
fn the_library_searches_and_sorts_in_the_core() {
    let mut host = default_theme();
    press_tab(&mut host, "Library");
    host.dispatch(Command::LibrarySearch("gam".into()));
    settle(&mut host);
    assert_eq!(values(&host, "default-game"), ["Gamma"]);
    assert_eq!(values(&host, "default-count"), ["1 game"]);

    host.dispatch(Command::LibrarySearch(String::new()));
    host.first("library-sort").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    press(&mut host, "sort-option", "played");
    assert_eq!(values(&host, "library-sort"), ["played"], "the picked sort");
    assert_eq!(host.count("sort-option"), 0, "the list closes");
}

#[test]
fn leaving_the_library_closes_the_sorts_and_gives_the_keys_back() {
    use slint::platform::Key;
    let mut host = default_theme();
    press_tab(&mut host, "Library");
    host.first("library-sort").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(host.count("sort-option") > 0);

    press_tab(&mut host, "Home");
    assert_eq!(host.count("sort-option"), 0, "the list closes");
    key(&mut host, Key::Return);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "Enter is Home's again");
    assert_eq!(host.core().view().home.library.sort, "title", "the sort is as it was");
}

#[test]
fn enter_on_sort_opens_the_sorts_and_the_keys_pick_one() {
    use slint::platform::Key;
    // The Catalog's sorts too.
    let mut host =
        default_theme_with(|h| h.built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default())));
    press_tab(&mut host, "Library");
    // Up to the bar, right to Sort, Enter.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "sort-option"), ["title", "played", "size", "rating", "release", "compat"]);
    assert_eq!(values(&host, "sort-current"), ["title"], "the sort in use is marked");
    assert_eq!(values(&host, "library-sort"), ["title"], "Enter opens the list, it changes nothing");
    assert_eq!(values(&host, "hint"), ["Pick", "Close"]);

    // Back closes it and changes nothing; the keys stay on Sort.
    key(&mut host, Key::Escape);
    assert_eq!(host.count("sort-option"), 0);
    assert_eq!(host.count("library-sort"), 1, "still in the Library");

    // Down twice and Enter: Size.
    key(&mut host, Key::Return);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    assert_eq!(host.count("sort-option"), 0, "Enter picks and closes");
    assert_eq!(values(&host, "library-sort"), ["size"]);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "sort-current"), ["size"]);
    key(&mut host, Key::Escape);

    // The Catalog's sorts.
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    host.first("library-sort").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(values(&host, "sort-option"), ["added", "title", "size", "smallest", "rating", "release", "compat"]);
}

#[test]
fn one_click_on_a_library_card_opens_its_game() {
    let mut host = default_theme();
    use slint::platform::Key;
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    // A card that isn't the selected one.
    let shown = values(&host, "default-game");
    let other = shown.iter().find(|t| t.as_str() != "Alpha").expect("another card").clone();
    press(&mut host, "default-game", &other);
    host.pump();
    settle(&mut host);
    let view = host.core().view();
    assert_eq!(view.screen, nexus_ui_api::ScreenId::GameDetail, "opened, not only selected");
    assert_eq!(view.detail.map(|d| d.title), Some(other));
}

#[test]
fn the_game_s_menu_lists_what_the_game_has_and_rates_it() {
    let mut host = default_theme();
    press(&mut host, "game-options", "Options");
    let items = values(&host, "game-menu");
    assert_eq!(
        items,
        ["Game Hub", "Verify files", "Rate how it runs", "Settings"],
        "no folder, log or list for the stub"
    );

    press(&mut host, "game-menu", "Rate how it runs");
    assert!(values(&host, "game-menu").is_empty(), "the menu closes");
    assert_eq!(values(&host, "result-prompt").len(), 1, "How far did it get?");
}

#[test]
fn settings_show_this_pc_and_how_each_emulator_runs_on_it() {
    let specs = nexus_core::Specs {
        cpu: "Apple M4 Max".into(),
        cores: 14,
        threads: 14,
        memory_gb: 36.0,
        avx2: None,
        gpus: Some(vec![nexus_core::Gpu {
            name: "Apple M4 Max".into(),
            memory_gb: None,
            discrete: false,
            unified: true,
            vulkan: Some((1, 3)),
        }]),
    };
    let mut host = composed(|h| h.hardware(specs).plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.until(Area::Hardware, |core| core.view().settings.hardware.known);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);

    // Down to the end of the sheet, as a pad does: the rows take the focus, so it scrolls.
    for _ in 0..20 {
        key(&mut host, slint::platform::Key::DownArrow);
    }
    assert_eq!(values(&host, "pc-cpu"), ["Apple M4 Max · 14 cores"]);
    assert_eq!(values(&host, "pc-memory"), ["36 GB"]);
    assert_eq!(values(&host, "pc-graphics"), ["Apple M4 Max · shared memory · Vulkan 1.3"]);
    assert_eq!(values(&host, "pc-verdict"), ["emu-a: Should run well"], "a plugin that asks for nothing");
}

#[test]
fn the_game_hub_stops_a_running_game() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);

    press(&mut host, "hub-stop", "Stop");
    let process = host.test.spawner.spawned()[0].clone();
    // The game watcher's rounds carry out the stop.
    let mut asked = false;
    for _ in 0..100 {
        asked = !process.signals().is_empty();
        if asked {
            break;
        }
        host.advance(std::time::Duration::from_millis(10));
    }
    assert!(asked, "Stop asks the game to close");
}

#[test]
fn a_service_is_listed_after_the_emulators_with_nothing_to_install() {
    let mut host = composed(|h| {
        h.plugin(store("store", vec![], None, std::time::Duration::ZERO)).plugin(playable_games("emu-a", vec!["Alpha"]))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    settle(&mut host);
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
fn letters_in_the_library_search_and_the_pad_and_keys_have_their_shortcuts() {
    use nexus_ui_host::PadButton;
    use slint::platform::Key;
    let mut host = default_theme();
    // Into the Library's grid.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    // Page Down (R2) goes three lines on, here to the last game.
    host.ui.pad(PadButton::RightTrigger).unwrap();
    host.pump();
    settle(&mut host);
    rest(&mut host);
    assert_eq!(host.core().view().home.library.selected, Some(2));

    // Square is the trailer's button, not a letter: in the grid it searches for nothing.
    host.ui.pad(PadButton::West).unwrap();
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().view().home.library.query, "");

    // A letter in the grid starts a search with it.
    let text: slint::SharedString = "g".into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().view().home.library.query, "g");
    assert_eq!(values(&host, "default-game"), ["Gamma"]);

    // More letters: the search runs 300 ms after the last one, not at each letter.
    let type_letter = |host: &mut Composed, letter: &str| {
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: letter.into() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: letter.into() });
    };
    let wait = |host: &mut Composed, ms: u64| {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
        host.pump();
    };
    type_letter(&mut host, "a");
    wait(&mut host, 200);
    type_letter(&mut host, "m");
    wait(&mut host, 200);
    assert_eq!(host.core().view().home.library.query, "g", "400 ms after the first letter, 200 after the last");
    wait(&mut host, 200);
    assert_eq!(host.core().view().home.library.query, "gam");

    // Ctrl+D: Downloads.
    let ctrl: slint::SharedString = Key::Control.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: ctrl.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "d".into() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "d".into() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: ctrl });
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Downloads);
}

#[test]
fn the_menu_watches_the_store_s_trailer() {
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    press(&mut host, "game-options", "Options");
    assert!(values(&host, "game-menu").contains(&"Watch trailer".to_owned()));
    // `press` carries out what the theme asked for; record it first.
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "game-menu")
        .find(|e| e.accessible_value().as_deref() == Some("Watch trailer"))
        .expect("the trailer item");
    found.invoke_accessible_default_action();
    let opened = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::OpenUrl(u) if u == "https://art.example/CUSA00000.mp4"));
    assert!(opened, "the trailer opens in the browser");
}

#[test]
fn the_key_hints_name_the_trailer_and_options_and_hide_over_the_grids() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"]))
            .plugin(store("store", vec![("CUSA00000", 4.5, "")], None, std::time::Duration::ZERO))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    assert_eq!(values(&host, "hint"), ["Select", "Search", "Trailer", "Options"], "Home, on a game with a trailer");

    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    let hub = values(&host, "hint");
    assert!(hub.ends_with(&["Back".into(), "Trailer".into(), "Options".into()]), "the Game Hub: {hub:?}");

    // The Library's grid: no pill over the cards.
    key(&mut host, Key::Escape);
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    assert_eq!(host.count("default-game"), 1, "in the Library");
    assert!(values(&host, "hint").is_empty(), "{:?}", values(&host, "hint"));

    // The game's menu over the Library has its own hints.
    key(&mut host, Key::Menu);
    assert_eq!(values(&host, "hint"), ["Pick", "Close"]);
    key(&mut host, Key::Escape);

    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    assert!(values(&host, "hint").is_empty(), "the Catalog's grid: {:?}", values(&host, "hint"));
}

#[test]
fn the_game_hub_s_screenshots_open_full_screen_and_move_with_the_keys() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    // Down opens the technical details, Down again is on the screenshots.
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "shot-viewer"), ["1 / 2"]);
    key(&mut host, Key::RightArrow);
    assert_eq!(values(&host, "shot-viewer"), ["2 / 2"]);
    key(&mut host, Key::RightArrow);
    assert_eq!(values(&host, "shot-viewer"), ["2 / 2"], "the last one stays");
    key(&mut host, Key::Escape);
    assert!(values(&host, "shot-viewer").is_empty(), "Escape closes it");
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "and stays in the Game Hub");
}

#[test]
fn down_from_the_screenshots_reads_on_down_the_game_hub() {
    use slint::platform::Key;
    // A long name: its ABOUT ("About <name>") runs far below the window.
    let long: &'static str = Box::leak("Alpha and the Long Road ".repeat(200).into_boxed_str());
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec![long])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    host.resize(1600.0, 900.0);
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::DownArrow);
    let about = |host: &Composed| {
        let text = host.first("hub-about");
        text.absolute_position().y + text.size().height
    };
    let at_shots = about(&host);
    assert!(at_shots > 900.0, "ABOUT ends below the window: {at_shots}");
    // Down from the screenshots: the page scrolls on.
    key(&mut host, Key::DownArrow);
    let read = about(&host);
    assert!(read < at_shots, "the page moved on: {at_shots} to {read}");
    // Up: the keys are on the screenshots again.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "shot-viewer"), ["1 / 2"]);
    key(&mut host, Key::Escape);
    // Down to the end of ABOUT; more presses stop there, so one Up moves at once.
    for _ in 0..30 {
        key(&mut host, Key::DownArrow);
    }
    let end = about(&host);
    assert!(end <= 900.0, "the end of ABOUT is in view: {end}");
    key(&mut host, Key::UpArrow);
    let up = about(&host);
    assert!(up > end, "Up scrolls back at once: {end} to {up}");
}

#[test]
fn the_game_hub_s_screenshot_row_scrolls_to_the_focused_shot() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    // A narrow window: the row has room for one shot.
    host.resize(900.0, 900.0);
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::RightArrow);
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "screenshot");
    let second = found.into_iter().find(|e| e.accessible_value().as_deref() == Some("2"));
    let second = second.expect("the focused shot is shown");
    let (x, width) = (second.absolute_position().x, second.size().width);
    assert!(x >= 0.0 && x + width <= 900.0, "the second shot is in the window: {x} to {}", x + width);
}

#[test]
fn the_game_hub_has_a_trailer_button_when_the_game_has_a_trailer() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    assert_eq!(values(&host, "hub-trailer"), ["Trailer"]);
    // The keys reach it: after the store's button, before the game's menu at the end.
    for _ in 0..10 {
        key(&mut host, Key::RightArrow);
    }
    key(&mut host, Key::LeftArrow);
    host.requests.borrow_mut().clear();
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let opened = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::OpenUrl(u) if u == "https://art.example/CUSA00000.mp4"));
    assert!(opened, "the trailer opens in the browser: {:?}", host.requests.borrow());
}

#[test]
fn a_trailer_plays_in_the_window_when_the_app_can() {
    use nexus_ui_host::{TrailerRequest, TrailerStatus, UiRequest};
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.set_trailers_available(true);
    host.ui.start().unwrap();
    home_line(&mut host);
    host.requests.borrow_mut().clear();

    // Square, or T: the trailer, in the window.
    let text: slint::SharedString = "t".into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let asked =
        host.requests.borrow().iter().any(
            |r| matches!(r, UiRequest::Trailer(TrailerRequest::Play(u)) if u == "https://art.example/CUSA00000.mp4"),
        );
    assert!(asked, "{:?}", host.requests.borrow());

    // The app's player says it plays.
    host.ui.set_trailer(Some(TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 }));
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "trailer-time"), ["playing 0:42 / 2:10"]);

    host.requests.borrow_mut().clear();
    for k in [Key::Return, Key::RightArrow, Key::Escape] {
        let text: slint::SharedString = k.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    }
    let got: Vec<_> = host
        .requests
        .borrow()
        .iter()
        .filter_map(|r| match r {
            UiRequest::Trailer(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(got, [TrailerRequest::TogglePause, TrailerRequest::Seek(10.0), TrailerRequest::Close]);

    // The close button, for a mouse.
    host.requests.borrow_mut().clear();
    host.first("trailer-close").invoke_accessible_default_action();
    let closed = host.requests.borrow().iter().any(|r| matches!(r, UiRequest::Trailer(TrailerRequest::Close)));
    assert!(closed, "{:?}", host.requests.borrow());
}

#[test]
fn right_to_left_the_trailer_s_progress_fills_from_the_right() {
    use nexus_ui_host::TrailerStatus;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.set_trailers_available(true);
    host.ui.start().unwrap();
    home_line(&mut host);
    host.resize(1600.0, 900.0);
    let text: slint::SharedString = "t".into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    host.ui.set_trailer(Some(TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 }));
    wait(&mut host, 100);
    let left = host.first("trailer-progress");
    assert!(left.absolute_position().x < 400.0, "left to right, from the left: {:?}", left.absolute_position());
    assert!(x_of(&host, "trailer-pause") < x_of(&host, "trailer-progress"), "the pause button on the left");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    wait(&mut host, 1000);
    let right = host.first("trailer-progress");
    let end = right.absolute_position().x + right.size().width;
    assert!(right.absolute_position().x > 800.0, "right to left, from the right: {:?}", right.absolute_position());
    assert!(x_of(&host, "trailer-pause") > end, "the pause button on the right");
}

#[test]
fn the_trailer_s_controls_hide_while_it_plays_and_a_key_shows_them() {
    use nexus_ui_host::TrailerStatus;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.set_trailers_available(true);
    host.ui.start().unwrap();
    home_line(&mut host);
    let text: slint::SharedString = "t".into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    // The shell starts the trailer: it opens, then the app's player says it plays.
    host.pump();
    let playing = TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 };
    host.ui.set_trailer(Some(playing.clone()));
    wait(&mut host, 100);
    assert_eq!(values(&host, "trailer-title"), ["Alpha"], "the controls name the game");
    assert_eq!(host.count("trailer-paused"), 0);

    // About 3 s with no key: the controls and the hints go.
    for _ in 0..4 {
        wait(&mut host, 1000);
    }
    assert!(values(&host, "trailer-title").is_empty(), "the controls hide");
    assert!(values(&host, "hint").is_empty(), "and the hints: {:?}", values(&host, "hint"));

    // Any key shows them again.
    let text: slint::SharedString = slint::platform::Key::UpArrow.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    wait(&mut host, 100);
    assert_eq!(values(&host, "trailer-title"), ["Alpha"]);

    // Paused: the controls stay, with the paused sign.
    host.ui.set_trailer(Some(TrailerStatus { playing: false, ..playing }));
    for _ in 0..5 {
        wait(&mut host, 1000);
    }
    assert_eq!(values(&host, "trailer-title"), ["Alpha"], "they stay while it is paused");
    assert_eq!(host.count("trailer-paused"), 1);
}

#[test]
fn the_launch_splash_shows_the_emulator_s_keys_and_takes_every_key_but_back_and_quit() {
    use nexus_ui_host::UiRequest;
    use slint::platform::Key;
    let keymap = "[[keymap]]\nbutton = \"Cross\"\nkeys = \"J\"\n[[keymap]]\nbutton = \"Circle\"\nkeys = \"L\"\n";
    let mut host = composed(|h| h.plugin(playable_games_with("emu-a", vec!["Alpha"], keymap)));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // Play from the game's Game Hub. No window yet: the game stays "starting", behind the splash.
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    host.test.windows.no_windows();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    settle(&mut host);
    assert_eq!(values(&host, "launch-key"), ["Cross J", "Circle L"]);
    assert!(values(&host, "hint").is_empty(), "no launcher hints: {:?}", values(&host, "hint"));

    // The keys go to the game, not to the launcher behind the splash.
    let screen = host.core().view().screen;
    host.requests.borrow_mut().clear();
    for k in [Key::DownArrow, Key::Return, Key::Backspace] {
        tap(&mut host, k);
    }
    assert!(host.requests.borrow().is_empty(), "no sound and no command: {:?}", host.requests.borrow());
    settle(&mut host);
    assert_eq!(host.core().view().screen, screen, "nothing opened or closed");

    // Escape goes on, as on the splash without the keys: the second one is Back, to the Game
    // Hub.
    key(&mut host, Key::Escape);
    key(&mut host, Key::Escape);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail);

    // Ctrl+Q quits.
    let ctrl: slint::SharedString = Key::Control.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: ctrl.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "q".into() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "q".into() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: ctrl });
    let quit = host.requests.borrow().iter().any(|r| matches!(r, UiRequest::Quit));
    assert!(quit, "Ctrl+Q closes the launcher");
}

#[test]
fn a_notice_opens_what_it_is_about() {
    // A game whose plugin can't start games: Play leaves a notice about it.
    let mut host = default_theme();
    let core = host.core().clone();
    let failed = host.rt.block_on(core.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap()))));
    assert!(failed.is_err());
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "notice-open"), ["Open the Game Hub"]);

    press(&mut host, "notice-open", "Open the Game Hub");
    let view = host.core().view();
    assert_eq!(view.screen, nexus_ui_api::ScreenId::GameDetail);
    assert_eq!(view.game.as_deref(), Some("emu-a:G0"));
    assert!(view.notices.is_empty(), "and the notice goes");
}

/// Moves the testing clock on by `ms`, then carries out what the theme asked for.
fn wait(host: &mut Composed, ms: u64) {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
    host.ui.render().unwrap();
    host.pump();
    host.ui.render().unwrap();
}

#[test]
fn a_notice_goes_after_a_while_and_one_with_a_link_stays_longer() {
    let mut host = default_theme();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["Couldn't read the folder"]);
    wait(&mut host, 3000);
    assert_eq!(values(&host, "notice").len(), 1, "it stays long enough to read");
    // After 4 s it slides out, then goes.
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(values(&host, "notice").is_empty());

    // A new notice joins the others, and each goes 4 s after it came.
    host.core().report("First".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    wait(&mut host, 3000);
    host.core().report("Second".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["First", "Second"], "both show");
    for _ in 0..4 {
        wait(&mut host, 500);
    }
    assert_eq!(values(&host, "notice"), ["Second"], "First went 4 s after it came");
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty());

    // A notice with something to open stays 10 s.
    let core = host.core().clone();
    assert!(
        host.rt.block_on(core.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())))).is_err()
    );
    host.ui.render().unwrap();
    wait(&mut host, 100);
    for _ in 0..8 {
        wait(&mut host, 1000);
    }
    assert_eq!(values(&host, "notice-open"), ["Open the Game Hub"], "still there after 8 s");
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty(), "gone after 10 s");
}

#[test]
fn delete_closes_the_notice() {
    let mut host = default_theme();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice").len(), 1);
    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    wait(&mut host, 1000);
    wait(&mut host, 1000);
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(values(&host, "notice").is_empty());
}

#[test]
fn notices_stack_up_to_three_and_delete_closes_only_the_newest() {
    let mut host = default_theme();
    for text in ["First", "Second", "Third", "Fourth"] {
        host.core().report(text.into());
    }
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["Second", "Third", "Fourth"], "the oldest goes");
    assert_eq!(values(&host, "notice-kind"), ["error", "error", "error"], "a problem is an error");

    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    wait(&mut host, 1000);
    wait(&mut host, 1000);
    assert_eq!(host.core().view().notices, ["Second", "Third"], "only the newest closes");
    assert_eq!(values(&host, "notice"), ["Second", "Third"]);
}

#[test]
fn each_notice_keeps_its_own_time_when_an_older_one_goes() {
    let mut host = default_theme();
    host.core().report("First".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    for _ in 0..4 {
        wait(&mut host, 500);
    }
    host.core().report("Second".into());
    host.ui.render().unwrap();
    // First goes 4 s after it came; Second stays its own 4 s.
    for _ in 0..7 {
        wait(&mut host, 500);
    }
    assert_eq!(host.core().view().notices, ["Second"], "First went");
    assert_eq!(values(&host, "notice"), ["Second"]);
    for _ in 0..3 {
        wait(&mut host, 500);
    }
    assert!(host.core().view().notices.is_empty(), "Second went 4 s after it came");
}

#[test]
fn the_download_sheet_has_no_options_hint_and_options_does_not_open_the_menu() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    assert!(values(&host, "hint").contains(&"Options".to_owned()), "{:?}", values(&host, "hint"));
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(host.count("download-agree"), 1);
    let hints = values(&host, "hint");
    assert!(!hints.contains(&"Options".to_owned()), "{hints:?}");
    assert_eq!(hints, ["Look up metadata", "Not now"]);

    key(&mut host, slint::platform::Key::Menu);
    assert_eq!(host.count("game-menu"), 0, "the sheet keeps the keys");
}

#[test]
fn the_launch_splash_names_the_controller_or_the_keyboard() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // No window yet: the game stays "starting", behind the splash.
    host.test.windows.no_windows();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    settle(&mut host);
    assert_eq!(values(&host, "launch-input"), ["Keyboard: the keys are on the emu-a page"]);

    host.ui.set_pad_name(Some("DualSense Wireless Controller".into()));
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "launch-input"), ["Controller: DualSense Wireless Controller"]);
}

#[test]
fn with_two_monitors_settings_choose_the_one_the_launcher_fills() {
    let mut host = default_theme();
    host.ui.set_monitors(vec!["Built-in Retina Display".into(), "LEN G32qc-10".into()]);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "monitor");
    press(&mut host, "monitor", "");
    assert_eq!(host.core().prefs().monitor.as_deref(), Some("LEN G32qc-10"), "the next monitor after the one it is on");
    host.ui.render().unwrap();
    settle(&mut host);
    press(&mut host, "monitor", "LEN G32qc-10");
    assert_eq!(host.core().prefs().monitor.as_deref(), Some("Built-in Retina Display"), "round again");
}

#[test]
fn the_keys_reach_the_library_s_filters_and_its_card_size() {
    let mut host = default_theme();
    use slint::platform::Key;
    // Up to the top bar, right to Library, and in.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "default-game").len(), 3);
    // Up from the grid to the systems; right to the first one.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "default-game").len(), 2, "the first system's games");
    key(&mut host, Key::LeftArrow);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "default-game").len(), 3, "back to All");
    // Up to the bar: search, sort, Comfortable, Compact.
    key(&mut host, Key::UpArrow);
    for _ in 0..3 {
        key(&mut host, Key::RightArrow);
    }
    assert_eq!(values(&host, "density"), ["comfortable"]);
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "density"), ["compact"]);
}

#[test]
fn the_genre_row_scrolls_to_the_focused_chip_and_fades_where_chips_are_hidden() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    // A narrow window: the three genre chips don't fit.
    host.resize(640.0, 1200.0);
    settle(&mut host);
    press_tab(&mut host, "Library");
    assert_eq!(host.count("filter:Adventure"), 1, "the store's genres");
    assert_eq!(host.count("genres-more-end"), 1, "chips hide past the far edge");
    assert_eq!(host.count("genres-more-start"), 0, "none before the first");

    // Up from the grid to the genres, then right to the last chip.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::RightArrow);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
    settle(&mut host);
    let row = host.first("genres");
    let chip = host.first("filter:Arcade");
    let row_end = row.absolute_position().x + row.size().width;
    let chip_end = chip.absolute_position().x + chip.size().width;
    assert!(chip_end <= row_end, "the focused chip is in view: it ends at {chip_end}, the row at {row_end}");
    assert_eq!(host.count("genres-more-start"), 1, "chips hide before the first shown");
    assert_eq!(host.count("genres-more-end"), 0, "the last chip shows");
}

/// The x of the first element labelled `label`.
fn x_of(host: &Composed, label: &str) -> f32 {
    host.first(label).absolute_position().x
}

#[test]
fn right_to_left_the_library_s_header_and_genre_row_are_mirrored() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            None,
            std::time::Duration::ZERO,
        ))
    });
    host.ui.start().unwrap();
    home_line(&mut host);
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.resize(640.0, 1200.0);
    settle(&mut host);
    press_tab(&mut host, "Library");

    // The bar: the sort on the right of the cards' size.
    assert!(x_of(&host, "library-sort") > x_of(&host, "density"), "the sort is right of the size");
    // The systems: the first on the right; the count on the left.
    assert!(x_of(&host, "filter:PS4") < x_of(&host, "filter:All") - 1.0, "the first system on the right");
    assert!(x_of(&host, "default-count") < x_of(&host, "filter:PS4"), "the count on the left");

    // The genres: the first on the right, and the chips hide past the left edge.
    let row = host.first("genres");
    let middle = row.absolute_position().x + row.size().width / 2.0;
    assert_eq!(host.count("filter:Arcade"), 0, "the last genre hides past the left edge");
    assert_eq!(host.count("genres-more-start"), 0, "none before the first");
    assert!(x_of(&host, "genres-more-end") < middle, "the far edge is on the left");

    // Up to the genres, then on to the last chip: Left, from right to left.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::LeftArrow);
    key(&mut host, Key::LeftArrow);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
    settle(&mut host);
    let row = host.first("genres");
    let chip = host.first("filter:Arcade");
    assert!(
        chip.absolute_position().x >= row.absolute_position().x,
        "the focused chip is in view: it starts at {}, the row at {}",
        chip.absolute_position().x,
        row.absolute_position().x
    );
    assert!(x_of(&host, "filter:Arcade") < x_of(&host, "filter:Adventure"), "the genres go from right to left");
    assert_eq!(host.count("genres-more-end"), 0, "the last chip shows");
    assert!(x_of(&host, "genres-more-start") > middle, "chips hide past the right edge");
}

#[test]
fn the_game_hub_s_cover_grows_from_the_card_it_opened_from() {
    use slint::platform::Key;
    let mut host = default_theme();
    press_tab(&mut host, "Library");
    key(&mut host, Key::RightArrow);
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "default-game");
    let card = found.into_iter().find(|e| e.accessible_item_selected() == Some(true)).expect("a selected card");
    let (from, width) = (card.absolute_position(), card.size().width);

    // Enter opens the Game Hub: its first frame has the cover where the card is.
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
    host.ui.render().unwrap();
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail);
    let cover = host.first("hub-cover");
    let at = cover.absolute_position();
    assert!(
        (at.x - from.x).abs() < 2.0 && (at.y - from.y).abs() < 2.0,
        "starts on the card: {at:?}, the card {from:?}"
    );
    assert!((cover.size().width - width).abs() < 2.0, "the card's size");
    assert_eq!(values(&host, "hub-cover"), ["grows"]);

    // Then it grows into its own place.
    settle(&mut host);
    let place = host.first("hub-cover").absolute_position();
    assert!((place.x - from.x).abs() > 2.0 || (place.y - from.y).abs() > 2.0, "it moved: {place:?}");

    // Opened with no card, the cover is at its place from the first frame, and fades in.
    key(&mut host, Key::Escape);
    host.dispatch(Command::OpenGame("emu-a:G1".parse().unwrap()));
    host.ui.render().unwrap();
    assert_eq!(values(&host, "hub-cover"), ["fades"]);
    let at = host.first("hub-cover").absolute_position();
    assert!((at.x - place.x).abs() < 2.0 && (at.y - place.y).abs() < 2.0, "at its place: {at:?}");
}

#[test]
fn without_genres_the_library_s_grid_starts_under_the_systems() {
    let mut host = default_theme();
    press_tab(&mut host, "Library");
    assert_eq!(host.count("filter:Action"), 0, "the stub games have no genres");
    let chip = host.first("filter:All");
    let below = chip.absolute_position().y + chip.size().height;
    let gap = card_y(&host, "Alpha") - below;
    assert!(gap < 60.0, "no empty band between the systems and the cards: {gap} px");
}

/// The Library with 40 games, at 1600 × 900.
fn a_long_library() -> Composed {
    let titles: Vec<&'static str> = (0..40).map(|n| &*Box::leak(format!("Game {n:02}").into_boxed_str())).collect();
    let mut host = composed(|h| h.plugin(games("emu-a", titles)));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    settle(&mut host);
    host
}

/// Where the Library card of `title` is on screen.
fn card_y(host: &Composed, title: &str) -> f32 {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "default-game");
    let card = found.into_iter().find(|e| e.accessible_value().as_deref() == Some(title));
    card.unwrap_or_else(|| panic!("no card {title}")).absolute_position().y
}

#[test]
fn the_library_grid_scrolls_only_when_the_selected_row_leaves_the_view() {
    use slint::platform::Key;
    let mut host = a_long_library();
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    // Compact cards: two rows in view.
    press(&mut host, "density", "comfortable");
    let top = card_y(&host, "Game 00");
    let columns = (0..40).take_while(|n| card_y(&host, &format!("Game {n:02}")) == top).count();
    let title = |row: usize| format!("Game {:02}", row * columns);
    let pitch = card_y(&host, &title(1)) - top;
    assert!(columns > 1 && pitch > 0.0, "{columns} columns, rows {pitch} apart");

    // The wheel moves the grid a little: the first row is still partly in view.
    let at = slint::LogicalPosition::new(800.0, 800.0);
    host.instance().window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
        position: at,
        delta_x: 0.0,
        delta_y: -100.0,
    });
    settle(&mut host);
    let wheeled = card_y(&host, "Game 00");
    assert!(wheeled < top - 50.0, "the wheel scrolls: {top} to {wheeled}");

    // Down to the second row, which is in view: the grid stays where the wheel left it.
    let second = card_y(&host, &title(1));
    key(&mut host, Key::DownArrow);
    rest(&mut host);
    assert_eq!(host.core().view().home.library.selected, Some(columns));
    assert_eq!(card_y(&host, &title(1)), second, "a row in view doesn't scroll");

    // Down to the third row, below the view: the grid scrolls by whole rows to show it.
    key(&mut host, Key::DownArrow);
    // A card out of view isn't found: count from the third row's own place.
    let rows = (top + 2.0 * pitch - card_y(&host, &title(2))) / pitch;
    assert!((rows - rows.round()).abs() < 0.01 && rows.round() >= 1.0, "scrolled {rows} rows");
    assert!(card_y(&host, &title(2)) + pitch <= 900.0, "the third row is in view");

    // Up to the first row, above the view: it comes back.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::UpArrow);
    assert_eq!(card_y(&host, "Game 00"), top);
}

#[test]
fn the_hero_and_the_game_hub_show_the_time_played() {
    let mut host = composed(|h| h.plugin(games("emu-a", vec!["Alpha"])));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let stats = serde_json::json!({ "secs": 4 * 3600 + 6 * 60, "count": 2, "last": now });
    host.core().kernel().config().set(&["app", "playtime", "emu-a:G0"], stats).unwrap();
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    assert_eq!(values(&host, "default-played"), ["PLAYED 4 HR 6 MIN"], "over the title, as the old launcher");
    assert_eq!(values(&host, "default-sub"), ["Last played today"], "beside the tile");

    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    assert_eq!(values(&host, "played"), ["4 hr 6 min · Today"], "one card: how long, and when last");
}

#[test]
fn a_new_game_s_hero_waits_for_its_details_but_not_for_ever() {
    // These games' details never load.
    let mut host = composed(|h| h.plugin(games_without_details("emu-a", vec!["Alpha", "Beta"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G0".parse().unwrap()));
    settle(&mut host);
    assert_eq!(values(&host, "default-title"), ["Alpha"]);

    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
    host.ui.render().unwrap();
    assert_eq!(values(&host, "default-title"), ["Alpha"], "the hero waits for Beta's details");

    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(500));
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "default-title"), ["Beta"], "and comes in without them");
}

#[test]
fn the_hero_shows_the_selected_games_details_once_loaded() {
    let mut host = default_theme();
    host.dispatch(Command::Select("emu-b:G0".parse().unwrap()));
    settle(&mut host);
    assert_eq!(values(&host, "default-title"), ["Gamma"]);
    assert_eq!(values(&host, "default-description"), ["About Gamma"]);
}

#[test]
fn the_direction_option_moves_the_top_bar_to_the_right() {
    let mut host = default_theme();
    let left = host.first("default-system").absolute_position().x;
    assert!(left < 400.0, "the systems start on the left: {left}");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    let right = host.first("default-system").absolute_position().x;
    assert!(right > 800.0, "the systems move to the right: {right}");
}

/// A key, as the window gets it, with nothing carried out and no time passing.
fn tap(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.ui.render().unwrap();
}

/// The value of the element labelled `label` that shows as selected.
fn focused(host: &Composed, label: &str) -> Vec<String> {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label);
    found
        .filter(|e| e.accessible_item_selected() == Some(true))
        .map(|e| e.accessible_value().unwrap_or_default().to_string())
        .collect()
}

/// Whether an element labelled `label` shows as selected.
fn lit(host: &Composed, label: &str) -> bool {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label)
        .any(|e| e.accessible_item_selected() == Some(true))
}

/// The selections the theme asked the core for, and not yet carried out.
fn selects(host: &Composed) -> Vec<String> {
    host.requests
        .borrow()
        .iter()
        .filter_map(|r| match r {
            nexus_ui_host::UiRequest::Core(Command::Select(key)) => Some(key.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_move_in_home_s_row_shows_at_once_and_the_core_hears_once_the_moves_stop() {
    use slint::platform::Key;
    let mut host = default_theme();
    let view = host.core().view();
    let first = view.home.selected.expect("a game");
    let titles: Vec<String> = view.home.games.iter().map(|g| g.title.clone()).collect();
    assert_eq!(focused(&host, "default-tile"), [titles[first].clone()]);

    let wait = |host: &mut Composed, ms: u64| {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
        host.ui.render().unwrap();
    };
    // Two moves 60 ms apart: the row shows the new game at once, before the core hears of it.
    tap(&mut host, Key::RightArrow);
    wait(&mut host, 60);
    tap(&mut host, Key::RightArrow);
    assert_eq!(focused(&host, "default-tile"), [titles[first + 2].clone()], "the theme moved");
    // 120 ms after the first move, 60 ms after the last: each move starts the wait again.
    wait(&mut host, 60);
    assert_eq!(selects(&host), Vec::<String>::new(), "the core hasn't heard yet");
    assert_eq!(host.core().view().home.selected, Some(first));

    // Once the moves stop, the core hears once, of the last game.
    wait(&mut host, 60);
    wait(&mut host, 60);
    assert_eq!(selects(&host), [view.home.games[first + 2].key.clone()], "one select, for the last move");
    host.pump();
    assert_eq!(host.core().view().home.selected, Some(first + 2));
    // A frame goes by, as in the app, so the theme sees the core caught up.
    rest(&mut host);
    assert_eq!(focused(&host, "default-tile"), [titles[first + 2].clone()]);

    // The core's selection changes for another reason: the row follows the core.
    host.dispatch(Command::Select(view.home.games[first].key.parse().unwrap()));
    rest(&mut host);
    assert_eq!(focused(&host, "default-tile"), [titles[first].clone()], "the row follows the core");
}

#[test]
fn a_key_that_acts_tells_the_core_of_the_move_first() {
    use slint::platform::Key;
    let mut host = default_theme();
    let view = host.core().view();
    let first = view.home.selected.expect("a game");
    tap(&mut host, Key::RightArrow);
    // Enter at once opens the game on screen, not the one the core had.
    key(&mut host, Key::Return);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail);
    assert_eq!(host.core().view().game.as_deref(), Some(view.home.games[first + 1].key.as_str()));
}

#[test]
fn a_move_in_the_library_grid_shows_at_once() {
    use slint::platform::Key;
    let mut host = default_theme();
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    let view = host.core().view();
    let first = view.home.library.selected.expect("a game");
    let titles: Vec<String> = view.home.library.games.iter().map(|g| g.title.clone()).collect();
    assert_eq!(focused(&host, "default-game"), [titles[first].clone()]);

    tap(&mut host, Key::RightArrow);
    assert_eq!(focused(&host, "default-game"), [titles[first + 1].clone()], "the card is focused at once");
    assert_eq!(host.core().view().home.library.selected, Some(first), "before the core hears of it");
    rest(&mut host);
    assert_eq!(host.core().view().home.library.selected, Some(first + 1));
}

#[test]
fn the_mouse_wheel_moves_along_home_s_row() {
    let mut host = default_theme();
    let first = host.core().view().home.selected.expect("a game");
    let tile = host.first("default-tile");
    let at = slint::LogicalPosition::new(800.0, tile.absolute_position().y + tile.size().height / 2.0);
    let wheel = |host: &mut Composed, delta_y: f32| {
        host.instance().window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: at,
            delta_x: 0.0,
            delta_y,
        });
        host.pump();
        settle(host);
    };
    wheel(&mut host, 120.0);
    rest(&mut host);
    assert_eq!(host.core().view().home.selected, Some(first + 1), "one notch, one game on");
    wheel(&mut host, -120.0);
    rest(&mut host);
    assert_eq!(host.core().view().home.selected, Some(first), "and back");
}

#[test]
fn holding_enter_does_not_repeat_it() {
    let mut host = default_theme();
    let text: slint::SharedString = slint::platform::Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressRepeated { text: text.clone() });
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Home, "a repeat of Enter does nothing");

    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "a press opens the game");
}

#[test]
fn selecting_a_game_keeps_the_tiles() {
    let mut host = default_theme();
    let tiles: Vec<_> =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "default-tile").collect();
    assert_eq!(tiles.len(), 3);
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    settle(&mut host);
    assert!(tiles.iter().all(|t| t.is_valid()), "the tiles from before the selection are still shown");
}

#[test]
fn the_options_are_the_layout_direction_and_animations() {
    let host = default_theme();
    let options = &host.ui.resolved().unwrap().merged.option_values;
    let mut names: Vec<&str> = options.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["direction", "motion"]);
    let choice = |name: &str| match &options[name] {
        nexus_ui_api::OptionValue::Choice(c) => c.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(choice("direction"), "ltr");
    assert_eq!(choice("motion"), "on");
}

/// `text` without its `Look.timed(...)` calls.
fn without_timed(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while let Some(at) = rest.find("Look.timed(") {
        out.push_str(&rest[..at]);
        let mut depth = 0;
        let mut end = rest.len();
        for (i, c) in rest[at..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 1 => {
                    end = at + i + 1;
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The times in `text` that are not 0, such as "40ms" or "6s".
fn fixed_times(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let after_name = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '-' || chars[i - 1] == '_');
        if !chars[i].is_ascii_digit() || after_name {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let number: String = chars[start..i].iter().collect();
        let unit: String = chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        if (unit == "ms" || unit == "s") && number.parse::<f64>().is_ok_and(|n| n != 0.0) {
            found.push(format!("{number}{unit}"));
        }
        i += unit.len();
    }
    found
}

/// The default theme's folder, and its .slint files.
fn theme_sources() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    fn slint_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                slint_files(&path, files);
            } else if path.extension().is_some_and(|e| e == "slint") {
                files.push(path);
            }
        }
    }
    let theme = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/default/theme");
    let mut files = Vec::new();
    slint_files(&theme, &mut files);
    assert!(files.len() > 10, "the theme's files: {files:?}");
    (theme, files)
}

#[test]
fn every_font_the_theme_names_ships_with_it() {
    let (theme, files) = theme_sources();
    let shipped: Vec<String> = std::fs::read_dir(theme.join("fonts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    let mut wrong = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            let Some(at) = line.find("font-family:") else { continue };
            let parts: Vec<&str> = line[at..].split('"').collect();
            // Every other part is quoted; one after == or != is a value compared, not a font.
            for pair in parts.chunks(2).filter(|p| p.len() == 2) {
                let (before, name) = (pair[0].trim_end(), pair[1]);
                if before.ends_with("==") || before.ends_with("!=") {
                    continue;
                }
                let file_name = name.replace(' ', "").to_lowercase();
                if !shipped.iter().any(|f| f.starts_with(&file_name)) {
                    wrong.push(format!("{}:{}: {name}", file.strip_prefix(&theme).unwrap().display(), number + 1));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "fonts the theme doesn't ship fall back to whatever the system has: {wrong:?}");
}

#[test]
fn every_animation_time_goes_through_look_timed_so_animations_off_cuts_it() {
    let (theme, files) = theme_sources();
    // With Animations off the motion tokens are 0, so "Tokens.motion-normal - 20ms" is -20 ms
    // and a bare "300ms" stays 300 ms. `Look.timed` makes both 0.
    let mut wrong = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            for field in ["duration:", "delay:"] {
                let Some(at) = line.find(field) else { continue };
                let value = &line[at + field.len()..];
                let value = &value[..value.find([';', '}']).unwrap_or(value.len())];
                if !fixed_times(&without_timed(value)).is_empty() {
                    let name = file.strip_prefix(&theme).unwrap().display();
                    wrong.push(format!("{name}:{}: {}", number + 1, value.trim()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "times outside Look.timed:\n{}", wrong.join("\n"));
}

#[test]
fn the_keys_move_through_settings_and_change_the_focused_row() {
    use slint::platform::Key;
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);

    // Enter on the first row, "Rescan game folders", scans the folders again.
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let asked = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::Core(Command::Library(nexus_core::LibraryCmd::Refresh))));
    assert!(asked, "Enter on Rescan asks the core to scan");
    host.pump();

    // Down to the end, THIS PC, then up past its rows into DOWNLOADS: its last row, "Keep
    // sharing finished downloads", then up past the VPN rows to "Download only through a
    // VPN". The theme's options are in Themes now.
    for _ in 0..30 {
        key(&mut host, Key::DownArrow);
    }
    let pc = host.core().view().settings.hardware;
    let pc_rows = if pc.known { 2 + pc.graphics.len() + pc.verdicts.len() } else { 1 };
    for _ in 0..pc_rows {
        key(&mut host, Key::UpArrow);
    }
    assert_eq!(host.first("seed-downloads").accessible_value().as_deref(), Some("on"));
    key(&mut host, Key::Return);
    assert_eq!(host.first("seed-downloads").accessible_value().as_deref(), Some("off"), "Enter stops sharing");
    // Up past "Add a VPN" and the row that says there is no VPN.
    for _ in 0..3 {
        key(&mut host, Key::UpArrow);
    }
    key(&mut host, Key::Return);
    assert_eq!(host.first("vpn-only").accessible_value().as_deref(), Some("on"), "Enter turns the VPN switch on");

    // Up once more, "Quit Nexus".
    key(&mut host, Key::UpArrow);
    assert_eq!(host.count("option:direction"), 0, "no theme options in Settings");
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let quit = host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Quit));
    assert!(quit, "Enter on Quit closes the launcher");
    host.pump();

    // Up one, "Start at login", and one more, "Interface sounds".
    key(&mut host, Key::UpArrow);
    assert_eq!(host.first("start-at-login").accessible_value().as_deref(), Some("off"));
    key(&mut host, Key::UpArrow);
    assert_eq!(host.first("sounds").accessible_value().as_deref(), Some("on"));
    key(&mut host, Key::Return);
    assert_eq!(host.first("sounds").accessible_value().as_deref(), Some("off"), "Enter turns the sounds off");
}

#[test]
fn a_first_play_asks_before_the_emulator_downloads_and_enter_agrees() {
    use slint::platform::Key;
    let mut host = composed(|h| h.plugin(games_needing_an_emulator("emu-x", vec!["Astro"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    settle(&mut host);

    assert_eq!(values(&host, "install-prompt"), ["Install Emu?"]);
    let says = values(&host, "install-prompt-text").concat();
    assert!(says.contains("Astro") && says.contains("1.0") && says.contains("300 MB"), "{says}");

    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let agreed =
        host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Core(Command::ConfirmInstall)));
    assert!(agreed, "Enter installs and plays");
}

#[test]
fn an_emulator_s_download_says_where_it_is_in_words() {
    let mut host = composed(|h| h.plugin(games_needing_an_emulator("emu-x", vec!["Astro"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    // The build isn't there: the download fails, and its card stays to say so.
    let core = host.core().clone();
    assert!(host.rt.block_on(core.dispatch(Command::ConfirmInstall)).is_err());
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(values(&host, "download-state"), ["Failed"], "a word for the player, not the core's \"failed\"");
}

#[test]
fn not_now_closes_the_install_prompt() {
    use slint::platform::Key;
    let mut host = composed(|h| h.plugin(games_needing_an_emulator("emu-x", vec!["Astro"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    settle(&mut host);
    key(&mut host, Key::Escape);
    assert!(values(&host, "install-prompt").is_empty(), "Escape says not now");
    assert!(host.core().view().install_prompt.is_none());
}

#[test]
fn settings_keep_the_launcher_behind_a_closed_game() {
    let mut host = composed(|h| h);
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(host.first("return-on-exit").accessible_value().as_deref(), Some("on"), "on by default");

    host.first("return-on-exit").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);

    assert_eq!(host.first("return-on-exit").accessible_value().as_deref(), Some("off"));
    assert!(!host.core().prefs().return_on_exit);
}

#[test]
fn settings_turn_automatic_updates_off() {
    // The row is in the UPDATES section, which lists the emulators.
    let mut host = composed(|h| h.plugin(games_needing_an_emulator("emu-x", vec!["Astro"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(host.first("auto-update").accessible_value().as_deref(), Some("on"));
    host.first("auto-update").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.first("auto-update").accessible_value().as_deref(), Some("off"));
    assert!(!host.core().view().settings.auto_update);
}

#[test]
fn settings_change_the_size_of_everything_and_the_rows_grow_with_it() {
    use slint::platform::Key;
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(values(&host, "scale"), ["1.0"]);
    let before = host.first("fullscreen").size().height;

    press(&mut host, "scale", "1.0");
    assert_eq!(host.core().view().settings.scale, 1.1);
    assert_eq!(values(&host, "scale"), ["1.1"]);
    assert!(host.first("fullscreen").size().height > before, "everything draws bigger at once");

    // Down past the game folders, Rescan, UPDATES' two rows and Full screen to the size:
    // right is bigger, left smaller.
    let folders = host.core().view().settings.folders.len();
    for _ in 0..folders + 4 {
        key(&mut host, Key::DownArrow);
    }
    key(&mut host, Key::RightArrow);
    assert_eq!(values(&host, "scale"), ["1.2"]);
    key(&mut host, Key::LeftArrow);
    key(&mut host, Key::LeftArrow);
    assert_eq!(values(&host, "scale"), ["1.0"]);
    // The largest goes round to the smallest.
    host.dispatch(Command::Pref(Pref::Scale, 1.4.into()));
    settle(&mut host);
    press(&mut host, "scale", "1.4");
    assert_eq!(values(&host, "scale"), ["0.8"]);
}

#[test]
fn nexus_scale_wins_over_the_setting_for_the_run() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    let before = host.first("fullscreen").size().height;
    host.ui.set_size(1.3);
    host.ui.render().unwrap();
    settle(&mut host);
    assert!(host.first("fullscreen").size().height > before);
    assert_eq!(values(&host, "scale"), ["1.0"], "the setting stays");
    let hint = host.first("scale").accessible_description().unwrap_or_default();
    assert_eq!(hint.as_str(), "NEXUS_SCALE sets it for this run");
}

#[test]
fn settings_start_the_launcher_at_login_and_say_why_the_system_refused() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    press(&mut host, "start-at-login", "off");
    assert_eq!(values(&host, "start-at-login"), ["on"]);
    assert_eq!(host.test.login_session.logins(), [true]);

    // The app logs a command that fails; the row says why.
    host.test.login_session.refuse_login("reg.exe: access is denied");
    host.first("start-at-login").invoke_accessible_default_action();
    let asked: Vec<_> = host.requests.borrow_mut().drain(..).collect();
    for request in asked {
        if let nexus_ui_host::UiRequest::Core(command) = request {
            assert!(host.rt.block_on(host.test.core.dispatch(command)).is_err());
        }
    }
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "start-at-login"), ["on"], "it stays as it was");
    let why = host.first("start-at-login").accessible_description().unwrap_or_default();
    assert_eq!(why.as_str(), "reg.exe: access is denied");
}

#[test]
fn settings_hide_start_at_login_for_a_build_that_isn_t_installed() {
    let mut host = composed(|h| {
        h.login_session().no_login();
        h
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(host.count("start-at-login"), 0);
}

/// The Emulators list, then KytyPS5's sheet.
fn kyty_page() -> Composed {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    settle(&mut host);
    host.first("plugin:emu-kyty").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
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
    settle(&mut host);
    assert_eq!(host.first("setting:own_build").accessible_value().as_deref(), Some("true"));
}

#[test]
fn a_plugin_turned_off_on_its_page_shows_only_the_switch() {
    let mut host = kyty_page();
    host.first("plugin-enabled").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.first("plugin-enabled").accessible_value().as_deref(), Some("off"));
    assert_eq!(host.count("setting:fullscreen"), 0, "its settings are hidden");
    assert_eq!(host.count("build"), 0);
}

#[test]
fn settings_have_no_emulator_settings() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(host.count("setting:fullscreen"), 0, "they are on the plugin's page");
    assert_eq!(host.count("folder:kyty-build"), 0, "and so are its own folders");
    assert_eq!(host.count("folder:games"), 1, "the game folders stay");
    assert_eq!(host.count("folder:packages"), 1, "and the packages folder, once for every emulator");
}

#[test]
fn the_emulators_list_names_each_console_and_its_program_s_build() {
    let mut host = composed(|h| {
        h.plugin(console_emulator("emu-k", "PS5", "KytyPS5", "KytyPS5-2026-10-21-e2dd48a", vec![]))
            .plugin(console_emulator("emu-s", "PS4", "shadPS4", "v.0.20.0", vec![]))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    settle(&mut host);
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
    settle(&mut host);
    let hint = host.first("plugin:emu-pc").accessible_description().unwrap_or_default().to_string();
    assert_eq!(hint, "No install needed", "PC games need no build");
}

#[test]
fn an_emulator_with_no_builds_has_no_build_row() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin("emu-pc".parse().unwrap())));
    settle(&mut host);
    assert!(host.core().view().plugin_page.is_some(), "PC's page");
    assert_eq!(host.count("build"), 0, "PC games need no build, so there is nothing to install");
    assert_eq!(host.count("uninstall"), 0);
}

#[test]
fn the_game_catalog_page_shows_its_sources_its_refresh_and_refresh_now() {
    let mut host = composed(|h| {
        h.plugin(games("emu-kyty", vec!["Alpha"]).console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // Sources with one PS5 forum, so the test doesn't depend on the project's own file, and
    // its feed and forum list.
    let sources = serde_json::json!({ "ps5": { "platform": "PS5",
        "forums": [{ "key": "ps5", "url": "https://xyz.net/forum/viewforum.php?f=546", "tag": "PS5" }] } });
    host.test.http.serve("https://xyz.dev/catalog-sources.json", sources.to_string());
    let feed = include_str!("../../../plugins/services/release-catalog/fixtures/forum-546.atom");
    host.test.http.serve(&release_catalog::refresh::feed_url("546"), feed);
    let list = include_str!("../../../plugins/services/release-catalog/fixtures/pvc-546.json");
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
    settle(&mut host);
    // The blocks: the theme's section cards carry no label a test can find, so the blocks are
    // checked in the Vm, and their place on the page by its label.
    assert_eq!(host.count("plugin-blocks"), 1, "the page draws the service's blocks");
    let labels: Vec<String> = match vm_field(&host, "plugin-page", "sections") {
        slint_interpreter::Value::Model(m) => slint::Model::iter(&m)
            .filter_map(|v| match v {
                slint_interpreter::Value::Struct(s) => match s.get_field("label") {
                    Some(slint_interpreter::Value::String(l)) => Some(l.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        other => panic!("sections is {other:?}"),
    };
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
    settle(&mut host);
    wait_for(&host, before + 1);
    // Once for the action. The core's read of the catalogs after it finds the copy fresh.
    host.advance(std::time::Duration::from_millis(100));
    assert_eq!(asked(&host), before + 1, "Refresh now asked the forums");
}

#[test]
fn a_section_card_grows_with_values_that_wrap() {
    struct Facts;
    #[nexus_api::async_trait]
    impl nexus_api::Slots for Facts {
        async fn items(&self, request: &nexus_api::SlotRequest) -> nexus_api::PluginResult<Vec<nexus_api::SlotItem>> {
            if request.slot != nexus_api::SlotId::EmulatorPanel {
                return Ok(vec![]);
            }
            let row = |label: &str, value: &str| nexus_api::SectionRow { label: label.into(), value: value.into() };
            let long = "Refreshed Oct 23, 2026 at 06:06 UTC: 16 new, 0 removed, and a few words more to wrap";
            Ok(vec![nexus_api::SlotItem {
                id: "refresh".into(),
                order: Default::default(),
                content: nexus_api::SlotContent::Section {
                    title: "Catalog refresh".into(),
                    rows: vec![row("PC", long), row("PS4", long), row("PS5", long)],
                },
            }])
        }
        async fn run_action(&self, _: &str, _: Option<&str>, _: &serde_json::Value) -> nexus_api::PluginResult<()> {
            Ok(())
        }
    }
    let facts = nexus_testkit::StubPlugin::from_toml(
        "id = \"facts\"\nkind = \"service\"\nname = \"Facts\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         capabilities = [\"slots\"]",
    )
    .provides(|_| Ok(nexus_api::Provided { slots: Some(std::sync::Arc::new(Facts)), ..Default::default() }));
    let mut host = composed(|h| h.plugin(facts));
    host.ui.start().unwrap();
    host.resize(700.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin(id("facts"))));
    settle(&mut host);

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
    settle(&mut host);
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
    settle(&mut host);
    assert_eq!(host.count("plugin:emu-a"), 0, "Themes lists themes only");
    host.first("plugin:default").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);

    assert_eq!(host.first("theme-current").accessible_value().as_deref(), Some("on"));
    assert_eq!(host.first("option:direction").accessible_value().as_deref(), Some("ltr"));
    host.first("option:direction").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.first("option:direction").accessible_value().as_deref(), Some("rtl"));
}

#[test]
fn emulators_lists_emulators_only() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Emulators));
    settle(&mut host);
    assert_eq!(host.count("plugin:emu-a"), 1);
    assert_eq!(host.count("plugin:default"), 0, "themes are in Themes");
}

#[test]
fn the_window_is_full_screen_by_default_and_settings_turn_it_off() {
    let mut host = default_theme();
    assert!(host.instance().window().is_fullscreen(), "full screen by default, as in the old launcher");
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    assert_eq!(host.first("fullscreen").accessible_value().as_deref(), Some("on"));
    host.first("fullscreen").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(!host.instance().window().is_fullscreen());
    assert_eq!(host.first("fullscreen").accessible_value().as_deref(), Some("off"));
}

#[test]
fn the_catalog_is_the_library_s_view_and_loads_a_page_at_a_time() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(games("emu-kyty", vec!["Alpha"]).console("PS5"))
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

/// A store that gives every game a cover at `https://art.example/<title id>.png`.
fn covers_store() -> nexus_testkit::StubPlugin {
    struct Covers;
    #[nexus_api::async_trait]
    impl nexus_api::Metadata for Covers {
        async fn describe(
            &self,
            game: &nexus_api::GameRef,
        ) -> nexus_api::PluginResult<Option<nexus_api::GameMetadata>> {
            Ok(Some(nexus_api::GameMetadata {
                icon: Some(nexus_api::ImageSource::Url(format!("https://art.example/{}.png", game.title_id))),
                ..Default::default()
            }))
        }
    }
    nexus_testkit::StubPlugin::from_toml(
        "id = \"store\"\nkind = \"service\"\nname = \"store\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         capabilities = [\"metadata\"]\n[permissions]\nnetwork = [\"art.example\"]",
    )
    .provides(|_| Ok(nexus_api::Provided { metadata: Some(std::sync::Arc::new(Covers)), ..Default::default() }))
}

/// The default theme on the Catalog, with three games a catalog offers: Astro, Bolt, Comet.
fn three_on_the_catalog() -> Composed {
    let mut host = composed(|h| {
        h.plugin(games("emu-kyty", vec![]).console("PS5"))
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
        h.plugin(games("emu-kyty", vec![]).console("PS5"))
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
        h.plugin(games("emu-kyty", vec![]).console("PS5"))
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
        h.plugin(games("emu-kyty", vec![]).console("PS5"))
            .plugin(covers_store())
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

/// An emulator with no games of its own that installs from the packages folder.
fn installing_emulator() -> nexus_testkit::StubPlugin {
    offering_emulator("")
}

const ASTRO_HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

/// A catalog that offers one release, Astro, open on its Game Hub.
fn astro_on_its_game_hub(home: &std::path::Path) -> Composed {
    astro_with(home, installing_emulator())
}

/// [`astro_on_its_game_hub`] with `emulator` as KytyPS5.
fn astro_with(home: &std::path::Path, emulator: nexus_testkit::StubPlugin) -> Composed {
    let hash = ASTRO_HASH;
    let mut host = composed(|h| {
        h.home(home).plugin(emulator).built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{hash}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }] });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.test.torrents.has(hash, "Astro", &[("Astro/astro.pkg", b"package")]);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
    settle(&mut host);
    host
}

/// Waits until the first download's state is `state`, then carries out what the theme asked
/// for when it saw it (the download sheet opens Downloads once the download is there).
fn until_torrent(host: &mut Composed, state: &str) {
    host.until(Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(|t| t.state == state));
    settle(host);
    host.pump();
    settle(host);
    assert_eq!(host.core().view().downloads.torrents.first().map(|t| t.state.clone()).as_deref(), Some(state));
}

#[test]
fn a_release_downloads_only_after_the_player_agrees_and_starts_it_in_downloads() {
    let hash = ASTRO_HASH;
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    assert_eq!(values(&host, "release-download"), [hash]);

    // The player is asked first; Not now asks nothing of the swarm.
    press(&mut host, "release-download", hash);
    assert_eq!(host.count("download-agree"), 1);
    host.first("download-not-now").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("download-agree"), 0);
    assert!(host.core().view().downloads.torrents.is_empty());

    press(&mut host, "release-download", hash);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    host.until(Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(|t| t.state == "ready"));
    settle(&mut host);
    // The sheet saw the download there, and asked for Downloads.
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("download-agree"), 0, "the sheet closed once the download was there");
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Downloads);
    assert_eq!(values(&host, "torrent"), ["Astro ready"]);
    assert!(host.test.torrents.started().is_empty(), "nothing downloads before Download in Downloads");

    press(&mut host, "torrent-action:start", hash);
    assert_eq!(host.test.torrents.started().len(), 1);
}

#[test]
fn the_download_sheet_shows_the_folder_and_the_player_types_another() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    let paths = vec![home.path().join("Downloads"), home.path().join("Other")];
    host.dispatch(Command::SetFolders { name: "packages".into(), paths });
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(values(&host, "download-folder"), [native("~/Downloads/PS5")]);

    // Up to the folder row; Enter opens it for typing, Enter again keeps the text.
    use slint::platform::Key;
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::Return);
    let folder = home.path().join("Other/Astro");
    for c in folder.to_str().unwrap().chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "download-folder"), [folder.to_str().unwrap()]);

    // The keys are back on Download.
    key(&mut host, Key::Return);
    until_torrent(&mut host, "ready");
    let shown = host.core().view().downloads.torrents[0].folder.clone();
    assert_eq!(shown, format!("~/Other/Astro/torrent-{ASTRO_HASH}"));
}

#[test]
fn the_keys_reach_each_download_s_buttons() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");
    use slint::platform::Key;

    // The download comes first; Up stays on it. Its first button is Download.
    assert_eq!(host.count("seed-downloads"), 0, "sharing is in Settings");
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::Return);
    assert_eq!(host.test.torrents.started().len(), 1, "Enter downloads");
    host.test.torrents.progress(ASTRO_HASH, 4);
    until_torrent(&mut host, "downloading");

    // Its buttons are Pause and Cancel now; Right moves to Cancel.
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    until_torrent(&mut host, "cancelled");
    assert_eq!(host.core().view().downloads.torrents[0].actions, ["start", "remove"]);
    // The buttons follow a list of the same length.
    assert_eq!((host.count("torrent-action:start"), host.count("torrent-action:remove")), (1, 1));
    assert_eq!(host.count("torrent-action:pause"), 0);

    // The selection stays on the card's second button, now Remove: Enter asks first, and Enter
    // again on Remove from Downloads removes it.
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "remove-title"), ["Remove Astro?"]);
    key(&mut host, Key::Return);
    host.until(Area::Torrents, |core| core.view().downloads.torrents.is_empty());
    assert!(host.core().view().downloads.torrents.is_empty(), "Enter removes it");
}

#[test]
fn a_click_on_a_download_s_button_moves_the_highlight_to_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");
    press(&mut host, "torrent-action:start", ASTRO_HASH);
    host.test.torrents.progress(ASTRO_HASH, 4);
    until_torrent(&mut host, "downloading");
    assert!(lit(&host, "torrent-action:pause") && !lit(&host, "torrent-open-folder"), "the first button");

    press(&mut host, "torrent-open-folder", ASTRO_HASH);
    assert!(lit(&host, "torrent-open-folder"), "one highlight, on what was clicked");
    assert!(!lit(&host, "torrent-action:pause"));
}

/// A mouse click in the middle of the first element labelled `label`: press, then release, as
/// a pointer does, and not the element's default action.
fn mouse_click(host: &mut Composed, label: &str) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let target = host.first(label);
    let (at, size) = (target.absolute_position(), target.size());
    let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    let window = host.instance().window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    host.pump();
    settle(host);
}

#[test]
fn one_mouse_click_presses_a_button() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");

    // The boot screen is over everything until the library and Home's art are in.
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    settle(&mut host);
    assert!(values(&host, "boot").is_empty());

    mouse_click(&mut host, "torrent-action:start");
    assert_eq!(host.test.torrents.started().len(), 1, "the first click downloads");
}

#[test]
fn left_and_right_in_downloads_play_the_move_sound() {
    use slint::platform::Key;
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");
    let moves = |host: &Composed| {
        let requests = host.requests.borrow();
        requests.iter().filter(|r| matches!(r, nexus_ui_host::UiRequest::PlaySound(s) if s == "move")).count()
    };
    let press_key = |host: &Composed, k: Key| {
        let text: slint::SharedString = k.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    };
    host.requests.borrow_mut().clear();
    press_key(&host, Key::RightArrow);
    assert_eq!(moves(&host), 1, "on to the card's next button");
    press_key(&host, Key::LeftArrow);
    assert_eq!(moves(&host), 2, "back to its first button");
    press_key(&host, Key::LeftArrow);
    assert_eq!(moves(&host), 2, "no sound where the keys can't go");
}

#[test]
fn a_download_s_card_opens_its_folder_once_it_starts() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");
    assert_eq!(host.count("torrent-open-folder"), 0, "no folder before it starts");

    press(&mut host, "torrent-action:start", ASTRO_HASH);
    use slint::platform::Key;
    // The keys reach it after the card's actions, Pause and Cancel.
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::RightArrow);
    host.requests.borrow_mut().clear();
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    let folder = home.path().join(format!("Downloads/PS5/torrent-{ASTRO_HASH}"));
    let opened = |host: &Composed| {
        host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::OpenPath(p) if *p == folder))
    };
    assert!(opened(&host), "Enter on Open folder: {:?}", host.requests.borrow());
    assert_eq!(host.core().view().downloads.torrents[0].state, "checking", "nothing else happened");

    host.requests.borrow_mut().clear();
    host.first("torrent-open-folder").invoke_accessible_default_action();
    assert!(opened(&host), "a click opens it too");
}

#[test]
fn a_download_s_card_shows_its_archives_unpacking_and_offers_to_remove_them() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    host.test.torrents.has(ASTRO_HASH, "Astro", &[("Astro/game.rar", b"archive")]);
    host.test.archives.holds("game.rar", &[("game.pkg", b"package")]);
    let held = host.test.archives.hold();
    press(&mut host, "release-download", ASTRO_HASH);
    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    until_torrent(&mut host, "ready");
    press(&mut host, "torrent-action:start", ASTRO_HASH);
    host.test.torrents.progress(ASTRO_HASH, 4);
    until_torrent(&mut host, "downloading");
    host.test.torrents.complete(ASTRO_HASH);
    until_torrent(&mut host, "complete");
    host.until(Area::Torrents, |core| core.view().downloads.torrents[0].unpack == "unpacking");
    settle(&mut host);

    assert_eq!(values(&host, "torrent"), ["Astro complete"]);
    let buttons = |host: &Composed| -> Vec<String> {
        ["start", "pause", "cancel", "remove", "stop-seeding", "install", "play", "remove-unpacked"]
            .into_iter()
            .filter(|a| host.count(&format!("torrent-action:{a}")) > 0)
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(buttons(&host), ["cancel", "stop-seeding"], "while it unpacks: Stop unpacking");

    held.release();
    host.until(Area::Torrents, |core| core.view().downloads.torrents[0].unpack == "unpacked");
    settle(&mut host);
    assert_eq!(buttons(&host), ["stop-seeding", "install", "remove-unpacked"]);
    press(&mut host, "torrent-action:remove-unpacked", ASTRO_HASH);
    assert!(!home.path().join(format!("Downloads/PS5/torrent-{ASTRO_HASH}/unpacked")).exists());
}

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

/// A catalog with these topics, open on its first offer's Game Hub.
fn first_offer_with(home: &std::path::Path, topics: serde_json::Value) -> Composed {
    first_offer_on(home, installing_emulator(), |h| h, topics)
}

/// [`first_offer_with`], for `emulator`, on a host that `build` changes first.
fn first_offer_on(
    home: &std::path::Path,
    emulator: nexus_testkit::StubPlugin,
    build: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost,
    topics: serde_json::Value,
) -> Composed {
    let mut host = composed(|h| {
        build(h)
            .home(home)
            .plugin(emulator)
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let count = topics.as_array().unwrap().len();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": count,
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
    let offer = host.core().view().catalog.games.games.first().cloned().expect("the offer");
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    settle(&mut host);
    host
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
        offering_emulator("vulkan = \"1.3\""),
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

/// Settings, down to the DOWNLOADS section, on a host whose VPNs `vpns` sets.
fn downloads_settings(vpns: impl FnOnce(&nexus_testkit::FakeNetwork)) -> Composed {
    let mut host = composed(|h| {
        vpns(h.network());
        h
    });
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "seed-downloads");
    host
}

fn description(host: &Composed, label: &str) -> String {
    host.first(label).accessible_description().unwrap_or_default().to_string()
}

#[test]
fn settings_list_the_vpns_and_say_where_downloads_go() {
    let mut host = downloads_settings(|n| {
        n.up("wg0");
        n.down("work");
    });
    assert_eq!(values(&host, "vpn"), ["wg0", "work"]);
    assert_eq!(description(&host, "vpn-only"), "Hides your IP address from other peers · uses your own VPN");

    press(&mut host, "vpn-only", "off");
    assert_eq!(values(&host, "vpn-only"), ["on"]);
    assert_eq!(description(&host, "vpn-only"), "Protected · downloads only use wg0");
    assert_eq!(description(&host, "vpn"), "WireGuard · connected · downloads use it");

    // Enter on a VPN the system can switch connects or disconnects it.
    press(&mut host, "vpn", "work");
    press(&mut host, "vpn", "wg0");
    assert_eq!(host.test.network.calls(), ["connect work", "disconnect wg0"]);
    assert_eq!(description(&host, "vpn-only"), "Protected · downloads only use work");
}

#[test]
fn settings_add_a_vpn_from_the_file_the_player_types() {
    let mut host = downloads_settings(|_| {});
    assert_eq!(host.count("vpn-none"), 1);
    host.first("vpn-import").invoke_accessible_default_action();
    settle(&mut host);
    let file = full_path("/vpn/se-1.conf");
    for c in file.chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(&mut host, slint::platform::Key::Return);
    let calls = host.test.network.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls, [format!("import {file}")]);
    assert_eq!(values(&host, "vpn"), ["se-1"]);
}

#[test]
fn where_the_system_can_t_tell_a_vpn_the_player_picks_the_network() {
    let mut host = downloads_settings(|n| {
        n.picks_only();
        n.up("en0");
        n.up("utun4");
    });
    press(&mut host, "vpn-only", "off");
    assert_eq!(description(&host, "vpn-only"), "Downloads wait until you pick the network your VPN uses");
    press(&mut host, "vpn", "utun4");
    assert_eq!(host.core().view().settings.vpn.chosen, "utun4");
    assert_eq!(description(&host, "vpn-only"), "Protected · downloads only use utun4");
    assert!(host.test.network.calls().is_empty(), "nothing is connected for the player");
}

#[test]
fn where_downloads_can_t_be_bound_the_vpn_switch_says_why_and_stays_off() {
    let mut host = downloads_settings(|n| n.unavailable("Not available on Windows"));
    assert_eq!(description(&host, "vpn-only"), "Not available on Windows");
    assert_eq!((host.count("vpn"), host.count("vpn-none"), host.count("vpn-import")), (0, 0, 0));
    press(&mut host, "vpn-only", "off");
    assert_eq!(values(&host, "vpn-only"), ["off"]);
}

#[test]
fn with_no_vpn_the_download_sheet_and_the_card_say_the_download_waits() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(true)));
    press(&mut host, "release-download", ASTRO_HASH);
    let privacy = values(&host, "download-privacy").concat();
    assert!(privacy.starts_with("Your VPN isn't connected: the download waits"), "{privacy}");

    host.first("download-agree").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    // The sheet saw the download there, and asked for Downloads.
    host.pump();
    settle(&mut host);
    assert_eq!(values(&host, "torrent-state"), ["Paused · waiting for your VPN"]);
    host.test.network.up("wg0");
    until_torrent(&mut host, "ready");
    assert_eq!(values(&host, "torrent-state"), ["Ready: look at the files, then download"]);
}

#[test]
fn with_the_vpn_connected_the_download_sheet_names_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    host.test.network.up("wg0");
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(true)));
    host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
    press(&mut host, "release-download", ASTRO_HASH);
    let privacy = values(&host, "download-privacy").concat();
    assert!(privacy.starts_with("It goes only through your VPN (wg0)"), "{privacy}");
}

/// The first bytes of a Linux program.
fn elf() -> Vec<u8> {
    let mut h = vec![0u8; 0x200];
    h[..4].copy_from_slice(b"\x7fELF");
    h[4] = 2;
    h[5] = 1;
    h[16..18].copy_from_slice(&2u16.to_le_bytes());
    h[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
    h
}

/// A PC game the catalog offers, downloaded and finished, on Downloads.
fn penguins_downloaded(home: &std::path::Path) -> Composed {
    let mut host = composed(|h| {
        h.home(home)
            .platform(Some(nexus_api::Platform::Linux))
            .built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "Penguins (v1.2) [amd64] [Native]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"), "game_info": {} }] });
    host.test.http.serve("https://xyz.dev/c/pc.json", catalog.to_string());
    host.test.torrents.has(ASTRO_HASH, "Penguins", &[("Penguins/penguins.x86_64", &elf())]);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    let offer = host.core().view().catalog.games.games[0].key.clone();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer.parse().unwrap(),
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    until_torrent(&mut host, "ready");
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::Start,
    }));
    host.test.torrents.complete(ASTRO_HASH);
    until_torrent(&mut host, "complete");
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    host
}

/// Presses `key` and carries out what the theme asked for, as the app would, where the core
/// may refuse: gives the refusals.
fn key_refused(host: &mut Composed, key: slint::platform::Key) -> Vec<String> {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    let requests: Vec<nexus_ui_host::UiRequest> = host.requests.borrow_mut().drain(..).collect();
    let mut refused = Vec::new();
    for request in requests {
        if let nexus_ui_host::UiRequest::Core(command) = request
            && let Err(e) = host.rt.block_on(host.test.core.dispatch(command))
        {
            refused.push(e.to_string());
        }
    }
    host.ui.render().unwrap();
    settle(host);
    refused
}

/// Waits until the first download's install is `state`.
fn until_install(host: &mut Composed, state: &str) {
    host.until(Area::Installs, |core| core.view().downloads.torrents.first().is_some_and(|t| t.install == state));
    settle(host);
    assert_eq!(host.core().view().downloads.torrents[0].install, state);
}

#[test]
fn install_asks_first_with_the_game_folder_and_what_the_emulator_installs_from() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());

    // Install asks first; Not now installs nothing.
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert_eq!(values(&host, "install-title"), ["Install game"]);
    assert_eq!(values(&host, "install-folder"), [native("~/Games/PC")]);
    let note = values(&host, "install-note");
    assert!(note[0].starts_with("Installs a PC game from its folder"), "{note:?}");
    host.first("install-not-now").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("install-agree"), 0);
    assert_eq!(host.core().view().downloads.torrents[0].install, "");

    // Install on the sheet installs, and the sheet closes once the install runs.
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(&mut host, "done");
    assert_eq!(host.count("install-agree"), 0, "the sheet closed");
    assert!(home.path().join("Games/PC/Penguins").is_dir());
}

#[test]
fn a_download_s_install_shows_on_its_card_only() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(&mut host, "done");
    assert_eq!(host.count("torrent"), 1, "the download's card");
    assert!(host.core().view().downloads.jobs.is_empty(), "no second card for the same install");
    assert!(values(&host, "download-state").is_empty());
}

#[test]
fn delete_files_removes_a_download_and_its_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));
    assert!(folder.is_dir(), "the downloaded files");
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::StopSeeding,
    }));

    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::DeleteFiles,
    }));
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(!folder.exists(), "its folder is deleted");
    assert!(home.path().join("Downloads/PC").is_dir(), "the downloads folder stays");
}

/// [`penguins_downloaded`], no longer sharing: its card has Remove.
fn penguins_to_remove(home: &std::path::Path) -> Composed {
    let mut host = penguins_downloaded(home);
    press(&mut host, "torrent-action:stop-seeding", ASTRO_HASH);
    assert_eq!(host.count("torrent-action:remove"), 1);
    host
}

#[test]
fn remove_asks_whether_to_keep_the_downloaded_files() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_to_remove(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));

    // Remove asks first; Not now (or Escape) changes nothing.
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    assert_eq!(values(&host, "remove-title"), ["Remove Penguins?"]);
    host.first("remove-not-now").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("remove-title"), 0);
    assert_eq!(host.core().view().downloads.torrents.len(), 1);
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    key(&mut host, slint::platform::Key::Escape);
    assert_eq!(host.count("remove-title"), 0, "Escape closes it");

    // Remove from Downloads forgets the download; its files stay.
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    host.first("remove-keep").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(folder.is_dir(), "the files stay");
    assert_eq!(host.count("remove-title"), 0);
}

#[test]
fn remove_and_delete_files_deletes_the_download_s_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_to_remove(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    host.first("remove-delete").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(!folder.exists(), "its folder is deleted");
}

/// [`penguins_downloaded`], installed: its key in the Library.
fn penguins_installed(host: &mut Composed) -> String {
    press(host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(host, "done");
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    let library = host.core().view().home.games;
    library.iter().find(|g| g.title.contains("Penguins")).expect("the installed game").key.clone()
}

#[test]
fn an_installed_game_s_card_plays_it_its_menu_uninstalls_it_and_then_its_card_downloads_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    // Installed: the card's main action is Play, as on every installed game's card.
    let play = values(&host, "card-play");
    assert_eq!(play.len(), 1, "the installed game's card: {play:?}");
    assert!(play[0].starts_with("Play "), "{play:?}");
    assert!(values(&host, "card-uninstall").is_empty(), "Uninstall is never the card's button");

    // Uninstall is in the game's menu, on its Game Hub, and asks first.
    let card = host.core().view().catalog.games.games.into_iter().find(|g| g.title.contains("Penguins")).unwrap();
    host.dispatch(Command::OpenGame(card.key.parse().unwrap()));
    settle(&mut host);
    press(&mut host, "game-options", "Options");
    press(&mut host, "game-menu", "Uninstall");
    let asked = values(&host, "uninstall-title");
    assert_eq!(asked.len(), 1, "it asks first");
    assert!(folder.is_dir(), "nothing deleted yet");
    press(&mut host, "uninstall-choice", "Uninstall");
    assert!(!folder.exists(), "its folder is deleted");
    settle(&mut host);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);

    assert!(values(&host, "card-play").is_empty());
    let download = values(&host, "card-get");
    assert!(!download.is_empty() && download.iter().all(|d| d.starts_with("Download ")), "{download:?}");
    press(&mut host, "card-get", &download[0]);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "its Game Hub, to pick a release");
}

#[test]
fn uninstall_deletes_an_installed_game_s_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let game = penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    assert!(folder.is_dir());

    host.dispatch(Command::UninstallGame(game.parse().unwrap()));
    assert!(!folder.exists(), "its folder is deleted");
    assert!(home.path().join("Games/PC").is_dir(), "the game folder stays");
    assert!(host.core().view().home.games.iter().all(|g| g.key != game), "it left the Library");
}

#[test]
fn the_game_s_menu_uninstalls_an_installed_game_after_asking() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let game = penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    host.dispatch(Command::OpenGame(game.parse().unwrap()));
    settle(&mut host);

    // Options has Uninstall; it asks first, and Not now changes nothing.
    press(&mut host, "game-options", "Options");
    assert!(values(&host, "game-menu").contains(&"Uninstall".to_owned()));
    press(&mut host, "game-menu", "Uninstall");
    let asks = values(&host, "uninstall-title");
    assert!(asks.len() == 1 && asks[0].starts_with("Uninstall "), "{asks:?}");
    assert_eq!(focused(&host, "uninstall-choice"), ["Not now"], "the safe answer has the keys");
    press(&mut host, "uninstall-choice", "Not now");
    assert!(values(&host, "game-menu").is_empty(), "the menu closes");
    assert!(folder.is_dir());

    // Uninstall deletes it.
    press(&mut host, "game-options", "Options");
    press(&mut host, "game-menu", "Uninstall");
    press(&mut host, "uninstall-choice", "Uninstall");
    assert!(!folder.exists(), "its folder is deleted");
    assert_eq!(
        host.core().view().screen,
        nexus_ui_api::ScreenId::Downloads,
        "its Game Hub closed: back where it came from"
    );
}

#[test]
fn the_install_sheet_takes_a_typed_folder_and_keeps_its_error_open() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);

    // Up to the folder row; Enter opens it for typing, Enter again keeps the text.
    use slint::platform::Key;
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::Return);
    let folder = home.path().join("Elsewhere");
    for c in folder.to_str().unwrap().chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(&mut host, Key::Return);
    assert_eq!(values(&host, "install-folder"), [folder.to_str().unwrap()]);

    // The keys are back on Install: the core refuses the folder, and the sheet says why.
    let refused = key_refused(&mut host, Key::Return);
    assert!(refused.iter().any(|e| e.contains("isn't in a game folder")), "{refused:?}");
    assert_eq!(host.count("install-agree"), 1, "the sheet stays open");
    let error = values(&host, "install-error");
    assert!(error.first().is_some_and(|e| e.contains("Pick a folder inside a game folder")), "{error:?}");
    assert_eq!(host.core().view().downloads.torrents[0].install, "", "nothing installs");

    // Escape closes it.
    key(&mut host, Key::Escape);
    assert_eq!(host.count("install-agree"), 0);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Downloads, "still on Downloads");
}

#[test]
fn the_download_sheet_stays_open_and_says_why_the_core_refused() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);

    // Up to the folder row; a folder outside every packages folder.
    use slint::platform::Key;
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::Return);
    let folder = home.path().join("Elsewhere");
    for c in folder.to_str().unwrap().chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(&mut host, Key::Return);
    let refused = key_refused(&mut host, Key::Return);
    assert!(refused.iter().any(|e| e.contains("isn't in a packages folder")), "{refused:?}");
    assert_eq!(host.count("download-agree"), 1, "the sheet stays open");
    let error = values(&host, "download-error");
    assert!(error.first().is_some_and(|e| e.contains("Pick a folder inside a packages folder")), "{error:?}");
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "still on the Game Hub");

    // The sheet asks again from the start: the old reason is gone.
    key(&mut host, Key::Escape);
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(host.count("download-error"), 0);
}

#[test]
fn the_download_sheet_sets_up_a_download_and_says_what_it_shares_and_shows() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    assert_eq!(values(&host, "download-title"), ["Set up a download"]);
    assert_eq!(values(&host, "download-agree"), ["Look up metadata"]);
    assert_eq!(
        values(&host, "download-consent"),
        ["Only download content you are allowed to get and share. While it downloads, it also uploads pieces to \
          other peers, at most 128 KiB/s. A finished download goes on sharing while Nexus is open: turn this off \
          in Settings (Keep sharing finished downloads), or stop it on its card in Downloads."]
    );
    let privacy = values(&host, "download-privacy").concat();
    assert!(
        privacy
            .starts_with("Looking up this torrent contacts its trackers, the DHT and peers: they see your IP address"),
        "{privacy}"
    );
    assert_eq!(
        values(&host, "download-steps"),
        ["First, Nexus looks up the torrent's metadata only: its files and size. Then you look at them in \
          Downloads and press Download. Installation is a separate step."]
    );
}

#[test]
fn downloads_sums_up_its_transfers_and_says_how_they_behave() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    assert_eq!(host.count("downloads-summary"), 0, "nothing to sum up");
    assert_eq!(host.count("downloads-footer"), 0);

    host.test.network.up("wg0");
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(true)));
    let offer = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer,
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    until_torrent(&mut host, "ready");
    assert_eq!(
        values(&host, "downloads-summary"),
        ["Protected by your VPN · wg0 · 0 downloading · 0 sharing · 1 in all"]
    );
    assert_eq!(
        values(&host, "downloads-note"),
        ["Downloads go on while Nexus is open, also when it is minimized, and stop when you close it. At the next \
          start, unfinished ones come back paused. Finished downloads go on sharing while Nexus is open: Stop \
          sharing stops one until the next start, and Keep sharing finished downloads, in Settings, stops them all."]
    );
    assert_eq!(
        values(&host, "downloads-footer"),
        ["Cancel never deletes your downloaded files, and Remove asks first whether they go too. Remove \
          unpacked files deletes only the copies Nexus unpacked."]
    );

    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::Start,
    }));
    host.test.torrents.progress(ASTRO_HASH, 4);
    until_torrent(&mut host, "downloading");
    assert_eq!(
        values(&host, "downloads-summary"),
        ["Protected by your VPN · wg0 · 1 downloading · 0 sharing · 1 in all"]
    );
}

#[test]
fn a_ready_download_lists_up_to_100_files_in_a_scroll_area() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    let names: Vec<String> = (0..120).map(|i| format!("Astro/part{i:03}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    host.test.torrents.has(ASTRO_HASH, "Astro", &files);
    let offer = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer,
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    until_torrent(&mut host, "ready");
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-file-summary"), ["120 files · 0 MB · first 100 shown"]);
    // The list holds all 100; the testing backend finds only the rows in view.
    assert_eq!(values(&host, "torrent-files"), ["100"]);
    assert_eq!(values(&host, "torrent-file").first().map(String::as_str), Some("Astro/part000.bin · 0 MB"));
}

#[test]
fn while_it_looks_up_or_installs_a_card_shows_a_button_that_does_nothing_and_cancel_install() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_with(home.path(), slow_installer());
    let held = host.test.torrents.hold();
    let offer: nexus_api::GameKey = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer,
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-placeholder"), ["Resolving…"]);
    // The keys skip it: the card's first button is Cancel.
    assert_eq!(host.core().view().downloads.torrents[0].actions, ["cancel"]);

    held.release();
    until_torrent(&mut host, "ready");
    assert_eq!(host.count("torrent-placeholder"), 0);
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::Start,
    }));
    host.test.torrents.complete(ASTRO_HASH);
    until_torrent(&mut host, "complete");
    host.dispatch(Command::GameInstall(nexus_core::GameInstallCmd::Download { id: ASTRO_HASH.into(), folder: None }));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-placeholder"), ["Installing…"]);
    press(&mut host, "torrent-action:cancel-install", ASTRO_HASH);
    until_install(&mut host, "cancelled");
    assert_eq!(host.count("torrent-placeholder"), 0);
}

/// Presses `key` and gives the core commands the theme asked for, without carrying them out.
fn key_asks(host: &mut Composed, key: slint::platform::Key) -> Vec<Command> {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    let requests: Vec<nexus_ui_host::UiRequest> = host.requests.borrow_mut().drain(..).collect();
    requests
        .into_iter()
        .filter_map(|r| if let nexus_ui_host::UiRequest::Core(command) = r { Some(command) } else { None })
        .collect()
}

#[test]
fn right_to_left_the_install_sheet_has_install_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert!(x_of(&host, "install-agree") < x_of(&host, "install-not-now"), "left to right, Install first");
    key(&mut host, Key::Escape);

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert!(x_of(&host, "install-agree") > x_of(&host, "install-not-now"), "right to left, Install on the right");
    // Right stays on Install, on the right; Left goes to Not now.
    key(&mut host, Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        asked.iter().any(|c| matches!(c, Command::GameInstall(nexus_core::GameInstallCmd::Download { .. }))),
        "Enter installs: {asked:?}"
    );

    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    key(&mut host, Key::LeftArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        !asked.iter().any(|c| matches!(c, Command::GameInstall(nexus_core::GameInstallCmd::Download { .. }))),
        "Not now: {asked:?}"
    );
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("install-agree"), 0, "Not now closed the sheet");
}

#[test]
fn right_to_left_the_download_sheet_has_look_up_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    press(&mut host, "release-download", ASTRO_HASH);
    assert!(x_of(&host, "download-agree") < x_of(&host, "download-not-now"), "left to right, Look up first");
    // Left to right, Right goes to Not now.
    key(&mut host, Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        !asked.iter().any(|c| matches!(c, Command::Torrent(nexus_core::TorrentCmd::Download { .. }))),
        "Not now: {asked:?}"
    );
    settle(&mut host);
    assert_eq!(host.count("download-agree"), 0, "Not now closed the sheet");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    press(&mut host, "release-download", ASTRO_HASH);
    assert!(x_of(&host, "download-agree") > x_of(&host, "download-not-now"), "right to left, Look up on the right");
    key(&mut host, Key::LeftArrow);
    key(&mut host, Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        asked.iter().any(|c| matches!(c, Command::Torrent(nexus_core::TorrentCmd::Download { .. }))),
        "Enter looks it up: {asked:?}"
    );
}

#[test]
fn right_to_left_the_install_prompt_has_install_and_play_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let mut host = composed(|h| h.plugin(games_needing_an_emulator("emu-x", vec!["Astro"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
    settle(&mut host);
    assert!(x_of(&host, "Install and play") > x_of(&host, "Not now"), "Install and play on the right");
    key(&mut host, Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(asked.iter().any(|c| matches!(c, Command::ConfirmInstall)), "Right stays on it: {asked:?}");
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
        h.plugin(games("emu-kyty", vec![]).console("PS5")).plugin(
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
        h.plugin(games_on("emu-kyty", "PS5", vec!["Alpha"]).console("PS5"))
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

/// The Catalog of one PS5 game, with Refresh from the forums on or off.
fn catalog_with_refresh(refresh: bool) -> Composed {
    let mut host = composed(|h| {
        h.plugin(games_on("emu-kyty", "PS5", vec![]).console("PS5"))
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

#[test]
fn a_packaged_install_updates_from_settings_then_offers_a_restart() {
    use nexus_core::launcher_update::{Install, package_key};
    let latest = "https://github.com/owner/nexus/releases/latest/download/latest.json";
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-9.9.9.x86_64.rpm";
    let mut host = composed(|h| {
        let h = h.install(Install::Rpm);
        let sha = h.http().serve(url, b"a package".to_vec());
        let key = package_key(Install::Rpm).unwrap();
        let manifest = serde_json::json!({ "version": "9.9.9", "assets": { key: { "url": url, "sha256": sha } } });
        h.http().serve(latest, manifest.to_string().into_bytes());
        h
    });
    host.core().kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "launcher-update");

    host.first("launcher-update").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.first("launcher-update").accessible_value().as_deref(), Some("available"));

    host.first("launcher-update").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.first("launcher-update").accessible_value().as_deref(), Some("installed"));

    host.first("launcher-update").invoke_accessible_default_action();
    let asked = host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Restart));
    assert!(asked, "the installed update offers a restart");
}

// The on-screen keyboard: ten keys a row, a numbers row and three letter rows (or the symbols
// layer), then the action row. Its keys are `osk-key`, valued with what they show; the
// highlighted one is selected, and its item index is row * 10 + column (the action row is 4).

const OSK_LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.-"];
const OSK_SYMBOLS: [&str; 4] = ["!@#$%^&*()", "-_=+[]{}\\|", ";:\"<>/?~`€", "£¥§°•«»¿¡ "];

fn pad(host: &mut Composed, button: nexus_ui_host::PadButton) {
    host.ui.pad(button).unwrap();
    host.pump();
    settle(host);
}

/// The highlighted key: its row and column.
fn osk_at(host: &Composed) -> (usize, usize) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "osk-key");
    let lit: Vec<_> = found.filter(|e| e.accessible_item_selected() == Some(true)).collect();
    let [key] = lit.as_slice() else { panic!("one highlighted key, got {}", lit.len()) };
    let index = key.accessible_item_index().expect("a key has its place");
    (index / 10, index % 10)
}

/// The d-pad to the key at `row`, `column`, one step at a time.
fn osk_go(host: &mut Composed, row: usize, column: usize) {
    use nexus_ui_host::PadButton;
    for _ in 0..40 {
        let (r, c) = osk_at(host);
        if (r, c) == (row, column) {
            return;
        }
        let button = if r < row {
            PadButton::Down
        } else if r > row {
            PadButton::Up
        } else if c < column {
            PadButton::Right
        } else {
            PadButton::Left
        };
        pad(host, button);
    }
    panic!("the highlight never reached {row}, {column}");
}

/// Types `text` with the controller alone: R1 for the layer, L1 for a capital, the d-pad to the
/// key and Cross.
fn osk_type(host: &mut Composed, text: &str) {
    use nexus_ui_host::PadButton;
    let find = |rows: [&str; 4], ch: char| {
        rows.iter().enumerate().find_map(|(r, row)| row.chars().position(|c| c == ch).map(|c| (r, c)))
    };
    for ch in text.chars() {
        let (symbols, (row, column)) = match find(OSK_LETTERS, ch.to_ascii_lowercase()) {
            Some(at) => (false, at),
            None => (true, find(OSK_SYMBOLS, ch).unwrap_or_else(|| panic!("no key for {ch:?}"))),
        };
        let layer = if symbols { "symbols" } else { "letters" };
        if !values(host, "osk")[0].starts_with(layer) {
            pad(host, PadButton::RightShoulder);
        }
        if ch.is_ascii_uppercase() && !values(host, "osk")[0].ends_with("shift") {
            pad(host, PadButton::LeftShoulder);
        }
        osk_go(host, row, column);
        pad(host, PadButton::South);
    }
}

/// The d-pad to the action key that shows `label`, and Cross.
fn osk_action(host: &mut Composed, label: &str) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "osk-key");
    let key = found
        .into_iter()
        .find(|e| e.accessible_value().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} key"));
    let index = key.accessible_item_index().unwrap();
    osk_go(host, index / 10, index % 10);
    pad(host, nexus_ui_host::PadButton::South);
}

/// Characters typed on a physical keyboard.
fn typed(host: &mut Composed, text: &str) {
    for ch in text.chars() {
        let s: slint::SharedString = ch.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: s.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: s });
    }
    host.pump();
    settle(host);
}

/// The Library, opened with the controller.
fn library_by_pad() -> Composed {
    use nexus_ui_host::PadButton;
    let mut host = default_theme();
    pad(&mut host, PadButton::Up);
    pad(&mut host, PadButton::Right);
    pad(&mut host, PadButton::South);
    assert_eq!(host.count("default-game"), 3, "in the Library");
    host
}

#[test]
fn the_controller_alone_types_a_library_search_on_the_on_screen_keyboard() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    assert_eq!(host.count("osk"), 0);

    // Triangle focuses the search: the keyboard opens on q.
    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk"), ["letters"]);
    assert_eq!(osk_at(&host), (1, 0), "on q");
    let first: Vec<String> = values(&host, "osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "1234567890", "a numbers row of ten keys");
    let actions: Vec<String> = values(&host, "osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Done"], "no Show: not a password");

    osk_type(&mut host, "gam");
    assert_eq!(values(&host, "osk-text"), ["gam"]);
    assert_eq!(values(&host, "library-search"), ["gam"], "the field has it");

    // R2 is Done: the keyboard closes and the search runs.
    pad(&mut host, PadButton::RightTrigger);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(values(&host, "default-game"), ["Gamma"]);
}

#[test]
fn done_on_the_keyboard_and_options_end_the_typing_too() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "al");
    osk_action(&mut host, "Done");
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "al");
    assert_eq!(values(&host, "default-game"), ["Alpha"]);

    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk-text"), ["al"], "it opens with the field's text");
    osk_type(&mut host, "x");
    pad(&mut host, PadButton::Start);
    assert_eq!(host.count("osk"), 0, "Options is Done");
    assert_eq!(host.core().view().home.library.query, "alx");
}

#[test]
fn a_physical_keyboard_types_straight_in_and_the_on_screen_keyboard_stays_shut() {
    let mut host = library_by_pad();
    // The controller came first; now the keyboard: / focuses the search.
    typed(&mut host, "/");
    assert_eq!(host.count("osk"), 0, "a key focused it");
    typed(&mut host, "gam");
    assert_eq!(values(&host, "library-search"), ["gam"]);
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(values(&host, "default-game"), ["Gamma"]);
    assert_eq!(host.count("osk"), 0);
}

#[test]
fn typing_on_a_physical_keyboard_goes_in_while_the_on_screen_one_is_open_and_enter_is_done() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    pad(&mut host, PadButton::Right);
    assert_eq!(osk_at(&host), (1, 1), "on w");

    typed(&mut host, "be");
    assert_eq!(values(&host, "osk-text"), ["be"], "straight in");
    assert_eq!(osk_at(&host), (1, 1), "the highlight stays");
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.count("osk"), 0, "Enter right after typing is Done");
    assert_eq!(host.core().view().home.library.query, "be");
    assert_eq!(values(&host, "default-game"), ["Beta"]);
}

#[test]
fn the_keyboard_s_shift_is_for_one_letter_and_r1_switches_to_the_symbols() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);

    // L1: Shift, for one letter.
    pad(&mut host, PadButton::LeftShoulder);
    assert_eq!(values(&host, "osk"), ["letters shift"]);
    assert!(values(&host, "osk-key").contains(&"Q".to_string()), "capitals on the keys");
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk"), ["letters"], "off after one letter");
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk-text"), ["Qq"]);

    // R1: the symbols, and back.
    pad(&mut host, PadButton::RightShoulder);
    assert_eq!(values(&host, "osk"), ["symbols"]);
    let first: Vec<String> = values(&host, "osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "!@#$%^&*()");
    assert!(values(&host, "osk-key").contains(&"ABC".to_string()), "the layer key goes back");
    pad(&mut host, PadButton::South);
    osk_type(&mut host, "€?");
    pad(&mut host, PadButton::RightShoulder);
    assert_eq!(values(&host, "osk"), ["letters"]);
    osk_type(&mut host, "Z");
    assert_eq!(values(&host, "osk-text"), ["Qq-€?Z"]);

    // Shift on the keyboard is a key too.
    osk_action(&mut host, "Shift");
    assert_eq!(values(&host, "osk"), ["letters shift"]);
}

#[test]
fn square_deletes_and_triangle_adds_a_space_as_their_keys_do() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "ab");
    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk-text"), ["ab "], "Triangle: a space");
    pad(&mut host, PadButton::West);
    pad(&mut host, PadButton::West);
    assert_eq!(values(&host, "osk-text"), ["a"], "Square: delete");

    osk_action(&mut host, "Space");
    osk_type(&mut host, "€");
    assert_eq!(values(&host, "osk-text"), ["a €"]);
    osk_action(&mut host, "Delete");
    assert_eq!(values(&host, "osk-text"), ["a "], "a symbol deletes whole");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    assert_eq!(values(&host, "osk-text"), [""], "nothing left to delete");
}

#[test]
fn circle_closes_the_keyboard_and_keeps_the_text() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "ga");
    pad(&mut host, PadButton::East);
    assert_eq!(host.count("osk"), 0, "Circle closes it");
    assert_eq!(values(&host, "library-search"), ["ga"], "the text stays");
    assert_eq!(host.core().view().home.library.query, "ga", "and searches");

    // Cross on the field opens it again, with the text.
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk-text"), ["ga"]);
}

/// A theme for tests: the default one with a password field on the Emulators screen. Its
/// `focus-password` button focuses the field.
fn with_a_password_field() -> (tempfile::TempDir, Composed) {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "secrets",
        "extends = \"default\"\nscreens = [\"emulators\"]",
        &[(
            "screens/emulators.slint",
            r#"import { TextField } from "../components/keyboard.slint";
export component EmulatorsScreen inherits Rectangle {
    in property <bool> open;
    in property <bool> pushed;
    field := TextField { x: 100px; y: 100px; width: 400px; height: 50px; name: "test-password"; password: true; }
    Rectangle {
        accessible-role: button;
        accessible-label: "focus-password";
        accessible-value: "password";
        accessible-action-default => { field.focus(); }
    }
}
"#,
        )],
    );
    let mut host = composed(|h| h.discover(dir.path()).plugin(games("emu-a", vec!["Alpha"])));
    host.ui.switch(&id("secrets")).expect("the test theme loads");
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    settle(&mut host);
    (dir, host)
}

#[test]
fn a_password_field_hides_its_text_and_the_show_key_shows_it() {
    use nexus_ui_host::PadButton;
    let (_dir, mut host) = with_a_password_field();
    // L1 three times: Settings, Themes, Emulators.
    for _ in 0..3 {
        pad(&mut host, PadButton::LeftShoulder);
    }
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Emulators);
    press(&mut host, "focus-password", "password");
    assert_eq!(host.count("osk"), 1, "the last input was the controller");
    let actions: Vec<String> = values(&host, "osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Show", "Done"]);

    osk_type(&mut host, "pw1");
    assert_eq!(values(&host, "osk-text"), ["●●●"]);
    assert_eq!(values(&host, "test-password"), ["●●●"], "the field hides it too");
    osk_action(&mut host, "Show");
    assert_eq!(values(&host, "osk-text"), ["pw1"]);
    assert_eq!(values(&host, "test-password"), ["pw1"]);
    assert!(values(&host, "osk-key").contains(&"Hide".to_string()));
    osk_action(&mut host, "Hide");
    assert_eq!(values(&host, "osk-text"), ["●●●"]);
}

#[test]
fn a_settings_text_box_opened_with_the_controller_types_on_the_keyboard_too() {
    use nexus_ui_host::PadButton;
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    settle(&mut host);
    // L1 from Home: Settings, on the game folders' row; Cross opens it for typing.
    pad(&mut host, PadButton::LeftShoulder);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Settings);
    assert_eq!(host.count("folder:games"), 1);
    pad(&mut host, PadButton::South);
    assert_eq!(host.count("osk"), 1, "the text box opened the keyboard");
    let before = values(&host, "osk-text")[0].clone();
    let typed = full_path("/x");
    osk_type(&mut host, &typed);
    assert_eq!(values(&host, "editor"), [format!("{before}{typed}")]);

    // R2: Done sets the folders.
    pad(&mut host, PadButton::RightTrigger);
    assert_eq!(values(&host, "folder:games"), [format!("{before}{typed}")]);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.count("editor"), 0, "the row is closed");
}
