//! Seam 3, composed host: switching themes at runtime.

use crate::common::*;
use futures::StreamExt;
use nexus_api::Event;
use nexus_core::{Command, Route, SessionCmd};
use nexus_ui_api::ThemeError;
use nexus_ui_host::{UiRequest, vm_text};
use slint_interpreter::{ComponentHandle, Value};

/// Every event published so far on a stream subscribed earlier, without waiting.
fn drain(stream: &mut futures::stream::BoxStream<'static, Event>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(Some(event)) = futures::FutureExt::now_or_never(stream.next()) {
        out.push(event);
    }
    out
}

fn with_examples(h: nexus_testkit::FakeHost) -> nexus_testkit::FakeHost {
    h.discover(&fixture_themes()).plugin(FakeCatalog::new("emu-a").playable().games(["Alpha", "Beta", "Gamma"]).build())
}

/// A `Vm` struct property's field, from the live instance. The switching tests read the `Vm`
/// globals themselves, where every other seam test reads `core.view()`: what they check is
/// that a new theme's tree is bound with the state the old one showed, and the core's view
/// doesn't change when the theme does.
pub fn vm_field(host: &Composed, property: &str, field: &str) -> slint_interpreter::Value {
    match host.instance().get_global_property("Vm", property).unwrap() {
        slint_interpreter::Value::Struct(s) => s.get_field(field).cloned().unwrap_or_default(),
        other => panic!("Vm.{property} is {other:?}"),
    }
}

#[test]
fn switching_keeps_screen_selection_and_running_games() {
    let mut host = composed(with_examples);
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G2".parse().unwrap())));
    host.dispatch(Command::OpenGame("emu-a:G1".parse().unwrap()));

    for theme in ["fixture-grid", "default"] {
        let before = host.instance().as_weak();
        host.ui.switch(&id(theme)).unwrap_or_else(|e| panic!("{theme}: {e}"));
        assert!(before.upgrade().is_none(), "{theme}: the old view tree was released");
        assert_eq!(host.ui.current(), Some(&id(theme)));

        // Read back through the new instance's globals: the new tree shows the same state.
        assert_eq!(vm_text(host.instance(), "screen").as_deref(), Some("game-detail"), "{theme}");
        assert_eq!(vm_text(host.instance(), "game").as_deref(), Some("emu-a:G1"), "{theme}");
        assert_eq!(vm_field(&host, "home", "selected"), Value::Number(1.0), "{theme}");
        assert_eq!(host.instance().get_global_property("Vm", "has-running").unwrap(), Value::Bool(true), "{theme}");
        assert_eq!(vm_field(&host, "running", "title"), Value::String("Gamma".into()), "{theme}");
    }
    assert_eq!(host.test.spawner.spawned().len(), 1, "the game kept running through every switch");
}

#[test]
fn a_switch_emits_theme_changed_and_saves_the_choice() {
    let mut host = composed(with_examples);
    host.ui.start().unwrap();
    let mut events = host.core().events();

    // The theme asks through its Host callback; the app carries it out.
    host.instance().invoke_global("Host", "switch-theme", &[Value::String("fixture-grid".into())]).unwrap();
    assert_eq!(*host.requests.borrow(), [UiRequest::SwitchTheme(id("fixture-grid"))]);
    host.pump();

    let changed: Vec<_> = drain(&mut events).into_iter().filter(|e| matches!(e, Event::ThemeChanged { .. })).collect();
    assert_eq!(changed, [Event::ThemeChanged { from: Some(id("default")), to: id("fixture-grid") }]);
    assert_eq!(host.core().kernel().config().get(&["app", "theme"]), serde_json::json!("fixture-grid"));
    assert_eq!(host.core().selected_theme(), Some(id("fixture-grid")));
}

fn assert_rolled_back(host: &Composed, events: &mut futures::stream::BoxStream<'static, Event>, err: ThemeError) {
    assert_eq!(host.ui.current(), Some(&id("fixture-grid")), "the old theme stays: {err}");
    assert_eq!(vm_text(host.instance(), "screen").as_deref(), Some("downloads"));
    assert!(drain(events).iter().all(|e| !matches!(e, Event::ThemeChanged { .. })), "no ThemeChanged");
    assert_eq!(host.core().kernel().config().get(&["app", "theme"]), serde_json::json!("fixture-grid"));
}

#[test]
fn a_theme_that_does_not_compile_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "broken",
        "extends = \"default\"\nscreens = [\"home\"]",
        &[("screens/home.slint", "export component HomeScreen inherits Rectangle { this is not slint }")],
    );
    let mut host = composed(|h| with_examples(h).discover(dir.path()));
    host.ui.start().unwrap();
    host.ui.switch(&id("fixture-grid")).unwrap();
    host.dispatch(Command::Navigate(Route::Downloads));
    let mut events = host.core().events();

    let err = host.ui.switch(&id("broken")).unwrap_err();
    assert!(matches!(err, ThemeError::Compile { .. }), "{err}");
    assert_rolled_back(&host, &mut events, err);
}

#[test]
fn a_theme_with_a_missing_asset_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "no-logo", "extends = \"default\"\n[theme.assets.images]\nlogo = \"assets/logo.png\"", &[]);
    let mut host = composed(|h| with_examples(h).discover(dir.path()));
    host.ui.start().unwrap();
    host.ui.switch(&id("fixture-grid")).unwrap();
    host.dispatch(Command::Navigate(Route::Downloads));
    let mut events = host.core().events();

    let err = host.ui.switch(&id("no-logo")).unwrap_err();
    assert!(matches!(err, ThemeError::Asset { .. }), "{err}");
    assert_rolled_back(&host, &mut events, err);
}

#[test]
fn a_broken_saved_theme_falls_back_to_the_default_theme_at_start_and_keeps_the_choice() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "broken",
        "extends = \"default\"\nscreens = [\"home\"]",
        &[("screens/home.slint", "export component HomeScreen inherits Rectangle { nope }")],
    );
    let mut host = composed(|h| {
        std::fs::write(h.data_dir().join("config.json"), r#"{"app": {"theme": "broken"}}"#).unwrap();
        h.discover(dir.path())
    });

    host.ui.start().unwrap();

    assert_eq!(host.ui.current(), Some(&id("default")));
    assert_eq!(host.core().kernel().config().get(&["app", "theme"]), serde_json::json!("broken"), "not overwritten");
}

#[test]
fn safe_mode_shows_the_default_theme_and_loads_no_third_party_themes() {
    let mut host = composed(|h| {
        std::fs::write(h.data_dir().join("config.json"), r#"{"app": {"theme": "fixture-grid"}}"#).unwrap();
        with_examples(h).safe_mode()
    });

    host.ui.start().unwrap();

    assert_eq!(host.ui.current(), Some(&id("default")));
    assert!(!host.ui.themes().contains(&id("fixture-grid")));
    assert!(host.ui.switch(&id("fixture-grid")).is_err());
}

#[test]
fn a_theme_option_changes_tokens_live_without_rebuilding_the_view() {
    let mut host = composed(with_examples);
    host.ui.start().unwrap();
    host.ui.switch(&id("fixture-grid")).unwrap();
    let before = host.instance().as_weak();

    host.instance()
        .invoke_global(
            "Host",
            "command",
            &[Value::String("SetThemeOption".into()), Value::String(r##"{"key":"accent","value":"#ff0000"}"##.into())],
        )
        .unwrap();
    host.pump();

    assert!(before.upgrade().is_some(), "same view tree");
    let accent = host.instance().get_global_property("Tokens", "color-accent").unwrap();
    assert_eq!(
        accent,
        Value::Brush(slint_interpreter::Brush::SolidColor(slint_interpreter::Color::from_rgb_u8(255, 0, 0)))
    );
    let options = host.core().view().settings.theme_options;
    assert_eq!(options.iter().find(|f| f.key == "accent").map(|f| f.value.as_str()), Some("#ff0000"));
}

#[test]
fn dev_reload_picks_up_an_edit_and_keeps_the_last_good_version_on_a_bad_one() {
    let dir = tempfile::tempdir().unwrap();
    let home = |label: &str| {
        format!(
            "export component HomeScreen inherits Rectangle {{ accessible-role: text; accessible-label: \"{label}\"; }}"
        )
    };
    write_theme(
        dir.path(),
        "live",
        "extends = \"default\"\nscreens = [\"home\"]",
        &[("screens/home.slint", &home("v1"))],
    );
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.switch(&id("live")).unwrap();
    assert_eq!(host.count("v1"), 1);
    assert!(!host.ui.reload_if_changed().unwrap(), "nothing changed yet");

    let file = dir.path().join("live/screens/home.slint");
    std::fs::write(&file, home("v2")).unwrap();
    bump_mtime(&file);
    assert!(host.ui.reload_if_changed().unwrap());
    assert_eq!(host.count("v2"), 1);

    std::fs::write(&file, "export component HomeScreen inherits Rectangle { broken").unwrap();
    bump_mtime(&file);
    assert!(host.ui.reload_if_changed().is_err());
    assert_eq!(host.count("v2"), 1, "the last good version stays on screen");
    assert!(!host.ui.reload_if_changed().unwrap(), "a broken edit is not retried until the next change");
}

/// Some filesystems keep modification times in whole seconds.
fn bump_mtime(path: &std::path::Path) {
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}
