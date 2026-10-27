//! The Game Hub: its menu, its screenshots and its cover.

use super::*;

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
