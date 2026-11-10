//! Home: the list of systems, its cover, and the way to each system's games and back.

use super::*;
use nexus_core::{Area, Route, SessionCmd};
use nexus_ui_api::ScreenId;

#[test]
fn home_lists_all_games_first_then_each_emulator_s_system() {
    let host = default_theme();
    let shown = host.values("systems-tile");
    assert_eq!(shown[0], "All games");
    let mut systems = shown[1..].to_vec();
    systems.sort();
    assert_eq!(systems, ["PlayStation 4", "PlayStation 5"]);
    assert_eq!(host.focused("systems-tile"), ["All games"], "the focus starts on All games");
    assert_eq!(host.count("default-tile"), 0, "no game tiles");
    assert_eq!(host.count("default-system"), 0, "no Home or Library tab");
}

#[test]
fn the_list_is_on_the_far_side_of_the_reading_direction() {
    let mut host = default_theme();
    let middle = 1600.0 / 2.0;
    let x = |host: &Composed| host.first("systems-tile").absolute_position().x;
    assert!(x(&host) > middle, "left to right, on the right: {}", x(&host));

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.settle();
    assert!(x(&host) < middle, "right to left, on the left: {}", x(&host));
}

#[test]
fn up_and_down_move_along_the_list_and_stop_at_its_end() {
    let mut host = default_theme();
    let shown = host.values("systems-tile");
    host.navigate(Key::DownArrow);
    assert_eq!(host.focused("systems-tile"), [shown[1].clone()]);
    host.navigate(Key::DownArrow);
    host.navigate(Key::DownArrow);
    assert_eq!(host.focused("systems-tile"), [shown[2].clone()], "it stops at the last");
    host.navigate(Key::UpArrow);
    assert_eq!(host.focused("systems-tile"), [shown[1].clone()]);
    host.navigate(Key::LeftArrow);
    host.navigate(Key::RightArrow);
    assert_eq!(host.focused("systems-tile"), [shown[1].clone()], "left and right do nothing");
}

#[test]
fn up_from_the_first_system_goes_to_the_top_bar() {
    let mut host = default_theme();
    host.navigate(Key::UpArrow);
    // The top bar's first button the keys reach: with no Catalog, Downloads.
    host.navigate(Key::Return);
    assert_eq!(host.core().view().screen, ScreenId::Downloads);
}

#[test]
fn the_cover_behind_the_list_follows_the_focus() {
    let mut host = default_theme();
    assert_eq!(host.value("systems-background"), "All games");
    focus_on(&mut host, "PlayStation 5");
    cover_follows(&mut host);
    assert_eq!(host.value("systems-background"), "PlayStation 5");
}

#[test]
fn enter_on_a_system_opens_the_library_on_its_games() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(library_titles(&host), ["Alpha", "Beta"]);
    assert_eq!(host.count("systems-back"), 1, "the way back to the systems");
}

#[test]
fn a_click_on_a_system_opens_the_library_on_its_games() {
    let mut host = default_theme();
    let ps5 = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "systems-tile")
        .find(|e| e.accessible_value().as_deref() == Some("PlayStation 5"))
        .unwrap();
    use slint::ComponentHandle;
    use slint::platform::{PointerEventButton, WindowEvent};
    let (at, size) = (ps5.absolute_position(), ps5.size());
    let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    let window = host.instance().window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    host.pump();
    host.settle();
    assert_eq!(library_titles(&host), ["Gamma"], "one click, not a select first");
}

#[test]
fn all_games_opens_the_library_on_every_game() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.navigate(Key::Escape);
    while host.focused("systems-tile") != ["All games"] {
        host.navigate(Key::UpArrow);
    }
    host.navigate(Key::Return);
    host.settle();
    assert_eq!(library_titles(&host), ["Alpha", "Beta", "Gamma"]);
}

#[test]
fn back_from_a_system_s_games_comes_back_to_it_and_clears_the_filter() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 5");
    host.navigate(Key::Return);
    assert_eq!(library_titles(&host), ["Gamma"]);
    host.navigate(Key::Escape);
    host.settle();
    assert_eq!(host.focused("systems-tile"), ["PlayStation 5"], "on the system it opened");
    assert_eq!(library_titles(&host), ["Alpha", "Beta", "Gamma"], "the filter is gone");
    assert_eq!(host.core().view().screen, ScreenId::Home);
}

#[test]
fn the_back_button_comes_back_to_the_list() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.press_first("systems-back");
    assert_eq!(host.focused("systems-tile"), ["PlayStation 4"]);
    assert_eq!(host.count("systems-back"), 0);
}

#[test]
fn a_game_in_a_system_s_library_opens_its_game_hub() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.navigate(Key::Return);
    rest(&mut host);
    host.settle();
    assert_eq!(host.core().view().screen, ScreenId::GameDetail);
    assert_eq!(host.core().view().game.as_deref(), Some("emu-a:G0"));
}

#[test]
fn menu_on_a_system_opens_its_emulator_s_page() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 5");
    host.navigate(Key::Menu);
    assert_eq!(host.core().view().screen, ScreenId::Plugin);
    assert_eq!(host.core().view().plugin_page.map(|p| p.plugin.id.to_string()).as_deref(), Some("emu-b"));
}

#[test]
fn menu_on_all_games_opens_nothing() {
    let mut host = default_theme();
    host.navigate(Key::Menu);
    assert_eq!(host.core().view().screen, ScreenId::Home);
}

#[test]
fn search_opens_every_game_with_the_search_box() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 5");
    host.navigate(Key::Return);
    host.navigate(Key::Escape);
    host.type_text("/");
    assert_eq!(library_titles(&host), ["Alpha", "Beta", "Gamma"]);
}

#[test]
fn the_running_game_s_system_is_marked() {
    let mut host = default_theme();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-b:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.dispatch(Command::Navigate(Route::Home));
    host.settle();
    assert_eq!(host.values("systems-tag"), ["PlayStation 5: ● PLAYING"]);
}

#[test]
fn an_emulator_with_no_build_installed_says_so() {
    let host = default_theme_with(|h| {
        h.plugin(FakeCatalog::kyty().system("PS5").needing_its_emulator().games(["Delta"]).build())
    });
    assert!(
        host.values("systems-tag").iter().any(|t| t.ends_with(": Not installed")),
        "{:?}",
        host.values("systems-tag")
    );
}

#[test]
fn the_classic_theme_comes_back_whole_after_the_default_theme() {
    let mut host = default_theme();
    host.ui.switch(&nexus_api::PluginId::new(theme_classic::ID).unwrap()).unwrap();
    host.settle();
    assert_eq!(host.count("systems-tile"), 0);
    assert_eq!(host.values("default-system"), ["Home", "Library"]);
    assert!(host.count("default-tile") > 0, "the Classic Home's game tiles");
}

#[test]
fn the_cover_changes_once_the_moves_stop_not_while_a_key_is_held() {
    use std::time::Duration;
    let mut host = default_theme();
    for _ in 0..2 {
        host.tap(Key::DownArrow);
        host.wait(Duration::from_millis(100));
    }
    assert_eq!(host.value("systems-background"), "All games", "still moving");
    host.wait(Duration::from_millis(100));
    assert_eq!(host.value("systems-background"), host.focused("systems-tile")[0]);
}

#[test]
fn after_a_game_starts_the_list_has_the_keys_and_the_library_s_filter_is_gone() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 5");
    host.navigate(Key::Return);
    assert_eq!(library_titles(&host), ["Gamma"]);
    host.dispatch(Command::Session(SessionCmd::Launch("emu-b:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.ui.render().unwrap();
    host.settle();
    host.pump();
    host.settle();
    assert_eq!(host.focused("systems-tile"), ["PlayStation 5"]);
    assert_eq!(library_titles(&host), ["Alpha", "Beta", "Gamma"], "the filter is gone");
    host.navigate(Key::UpArrow);
    assert_ne!(host.focused("systems-tile"), ["PlayStation 5"], "Up moves along the list");
}

#[test]
fn only_the_running_game_s_emulator_is_marked_when_two_emulators_share_a_name() {
    let mut host = default_theme_of(|h| {
        h.plugin(FakeCatalog::new("emu-a").emulator("Same").playable().system("PS4").games(["Alpha"]).build())
            .plugin(FakeCatalog::new("emu-b").emulator("Same").playable().system("PS5").games(["Gamma"]).build())
    });
    host.dispatch(Command::Session(SessionCmd::Launch("emu-b:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.dispatch(Command::Navigate(Route::Home));
    host.settle();
    assert_eq!(host.values("systems-tag"), ["PlayStation 5: ● PLAYING"]);
}

#[test]
fn the_mouse_wheel_moves_along_the_list_down_and_up() {
    use slint::ComponentHandle;
    use slint::platform::WindowEvent;
    let mut host = default_theme();
    let shown = host.values("systems-tile");
    let tile = host.first("systems-tile");
    let position = slint::LogicalPosition::new(tile.absolute_position().x + 4.0, tile.absolute_position().y + 4.0);
    // A notch of the wheel toward the user: Slint's delta is negative, as for a list that
    // scrolls down.
    host.instance().window().dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: -120.0 });
    host.settle();
    assert_eq!(host.focused("systems-tile"), [shown[1].clone()], "wheel down: the next system");
    host.instance().window().dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: 120.0 });
    host.settle();
    assert_eq!(host.focused("systems-tile"), [shown[0].clone()], "wheel up: back");
}

/// Starts `game`, as Play does, and ends it once the theme has shown it running.
fn play_and_end(host: &mut Composed, game: &str, while_running: impl FnOnce(&mut Composed)) {
    host.dispatch(Command::Session(nexus_core::SessionCmd::Launch(game.parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    shown(host);
    while_running(host);
    let process = host.test.spawner.spawned()[0].clone();
    process.exit(0);
    host.until(Area::Sessions, |core| !core.game_running());
    shown(host);
}

/// The theme sees the change and the core carries out what the theme asked for.
fn shown(host: &mut Composed) {
    host.settle();
    host.pump();
    host.settle();
}

/// The title of the Library's selected game, as the core has it.
fn selected_title(host: &Composed) -> Option<String> {
    let library = host.core().view().home.library;
    library.selected.map(|i| library.games[i].title.clone())
}

#[test]
fn a_game_played_from_a_system_s_games_comes_back_to_them_when_it_ends() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.settle();

    play_and_end(&mut host, "emu-a:G1", |host| {
        assert_eq!(host.count("systems-back"), 0, "while it runs, the list shows it with Resume");
    });

    assert_eq!(host.count("systems-back"), 1, "back in the system's games");
    assert_eq!(library_titles(&host), ["Alpha", "Beta"], "the system's filter is back");
    assert_eq!(selected_title(&host).as_deref(), Some("Beta"), "on the game that was played");
}

#[test]
fn a_player_who_moves_while_the_game_runs_stays_where_they_went() {
    let mut host = default_theme();
    focus_on(&mut host, "PlayStation 4");
    host.navigate(Key::Return);
    host.settle();

    play_and_end(&mut host, "emu-a:G1", |host| {
        host.press("systems-tile", "PlayStation 5");
        host.settle();
    });

    assert_eq!(library_titles(&host), ["Gamma"], "still in the games they opened");
}
