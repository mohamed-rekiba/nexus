//! Seam 1, core app API: the machine's settings from a system plugin. The core reads the
//! network, Wi-Fi, Bluetooth, sound, power and battery from the first system plugin that
//! provides each, shows each one nobody provides as unavailable, runs long work (joining,
//! pairing, installing) as jobs, and stays up when the plugin hangs or panics.

use std::path::PathBuf;

use nexus_api::{
    AudioLevel, AudioOutput, Battery, BluetoothDevice, Connection, InstallDisk, Installed, NetworkStatus, OsUpdate,
    WifiNetwork,
};
use nexus_core::system::{Place, SystemCommand};
use nexus_core::{Area, Command};
use nexus_testkit::{FakeHost, FakeSystem, StubPlugin};

fn wifi(ssid: &str, signal: u8, secured: bool) -> WifiNetwork {
    WifiNetwork { ssid: ssid.into(), signal, secured, saved: false, active: false }
}

fn device(address: &str, name: &str, paired: bool, connected: bool) -> BluetoothDevice {
    BluetoothDevice { address: address.into(), name: name.into(), paired, connected }
}

fn output(name: &str, description: &str, current: bool) -> AudioOutput {
    AudioOutput { name: name.into(), description: description.into(), current }
}

/// A machine on wired network, with Wi-Fi in reach, one controller connected, sound on the TV
/// and a laptop's battery.
fn machine() -> FakeSystem {
    let system = FakeSystem::new();
    system.with(|m| {
        m.network = NetworkStatus { connection: Connection::Wired, name: "Wired connection 1".into(), signal: None };
        m.wifi = vec![wifi("Home", 82, true), wifi("Cafe", 30, false)];
        m.wifi_scanned = vec![wifi("Home", 82, true), wifi("Cafe", 30, false), wifi("Neighbour", 10, true)];
        m.passwords = vec![("Home".into(), "secret123".into())];
        m.bluetooth_on = true;
        m.devices = vec![device("AA:BB", "DualSense", true, true), device("CC:DD", "Headset", true, false)];
        m.found = vec![device("EE:FF", "Xbox Controller", false, false)];
        m.outputs = vec![output("hdmi", "HDMI / DisplayPort", true), output("headset", "Headset", false)];
        m.level = AudioLevel { volume: 40, muted: false };
        m.battery = Some(Battery { percent: 18, charging: false });
    });
    system
}

async fn ask(cmd: SystemCommand, t: &nexus_testkit::TestCore) {
    t.core.dispatch(Command::System(cmd)).await.unwrap();
}

#[tokio::test]
async fn without_a_system_plugin_each_setting_is_unavailable_and_says_why() {
    let t = FakeHost::new().boot_core().await;
    t.core.dispatch(Command::System(SystemCommand::Refresh)).await.unwrap();
    let vm = t.core.view().system;
    assert!(!vm.whole_screen, "a desktop");
    assert!(!vm.network_available && !vm.audio_available && !vm.bluetooth_available && !vm.power_available);
    assert!(!vm.network_reason.is_empty());
    assert!(!vm.battery_shown);
    let joined = t.core.dispatch(Command::System(SystemCommand::JoinWifi { ssid: "Home".into(), password: None }));
    assert!(joined.await.is_err(), "nothing to join with");
}

#[tokio::test]
async fn the_machine_s_network_sound_bluetooth_and_battery_are_read() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.network_available && vm.audio_available && vm.bluetooth_available && vm.power_available);
    assert_eq!((vm.connection.as_str(), vm.network_name.as_str()), ("wired", "Wired connection 1"));
    assert_eq!(vm.wifi.iter().map(|w| (w.ssid.as_str(), w.bars)).collect::<Vec<_>>(), [("Home", 4), ("Cafe", 2)]);
    assert_eq!((vm.volume, vm.muted), (40, false));
    assert_eq!(
        vm.outputs.iter().filter(|o| o.current).map(|o| o.description.as_str()).collect::<Vec<_>>(),
        ["HDMI / DisplayPort"]
    );
    assert!(vm.bluetooth_on);
    assert_eq!(vm.connected_devices, 1);
    assert_eq!((vm.battery_shown, vm.battery_percent, vm.battery_charging), (true, 18, false));
}

#[tokio::test]
async fn a_scan_finds_more_networks_and_joining_a_secured_one_takes_its_password() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::ScanWifi, &t).await;
    assert!(t.core.view().system.wifi.iter().any(|w| w.ssid == "Neighbour"), "the scan's networks");

    ask(SystemCommand::JoinWifi { ssid: "Home".into(), password: Some("wrong".into()) }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().expect("a join job");
    assert_eq!((job.kind.as_str(), job.target.as_str(), job.state.as_str()), ("join", "Home", "failed"));
    assert!(job.text.contains("password is wrong"), "{}", job.text);

    ask(SystemCommand::JoinWifi { ssid: "Home".into(), password: Some("secret123".into()) }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("done", "Connected"));
    // The job's end reads the machine again.
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let vm = t.core.view().system;
    assert_eq!((vm.connection.as_str(), vm.network_name.as_str(), vm.signal_bars), ("wifi", "Home", 4));
}

#[tokio::test]
async fn a_running_join_shows_until_it_ends_and_can_be_cancelled() {
    let system = machine();
    system.with(|m| m.hold_jobs = true);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::JoinWifi { ssid: "Cafe".into(), password: None }, &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str(), job.percent), ("running", "Joining Cafe", -1));

    t.core.dispatch(Command::System(SystemCommand::CancelJob(job.id as u64))).await.unwrap();
    system.finish_jobs();
    assert_eq!(t.core.view().system.jobs.last().unwrap().state, "done");
    assert!(t.core.dispatch(Command::System(SystemCommand::CancelJob(job.id as u64))).await.is_err(), "it ended");
}

#[tokio::test]
async fn a_scan_finds_a_controller_to_pair_and_pairing_connects_it() {
    let system = machine();
    system.with(|m| m.pair_refusals = vec![("CC:DD".into(), "The device didn't answer".into())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::ScanBluetooth, &t).await;
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let found = t.core.view().system.devices;
    assert!(found.iter().any(|d| d.name == "Xbox Controller" && !d.paired), "{found:?}");

    ask(SystemCommand::Pair("EE:FF".into()), &t).await;
    t.clock.advance(std::time::Duration::from_millis(50)).await;
    let vm = t.core.view().system;
    assert_eq!(vm.jobs.last().map(|j| (j.kind.as_str(), j.state.as_str())), Some(("pair", "done")));
    assert!(vm.devices.iter().any(|d| d.address == "EE:FF" && d.paired && d.connected));
    assert_eq!(vm.connected_devices, 2);

    ask(SystemCommand::Pair("CC:DD".into()), &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("failed", "The device didn't answer"));

    ask(SystemCommand::DisconnectDevice("AA:BB".into()), &t).await;
    ask(SystemCommand::RemoveDevice("CC:DD".into()), &t).await;
    ask(SystemCommand::SetBluetooth(false), &t).await;
    let vm = t.core.view().system;
    assert!(!vm.bluetooth_on);
    assert!(vm.devices.iter().all(|d| d.address != "CC:DD"));
    assert!(vm.devices.iter().any(|d| d.address == "AA:BB" && !d.connected));
}

#[tokio::test]
async fn volume_mute_and_the_output_change_and_show_at_once() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    ask(SystemCommand::SetVolume(150), &t).await;
    assert_eq!(t.core.view().system.volume, 100, "at most 100");
    assert_eq!(system.read(|m| m.level.volume), 100);
    ask(SystemCommand::SetMuted(true), &t).await;
    assert!(t.core.view().system.muted);
    ask(SystemCommand::SetOutput("headset".into()), &t).await;
    let current: Vec<_> = t.core.view().system.outputs.into_iter().filter(|o| o.current).map(|o| o.name).collect();
    assert_eq!(current, ["headset"]);
    assert!(t.core.dispatch(Command::System(SystemCommand::SetOutput("nope".into()))).await.is_err());
}

#[tokio::test]
async fn restart_and_shut_down_are_asked_of_the_system() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Restart, &t).await;
    ask(SystemCommand::ShutDown, &t).await;
    assert_eq!(system.read(|m| (m.restarts, m.shutdowns)), (1, 1));
}

#[tokio::test]
async fn a_setting_the_plugin_can_t_use_here_shows_its_reason() {
    let system = machine();
    system.with(|m| m.unavailable = vec![("bluetooth", "No Bluetooth adapter".into())]);
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(!vm.bluetooth_available);
    assert_eq!(vm.bluetooth_reason, "No Bluetooth adapter");
    assert!(vm.network_available, "the rest still works");
}

#[tokio::test]
async fn a_plugin_providing_some_capabilities_leaves_the_others_unavailable() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin_with(&["power"])).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert!(vm.power_available && vm.battery_shown);
    assert!(!vm.network_available && !vm.audio_available && !vm.bluetooth_available);
}

#[tokio::test]
async fn a_hanging_or_panicking_system_plugin_leaves_the_core_working() {
    let system = machine();
    system.with(|m| {
        m.hangs = vec!["status"];
        m.panics = vec!["battery"];
    });
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!(vm.connection, "none", "the hung read gave nothing");
    assert!(!vm.battery_shown, "the panicking read gave nothing");
    assert_eq!(vm.volume, 40, "the other reads still came");
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
}

#[tokio::test]
async fn the_view_says_whether_nexus_is_the_whole_screen() {
    let place = Place { whole_screen: true, nexus_os: false, stick: false };
    let t = FakeHost::new().place(place).boot_core().await;
    let vm = t.core.view().system;
    assert!(vm.whole_screen && !vm.nexus_os && !vm.stick);
}

#[tokio::test]
async fn only_a_system_plugin_may_provide_the_machine_s_settings_or_run_programs_long() {
    let service = StubPlugin::from_toml(
        "id = \"svc\"\nname = \"Svc\"\nversion = \"1.0.0\"\nkind = \"service\"\nhost_api = \"*\"\ncapabilities = [\"network\"]\n",
    );
    let system_with_games = StubPlugin::from_toml(
        "id = \"sys\"\nname = \"Sys\"\nversion = \"1.0.0\"\nkind = \"system\"\nhost_api = \"*\"\ncapabilities = [\"catalog\"]\n",
    );
    let long = StubPlugin::from_toml(
        "id = \"emu\"\nname = \"Emu\"\nversion = \"1.0.0\"\nkind = \"emulator\"\nhost_api = \"*\"\ncapabilities = []\n\
         [permissions]\nprocess = [\"system:sleep\"]\nlong_running = [\"system:sleep\"]\n",
    );
    let unlisted = StubPlugin::from_toml(
        "id = \"sys2\"\nname = \"Sys2\"\nversion = \"1.0.0\"\nkind = \"system\"\nhost_api = \"*\"\ncapabilities = []\n\
         [permissions]\npolkit = [\"system:systemctl\"]\n",
    );
    let t = FakeHost::new().plugin(service).plugin(system_with_games).plugin(long).plugin(unlisted).boot_core().await;
    let rejected: Vec<String> = t.report.rejected.iter().map(|r| r.source.clone()).collect();
    for id in ["svc", "sys", "emu", "sys2"] {
        assert!(rejected.contains(&id.to_string()), "{id} is refused: {rejected:?}");
    }
}

#[tokio::test]
async fn on_the_stick_the_disks_are_listed_and_installing_reports_its_progress() {
    let system = machine();
    system.with(|m| {
        m.disks = vec![InstallDisk {
            path: PathBuf::from("/dev/nvme0n1"),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
        m.install_steps = vec![(10, "Preparing the disk".into()), (60, "Copying Nexus OS".into())];
        m.install_result = Some(Ok(Installed { edition: "minimal-nvidia".into(), mok: true, nvidia_later: false }));
        m.hold_jobs = true;
    });
    let place = Place { whole_screen: true, nexus_os: true, stick: true };
    let t = FakeHost::new().place(place).plugin(system.plugin()).boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let disk = t.core.view().system.disks.first().cloned().expect("the disk");
    assert_eq!((disk.path.as_str(), disk.size.as_str(), disk.windows), ("/dev/nvme0n1", "512 GB", true));

    ask(SystemCommand::InstallOs(PathBuf::from("/dev/nvme0n1")), &t).await;
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!(
        (job.kind.as_str(), job.state.as_str(), job.percent, job.text.as_str()),
        ("install", "running", 60, "Copying Nexus OS")
    );
    system.finish_jobs();
    let job = t.core.view().system.jobs.last().cloned().unwrap();
    assert_eq!((job.state.as_str(), job.text.as_str()), ("done", "Nexus OS is installed"));
    assert!(job.mok && !job.nvidia_later, "Secure Boot's key screen comes at the restart");
    assert_eq!(system.read(|m| m.installed_on.clone()), [PathBuf::from("/dev/nvme0n1")]);

    ask(SystemCommand::ChoosePartitions, &t).await;
    assert_eq!(system.read(|m| m.partition_choices), 1);
}

#[tokio::test]
async fn away_from_the_stick_nexus_os_isn_t_installed() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    let install = t.core.dispatch(Command::System(SystemCommand::InstallOs(PathBuf::from("/dev/sda"))));
    assert!(install.await.is_err());
    assert!(system.read(|m| m.installed_on.is_empty()));
    ask(SystemCommand::Refresh, &t).await;
    assert!(t.core.view().system.disks.is_empty());
}

#[tokio::test]
async fn on_nexus_os_a_staged_update_says_restart_and_a_newer_launcher_asks_the_os_to_check() {
    use nexus_core::launcher_update::Install;
    let system = machine();
    system.with(|m| m.os_update = OsUpdate::Staged { version: "2026.10.24".into() });
    let host = FakeHost::new()
        .install(Install::NexusOs)
        .place(Place { whole_screen: true, nexus_os: true, stick: false })
        .plugin(system.plugin());
    let latest = "https://github.com/owner/nexus/releases/latest/download/latest.json";
    host.http().serve(latest, br#"{ "version": "9.9.9", "assets": {} }"#.to_vec());
    let t = host.boot_core().await;
    ask(SystemCommand::Refresh, &t).await;
    let vm = t.core.view().system;
    assert_eq!(vm.os_update, "staged");
    assert!(vm.os_update_text.contains("Restart") && vm.os_update_text.contains("2026.10.24"), "{}", vm.os_update_text);

    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();
    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    // The check runs in a task of its own.
    t.clock.run_until_idle().await;
    assert_eq!(system.read(|m| m.os_checks), 1, "the OS looks for the update that brings it");
}

#[tokio::test]
async fn the_machine_is_read_again_every_five_seconds_on_the_clock_and_not_between() {
    let system = machine();
    let t = FakeHost::new().plugin(system.plugin()).boot_core().await;
    t.until(Area::System, |core| core.view().system.volume == 40).await;

    // Changed outside Nexus: the next read is five seconds after the first.
    system.with(|m| m.level.volume = 70);
    t.clock.advance(std::time::Duration::from_millis(4500)).await;
    assert_eq!(t.core.view().system.volume, 40, "not read again yet");
    t.clock.advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(t.core.view().system.volume, 70);
}
