//! The Library in the Default theme: one card size, the system's cover behind it, and a short
//! header: the systems in the top bar, then search, sort and the genre list in one row.

use super::*;

/// The Default theme on PS4 (Alpha, Beta) and PS5 (Gamma), with a store that gives Alpha its
/// genres (Action, Adventure, Arcade).
fn with_genres() -> Composed {
    default_theme_with(|h| h.plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build()))
}

/// Opens the Library on the system named `name` from the list.
fn open(host: &mut Composed, name: &str) {
    focus_on(host, name);
    host.navigate(Key::Return);
    host.settle();
}

fn y_of(host: &Composed, label: &str) -> f32 {
    host.first(label).absolute_position().y
}

fn bottom_of(host: &Composed, label: &str) -> f32 {
    let e = host.first(label);
    e.absolute_position().y + e.size().height
}

/// The genre the core filters by: its label, or "" for every genre.
fn genre_on(host: &Composed) -> String {
    let library = host.core().view().home.library;
    library.genres.iter().find(|g| g.on && !g.id.is_empty()).map(|g| g.label.clone()).unwrap_or_default()
}

#[test]
fn the_library_has_one_card_size_and_no_choice_of_it() {
    let mut host = default_theme();
    open(&mut host, "All games");
    assert_eq!(host.count("default-game"), 3);
    assert_eq!(host.count("density"), 0, "no Comfortable or Compact");
    // The bar ends at the sort: right past it there is nothing more, so Enter opens the sorts.
    host.navigate(Key::UpArrow);
    for _ in 0..4 {
        host.navigate(Key::RightArrow);
    }
    host.navigate(Key::Return);
    assert!(host.count("sort-option") > 0, "the keys are still on the sort");
}

#[test]
fn the_cover_follows_a_system_picked_with_the_keys_and_goes_back_to_the_list_s() {
    let mut host = default_theme();
    open(&mut host, "PlayStation 5");
    // Up to the systems, and along to PS4.
    host.navigate(Key::UpArrow);
    host.navigate(Key::UpArrow);
    while host.focused("filter:PS4").is_empty() {
        host.navigate(Key::RightArrow);
    }
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(host.value("systems-background"), "PlayStation 4");
    // Back to the list: the cover is the focused entry's again.
    host.navigate(Key::Escape);
    host.settle();
    assert_eq!(host.focused("systems-tile"), ["PlayStation 5"]);
    assert_eq!(host.value("systems-background"), "PlayStation 5");
}

#[test]
fn the_library_shows_its_system_s_cover_and_follows_the_system_filter() {
    let mut host = default_theme();
    open(&mut host, "PlayStation 5");
    assert_eq!(host.value("systems-background"), "PlayStation 5", "not the first game's art");

    host.press_first("filter:PS4");
    host.settle();
    assert_eq!(library_titles(&host), ["Alpha", "Beta"]);
    assert_eq!(host.value("systems-background"), "PlayStation 4");

    host.press_first("filter:All");
    host.settle();
    assert_eq!(host.value("systems-background"), "All games");
}

#[test]
fn all_games_shows_its_own_cover_behind_the_library() {
    let mut host = default_theme();
    open(&mut host, "All games");
    assert_eq!(host.value("systems-background"), "All games");
}

#[test]
fn the_systems_are_in_the_top_bar_and_the_grid_starts_under_the_search_row() {
    let mut host = with_genres();
    open(&mut host, "All games");
    assert!(bottom_of(&host, "filter:All") <= y_of(&host, "library-search"), "the systems are above the search row");
    assert!(
        y_of(&host, "filter:All") < bottom_of(&host, "systems-back"),
        "the systems share the top bar's line with the way back"
    );
    assert_eq!(host.count("genres"), 0, "no row of genre chips");
    let gap = y_of(&host, "default-game") - bottom_of(&host, "library-search");
    assert!(gap < 60.0, "the cards start right under the search row: {gap} px");
}

#[test]
fn the_genre_list_opens_from_its_button_and_the_keys_pick_a_genre() {
    let mut host = with_genres();
    open(&mut host, "All games");
    assert_eq!(host.value("library-genre"), "All genres");

    // Up from the grid to the bar, then right past the search and the sort to the genres.
    host.navigate(Key::UpArrow);
    host.navigate(Key::RightArrow);
    host.navigate(Key::RightArrow);
    host.navigate(Key::Return);
    let options = host.values("genre-option");
    assert_eq!(options.first().map(String::as_str), Some("All genres"));
    assert!(options.contains(&"Adventure".to_string()), "the store's genres: {options:?}");

    // Down to Adventure, and pick it.
    let at = options.iter().position(|o| o == "Adventure").unwrap();
    for _ in 0..at {
        host.navigate(Key::DownArrow);
    }
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(host.count("genre-option"), 0, "the list closes");
    assert_eq!(genre_on(&host), "Adventure");
    assert!(!library_titles(&host).contains(&"Beta".to_string()), "Beta has no genre: {:?}", library_titles(&host));
    assert_eq!(host.value("library-genre"), "Adventure");
}

#[test]
fn a_click_picks_a_genre_and_back_closes_the_list_without_one() {
    let mut host = with_genres();
    open(&mut host, "All games");
    host.press_first("library-genre");
    assert!(host.count("genre-option") > 1);
    host.navigate(Key::Escape);
    assert_eq!(host.count("genre-option"), 0, "Back closes it");
    assert_eq!(genre_on(&host), "", "and picks nothing");

    host.press_first("library-genre");
    host.press("genre-option", "Arcade");
    host.settle();
    assert_eq!(genre_on(&host), "Arcade");
}

#[test]
fn without_genres_the_bar_has_no_genre_button() {
    let mut host = default_theme();
    open(&mut host, "All games");
    assert_eq!(host.count("library-genre"), 0);
}

#[test]
fn up_from_the_grid_goes_to_the_bar_then_the_systems_then_the_top_bar() {
    let mut host = default_theme();
    open(&mut host, "All games");
    host.navigate(Key::UpArrow);
    host.navigate(Key::UpArrow);
    // On the systems: right to the first one after All, and Enter filters by it.
    host.navigate(Key::RightArrow);
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(library_titles(&host).len(), 2, "the PS4's games: {:?}", library_titles(&host));
    // Up again: the top bar, whose first button the keys reach is Downloads.
    host.navigate(Key::UpArrow);
    host.navigate(Key::Return);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Downloads);
}
