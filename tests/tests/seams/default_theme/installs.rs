//! Installs from Downloads: the install sheet, removing a download, and uninstalling.

use super::*;

/// The first bytes of a Linux program.
fn elf() -> Vec<u8> {
    let mut h = vec![0u8; 0x200];
    h[..4].copy_from_slice(b"\x7fELF");
    h[4] = 2;
    h[5] = 1;
    h[16..18].copy_from_slice(&2u16.to_le_bytes());
    h[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
    h
}

/// A PC game the catalog offers, downloaded and finished, on Downloads.
fn penguins_downloaded(home: &std::path::Path) -> Composed {
    let mut host = composed(|h| {
        h.home(home)
            .platform(Some(nexus_api::Platform::Linux))
            .built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "Penguins (v1.2) [amd64] [Native]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{ASTRO_HASH}"), "game_info": {} }] });
    host.test.http.serve("https://xyz.dev/c/pc.json", catalog.to_string());
    host.test.torrents.has(ASTRO_HASH, "Penguins", &[("Penguins/penguins.x86_64", &elf())]);
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    let offer = host.core().view().catalog.games.games[0].key.clone();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer.parse().unwrap(),
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    until_torrent(&mut host, "ready");
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::Start,
    }));
    host.test.torrents.complete(ASTRO_HASH);
    until_torrent(&mut host, "complete");
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    host
}

/// Waits until the first download's install is `state`.
fn until_install(host: &mut Composed, state: &str) {
    host.until(Area::Installs, |core| core.view().downloads.torrents.first().is_some_and(|t| t.install == state));
    settle(host);
    assert_eq!(host.core().view().downloads.torrents[0].install, state);
}

#[test]
fn install_asks_first_with_the_game_folder_and_what_the_emulator_installs_from() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());

    // Install asks first; Not now installs nothing.
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert_eq!(values(&host, "install-title"), ["Install game"]);
    assert_eq!(values(&host, "install-folder"), [native("~/Games/PC")]);
    let note = values(&host, "install-note");
    assert!(note[0].starts_with("Installs a PC game from its folder"), "{note:?}");
    host.first("install-not-now").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("install-agree"), 0);
    assert_eq!(host.core().view().downloads.torrents[0].install, "");

    // Install on the sheet installs, and the sheet closes once the install runs.
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(&mut host, "done");
    assert_eq!(host.count("install-agree"), 0, "the sheet closed");
    assert!(home.path().join("Games/PC/Penguins").is_dir());
}

#[test]
fn a_download_s_install_shows_on_its_card_only() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(&mut host, "done");
    assert_eq!(host.count("torrent"), 1, "the download's card");
    assert!(host.core().view().downloads.jobs.is_empty(), "no second card for the same install");
    assert!(values(&host, "download-state").is_empty());
}

#[test]
fn delete_files_removes_a_download_and_its_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));
    assert!(folder.is_dir(), "the downloaded files");
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::StopSeeding,
    }));

    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::DeleteFiles,
    }));
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(!folder.exists(), "its folder is deleted");
    assert!(home.path().join("Downloads/PC").is_dir(), "the downloads folder stays");
}

/// [`penguins_downloaded`], no longer sharing: its card has Remove.
fn penguins_to_remove(home: &std::path::Path) -> Composed {
    let mut host = penguins_downloaded(home);
    press(&mut host, "torrent-action:stop-seeding", ASTRO_HASH);
    assert_eq!(host.count("torrent-action:remove"), 1);
    host
}

#[test]
fn remove_asks_whether_to_keep_the_downloaded_files() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_to_remove(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));

    // Remove asks first; Not now (or Escape) changes nothing.
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    assert_eq!(values(&host, "remove-title"), ["Remove Penguins?"]);
    host.first("remove-not-now").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("remove-title"), 0);
    assert_eq!(host.core().view().downloads.torrents.len(), 1);
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    key(&mut host, slint::platform::Key::Escape);
    assert_eq!(host.count("remove-title"), 0, "Escape closes it");

    // Remove from Downloads forgets the download; its files stay.
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    host.first("remove-keep").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(folder.is_dir(), "the files stay");
    assert_eq!(host.count("remove-title"), 0);
}

#[test]
fn remove_and_delete_files_deletes_the_download_s_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_to_remove(home.path());
    let folder = home.path().join(format!("Downloads/PC/torrent-{ASTRO_HASH}"));
    press(&mut host, "torrent-action:remove", ASTRO_HASH);
    host.first("remove-delete").invoke_accessible_default_action();
    host.pump();
    settle(&mut host);
    assert!(host.core().view().downloads.torrents.is_empty());
    assert!(!folder.exists(), "its folder is deleted");
}

/// [`penguins_downloaded`], installed: its key in the Library.
fn penguins_installed(host: &mut Composed) -> String {
    press(host, "torrent-action:install", ASTRO_HASH);
    host.first("install-agree").invoke_accessible_default_action();
    host.pump();
    until_install(host, "done");
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    let library = host.core().view().home.games;
    library.iter().find(|g| g.title.contains("Penguins")).expect("the installed game").key.clone()
}

#[test]
fn an_installed_game_s_card_plays_it_its_menu_uninstalls_it_and_then_its_card_downloads_it() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);
    // Installed: the card's main action is Play, as on every installed game's card.
    let play = values(&host, "card-play");
    assert_eq!(play.len(), 1, "the installed game's card: {play:?}");
    assert!(play[0].starts_with("Play "), "{play:?}");
    assert!(values(&host, "card-uninstall").is_empty(), "Uninstall is never the card's button");

    // Uninstall is in the game's menu, on its Game Hub, and asks first.
    let card = host.core().view().catalog.games.games.into_iter().find(|g| g.title.contains("Penguins")).unwrap();
    host.dispatch(Command::OpenGame(card.key.parse().unwrap()));
    settle(&mut host);
    press(&mut host, "game-options", "Options");
    press(&mut host, "game-menu", "Uninstall");
    let asked = values(&host, "uninstall-title");
    assert_eq!(asked.len(), 1, "it asks first");
    assert!(folder.is_dir(), "nothing deleted yet");
    press(&mut host, "uninstall-choice", "Uninstall");
    assert!(!folder.exists(), "its folder is deleted");
    settle(&mut host);
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    settle(&mut host);

    assert!(values(&host, "card-play").is_empty());
    let download = values(&host, "card-get");
    assert!(!download.is_empty() && download.iter().all(|d| d.starts_with("Download ")), "{download:?}");
    press(&mut host, "card-get", &download[0]);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::GameDetail, "its Game Hub, to pick a release");
}

#[test]
fn uninstall_deletes_an_installed_game_s_folder() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let game = penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    assert!(folder.is_dir());

    host.dispatch(Command::UninstallGame(game.parse().unwrap()));
    assert!(!folder.exists(), "its folder is deleted");
    assert!(home.path().join("Games/PC").is_dir(), "the game folder stays");
    assert!(host.core().view().home.games.iter().all(|g| g.key != game), "it left the Library");
}

#[test]
fn the_game_s_menu_uninstalls_an_installed_game_after_asking() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    let game = penguins_installed(&mut host);
    let folder = home.path().join("Games/PC/Penguins");
    host.dispatch(Command::OpenGame(game.parse().unwrap()));
    settle(&mut host);

    // Options has Uninstall; it asks first, and Not now changes nothing.
    press(&mut host, "game-options", "Options");
    assert!(values(&host, "game-menu").contains(&"Uninstall".to_owned()));
    press(&mut host, "game-menu", "Uninstall");
    let asks = values(&host, "uninstall-title");
    assert!(asks.len() == 1 && asks[0].starts_with("Uninstall "), "{asks:?}");
    assert_eq!(focused(&host, "uninstall-choice"), ["Not now"], "the safe answer has the keys");
    press(&mut host, "uninstall-choice", "Not now");
    assert!(values(&host, "game-menu").is_empty(), "the menu closes");
    assert!(folder.is_dir());

    // Uninstall deletes it.
    press(&mut host, "game-options", "Options");
    press(&mut host, "game-menu", "Uninstall");
    press(&mut host, "uninstall-choice", "Uninstall");
    assert!(!folder.exists(), "its folder is deleted");
    assert_eq!(
        host.core().view().screen,
        nexus_ui_api::ScreenId::Downloads,
        "its Game Hub closed: back where it came from"
    );
}

#[test]
fn the_install_sheet_takes_a_typed_folder_and_keeps_its_error_open() {
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);

    // Up to the folder row; Enter opens it for typing, Enter again keeps the text.
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
    assert_eq!(values(&host, "install-folder"), [folder.to_str().unwrap()]);

    // The keys are back on Install: the core refuses the folder, and the sheet says why.
    let refused = key_refused(&mut host, Key::Return);
    assert!(refused.iter().any(|e| e.contains("isn't in a game folder")), "{refused:?}");
    assert_eq!(host.count("install-agree"), 1, "the sheet stays open");
    let error = values(&host, "install-error");
    assert!(error.first().is_some_and(|e| e.contains("Pick a folder inside a game folder")), "{error:?}");
    assert_eq!(host.core().view().downloads.torrents[0].install, "", "nothing installs");

    // Escape closes it.
    key(&mut host, Key::Escape);
    assert_eq!(host.count("install-agree"), 0);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Downloads, "still on Downloads");
}

#[test]
fn a_ready_download_lists_up_to_100_files_in_a_scroll_area() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_on_its_game_hub(home.path());
    let names: Vec<String> = (0..120).map(|i| format!("Astro/part{i:03}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    host.test.torrents.has(ASTRO_HASH, "Astro", &files);
    let offer = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer,
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    until_torrent(&mut host, "ready");
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-file-summary"), ["120 files · 0 MB · first 100 shown"]);
    // The list holds all 100; the testing backend finds only the rows in view.
    assert_eq!(values(&host, "torrent-files"), ["100"]);
    assert_eq!(values(&host, "torrent-file").first().map(String::as_str), Some("Astro/part000.bin · 0 MB"));
}

#[test]
fn while_it_looks_up_or_installs_a_card_shows_a_button_that_does_nothing_and_cancel_install() {
    let home = tempfile::tempdir().unwrap();
    let mut host = astro_with(home.path(), slow_installer());
    let held = host.test.torrents.hold();
    let offer: nexus_api::GameKey = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer,
        release: ASTRO_HASH.into(),
        folder: None,
    }));
    host.dispatch(Command::Navigate(nexus_core::Route::Downloads));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-placeholder"), ["Resolving…"]);
    // The keys skip it: the card's first button is Cancel.
    assert_eq!(host.core().view().downloads.torrents[0].actions, ["cancel"]);

    held.release();
    until_torrent(&mut host, "ready");
    assert_eq!(host.count("torrent-placeholder"), 0);
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: ASTRO_HASH.into(),
        action: nexus_core::TorrentAction::Start,
    }));
    host.test.torrents.complete(ASTRO_HASH);
    until_torrent(&mut host, "complete");
    host.dispatch(Command::GameInstall(nexus_core::GameInstallCmd::Download { id: ASTRO_HASH.into(), folder: None }));
    settle(&mut host);
    assert_eq!(values(&host, "torrent-placeholder"), ["Installing…"]);
    press(&mut host, "torrent-action:cancel-install", ASTRO_HASH);
    until_install(&mut host, "cancelled");
    assert_eq!(host.count("torrent-placeholder"), 0);
}

#[test]
fn right_to_left_the_install_sheet_has_install_on_the_right_and_the_keys_follow() {
    use slint::platform::Key;
    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert!(x_of(&host, "install-agree") < x_of(&host, "install-not-now"), "left to right, Install first");
    key(&mut host, Key::Escape);

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    assert!(x_of(&host, "install-agree") > x_of(&host, "install-not-now"), "right to left, Install on the right");
    // Right stays on Install, on the right; Left goes to Not now.
    key(&mut host, Key::RightArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        asked.iter().any(|c| matches!(c, Command::GameInstall(nexus_core::GameInstallCmd::Download { .. }))),
        "Enter installs: {asked:?}"
    );

    let home = tempfile::tempdir().unwrap();
    let mut host = penguins_downloaded(home.path());
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    settle(&mut host);
    press(&mut host, "torrent-action:install", ASTRO_HASH);
    key(&mut host, Key::LeftArrow);
    let asked = key_asks(&mut host, Key::Return);
    assert!(
        !asked.iter().any(|c| matches!(c, Command::GameInstall(nexus_core::GameInstallCmd::Download { .. }))),
        "Not now: {asked:?}"
    );
    host.pump();
    settle(&mut host);
    assert_eq!(host.count("install-agree"), 0, "Not now closed the sheet");
}
