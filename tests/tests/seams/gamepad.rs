//! Seam 3, composed host: a gamepad drives the default theme through the keys it already handles.

use crate::common::*;
use nexus_core::{Command, Route};
use nexus_ui_api::ScreenId;
use nexus_ui_host::{PadButton, vm_text};

fn tv() -> Composed {
    let mut host = composed(|h| h.plugin(games("emu-a", vec!["Alpha", "Beta", "Gamma"])));
    host.ui.start().unwrap();
    host.resize(1920.0, 1080.0);
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G0".parse().unwrap()));
    host
}

fn press(host: &mut Composed, button: PadButton) {
    host.ui.pad(button).unwrap();
    host.pump();
}

#[test]
fn the_d_pad_moves_the_selection_and_the_buttons_open_and_go_back() {
    let mut host = tv();

    // Right and left move along the tile row.
    press(&mut host, PadButton::Right);
    press(&mut host, PadButton::Right);
    rest(&mut host);
    assert_eq!(host.core().view().home.selected, Some(2), "two steps right");
    press(&mut host, PadButton::Left);
    rest(&mut host);
    assert_eq!(host.core().view().home.selected, Some(1));
    assert_eq!(vm_text(host.instance(), "input-mode").as_deref(), Some("gamepad"));

    // A opens the Game Hub; B goes back to the row.
    press(&mut host, PadButton::South);
    assert_eq!(host.core().view().screen, ScreenId::GameDetail);
    assert_eq!(host.core().view().game.as_deref(), Some("emu-a:G1"));
    press(&mut host, PadButton::East);
    assert_eq!(host.core().view().screen, ScreenId::Home);
    settle(&mut host);
    assert!(host.count("default-tile") > 0, "back on the row");

    // Up to the top bar, right to Library, A opens it; B leaves it.
    press(&mut host, PadButton::Up);
    press(&mut host, PadButton::Right);
    press(&mut host, PadButton::South);
    settle(&mut host);
    assert_eq!(host.count("default-game"), 3, "the Library grid");
    press(&mut host, PadButton::East);
    settle(&mut host);
    assert_eq!(host.count("default-game"), 0, "B leaves the Library for the row");
    assert!(host.count("default-tile") > 0);
}

#[test]
fn the_shoulder_button_goes_to_the_next_section_and_the_guide_button_home() {
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha", "Beta", "Gamma"]))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));

    press(&mut host, PadButton::RightShoulder);
    assert_eq!(host.core().view().screen, ScreenId::Catalog);
    host.dispatch(Command::Navigate(Route::Downloads));
    press(&mut host, PadButton::Guide);
    assert_eq!(host.core().view().screen, ScreenId::Home);
}

#[test]
fn while_a_game_is_in_front_the_pad_is_the_game_s_and_a_ps_button_tap_too() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
    watch_until(&mut host, "the game is in front", |core| core.game_in_front());
    assert_eq!(host.core().view().screen, ScreenId::Running);
    let process = host.test.spawner.spawned()[0].clone();
    // What the theme asked for as the game started (it selects the running game).
    host.pump();

    // Cross in the game is the game's: the launcher behind it does nothing. A tap of the PS
    // button does nothing either: only a hold reaches the launcher, and closes the game.
    for button in [PadButton::South, PadButton::Right, PadButton::Guide, PadButton::South, PadButton::East] {
        host.ui.pad(button).unwrap();
    }
    assert!(host.requests.borrow().is_empty(), "{:?}", host.requests.borrow());
    assert!(host.core().game_in_front());
    assert!(process.signals().is_empty(), "the game still runs");
}

#[test]
fn holding_the_ps_button_closes_a_running_game_at_once_and_opens_no_power_menu() {
    use nexus_testkit::Signal;
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
    watch_until(&mut host, "the game is in front", |core| core.game_in_front());
    let process = host.test.spawner.spawned()[0].clone();
    host.pump();

    press(&mut host, PadButton::GuideHeld);
    watch_until(&mut host, "the game was asked to close", |_| process.signals().contains(&Signal::Terminate));
    assert_eq!(host.count("power-menu"), 0, "no power menu while a game runs");
}

#[test]
fn while_the_launcher_s_window_is_not_focused_the_pad_is_the_other_window_s() {
    // A game in front that the desktop can't name (Wayland without xdotool): the launcher's
    // window lost the focus, and that's enough.
    let mut host = tv();
    host.dispatch(Command::Navigate(Route::Downloads));
    host.ui.set_window_focused(false);
    for button in [PadButton::Right, PadButton::South, PadButton::East] {
        host.ui.pad(button).unwrap();
    }
    assert!(host.requests.borrow().is_empty(), "{:?}", host.requests.borrow());

    // The PS button still works: it is how the player comes back.
    press(&mut host, PadButton::Guide);
    assert_eq!(host.core().view().screen, ScreenId::Home);

    // Focused again, the pad is the launcher's.
    host.ui.set_window_focused(true);
    press(&mut host, PadButton::South);
    assert_eq!(host.core().view().screen, ScreenId::GameDetail);
}

#[test]
fn after_a_game_closes_the_launcher_asks_how_far_it_got() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
    watch_until(&mut host, "the game is in front", |core| core.game_in_front());
    host.pump();
    host.test.spawner.spawned()[0].exit(0);
    watch_until(&mut host, "the game closed", |core| !core.game_running());
    let asked = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "result-prompt")
        .map(|e| e.accessible_value().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert_eq!(asked, ["How far did Alpha get?"]);

    // Cross on the first answer: it reaches gameplay.
    press(&mut host, PadButton::South);

    assert!(host.core().view().result_prompt.is_none());
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    assert_eq!(host.core().view().detail.unwrap().your_result, "ingame");
}

#[test]
fn the_select_button_closes_the_notice() {
    let mut host = tv();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    press(&mut host, PadButton::Select);
    // Less than the 4 s a notice stays: the button closed it, not the wait.
    for _ in 0..2 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(500));
        host.ui.render().unwrap();
        host.pump();
    }
    assert!(host.core().view().notices.is_empty(), "Select closes the notice");
}

#[test]
fn a_held_cross_does_not_repeat_it() {
    let mut host = tv();
    // A held button repeats: the theme hears a repeat, and a repeated Cross does nothing.
    host.ui.pad_repeated(PadButton::South).unwrap();
    host.pump();
    assert_eq!(host.core().view().screen, ScreenId::Home);
    // A held d-pad still moves.
    host.ui.pad_repeated(PadButton::Right).unwrap();
    rest(&mut host);
    assert_eq!(host.core().view().home.selected, Some(1));
    press(&mut host, PadButton::South);
    assert_eq!(host.core().view().screen, ScreenId::GameDetail, "a press opens the game");
}
