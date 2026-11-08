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
