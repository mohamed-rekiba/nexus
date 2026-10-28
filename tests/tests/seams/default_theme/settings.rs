//! Settings: the rows, the keys through them, the VPN and the theme's options.

use super::*;

#[test]
fn settings_show_this_pc_and_how_each_emulator_runs_on_it() {
    let specs = nexus_core::Specs {
        cpu: "Apple M4 Max".into(),
        cores: 14,
        threads: 14,
        memory_gb: 36.0,
        avx2: None,
        gpus: Some(vec![nexus_core::Gpu {
            name: "Apple M4 Max".into(),
            memory_gb: None,
            discrete: false,
            unified: true,
            vulkan: Some((1, 3)),
        }]),
    };
    let mut host =
        composed(|h| h.hardware(specs).plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.until(Area::Hardware, |core| core.view().settings.hardware.known);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("This PC");

    assert_eq!(host.values("pc-cpu"), ["Apple M4 Max · 14 cores"]);
    assert_eq!(host.values("pc-memory"), ["36 GB"]);
    assert_eq!(host.values("pc-graphics"), ["Apple M4 Max · shared memory · Vulkan 1.3"]);
    assert_eq!(host.values("pc-verdict"), ["emu-a: Should run well"], "a plugin that asks for nothing");
}

#[test]
fn with_two_monitors_settings_choose_the_one_the_launcher_fills() {
    let mut host = default_theme();
    host.ui.set_monitors(vec!["Built-in Retina Display".into(), "LEN G32qc-10".into()]);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    host.press("monitor", "");
    assert_eq!(host.core().prefs().monitor.as_deref(), Some("LEN G32qc-10"), "the next monitor after the one it is on");
    host.ui.render().unwrap();
    host.settle();
    host.press("monitor", "LEN G32qc-10");
    assert_eq!(host.core().prefs().monitor.as_deref(), Some("Built-in Retina Display"), "round again");
}

#[test]
fn the_direction_option_moves_the_top_bar_to_the_right() {
    let mut host = default_theme();
    let left = host.first("default-system").absolute_position().x;
    assert!(left < 400.0, "the systems start on the left: {left}");

    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.settle();
    let right = host.first("default-system").absolute_position().x;
    assert!(right > 800.0, "the systems move to the right: {right}");
}

#[test]
fn the_options_are_the_layout_direction_and_animations() {
    let host = default_theme();
    let options = &host.ui.resolved().unwrap().merged.option_values;
    let mut names: Vec<&str> = options.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["direction", "motion"]);
    let choice = |name: &str| match &options[name] {
        nexus_ui_api::OptionValue::Choice(c) => c.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(choice("direction"), "ltr");
    assert_eq!(choice("motion"), "on");
}

#[test]
fn the_keys_move_through_settings_tabs_and_change_the_focused_row() {
    use slint::platform::Key;
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();

    // Enter on the first row, "Rescan game folders", scans the folders again.
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let asked = host
        .requests
        .borrow()
        .iter()
        .any(|r| matches!(r, nexus_ui_host::UiRequest::Core(Command::Library(nexus_core::LibraryCmd::Refresh))));
    assert!(asked, "Enter on Rescan asks the core to scan");
    host.pump();

    // Up from the first row goes to the tabs, Right three times to DOWNLOADS, Down into its
    // rows: "Download only through a VPN", then down past the row that says there is no VPN and
    // "Add a VPN" to "Keep sharing finished downloads".
    host.navigate(Key::UpArrow);
    for _ in 0..3 {
        host.navigate(Key::RightArrow);
    }
    assert_eq!(host.count("seed-downloads"), 1, "the tab shows its rows");
    assert_eq!(host.count("sounds"), 0, "and no other tab's");
    host.navigate(Key::DownArrow);
    host.navigate(Key::Return);
    assert_eq!(host.first("vpn-only").accessible_value().as_deref(), Some("on"), "Enter turns the VPN switch on");
    for _ in 0..3 {
        host.navigate(Key::DownArrow);
    }
    assert_eq!(host.first("seed-downloads").accessible_value().as_deref(), Some("on"));
    host.navigate(Key::Return);
    assert_eq!(host.first("seed-downloads").accessible_value().as_deref(), Some("off"), "Enter stops sharing");
    // Down at the last row stays there: it doesn't go on to another tab.
    host.navigate(Key::DownArrow);
    assert_eq!(host.count("seed-downloads"), 1);

    // Back up to the tabs and Left to LAUNCHER: Down to "Interface sounds" and "Start at login".
    for _ in 0..4 {
        host.navigate(Key::UpArrow);
    }
    host.navigate(Key::UpArrow);
    host.navigate(Key::LeftArrow);
    host.navigate(Key::DownArrow);
    for _ in 0..3 {
        host.navigate(Key::DownArrow);
    }
    assert_eq!(host.first("sounds").accessible_value().as_deref(), Some("on"));
    host.navigate(Key::Return);
    assert_eq!(host.first("sounds").accessible_value().as_deref(), Some("off"), "Enter turns the sounds off");
    host.navigate(Key::DownArrow);
    assert_eq!(host.first("start-at-login").accessible_value().as_deref(), Some("off"));
    // Up to the tabs and Right to POWER: "Quit Nexus".
    for _ in 0..5 {
        host.navigate(Key::UpArrow);
    }
    for _ in 0..3 {
        host.navigate(Key::RightArrow);
    }
    host.navigate(Key::DownArrow);
    assert_eq!(host.count("option:direction"), 0, "no theme options in Settings");
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    let quit = host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Quit));
    assert!(quit, "Enter on Quit closes the launcher");
    host.pump();
}

#[test]
fn settings_keep_the_launcher_behind_a_closed_game() {
    let mut host = composed(|h| h);
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    assert_eq!(host.first("return-on-exit").accessible_value().as_deref(), Some("on"), "on by default");

    host.first("return-on-exit").invoke_accessible_default_action();
    host.pump();
    host.settle();

    assert_eq!(host.first("return-on-exit").accessible_value().as_deref(), Some("off"));
    assert!(!host.core().prefs().return_on_exit);
}

#[test]
fn settings_turn_automatic_updates_off() {
    // The row is in the UPDATES section, which lists the emulators.
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Astro"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Updates");
    assert_eq!(host.first("auto-update").accessible_value().as_deref(), Some("on"));
    host.first("auto-update").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("auto-update").accessible_value().as_deref(), Some("off"));
    assert!(!host.core().view().settings.auto_update);
}

#[test]
fn settings_change_the_size_of_everything_and_the_rows_grow_with_it() {
    use slint::platform::Key;
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    assert_eq!(host.values("scale"), ["1.0"]);
    let before = host.first("fullscreen").size().height;

    host.press("scale", "1.0");
    assert_eq!(host.core().view().settings.scale, 1.1);
    assert_eq!(host.values("scale"), ["1.1"]);
    assert!(host.first("fullscreen").size().height > before, "everything draws bigger at once");

    // Down past Full screen to the size: right is bigger, left smaller.
    for _ in 0..2 {
        host.navigate(Key::DownArrow);
    }
    host.navigate(Key::RightArrow);
    assert_eq!(host.values("scale"), ["1.2"]);
    host.navigate(Key::LeftArrow);
    host.navigate(Key::LeftArrow);
    assert_eq!(host.values("scale"), ["1.0"]);
    // The largest goes round to the smallest.
    host.dispatch(Command::Pref(Pref::Scale, 1.4.into()));
    host.settle();
    host.press("scale", "1.4");
    assert_eq!(host.values("scale"), ["0.8"]);
}

#[test]
fn nexus_scale_wins_over_the_setting_for_the_run() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    let before = host.first("fullscreen").size().height;
    host.ui.set_size(1.3);
    host.ui.render().unwrap();
    host.settle();
    assert!(host.first("fullscreen").size().height > before);
    assert_eq!(host.values("scale"), ["1.0"], "the setting stays");
    let hint = host.first("scale").accessible_description().unwrap_or_default();
    assert_eq!(hint.as_str(), "NEXUS_SCALE sets it for this run");
}

#[test]
fn settings_start_the_launcher_at_login_and_say_why_the_system_refused() {
    let mut host = default_theme();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    host.press("start-at-login", "off");
    assert_eq!(host.values("start-at-login"), ["on"]);
    assert_eq!(host.test.login_session.logins(), [true]);

    // The app logs a command that fails; the row says why.
    host.test.login_session.refuse_login("reg.exe: access is denied");
    host.first("start-at-login").invoke_accessible_default_action();
    let asked: Vec<_> = host.requests.borrow_mut().drain(..).collect();
    for request in asked {
        if let nexus_ui_host::UiRequest::Core(command) = request {
            assert!(host.rt.block_on(host.test.core.dispatch(command)).is_err());
        }
    }
    host.ui.render().unwrap();
    host.settle();
    assert_eq!(host.values("start-at-login"), ["on"], "it stays as it was");
    let why = host.first("start-at-login").accessible_description().unwrap_or_default();
    assert_eq!(why.as_str(), "reg.exe: access is denied");
}

#[test]
fn settings_hide_start_at_login_for_a_build_that_isn_t_installed() {
    let mut host = composed(|h| {
        h.login_session().no_login();
        h
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    assert_eq!(host.count("start-at-login"), 0);
}

#[test]
fn settings_have_no_emulator_settings() {
    let mut host = composed(|h| h.built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    assert_eq!(host.count("setting:fullscreen"), 0, "they are on the plugin's page");
    assert_eq!(host.count("folder:kyty-build"), 0, "and so are its own folders");
    assert_eq!(host.count("folder:games"), 1, "the game folders stay");
    assert_eq!(host.count("folder:packages"), 1, "and the packages folder, once for every emulator");
}

#[test]
fn the_window_is_full_screen_by_default_and_settings_turn_it_off() {
    let mut host = default_theme();
    assert!(host.instance().window().is_fullscreen(), "full screen by default, as in the old launcher");
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Launcher");
    assert_eq!(host.first("fullscreen").accessible_value().as_deref(), Some("on"));
    host.first("fullscreen").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert!(!host.instance().window().is_fullscreen());
    assert_eq!(host.first("fullscreen").accessible_value().as_deref(), Some("off"));
}

/// Settings, down to the DOWNLOADS section, on a host whose VPNs `vpns` sets.
fn downloads_settings(vpns: impl FnOnce(&nexus_testkit::FakeNetwork)) -> Composed {
    let mut host = composed(|h| {
        vpns(h.network());
        h
    });
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Downloads");
    host
}

#[test]
fn settings_list_the_vpns_and_say_where_downloads_go() {
    let mut host = downloads_settings(|n| {
        n.up("wg0");
        n.down("work");
    });
    assert_eq!(host.values("vpn"), ["wg0", "work"]);
    assert_eq!(host.description("vpn-only"), "Hides your IP address from other peers · uses your own VPN");

    host.press("vpn-only", "off");
    assert_eq!(host.values("vpn-only"), ["on"]);
    assert_eq!(host.description("vpn-only"), "Protected · downloads only use wg0");
    assert_eq!(host.description("vpn"), "WireGuard · connected · downloads use it");

    // Enter on a VPN the system can switch connects or disconnects it.
    host.press("vpn", "work");
    host.press("vpn", "wg0");
    assert_eq!(host.test.network.calls(), ["connect work", "disconnect wg0"]);
    assert_eq!(host.description("vpn-only"), "Protected · downloads only use work");
}

#[test]
fn settings_add_a_vpn_from_the_file_the_player_types() {
    let mut host = downloads_settings(|_| {});
    assert_eq!(host.count("vpn-none"), 1);
    host.first("vpn-import").invoke_accessible_default_action();
    host.settle();
    let file = full_path("/vpn/se-1.conf");
    host.send_text(&file);
    host.navigate(slint::platform::Key::Return);
    let calls = host.test.network.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls, [format!("import {file}")]);
    assert_eq!(host.values("vpn"), ["se-1"]);
}

#[test]
fn where_the_system_can_t_tell_a_vpn_the_player_picks_the_network() {
    let mut host = downloads_settings(|n| {
        n.picks_only();
        n.up("en0");
        n.up("utun4");
    });
    host.press("vpn-only", "off");
    assert_eq!(host.description("vpn-only"), "Downloads wait until you pick the network your VPN uses");
    host.press("vpn", "utun4");
    assert_eq!(host.core().view().settings.vpn.chosen, "utun4");
    assert_eq!(host.description("vpn-only"), "Protected · downloads only use utun4");
    assert!(host.test.network.calls().is_empty(), "nothing is connected for the player");
}

#[test]
fn where_downloads_can_t_be_bound_the_vpn_switch_says_why_and_stays_off() {
    let mut host = downloads_settings(|n| n.unavailable("Not available on Windows"));
    assert_eq!(host.description("vpn-only"), "Not available on Windows");
    assert_eq!((host.count("vpn"), host.count("vpn-none"), host.count("vpn-import")), (0, 0, 0));
    host.press("vpn-only", "off");
    assert_eq!(host.values("vpn-only"), ["off"]);
}

#[test]
fn a_packaged_install_updates_from_settings_then_offers_a_restart() {
    use nexus_core::launcher_update::{Install, package_key};
    let latest = "https://github.com/owner/nexus/releases/latest/download/latest.json";
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-9.9.9.x86_64.rpm";
    let mut host = composed(|h| {
        let h = h.install(Install::Rpm);
        let sha = h.http().serve(url, b"a package".to_vec());
        let key = package_key(Install::Rpm).unwrap();
        let manifest = serde_json::json!({ "version": "9.9.9", "assets": { key: { "url": url, "sha256": sha } } });
        h.http().serve(latest, manifest.to_string().into_bytes());
        h
    });
    host.core().kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();
    host.ui.start().unwrap();
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    host.settle();
    host.settings_tab("Updates");

    host.first("launcher-update").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("launcher-update").accessible_value().as_deref(), Some("available"));

    host.first("launcher-update").invoke_accessible_default_action();
    host.pump();
    host.settle();
    assert_eq!(host.first("launcher-update").accessible_value().as_deref(), Some("installed"));

    host.first("launcher-update").invoke_accessible_default_action();
    let asked = host.requests.borrow().iter().any(|r| matches!(r, nexus_ui_host::UiRequest::Restart));
    assert!(asked, "the installed update offers a restart");
}
