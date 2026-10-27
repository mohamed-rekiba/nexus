//! Seam 3, composed host: theme sounds. A theme asks for a sound by name; the host finds the
//! file the theme declared and gives it to the app to play, unless the player turned sounds off.

use crate::common::*;
use nexus_core::Command;
use nexus_ui_host::{PadButton, UiRequest};

fn tv() -> Composed {
    let mut host = composed(|h| h.plugin(games("emu-a", vec!["Alpha", "Beta"])));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host.dispatch(Command::Select("emu-a:G0".parse().unwrap()));
    host
}

fn asked_for(host: &Composed) -> Vec<String> {
    host.requests
        .borrow()
        .iter()
        .filter_map(|r| if let UiRequest::PlaySound(name) = r { Some(name.clone()) } else { None })
        .collect()
}

#[test]
fn a_step_in_the_default_theme_plays_its_move_sound() {
    let mut host = tv();
    host.ui.pad(PadButton::Down).unwrap();

    assert_eq!(asked_for(&host), ["move"]);
    let declared = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/default/theme/sounds/move.wav"),
    )
    .unwrap();
    assert_eq!(host.ui.sound("move").as_deref(), Some(declared.as_slice()));
    assert_eq!(host.ui.sound("no-such-sound"), None);
}

#[test]
fn the_player_can_turn_sounds_off_on_the_settings_screen() {
    let mut host = tv();
    assert!(host.ui.sound("move").is_some());
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));

    host.first("sounds").invoke_accessible_default_action();
    host.pump();

    assert!(!host.core().view().settings.sounds);
    assert_eq!(host.ui.sound("move"), None);
}

#[test]
fn a_theme_whose_sound_file_is_missing_does_not_load_and_the_old_one_stays() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "quiet", "extends = \"default\"\n[theme.assets.sounds]\nclick = \"sounds/click.wav\"", &[]);
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();

    let err = host.ui.switch(&id("quiet")).unwrap_err();

    assert!(err.to_string().contains("click.wav"), "{err}");
    assert_eq!(host.ui.current(), Some(&id("default")));
}

#[test]
fn the_default_theme_has_upstream_s_five_sounds() {
    let host = tv();
    for name in ["move", "select", "back", "error", "start"] {
        let declared = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../plugins/themes/default/theme/sounds/{name}.wav")),
        )
        .unwrap();
        assert_eq!(host.ui.sound(name).as_deref(), Some(declared.as_slice()), "{name}");
    }
}

#[test]
fn opening_a_game_selects_and_leaving_the_library_goes_back() {
    let mut host = tv();
    settle(&mut host);
    host.requests.borrow_mut().clear();
    host.ui.pad(PadButton::South).unwrap();
    assert_eq!(asked_for(&host), ["select"], "Cross on the row opens the Game Hub");
    host.pump();
    settle(&mut host);

    host.requests.borrow_mut().clear();
    host.ui.pad(PadButton::East).unwrap();
    assert_eq!(asked_for(&host), ["back"], "Circle leaves the Game Hub");
    host.pump();
    settle(&mut host);

    // Up to the top bar, right to Library, Cross opens it; Circle leaves it.
    for button in [PadButton::Up, PadButton::Right] {
        host.ui.pad(button).unwrap();
    }
    host.requests.borrow_mut().clear();
    host.ui.pad(PadButton::South).unwrap();
    assert_eq!(asked_for(&host), ["select"], "the Library opens");
    settle(&mut host);
    host.requests.borrow_mut().clear();
    host.ui.pad(PadButton::East).unwrap();
    assert_eq!(asked_for(&host), ["back"], "and closes");

    // On Home's row, back goes nowhere: no sound.
    settle(&mut host);
    host.requests.borrow_mut().clear();
    host.ui.pad(PadButton::East).unwrap();
    assert_eq!(asked_for(&host), Vec::<String>::new());
}

#[test]
fn a_game_that_starts_plays_start_and_one_that_crashes_plays_error() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    settle(&mut host);
    host.requests.borrow_mut().clear();
    host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
    watch_until(&mut host, "the game is in front", |core| core.game_in_front());
    assert!(asked_for(&host).contains(&"start".to_owned()), "{:?}", asked_for(&host));

    host.requests.borrow_mut().clear();
    host.test.spawner.spawned()[0].exit(1);
    watch_until(&mut host, "the game closed", |core| !core.game_running());
    assert!(asked_for(&host).contains(&"error".to_owned()), "{:?}", asked_for(&host));
}

#[test]
fn a_notice_that_is_not_a_problem_plays_no_error() {
    let mut host = composed(|h| h.plugin(playable_games("emu-a", vec!["Alpha"])));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    settle(&mut host);
    host.test.windows.set_problem("Install xdotool so games come to the front and the launcher comes back");
    host.requests.borrow_mut().clear();
    host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
    watch_until(&mut host, "the notice shows", |core| !core.view().notices.is_empty());
    settle(&mut host);
    assert!(!asked_for(&host).contains(&"error".to_owned()), "advice: {:?}", asked_for(&host));
}

#[test]
fn a_problem_plays_error_once() {
    let mut host = tv();
    settle(&mut host);
    host.requests.borrow_mut().clear();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(asked_for(&host), ["error"]);
    host.requests.borrow_mut().clear();
    settle(&mut host);
    assert_eq!(asked_for(&host), Vec::<String>::new(), "once");
}
