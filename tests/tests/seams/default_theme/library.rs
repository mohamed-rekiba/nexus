//! The Library: its filters, search, sorts and grid.

use super::*;

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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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

#[test]
fn right_to_left_the_library_s_header_and_genre_row_are_mirrored() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").system("PS4").games(titles).build()));
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
