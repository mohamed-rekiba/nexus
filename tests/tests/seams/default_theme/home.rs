//! Home: the boot screen, the tile row and its hero, the top bar, and the first run.

use super::*;

#[test]
fn the_boot_screen_shows_until_the_library_is_in() {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build()));
    host.ui.start().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "boot"), ["Loading..."], "from the first frame");

    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    settle(&mut host);
    assert!(values(&host, "boot").is_empty(), "it fades out over Home");
}

#[test]
fn home_shows_the_store_s_tagline() {
    let tagline = "Build whatever you can imagine, in a world that is yours.";
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, tagline).build())
    });
    host.ui.start().unwrap();
    assert_eq!(home_line(&mut host), tagline);
}

#[test]
fn without_a_tagline_home_shows_the_game_s_facts_on_one_line() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").without_description().games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
    let mut host =
        composed(|h| h.plugin(FakeCatalog::new("emu-a").system("PS4").without_description().games([LONG]).build()));
    host.ui.start().unwrap();
    let text = home_line(&mut host);
    let kept = text.strip_suffix('…').unwrap_or_else(|| panic!("ends with …: {text}"));
    assert!(text.chars().count() <= nexus_ui_api::SUMMARY_CHARS + 1, "{} characters", text.chars().count());
    assert!(LONG.starts_with(kept) && LONG[kept.len()..].starts_with(' '), "cut after a whole word: {text}");
}

#[test]
fn a_rating_counts_its_players_in_thousands() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.44, "").build())
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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").icon(art).build())
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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").icon("https://art.example/alpha.png").build())
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
    let mut host =
        composed(|h| h.first_run().plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build()));
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
fn the_key_hints_name_the_trailer_and_options_and_hide_over_the_grids() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
fn a_new_game_s_hero_waits_for_its_details_but_not_for_ever() {
    // These games' details never load.
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").without_details().games(["Alpha", "Beta"]).build())
    });
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
