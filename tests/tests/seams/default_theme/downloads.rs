//! Downloads: the download sheet, each download's card and its buttons.

use super::*;

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
