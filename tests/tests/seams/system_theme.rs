//! Seam 3: the default theme's machine settings, over a fake system plugin. The top bar's
//! network, sound and Bluetooth icons and their panels show when Nexus is the whole screen;
//! the battery wherever the PC has one; Settings → System; and on Nexus OS's USB stick, the
//! Welcome and Install screens.

use std::path::PathBuf;

use crate::common::*;
use nexus_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus, WifiNetwork,
};
use nexus_core::Command;
use nexus_core::system::{Place, SystemCommand};
use nexus_testkit::FakeSystem;
use slint_interpreter::ComponentHandle;

const WHOLE_SCREEN: Place = Place { whole_screen: true, nexus_os: true, stick: false };

fn machine() -> FakeSystem {
    let system = FakeSystem::new();
    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wired, name: "Wired connection 1".into(), signal: None };
        m.wifi = vec![
            WifiNetwork { ssid: "Home".into(), signal: 82, secured: true, saved: false, active: false },
            WifiNetwork { ssid: "Cafe".into(), signal: 30, secured: false, saved: false, active: false },
        ];
        m.passwords = vec![("Home".into(), "secret123".into())];
        m.bluetooth_on = true;
        m.devices =
            vec![BluetoothDevice { address: "AA:BB".into(), name: "DualSense".into(), paired: true, connected: true }];
        m.outputs =
            vec![AudioOutput { name: "hdmi".into(), description: "TV or monitor (HDMI)".into(), current: true }];
        m.level = AudioLevel { volume: 40, muted: false };
        m.battery = Some(Battery { percent: 18, charging: false });
    });
    system
}

/// The default theme over `system`, at `place`, with the machine read once.
fn shown(system: &FakeSystem, place: Place) -> Composed {
    let plugin = system.plugin();
    let mut host = composed(|h| {
        h.place(place).plugin(plugin).plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
    });
    host.ui.start().unwrap();
    host.resize(1600.0, 900.0);
    host.dispatch(Command::System(SystemCommand::Refresh));
    settle(&mut host);
    host
}

fn value(host: &Composed, label: &str) -> String {
    host.first(label).accessible_value().unwrap_or_default().to_string()
}

fn press(host: &mut Composed, label: &str) {
    host.first(label).invoke_accessible_default_action();
    host.pump();
    settle(host);
}

fn key(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    typed(host, text);
}

fn typed(host: &mut Composed, text: slint::SharedString) {
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
    settle(host);
}

/// Down in Settings until the row labelled `label` is in view: rows below the sheet's edge are
/// not drawn.
fn down_to(host: &mut Composed, label: &str) {
    for _ in 0..60 {
        if host.count(label) > 0 {
            return;
        }
        key(host, slint::platform::Key::DownArrow);
    }
    panic!("no {label} in Settings");
}

fn refreshed(host: &mut Composed) {
    host.dispatch(Command::System(SystemCommand::Refresh));
    settle(host);
}

#[test]
fn on_a_desktop_only_the_battery_shows_in_the_top_bar() {
    let host = shown(&machine(), Place::default());
    assert_eq!(host.count("status:network") + host.count("status:sound") + host.count("status:bluetooth"), 0);
    assert_eq!(value(&host, "battery"), "18");
    assert_eq!(host.count("system-internet"), 0, "no SYSTEM section in Settings");
}

#[test]
fn a_pc_without_a_battery_shows_none() {
    let system = machine();
    system.with(|m| m.battery = None);
    let host = shown(&system, WHOLE_SCREEN);
    assert_eq!(host.count("battery"), 0);
}

#[test]
fn as_the_whole_screen_the_top_bar_shows_the_network_sound_and_bluetooth() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    assert_eq!(value(&host, "status:network"), "wired");
    assert_eq!(value(&host, "status:sound"), "40");
    assert_eq!(value(&host, "status:bluetooth"), "on 1");

    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wifi, name: "Home".into(), signal: Some(50) };
        m.level.muted = true;
    });
    refreshed(&mut host);
    assert_eq!(value(&host, "status:network"), "wifi 3");
    assert_eq!(value(&host, "status:sound"), "muted");
}

#[test]
fn the_network_panel_joins_a_secured_network_with_its_password() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    press(&mut host, "status:network");
    assert_eq!(value(&host, "system-panel"), "network");

    // Look for networks, then Home (secured, not saved) and Cafe.
    use slint::platform::Key;
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    assert_eq!(host.count("wifi-password"), 1, "Home asks for its password");
    for c in "secret123".chars() {
        typed(&mut host, c.to_string().into());
    }
    key(&mut host, Key::Return);
    assert_eq!(host.count("wifi-password"), 0, "the password went with the join");
    assert_eq!(system.read(|m| m.network.name.clone()), "Home");
    refreshed(&mut host);
    assert_eq!(value(&host, "system-job"), "Connected");

    key(&mut host, Key::Escape);
    assert_eq!(host.count("system-panel"), 0, "Back closes the panel");
}

#[test]
fn a_wrong_password_says_so_in_the_panel() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    press(&mut host, "status:network");
    use slint::platform::Key;
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    for c in "nope".chars() {
        typed(&mut host, c.to_string().into());
    }
    key(&mut host, Key::Return);
    assert_eq!(value(&host, "system-job"), "The password is wrong");
    assert_eq!(system.read(|m| m.network.connection), Connection::Wired);
}

#[test]
fn the_sound_panel_changes_the_volume_with_left_and_right_and_mutes() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    press(&mut host, "status:sound");
    use slint::platform::Key;
    key(&mut host, Key::RightArrow);
    assert_eq!(system.read(|m| m.level.volume), 50);
    assert_eq!(value(&host, "volume"), "50");
    key(&mut host, Key::LeftArrow);
    key(&mut host, Key::LeftArrow);
    assert_eq!(system.read(|m| m.level.volume), 30);
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    assert!(system.read(|m| m.level.muted));
    assert_eq!(value(&host, "status:sound"), "muted");
}

#[test]
fn the_bluetooth_panel_turns_it_off_and_a_missing_adapter_says_why() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    press(&mut host, "status:bluetooth");
    assert_eq!(value(&host, "bluetooth-device"), "DualSense");
    key(&mut host, slint::platform::Key::Return);
    assert!(!system.read(|m| m.bluetooth_on));
    assert_eq!(value(&host, "status:bluetooth"), "off");

    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "This PC has no Bluetooth adapter".into())]);
    let mut host = shown(&system, WHOLE_SCREEN);
    press(&mut host, "status:bluetooth");
    assert_eq!(value(&host, "system-reason"), "This PC has no Bluetooth adapter");
}

#[test]
fn settings_system_opens_the_panels_and_restart_asks_again_first() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "system-internet");
    assert_eq!(value(&host, "system-internet"), "wired");
    down_to(&mut host, "system-sound");
    press(&mut host, "system-sound");
    assert_eq!(value(&host, "system-panel"), "sound");
    key(&mut host, slint::platform::Key::Escape);

    down_to(&mut host, "system-restart");
    press(&mut host, "system-restart");
    assert_eq!(value(&host, "system-restart"), "armed");
    assert_eq!(system.read(|m| m.restarts), 0, "the first press only asks");
    press(&mut host, "system-restart");
    assert_eq!(system.read(|m| m.restarts), 1);
}

#[test]
fn in_the_nexus_session_log_out_asks_again_then_leaves_for_the_login_screen() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, nexus_os: false, stick: false });
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "system-log-out");
    let asked = |host: &Composed| host.requests.borrow().contains(&nexus_ui_host::UiRequest::LogOut);
    host.first("system-log-out").invoke_accessible_default_action();
    settle(&mut host);
    assert_eq!(value(&host, "system-log-out"), "armed");
    assert!(!asked(&host), "the first press only asks");
    host.first("system-log-out").invoke_accessible_default_action();
    assert!(asked(&host));
}

#[test]
fn as_the_whole_screen_quit_is_restart_nexus_since_the_session_starts_it_again() {
    for (place, row) in
        [(Place { whole_screen: false, nexus_os: false, stick: false }, "quit"), (WHOLE_SCREEN, "restart-nexus")]
    {
        let system = machine();
        let mut host = shown(&system, place);
        host.dispatch(Command::Navigate(nexus_core::Route::Settings));
        settle(&mut host);
        down_to(&mut host, row);
        assert_eq!(host.count(if row == "quit" { "restart-nexus" } else { "quit" }), 0, "{place:?}");
        host.first(row).invoke_accessible_default_action();
        assert!(host.requests.borrow().contains(&nexus_ui_host::UiRequest::Quit), "{place:?}");
    }
}

#[test]
fn on_nexus_os_there_is_no_log_out_its_session_would_only_start_again() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "system-shut-down");
    assert_eq!(host.count("system-log-out"), 0);
}

/// The power menu's items, in order.
fn power_items(host: &Composed) -> Vec<String> {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-menu")
        .filter_map(|e| e.accessible_value())
        .map(|v| v.to_string())
        .collect()
}

fn power_item(host: &mut Composed, label: &str) {
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "power-menu")
        .find(|e| e.accessible_value().is_some_and(|v| v.starts_with(label)))
        .unwrap_or_else(|| panic!("no {label} in the power menu"));
    found.invoke_accessible_default_action();
    settle(host);
}

fn ctrl_p(host: &mut Composed) {
    let ctrl: slint::SharedString = slint::platform::Key::Control.into();
    let window = host.instance().window();
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: ctrl.clone() });
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "p".into() });
    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "p".into() });
    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text: ctrl });
    host.pump();
    settle(host);
}

#[test]
fn holding_the_guide_button_opens_the_power_menu_with_log_out_in_the_nexus_session() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, nexus_os: false, stick: false });
    assert!(power_items(&host).is_empty());
    host.ui.pad(nexus_ui_host::PadButton::GuideHeld).unwrap();
    host.pump();
    settle(&mut host);
    assert_eq!(power_items(&host), ["Log out", "Restart Nexus", "Restart", "Shut down"]);
    power_item(&mut host, "Log out");
    assert!(host.requests.borrow().contains(&nexus_ui_host::UiRequest::LogOut), "Log out acts at once");
}

#[test]
fn ctrl_p_opens_the_power_menu_and_shut_down_asks_again_first() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    ctrl_p(&mut host);
    assert_eq!(power_items(&host), ["Restart Nexus", "Restart", "Shut down"], "Nexus OS: no Log out");
    power_item(&mut host, "Shut down");
    assert_eq!(system.read(|m| m.shutdowns), 0, "the first press only asks");
    assert!(power_items(&host).contains(&"Press again to shut down".to_owned()));
    power_item(&mut host, "Press again to shut down");
    host.pump();
    settle(&mut host);
    assert_eq!(system.read(|m| m.shutdowns), 1);
}

#[test]
fn on_a_desktop_the_power_menu_only_quits_and_escape_closes_it() {
    let system = machine();
    let mut host = shown(&system, Place::default());
    ctrl_p(&mut host);
    assert_eq!(power_items(&host), ["Quit Nexus"]);
    key(&mut host, slint::platform::Key::Escape);
    assert!(power_items(&host).is_empty(), "closed");
    ctrl_p(&mut host);
    power_item(&mut host, "Quit Nexus");
    assert!(host.requests.borrow().contains(&nexus_ui_host::UiRequest::Quit));
}

#[test]
fn on_the_stick_welcome_leads_to_a_disk_a_firm_question_and_the_install() {
    let system = machine();
    system.with(|m| {
        m.disks = vec![InstallDisk {
            path: PathBuf::from("/dev/nvme0n1"),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
        m.install_steps = vec![(40, "Copying Nexus OS".into())];
        m.install_result = Some(Ok(Installed { edition: "minimal".into(), mok: true, nvidia_later: false }));
        m.hold_jobs = true;
    });
    let mut host = shown(&system, Place { whole_screen: true, nexus_os: true, stick: true });
    assert_eq!(value(&host, "install-os"), "welcome");
    assert_eq!(value(&host, "install-title"), "Welcome to Nexus OS");

    use slint::platform::Key;
    key(&mut host, Key::DownArrow);
    key(&mut host, Key::Return);
    assert_eq!(value(&host, "install-os"), "disks");
    assert_eq!(value(&host, "install-disk"), "Samsung 970 · 512 GB");

    key(&mut host, Key::Return);
    assert_eq!(value(&host, "install-os"), "confirm");
    assert_eq!(value(&host, "install-title"), "Erase Samsung 970 (512 GB)?");
    assert!(value(&host, "install-text").contains("including Windows"), "{}", value(&host, "install-text"));
    assert!(system.read(|m| m.installed_on.is_empty()), "nothing erased yet");

    key(&mut host, Key::Return);
    assert_eq!(value(&host, "install-os"), "installing");
    assert_eq!(value(&host, "install-progress"), "40");
    assert_eq!(system.read(|m| m.installed_on.clone()), [PathBuf::from("/dev/nvme0n1")]);

    system.finish_jobs();
    settle(&mut host);
    assert_eq!(value(&host, "install-os"), "done");
    assert!(value(&host, "install-text").contains("Enroll MOK"), "Secure Boot's key screen is explained");
    key(&mut host, Key::Return);
    assert_eq!(system.read(|m| m.restarts), 1, "Restart");
}

#[test]
fn on_the_stick_try_it_leaves_the_launcher_and_choose_partitions_starts_that_installer() {
    let system = machine();
    let mut host = shown(&system, Place { whole_screen: true, nexus_os: true, stick: true });
    use slint::platform::Key;
    key(&mut host, Key::Return);
    assert_eq!(host.count("install-os"), 0, "Try it");

    host.dispatch(Command::Navigate(nexus_core::Route::Settings));
    settle(&mut host);
    down_to(&mut host, "system-install");
    press(&mut host, "system-install");
    assert_eq!(value(&host, "install-os"), "disks");
    // No disk: "Choose partitions instead" is first.
    key(&mut host, Key::Return);
    assert_eq!(system.read(|m| m.partition_choices), 1);
}

#[test]
fn the_keys_reach_the_icons_at_the_end_of_the_top_bar_and_move_between_panels() {
    let system = machine();
    let mut host = shown(&system, WHOLE_SCREEN);
    use slint::platform::Key;
    key(&mut host, Key::UpArrow);
    // Past the tabs and the screen buttons, the last stop is Bluetooth.
    for _ in 0..12 {
        key(&mut host, Key::RightArrow);
    }
    key(&mut host, Key::Return);
    assert_eq!(value(&host, "system-panel"), "bluetooth");
    key(&mut host, Key::LeftArrow);
    assert_eq!(value(&host, "system-panel"), "sound", "left goes to the panel beside");
    key(&mut host, Key::Escape);
    assert_eq!(host.count("system-panel"), 0);
}
