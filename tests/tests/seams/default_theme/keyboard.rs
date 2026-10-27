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
        pad(host, button);
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
        if !values(host, "osk")[0].starts_with(layer) {
            pad(host, PadButton::RightShoulder);
        }
        if ch.is_ascii_uppercase() && !values(host, "osk")[0].ends_with("shift") {
            pad(host, PadButton::LeftShoulder);
        }
        osk_go(host, row, column);
        pad(host, PadButton::South);
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
    pad(host, nexus_ui_host::PadButton::South);
}

/// Characters typed on a physical keyboard.
fn typed(host: &mut Composed, text: &str) {
    for ch in text.chars() {
        let s: slint::SharedString = ch.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: s.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: s });
    }
    host.pump();
    settle(host);
}

/// The Library, opened with the controller.
fn library_by_pad() -> Composed {
    use nexus_ui_host::PadButton;
    let mut host = default_theme();
    pad(&mut host, PadButton::Up);
    pad(&mut host, PadButton::Right);
    pad(&mut host, PadButton::South);
    assert_eq!(host.count("default-game"), 3, "in the Library");
    host
}

#[test]
fn the_controller_alone_types_a_library_search_on_the_on_screen_keyboard() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    assert_eq!(host.count("osk"), 0);

    // Triangle focuses the search: the keyboard opens on q.
    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk"), ["letters"]);
    assert_eq!(osk_at(&host), (1, 0), "on q");
    let first: Vec<String> = values(&host, "osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "1234567890", "a numbers row of ten keys");
    let actions: Vec<String> = values(&host, "osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Done"], "no Show: not a password");

    osk_type(&mut host, "gam");
    assert_eq!(values(&host, "osk-text"), ["gam"]);
    assert_eq!(values(&host, "library-search"), ["gam"], "the field has it");

    // R2 is Done: the keyboard closes and the search runs.
    pad(&mut host, PadButton::RightTrigger);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(values(&host, "default-game"), ["Gamma"]);
}

#[test]
fn done_on_the_keyboard_and_options_end_the_typing_too() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "al");
    osk_action(&mut host, "Done");
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.core().view().home.library.query, "al");
    assert_eq!(values(&host, "default-game"), ["Alpha"]);

    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk-text"), ["al"], "it opens with the field's text");
    osk_type(&mut host, "x");
    pad(&mut host, PadButton::Start);
    assert_eq!(host.count("osk"), 0, "Options is Done");
    assert_eq!(host.core().view().home.library.query, "alx");
}

#[test]
fn a_physical_keyboard_types_straight_in_and_the_on_screen_keyboard_stays_shut() {
    let mut host = library_by_pad();
    // The controller came first; now the keyboard: / focuses the search.
    typed(&mut host, "/");
    assert_eq!(host.count("osk"), 0, "a key focused it");
    typed(&mut host, "gam");
    assert_eq!(values(&host, "library-search"), ["gam"]);
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.core().view().home.library.query, "gam");
    assert_eq!(values(&host, "default-game"), ["Gamma"]);
    assert_eq!(host.count("osk"), 0);
}

#[test]
fn typing_on_a_physical_keyboard_goes_in_while_the_on_screen_one_is_open_and_enter_is_done() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    pad(&mut host, PadButton::Right);
    assert_eq!(osk_at(&host), (1, 1), "on w");

    typed(&mut host, "be");
    assert_eq!(values(&host, "osk-text"), ["be"], "straight in");
    assert_eq!(osk_at(&host), (1, 1), "the highlight stays");
    key(&mut host, slint::platform::Key::Return);
    assert_eq!(host.count("osk"), 0, "Enter right after typing is Done");
    assert_eq!(host.core().view().home.library.query, "be");
    assert_eq!(values(&host, "default-game"), ["Beta"]);
}

#[test]
fn the_keyboard_s_shift_is_for_one_letter_and_r1_switches_to_the_symbols() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);

    // L1: Shift, for one letter.
    pad(&mut host, PadButton::LeftShoulder);
    assert_eq!(values(&host, "osk"), ["letters shift"]);
    assert!(values(&host, "osk-key").contains(&"Q".to_string()), "capitals on the keys");
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk"), ["letters"], "off after one letter");
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk-text"), ["Qq"]);

    // R1: the symbols, and back.
    pad(&mut host, PadButton::RightShoulder);
    assert_eq!(values(&host, "osk"), ["symbols"]);
    let first: Vec<String> = values(&host, "osk-key").into_iter().take(10).collect();
    assert_eq!(first.concat(), "!@#$%^&*()");
    assert!(values(&host, "osk-key").contains(&"ABC".to_string()), "the layer key goes back");
    pad(&mut host, PadButton::South);
    osk_type(&mut host, "€?");
    pad(&mut host, PadButton::RightShoulder);
    assert_eq!(values(&host, "osk"), ["letters"]);
    osk_type(&mut host, "Z");
    assert_eq!(values(&host, "osk-text"), ["Qq-€?Z"]);

    // Shift on the keyboard is a key too.
    osk_action(&mut host, "Shift");
    assert_eq!(values(&host, "osk"), ["letters shift"]);
}

#[test]
fn square_deletes_and_triangle_adds_a_space_as_their_keys_do() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "ab");
    pad(&mut host, PadButton::North);
    assert_eq!(values(&host, "osk-text"), ["ab "], "Triangle: a space");
    pad(&mut host, PadButton::West);
    pad(&mut host, PadButton::West);
    assert_eq!(values(&host, "osk-text"), ["a"], "Square: delete");

    osk_action(&mut host, "Space");
    osk_type(&mut host, "€");
    assert_eq!(values(&host, "osk-text"), ["a €"]);
    osk_action(&mut host, "Delete");
    assert_eq!(values(&host, "osk-text"), ["a "], "a symbol deletes whole");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    osk_action(&mut host, "Delete");
    assert_eq!(values(&host, "osk-text"), [""], "nothing left to delete");
}

#[test]
fn circle_closes_the_keyboard_and_keeps_the_text() {
    use nexus_ui_host::PadButton;
    let mut host = library_by_pad();
    pad(&mut host, PadButton::North);
    osk_type(&mut host, "ga");
    pad(&mut host, PadButton::East);
    assert_eq!(host.count("osk"), 0, "Circle closes it");
    assert_eq!(values(&host, "library-search"), ["ga"], "the text stays");
    assert_eq!(host.core().view().home.library.query, "ga", "and searches");

    // Cross on the field opens it again, with the text.
    pad(&mut host, PadButton::South);
    assert_eq!(values(&host, "osk-text"), ["ga"]);
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
    settle(&mut host);
    (dir, host)
}

#[test]
fn a_password_field_hides_its_text_and_the_show_key_shows_it() {
    use nexus_ui_host::PadButton;
    let (_dir, mut host) = with_a_password_field();
    // L1 three times: Settings, Themes, Emulators.
    for _ in 0..3 {
        pad(&mut host, PadButton::LeftShoulder);
    }
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Emulators);
    press(&mut host, "focus-password", "password");
    assert_eq!(host.count("osk"), 1, "the last input was the controller");
    let actions: Vec<String> = values(&host, "osk-key").into_iter().skip(40).collect();
    assert_eq!(actions, ["Shift", "Symbols", "Space", "Delete", "Show", "Done"]);

    osk_type(&mut host, "pw1");
    assert_eq!(values(&host, "osk-text"), ["●●●"]);
    assert_eq!(values(&host, "test-password"), ["●●●"], "the field hides it too");
    osk_action(&mut host, "Show");
    assert_eq!(values(&host, "osk-text"), ["pw1"]);
    assert_eq!(values(&host, "test-password"), ["pw1"]);
    assert!(values(&host, "osk-key").contains(&"Hide".to_string()));
    osk_action(&mut host, "Hide");
    assert_eq!(values(&host, "osk-text"), ["●●●"]);
}

#[test]
fn a_settings_text_box_opened_with_the_controller_types_on_the_keyboard_too() {
    use nexus_ui_host::PadButton;
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    settle(&mut host);
    // L1 from Home: Settings, on the game folders' row; Cross opens it for typing.
    pad(&mut host, PadButton::LeftShoulder);
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Settings);
    assert_eq!(host.count("folder:games"), 1);
    pad(&mut host, PadButton::South);
    assert_eq!(host.count("osk"), 1, "the text box opened the keyboard");
    let before = values(&host, "osk-text")[0].clone();
    let typed = full_path("/x");
    osk_type(&mut host, &typed);
    assert_eq!(values(&host, "editor"), [format!("{before}{typed}")]);

    // R2: Done sets the folders.
    pad(&mut host, PadButton::RightTrigger);
    assert_eq!(values(&host, "folder:games"), [format!("{before}{typed}")]);
    assert_eq!(host.count("osk"), 0);
    assert_eq!(host.count("editor"), 0, "the row is closed");
}
