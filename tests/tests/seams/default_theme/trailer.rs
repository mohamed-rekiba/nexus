//! The trailer: the button, the player and its controls.

use super::*;

#[test]
fn the_menu_watches_the_store_s_trailer() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
fn the_game_hub_has_a_trailer_button_when_the_game_has_a_trailer() {
    use slint::platform::Key;
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
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
