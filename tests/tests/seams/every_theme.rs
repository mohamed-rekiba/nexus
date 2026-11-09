//! Seam 3: one suite run against every built-in theme. Each theme stands alone and draws Settings,
//! the Game Hub, Downloads and the on-screen keyboard with the kit, so these work the same way
//! in each.

use crate::common::*;
use nexus_core::{Command, Route};
use nexus_ui_api::ScreenId;
use nexus_ui_host::PadButton;
use slint::platform::Key;
use slint_interpreter::ComponentHandle;

/// Two emulators' games.
fn two_emulators(h: nexus_testkit::FakeHost) -> nexus_testkit::FakeHost {
    h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
        .plugin(FakeCatalog::new("emu-b").system("PS5").games(["Gamma"]).build())
}

/// The built-in themes.
const THEMES: [&str; 2] = [theme_classic::ID, theme_default::ID];

/// Nexus in `theme` with the plugins `build` adds, at 1600×900.
fn in_theme(theme: &str, build: impl FnOnce(nexus_testkit::FakeHost) -> nexus_testkit::FakeHost) -> Composed {
    let mut host = composed_first_run(build);
    host.ui.start_with(Some(id(theme))).unwrap();
    assert_eq!(host.ui.current(), Some(&id(theme)));
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    host
}

#[test]
fn every_screen_renders_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| {
            two_emulators(h).built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        for (route, screen) in [
            (Route::Catalog, ScreenId::Catalog),
            (Route::Downloads, ScreenId::Downloads),
            (Route::Settings, ScreenId::Settings),
            (Route::Emulators, ScreenId::Emulators),
            (Route::Themes, ScreenId::Themes),
            (Route::Plugin(id("emu-a")), ScreenId::Plugin),
            (Route::Home, ScreenId::Home),
        ] {
            host.dispatch(Command::Navigate(route));
            host.settle();
            assert_eq!(host.core().view().screen, screen, "{theme}");
        }
        host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
        host.settle();
        assert_eq!(host.core().view().screen, ScreenId::GameDetail, "{theme}");
    }
}

#[test]
fn the_keys_move_through_the_settings_tabs_and_act_on_a_row_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, two_emulators);
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        // The Games tab, down to its first row: Enter on "Rescan game folders" scans again.
        host.settings_tab("Games");
        host.navigate(Key::DownArrow);
        // A press alone: `navigate` would carry out the request before the check.
        let text: slint::SharedString = Key::Return.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
        let asked =
            host.requests.borrow().iter().any(|r| {
                matches!(r, nexus_ui_host::UiRequest::Core(Command::Library(nexus_core::LibraryCmd::Refresh)))
            });
        assert!(asked, "{theme}: Enter on Rescan asks the core to scan");
        host.pump();
        // Up to the tabs, Right twice to Downloads: its rows, and no other tab's.
        host.navigate(Key::UpArrow);
        host.navigate(Key::RightArrow);
        host.navigate(Key::RightArrow);
        assert_eq!(host.count("seed-downloads"), 1, "{theme}");
        assert_eq!(host.count("sounds"), 0, "{theme}");
    }
}

#[test]
fn the_backup_tab_backs_up_and_lists_the_file_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = in_theme(theme, two_emulators);
        let folder = serde_json::json!(backups.path().display().to_string());
        host.dispatch(Command::Pref(nexus_core::Pref::BackupFolder, folder));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Backup");
        host.pump();
        host.settle();
        assert_eq!(host.count("backup"), 0, "{theme}");
        host.press_first("back-up");
        host.settle();
        assert_eq!(host.count("backup"), 1, "{theme}: the new backup is listed");
        host.press_first("backup-browse");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: Restore from another folder opens the browser");
        assert_eq!(host.value("backup-browser"), backups.path().display().to_string(), "{theme}: at the backup folder");
    }
}

#[test]
fn a_settings_text_box_opens_the_on_screen_keyboard_and_circle_closes_it_in_every_theme() {
    for theme in THEMES {
        let mut host = in_theme(theme, |h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
        host.dispatch(Command::Navigate(Route::Settings));
        host.settle();
        host.settings_tab("Games");
        host.navigate(Key::DownArrow);
        assert_eq!(host.count("folder:games"), 1, "{theme}");
        host.pad(PadButton::South);
        assert_eq!(host.count("osk"), 1, "{theme}: the text box opened the keyboard");
        host.pad(PadButton::East);
        assert_eq!(host.count("osk"), 0, "{theme}: Circle closed it");
    }
}

/// Nexus in `theme`, with `backups` as the backup folder, a backup in `backups/old-pc`, and the
/// browser open at `backups`.
fn browsing(theme: &str, backups: &std::path::Path) -> Composed {
    use nexus_core::BackupCmd;
    let mut host = in_theme(theme, two_emulators);
    let old_pc = backups.join("old-pc");
    let folder = serde_json::json!(old_pc.display().to_string());
    host.dispatch(Command::Pref(nexus_core::Pref::BackupFolder, folder));
    host.dispatch(Command::Backup(BackupCmd::BackUp { large: false }));
    std::fs::create_dir_all(backups.join("photos")).unwrap();
    let folder = serde_json::json!(backups.display().to_string());
    host.dispatch(Command::Pref(nexus_core::Pref::BackupFolder, folder));
    host.dispatch(Command::Backup(BackupCmd::Browse(Some(backups.into()))));
    host.settle();
    host
}

/// A key as the window gets it, then carried out: "/" is △ on a pad.
fn text(host: &mut Composed, text: &str) {
    host.type_text(text);
}

#[test]
fn the_backup_browser_s_keys_move_open_and_go_up_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        assert_eq!(host.count("backup-browser"), 1, "{theme}: over everything");
        assert_eq!(host.values("browser-folder"), ["old-pc", "photos"], "{theme}: the folder with a backup first");
        assert_eq!(host.focused("browser-folder"), ["old-pc"], "{theme}");
        assert_eq!(host.description("browser-folder"), "1 backup", "{theme}");

        host.navigate(Key::DownArrow);
        assert_eq!(host.focused("browser-folder"), ["photos"], "{theme}");
        host.navigate(Key::UpArrow);
        host.navigate(Key::Return);
        assert!(host.value("backup-browser").ends_with("old-pc"), "{theme}: Enter opens the folder");
        assert_eq!(host.count("browser-backup"), 1, "{theme}");
        assert_eq!(host.values("browser-crumb").last().map(String::as_str), Some("old-pc"), "{theme}");

        host.navigate(Key::Escape);
        assert_eq!(host.value("backup-browser"), backups.path().display().to_string(), "{theme}: ○ goes up a folder");

        // The places: Left goes there, Enter goes to one, ○ there closes.
        host.navigate(Key::LeftArrow);
        assert_eq!(host.focused("browser-place").len(), 1, "{theme}: the places have the focus");
        assert!(host.focused("browser-folder").is_empty(), "{theme}");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("browser-folder").len(), 1, "{theme}: back in the files");
        host.navigate(Key::LeftArrow);
        host.navigate(Key::Escape);
        assert_eq!(host.count("backup-browser"), 0, "{theme}: closed");
        assert!(!host.core().view().settings.backup.browser.open, "{theme}");
    }
}

#[test]
fn typing_filters_the_backup_browser_and_escape_clears_it_first_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        text(&mut host, "PHO");
        assert_eq!(host.value("browser-filter"), "PHO", "{theme}: every key typed before the core answered");
        assert_eq!(host.values("browser-folder"), ["photos"], "{theme}: in any case");
        host.navigate(Key::Backspace);
        assert_eq!(host.value("browser-filter"), "PH", "{theme}");
        text(&mut host, "zz");
        assert_eq!(host.count("browser-folder"), 0, "{theme}");
        assert_eq!(host.count("browser-empty"), 1, "{theme}: it says nothing holds it");
        host.navigate(Key::Escape);
        assert_eq!(host.count("browser-filter"), 0, "{theme}: ○ clears the text first");
        assert_eq!(host.count("browser-folder"), 2, "{theme}");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: and stays open");
    }
}

#[test]
fn a_restore_from_the_backup_browser_asks_once_then_restores_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        host.navigate(Key::Return);
        assert_eq!(host.focused("browser-backup").len(), 1, "{theme}");

        // △ asks; ○ says no, and nothing is restored.
        text(&mut host, "/");
        assert_eq!(host.count("browser-confirm"), 1, "{theme}: △ asks first");
        host.navigate(Key::Escape);
        assert_eq!(host.count("browser-confirm"), 0, "{theme}");
        assert_eq!(host.count("backup-browser"), 1, "{theme}: the browser stays");

        // Enter asks too; Right goes to Cancel, Enter on it says no.
        host.navigate(Key::Return);
        assert_eq!(host.focused("browser-confirm-restore").len(), 1, "{theme}: Restore is first");
        host.navigate(Key::RightArrow);
        assert_eq!(host.focused("browser-confirm-cancel").len(), 1, "{theme}");
        host.navigate(Key::Return);
        assert_eq!(host.count("browser-confirm"), 0, "{theme}");
        assert!(!host.core().view().settings.backup.restart, "{theme}: nothing restored");

        // Restore: the backup goes back, and the browser closes.
        host.navigate(Key::Return);
        host.navigate(Key::Return);
        host.until(nexus_core::Area::Backup, |core| core.view().settings.backup.restart);
        host.settle();
        assert_eq!(host.count("backup-browser"), 0, "{theme}: a restore closes the browser");
    }
}

#[test]
fn the_mouse_focuses_with_a_click_and_opens_with_the_default_action_in_every_theme() {
    for theme in THEMES {
        let backups = tempfile::tempdir().unwrap();
        let mut host = browsing(theme, backups.path());
        host.click("browser-place");
        assert_eq!(host.focused("browser-place").len(), 1, "{theme}: a click focuses");
        assert!(
            host.value("backup-browser").ends_with(&*backups.path().display().to_string()),
            "{theme}: and opens nothing"
        );
        host.press("browser-folder", "photos");
        assert!(host.value("backup-browser").ends_with("photos"), "{theme}");
        host.press_first("browser-close");
        assert_eq!(host.count("backup-browser"), 0, "{theme}");
    }
}
