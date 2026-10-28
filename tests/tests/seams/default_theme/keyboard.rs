//! The on-screen keyboard and the physical one.

use super::*;

// The on-screen keyboard: ten keys a row, a numbers row and three letter rows (or the symbols
// layer), then the action row. Its keys are `osk-key`, valued with what they show; the
// highlighted one is selected, and its item index is row * 10 + column (the action row is 4).

const OSK_LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.-"];

const OSK_SYMBOLS: [&str; 4] = ["!@#$%^&*()", "-_=+[]{}\\|", ";:\"<>/?~`€", "£¥§°•«»¿¡ "];

/// The highlighted key: its row and column.
fn osk_at(host: &Composed) -> (usize, usize) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "osk-key");
    let lit: Vec<_> = found.filter(|e| e.accessible_item_selected() == Some(true)).collect();
    let [key] = lit.as_slice() else { panic!("one highlighted key, got {}", lit.len()) };
    let index = key.accessible_item_index().expect("a key has its place");
    (index / 10, index % 10)
}

/// The d-pad to the key at `row`, `column`, one step at a time.
fn osk_go(host: &mut Composed, row: usize, column: usize) {
    use nexus_ui_host::PadButton;
    for _ in 0..40 {
        let (r, c) = osk_at(host);
        if (r, c) == (row, column) {
            return;
        }
        let button = if r < row {
            PadButton::Down
        } else if r > row {
            PadButton::Up
        } else if c < column {
            PadButton::Right
        } else {
            PadButton::Left
        };
        host.pad(button);
    }
    panic!("the highlight never reached {row}, {column}");
}

/// Types `text` with the controller alone: R1 for the layer, L1 for a capital, the d-pad to the
/// key and Cross.
fn osk_type(host: &mut Composed, text: &str) {
    use nexus_ui_host::PadButton;
    let find = |rows: [&str; 4], ch: char| {
        rows.iter().enumerate().find_map(|(r, row)| row.chars().position(|c| c == ch).map(|c| (r, c)))
    };
    for ch in text.chars() {
        let (symbols, (row, column)) = match find(OSK_LETTERS, ch.to_ascii_lowercase()) {
            Some(at) => (false, at),
            None => (true, find(OSK_SYMBOLS, ch).unwrap_or_else(|| panic!("no key for {ch:?}"))),
        };
        let layer = if symbols { "symbols" } else { "letters" };
        if !host.values("osk")[0].starts_with(layer) {
            host.pad(PadButton::RightShoulder);
        }
        if ch.is_ascii_uppercase() && !host.values("osk")[0].ends_with("shift") {
            host.pad(PadButton::LeftShoulder);
        }
        osk_go(host, row, column);
        host.pad(PadButton::South);
    }
}

/// The d-pad to the action key that shows `label`, and Cross.
fn osk_action(host: &mut Composed, label: &str) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "osk-key");
    let key = found
        .into_iter()
        .find(|e| e.accessible_value().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} key"));
    let index = key.accessible_item_index().unwrap();
    osk_go(host, index / 10, index % 10);
    host.pad(nexus_ui_host::PadButton::South);
}

/// The Library, opened with the controller.
fn library_by_pad() -> Composed {
    use nexus_ui_host::PadButton;
    let mut host = default_theme();
    host.pad(PadButton::Up);
    host.pad(PadButton::Right);
    host.pad(PadButton::South);
    assert_eq!(host.count("default-game"), 3, "in the Library");
    host
}

#[test]
fn the_controller_alone_types_a_library_search_on_the_on_screen_keyboard() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    assert_eq!(host.count("osk"), 0);

    // Triangle focuses the search: the keyboard opens on q.
    host.pad(PadButton::North);
    assert_eq!(host.values("osk"), ["letters"]);
    assert_eq!(osk_at(&host), (1, 0), "on q");
    let first: Vec<String> = host.values("osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "1234567890", "a numbers row of ten keys");
    let actions: Vec<String> = host.values("osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Done"], "no Show: not a password");

    osk_type(&mut host, "gam");
    assert_eq!(host.values("osk-text"), ["gam"]);
    assert_eq!(host.values("library-search"), ["gam"], "the field has it");

    // R2 is Done: the keyboard closes and the search runs.
    host.pad(PadButton::RightTrigger);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(host.values("default-game"), ["Gamma"]);
}

#[test]
fn done_on_the_keyboard_and_options_end_the_typing_too() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    host.pad(PadButton::North);
    osk_type(&mut host, "al");
    osk_action(&mut host, "Done");
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "al");
    assert_eq!(host.values("default-game"), ["Alpha"]);

    host.pad(PadButton::North);
    assert_eq!(host.values("osk-text"), ["al"], "it opens with the field's text");
    osk_type(&mut host, "x");
    host.pad(PadButton::Start);
    assert_eq!(host.count("osk"), 0, "Options is Done");
    assert_eq!(host.core().view().home.library.query, "alx");
}

#[test]
fn a_physical_keyboard_types_straight_in_and_the_on_screen_keyboard_stays_shut() {
    let mut host = library_by_pad();
    // The controller came first; now the keyboard: / focuses the search.
    host.type_text("/");
    assert_eq!(host.count("osk"), 0, "a key focused it");
    host.type_text("gam");
    assert_eq!(host.values("library-search"), ["gam"]);
    host.navigate(slint::platform::Key::Return);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(host.values("default-game"), ["Gamma"]);
    assert_eq!(host.count("osk"), 0);
}

#[test]
fn typing_on_a_physical_keyboard_goes_in_while_the_on_screen_one_is_open_and_enter_is_done() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    host.pad(PadButton::North);
    host.pad(PadButton::Right);
    assert_eq!(osk_at(&host), (1, 1), "on w");

    host.type_text("be");
    assert_eq!(host.values("osk-text"), ["be"], "straight in");
    assert_eq!(osk_at(&host), (1, 1), "the highlight stays");
    host.navigate(slint::platform::Key::Return);
    assert_eq!(host.count("osk"), 0, "Enter right after typing is Done");
    assert_eq!(host.core().view().home.library.query, "be");
    assert_eq!(host.values("default-game"), ["Beta"]);
}

#[test]
fn the_keyboard_s_shift_is_for_one_letter_and_r1_switches_to_the_symbols() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    host.pad(PadButton::North);

    // L1: Shift, for one letter.
    host.pad(PadButton::LeftShoulder);
    assert_eq!(host.values("osk"), ["letters shift"]);
    assert!(host.values("osk-key").contains(&"Q".to_string()), "capitals on the keys");
    host.pad(PadButton::South);
    assert_eq!(host.values("osk"), ["letters"], "off after one letter");
    host.pad(PadButton::South);
    assert_eq!(host.values("osk-text"), ["Qq"]);

    // R1: the symbols, and back.
    host.pad(PadButton::RightShoulder);
    assert_eq!(host.values("osk"), ["symbols"]);
    let first: Vec<String> = host.values("osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "!@#$%^&*()");
    assert!(host.values("osk-key").contains(&"ABC".to_string()), "the layer key goes back");
    host.pad(PadButton::South);
    osk_type(&mut host, "€?");
    host.pad(PadButton::RightShoulder);
    assert_eq!(host.values("osk"), ["letters"]);
    osk_type(&mut host, "Z");
    assert_eq!(host.values("osk-text"), ["Qq-€?Z"]);

    // Shift on the keyboard is a key too.
    osk_action(&mut host, "Shift");
    assert_eq!(host.values("osk"), ["letters shift"]);
}

#[test]
fn square_deletes_and_triangle_adds_a_space_as_their_keys_do() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    host.pad(PadButton::North);
    osk_type(&mut host, "ab");
    host.pad(PadButton::North);
    assert_eq!(host.values("osk-text"), ["ab "], "Triangle: a space");
    host.pad(PadButton::West);
    host.pad(PadButton::West);
    assert_eq!(host.values("osk-text"), ["a"], "Square: delete");

    osk_action(&mut host, "Space");
    osk_type(&mut host, "€");
    assert_eq!(host.values("osk-text"), ["a €"]);
    osk_action(&mut host, "Delete");
    assert_eq!(host.values("osk-text"), ["a "], "a symbol deletes whole");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    assert_eq!(host.values("osk-text"), [""], "nothing left to delete");
}

#[test]
fn circle_closes_the_keyboard_and_keeps_the_text() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    host.pad(PadButton::North);
    osk_type(&mut host, "ga");
    host.pad(PadButton::East);
    assert_eq!(host.count("osk"), 0, "Circle closes it");
    assert_eq!(host.values("library-search"), ["ga"], "the text stays");
    assert_eq!(host.core().view().home.library.query, "ga", "and searches");

    // Cross on the field opens it again, with the text.
    host.pad(PadButton::South);
    assert_eq!(host.values("osk-text"), ["ga"]);
}

/// A theme for tests: the default one with a password field on the Emulators screen. Its
/// `focus-password` button focuses the field.
fn with_a_password_field() -> (tempfile::TempDir, Composed) {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "secrets",
        "extends = \"default\"\nscreens = [\"emulators\"]",
        &[(
            "screens/emulators.slint",
            r#"import { TextField } from "../components/keyboard.slint";
export component EmulatorsScreen inherits Rectangle {
    in property <bool> open;
    in property <bool> pushed;
    field := TextField { x: 100px; y: 100px; width: 400px; height: 50px; name: "test-password"; password: true; }
    Rectangle {
        accessible-role: button;
        accessible-label: "focus-password";
        accessible-value: "password";
        accessible-action-default => { field.focus(); }
    }
}
"#,
        )],
    );
    let mut host =
        composed(|h| h.discover(dir.path()).plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build()));
    host.ui.switch(&id("secrets")).expect("the test theme loads");
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.resize(1600.0, 900.0);
    host.settle();
    (dir, host)
}

#[test]
fn a_password_field_hides_its_text_and_the_show_key_shows_it() {
    use nexus_ui_host::PadButton;
    let (_dir, mut host) = with_a_password_field();
    // L1 three times: Settings, Themes, Emulators.
    for _ in 0..3 {
        host.pad(PadButton::LeftShoulder);
    }
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Emulators);
    host.press("focus-password", "password");
    assert_eq!(host.count("osk"), 1, "the last input was the controller");
    let actions: Vec<String> = host.values("osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Show", "Done"]);

    osk_type(&mut host, "pw1");
    assert_eq!(host.values("osk-text"), ["●●●"]);
    assert_eq!(host.values("test-password"), ["●●●"], "the field hides it too");
    osk_action(&mut host, "Show");
    assert_eq!(host.values("osk-text"), ["pw1"]);
    assert_eq!(host.values("test-password"), ["pw1"]);
    assert!(host.values("osk-key").contains(&"Hide".to_string()));
    osk_action(&mut host, "Hide");
    assert_eq!(host.values("osk-text"), ["●●●"]);
}

#[test]
fn a_settings_text_box_opened_with_the_controller_types_on_the_keyboard_too() {
    use nexus_ui_host::PadButton;
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    host.settle();
    // L1 from Home: Settings, on its first tab. The Games tab, down to the game folders' row;
    // Cross opens it for typing.
    host.pad(PadButton::LeftShoulder);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Settings);
    host.settings_tab("Games");
    host.navigate(slint::platform::Key::DownArrow);
    assert_eq!(host.count("folder:games"), 1);
    host.pad(PadButton::South);
    assert_eq!(host.count("osk"), 1, "the text box opened the keyboard");
    let before = host.values("osk-text")[0].clone();
    let typed = full_path("/x");
    osk_type(&mut host, &typed);
    assert_eq!(host.values("editor"), [format!("{before}{typed}")]);

    // R2: Done sets the folders.
    host.pad(PadButton::RightTrigger);
    assert_eq!(host.values("folder:games"), [format!("{before}{typed}")]);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.count("editor"), 0, "the row is closed");
}
