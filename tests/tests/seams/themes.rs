//! Seam 3, composed host: theme loading, inheritance and the theme rules.

use crate::common::*;
use nexus_api::{Files, RelPath};
use nexus_core::Command;
use nexus_ui_api::{ComponentId, ScreenId, ThemeError, TokenValue};

#[test]
fn the_default_theme_is_complete_and_compiles() {
    let files = theme_default::DefaultTheme::files();
    for screen in ScreenId::ALL {
        assert!(files.read(&RelPath::new(screen.file()).unwrap()).is_some(), "default has no {}", screen.file());
    }
    for component in ComponentId::ALL {
        assert!(files.read(&RelPath::new(component.file()).unwrap()).is_some(), "default has no {}", component.file());
    }

    let mut host = composed(|h| h);
    host.ui.start().expect("the default theme starts");
    assert_eq!(host.ui.current(), Some(&id("default")));
    // Every token has a value: merging would have failed otherwise.
    assert_eq!(host.ui.resolved().unwrap().merged.tokens.len(), nexus_ui_api::TOKENS.len());
}

#[test]
fn every_default_screen_renders() {
    let mut host = composed(|h| {
        h.plugin(games("emu-a", vec!["Alpha", "Beta"]))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    let routes = [
        ("home", "Home"),
        ("catalog", "Catalog"),
        ("downloads", "Downloads"),
        ("settings", "Settings"),
        ("emulators", "Emulators"),
        ("themes", "Themes"),
    ];
    for (screen, route) in routes {
        let args = [
            slint_interpreter::Value::String("Navigate".into()),
            slint_interpreter::Value::String(format!("{route:?}").into()),
        ];
        host.instance().invoke_global("Host", "command", &args).unwrap();
        host.pump();
        assert_eq!(nexus_ui_host::vm_text(host.instance(), "screen").as_deref(), Some(screen));
    }
    host.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap()));
    assert_eq!(nexus_ui_host::vm_text(host.instance(), "screen").as_deref(), Some("game-detail"));
}

#[test]
fn a_child_theme_falls_back_to_its_parent_for_what_it_leaves_out() {
    let dir = tempfile::tempdir().unwrap();
    // Overrides one component and one token; no screens, no shell.
    write_theme(
        dir.path(),
        "child",
        "extends = \"default\"\ncomponents = [\"card\"]\n[theme.tokens]\ncolor-accent = \"#ff0000\"",
        &[(
            "components/card.slint",
            r#"import { GameCard } from "../types.slint";
import { Tokens } from "../tokens.slint";
export component Card inherits Rectangle {
    in property <GameCard> game;
    in property <length> cover-height;
    callback activated();
    width: 100px;
    height: 100px;
    background: Tokens.color-accent;
    accessible-role: text;
    accessible-label: "child-card";
}
"#,
        )],
    );
    let mut host = composed(|h| {
        h.discover(dir.path())
            .plugin(games("emu-kyty", vec!["Alpha", "Beta", "Gamma"]).console("PS5"))
            .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.switch(&id("child")).expect("the child theme loads");
    // A catalog with three games.
    let topics: Vec<serde_json::Value> = ["Alpha", "Beta", "Gamma"]
        .iter()
        .enumerate()
        .map(|(n, title)| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] {title}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "game_info": { "name": title } })
        })
        .collect();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 3,
        "topics": topics });
    host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    host.dispatch(Command::SetConfig {
        plugin: id("release-catalog"),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
    });
    host.catalogs_fetched();
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));

    let resolved = host.ui.resolved().unwrap();
    assert_eq!(resolved.inherited_screens().len(), ScreenId::ALL.len(), "every screen comes from default");
    assert_eq!(resolved.merged.tokens["color-accent"], TokenValue::Color(0xff0000ff));
    assert_eq!(resolved.merged.tokens["color-bg"], TokenValue::Color(0x05070fff), "inherited from default");
    // The default Catalog screen draws the child's Card for each game.
    host.dispatch(Command::Navigate(nexus_core::Route::Catalog));
    host.dispatch(Command::Search("a".into()));
    settle(&mut host);
    assert_eq!(host.count("child-card"), 3);
}

#[test]
fn a_theme_whose_parent_is_missing_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "orphan", "extends = \"nowhere\"", &[]);
    let mut host = composed(|h| h.discover(dir.path()));

    let report = &host.test.host.report;
    assert!(report.rejection(&id("orphan")).is_some(), "the kernel refuses it: its parent is a missing dependency");
    assert!(matches!(host.ui.switch(&id("orphan")), Err(ThemeError::Unknown(_))));
}

#[test]
fn a_theme_built_for_another_ui_api_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "future", "extends = \"default\"", &[]);
    let toml = dir.path().join("future/theme.toml");
    let text = std::fs::read_to_string(&toml).unwrap().replace("ui_api = \"^0.2\"", "ui_api = \"^9\"");
    std::fs::write(&toml, text).unwrap();
    let mut host = composed(|h| h.discover(dir.path()));

    assert!(matches!(host.ui.switch(&id("future")), Err(ThemeError::UiApi { .. })));
}

#[test]
fn imports_that_leave_the_theme_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("evil.slint"), "export component Evil inherits Rectangle {}").unwrap();
    let escape = format!(
        "import {{ Evil }} from \"../../../../../../../..{}/evil.slint\";\nexport component HomeScreen inherits Rectangle {{ Evil {{}} }}",
        outside.path().display()
    );
    write_theme(
        dir.path(),
        "escape",
        "extends = \"default\"\nscreens = [\"home\"]",
        &[("screens/home.slint", &escape)],
    );
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("escape")).unwrap_err();
    assert!(
        matches!(&err, ThemeError::Compile { diagnostics, .. } if diagnostics.contains("outside the theme")),
        "{err}"
    );
}

#[test]
fn image_urls_and_font_imports_in_markup_are_refused() {
    for (name, source) in [
        ("pic", "export component HomeScreen inherits Rectangle { Image { source: @image-url(\"/etc/x.png\"); } }"),
        ("font", "import \"/Library/Fonts/x.ttf\";\nexport component HomeScreen inherits Rectangle {}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_theme(dir.path(), name, "extends = \"default\"\nscreens = [\"home\"]", &[("screens/home.slint", source)]);
        let mut host = composed(|h| h.discover(dir.path()));
        let err = host.ui.switch(&id(name)).unwrap_err();
        assert!(matches!(err, ThemeError::File { .. }), "{name}: {err}");
    }
}

#[test]
fn a_theme_may_not_ship_the_files_the_host_serves() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "faker", "extends = \"default\"", &[("types.slint", "export global Vm {}")]);
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("faker")).unwrap_err();
    assert!(matches!(&err, ThemeError::File { file, .. } if file == "types.slint"), "{err}");
}

#[test]
fn a_screen_that_exports_the_wrong_name_is_refused_with_the_file_named() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "misnamed",
        "extends = \"default\"\nscreens = [\"home\"]",
        &[("screens/home.slint", "export component Home inherits Rectangle {}")],
    );
    let mut host = composed(|h| h.discover(dir.path()));

    let err = host.ui.switch(&id("misnamed")).unwrap_err();
    assert!(matches!(&err, ThemeError::File { file, .. } if file == "screens/home.slint"), "{err}");
}

#[test]
fn the_default_settings_screen_maps_a_folder_through_the_host() {
    let plugin = nexus_testkit::StubPlugin::from_toml(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC games\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"",
    );
    let mut host = composed(|h| h.plugin(plugin));
    host.ui.start().unwrap();
    let games = tempfile::tempdir().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);

    assert_eq!(host.count("folder:games"), 1, "a row for the folder");
    // As the theme sends the folder's text.
    let args = serde_json::json!({ "name": "games", "paths": games.path().display().to_string() });
    host.instance()
        .invoke_global(
            "Host",
            "command",
            &[
                slint_interpreter::Value::String("SetFolders".into()),
                slint_interpreter::Value::String(args.to_string().into()),
            ],
        )
        .unwrap();
    host.pump();

    let saved = host.core().kernel().config().get(&["app", "folders", "games"]);
    assert_eq!(saved, serde_json::json!([games.path().display().to_string()]));
    let shown = host.first("folder:games").accessible_value().unwrap_or_default();
    assert_eq!(shown.as_str(), games.path().display().to_string());
}

#[test]
fn a_theme_reads_its_option_values_from_the_options_global() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "opts",
        "extends = \"default\"\nscreens = [\"home\"]\n\
         [theme.options.motion]\nlabel = \"Motion\"\ntype = \"choice\"\ndefault = \"off\"\n\
         [theme.options.motion.choices.off]\n[theme.options.motion.choices.pan]\n\
         [theme.options.tint]\nlabel = \"Tint\"\ntype = \"color\"\ntoken = \"color-accent\"\ndefault = \"#ff0000\"",
        &[(
            "screens/home.slint",
            r#"import { Options } from "../options.slint";
export component HomeScreen inherits Rectangle {
    Text {
        text: Options.motion;
        color: Options.tint;
        accessible-role: text;
        accessible-label: "motion";
        accessible-value: self.text;
    }
}"#,
        )],
    );
    let mut host = composed(|h| h.discover(dir.path()));
    host.ui.start().unwrap();
    host.ui.switch(&id("opts")).unwrap();
    assert_eq!(host.first("motion").accessible_value().as_deref(), Some("off"), "unset options read as their default");

    host.dispatch(Command::SetThemeOption { key: "motion".into(), value: "pan".into() });
    assert_eq!(host.first("motion").accessible_value().as_deref(), Some("pan"));

    host.dispatch(Command::SetThemeOption { key: "motion".into(), value: "no-such-choice".into() });
    assert_eq!(
        host.first("motion").accessible_value().as_deref(),
        Some("off"),
        "an unknown choice reads as the default"
    );
}

#[test]
fn the_default_settings_screen_edits_a_folder_with_the_keys() {
    use slint::platform::{Key, WindowEvent};
    let plugin = nexus_testkit::StubPlugin::from_toml(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC games\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"",
    );
    let mut host = composed(|h| h.plugin(plugin));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    let press = |host: &Composed, text: slint::SharedString| {
        use slint_interpreter::ComponentHandle;
        host.instance().window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(WindowEvent::KeyReleased { text });
    };

    // The focus starts on the folder's row. Enter opens its text box; Enter again saves it.
    press(&host, Key::Return.into());
    host.pump();
    settle(&mut host);
    let typed = full_path("/games");
    for c in typed.chars() {
        press(&host, c.to_string().into());
    }
    press(&host, Key::Return.into());
    host.pump();
    settle(&mut host);
    assert_eq!(host.core().kernel().config().get(&["app", "folders", "games"]), serde_json::json!([typed]));

    // The keys reach the sheet again: Down moves on to the Rescan row, and Enter there scans.
    press(&host, Key::DownArrow.into());
    press(&host, Key::Return.into());
    let asked = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::Core(Command::Library(nexus_core::LibraryCmd::Refresh))));
    assert!(asked, "Enter on Rescan, after the text box closed, asks the core to scan");
}
