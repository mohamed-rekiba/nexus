//! Playing a game: the launch splash, the running game and the time played.

use super::*;

#[test]
fn the_game_hub_stops_a_running_game() {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    host.settle();

    host.press("hub-stop", "Stop");
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
fn the_launch_splash_shows_the_emulator_s_keys_and_takes_every_key_but_back_and_quit() {
    use nexus_ui_host::UiRequest;
    use slint::platform::Key;
    let emulator = FakeCatalog::new("emu-a").playable().key("Cross", "J").key("Circle", "L").games(["Alpha"]);
    let mut host = composed(|h| h.plugin(emulator.build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // Play from the game's Game Hub. No window yet: the game stays "starting", behind the splash.
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    host.test.windows.no_windows();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.settle();
    assert_eq!(host.values("launch-key"), ["Cross J", "Circle L"]);
    assert!(host.values("hint").is_empty(), "no launcher hints: {:?}", host.values("hint"));

    // The keys go to the game, not to the launcher behind the splash.
    let screen = host.core().view().screen;
    host.requests.borrow_mut().clear();
    for k in [Key::DownArrow, Key::Return, Key::Backspace] {
        host.tap(k);
    }
    assert!(host.requests.borrow().is_empty(), "no sound and no command: {:?}", host.requests.borrow());
    host.settle();
    assert_eq!(host.core().view().screen, screen, "nothing opened or closed");

    // Escape goes on, as on the splash without the keys: the second one is Back, to the Game
    // Hub.
    host.navigate(Key::Escape);
    host.navigate(Key::Escape);
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
fn the_launch_splash_names_the_controller_or_the_keyboard() {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // No window yet: the game stays "starting", behind the splash.
    host.test.windows.no_windows();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.settle();
    assert_eq!(host.values("launch-input"), ["Keyboard: the keys are on the emu-a page"]);

    host.ui.set_pad_name(Some("DualSense Wireless Controller".into()));
    host.ui.render().unwrap();
    host.settle();
    assert_eq!(host.values("launch-input"), ["Controller: DualSense Wireless Controller"]);
}

#[test]
fn the_hero_and_the_game_hub_show_the_time_played() {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build()));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let stats = serde_json::json!({ "secs": 4 * 3600 + 6 * 60, "count": 2, "last": now });
    host.core().kernel().config().set(&["app", "playtime", "emu-a:G0"], stats).unwrap();
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G0".parse().unwrap()));
    host.settle();
    assert_eq!(host.values("default-played"), ["PLAYED 4 HR 6 MIN"], "over the title");
    assert_eq!(host.values("default-sub"), ["Last played today"], "beside the tile");

    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    host.settle();
    assert_eq!(host.values("played"), ["4 hr 6 min · Today"], "one card: how long, and when last");
}

/// The Classic theme on two playable systems: PS4 with Alpha and Beta, PS5 with Gamma.
fn playable_classic() -> Composed {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").playable().system("PS4").games(["Alpha", "Beta"]).build())
            .plugin(FakeCatalog::new("emu-b").playable().system("PS5").games(["Gamma"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

/// Starts `game`, as Play does, and ends it once the theme has shown it running.
fn play_and_end(host: &mut Composed, game: &str, while_running: impl FnOnce(&mut Composed)) {
    host.dispatch(Command::Session(SessionCmd::Launch(game.parse().unwrap())));
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

#[test]
fn a_game_played_from_the_library_comes_back_to_it_when_it_ends() {
    let mut host = playable_classic();
    host.press("default-system", "Library");
    host.press_first("filter:PS4");

    play_and_end(&mut host, "emu-a:G1", |host| {
        assert_eq!(host.values("default-tile").len(), 3, "while it runs, Home shows it with Resume");
    });

    assert!(host.values("default-tile").is_empty(), "back in the Library");
    assert_eq!(host.values("default-count"), ["2 games"], "still on the PS4 games");
    let library = host.core().view().home.library;
    let selected = library.selected.map(|i| library.games[i].title.clone());
    assert_eq!(selected.as_deref(), Some("Beta"), "on the game that was played");
}

#[test]
fn a_player_who_moves_on_home_while_the_game_runs_stays_there() {
    let mut host = playable_classic();
    host.press("default-system", "Library");

    play_and_end(&mut host, "emu-a:G1", |host| host.press("default-system", "Home"));

    assert_eq!(host.values("default-tile").len(), 3, "still on Home, where they went");
}
