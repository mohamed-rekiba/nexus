//! Seam 3, composed host: plugin slot items render in every theme, laid out by the theme.

use crate::common::*;
use nexus_core::Command;
use slint_interpreter::Value;

const GAMES: [&str; 3] = ["Alpha", "Beta", "Gamma"];

fn shown(theme: &str, width: f32, height: f32) -> Composed {
    let mut host = composed(|h| {
        h.discover(&fixture_themes()).plugin(FakeCatalog::new("emu-a").system("PS4").games(GAMES.to_vec()).build())
    });
    host.ui.start().unwrap();
    host.ui.switch(&id(theme)).unwrap();
    host.resize(width, height);
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    if theme == "default" {
        // The default theme shows games, and their badges, in the Library.
        let tabs = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "default-system");
        let library =
            tabs.into_iter().find(|t| t.accessible_value().as_deref() == Some("Library")).expect("a Library tab");
        library.invoke_accessible_default_action();
        host.pump();
        host.settle();
    }
    host
}

#[test]
fn the_same_plugin_badges_render_in_both_themes_in_different_places() {
    let mut positions = Vec::new();
    for theme in ["fixture-grid", "default"] {
        let host = shown(theme, 1600.0, 900.0);
        assert_eq!(host.count("emu-a:region"), GAMES.len(), "{theme}: one region badge per game");
        let badge = host.first("emu-a:region");
        positions.push((badge.absolute_position().x, badge.absolute_position().y));
    }
    assert_ne!(positions[0], positions[1], "the themes lay the same badge out differently");
}

#[test]
fn slot_actions_and_sections_render_on_the_detail_screen_of_both_themes() {
    for theme in ["fixture-grid", "default"] {
        let mut host = shown(theme, 1600.0, 900.0);
        host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
        assert_eq!(host.count("emu-a:verify"), 1, "{theme}: the action button");
        assert_eq!(host.count("emu-a:info"), 1, "{theme}: the section");
        assert!(host.count("emu-a:region") >= 1, "{theme}: the badge");

        // Picking the action reaches the plugin through the core. What the screen asked for
        // before (the grid's covers) is handled first.
        host.pump();
        host.first("emu-a:verify").invoke_accessible_default_action();
        assert!(matches!(
            host.requests.borrow().as_slice(),
            [nexus_ui_host::UiRequest::Core(Command::RunAction { action, .. })] if action == "verify"
        ));
        host.pump();
    }
}

#[test]
fn the_desktop_grid_reflows_at_its_compact_breakpoint() {
    let wide = shown("fixture-grid", 1600.0, 900.0);
    let compact = shown("fixture-grid", 800.0, 700.0);
    let breakpoint = |h: &Composed| match vm_field(h, "viewport", "breakpoint") {
        Value::String(s) => s.to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!(breakpoint(&wide), "wide");
    assert_eq!(breakpoint(&compact), "compact");

    let y = |h: &Composed, title: &str| h.first(title).absolute_position().y;
    assert_eq!(y(&wide, "Alpha"), y(&wide, "Gamma"), "wide: one row");
    assert_eq!(y(&compact, "Alpha"), y(&compact, "Beta"), "compact: two per row");
    assert!(y(&compact, "Gamma") > y(&compact, "Alpha"), "compact: the third game wraps");
}

/// An emulator whose manifest lists its keyboard keys plays with a controller or the keyboard.
#[test]
fn default_theme_shows_the_controllers_of_an_emulator_with_keyboard_keys() {
    let mut host =
        composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().key("Cross", "J").games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));

    // The controllers are on the emulator's page.
    host.dispatch(Command::Navigate(nexus_core::Route::Plugin("emu-a".parse().unwrap())));
    let profiles = host.first("controllers:emu-a").accessible_value().unwrap_or_default();
    assert_eq!(profiles.as_str(), "Controllers: Controller (DualSense, DualShock 4, Xbox and most others), Keyboard");
}
