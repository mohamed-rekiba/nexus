//! The Lichii plugin on a fake Linux host: the bare AppImage of a pre-release, Switch game
//! files (but not their updates) from the player's folder, and the launch.

use emu_lichii::{LichiiPlugin, RELEASES};
use nexus_api::*;
use nexus_api::{Controller, HOST_OPEN_EMULATOR};
use nexus_core::{Command, SessionCmd};
use nexus_testkit::{FakeHost, FakeHttp, FakeSpawner, TestCore, contract};

const TAG: &str = "build-93";

fn lichii() -> PluginId {
    PluginId::new("emu-lichii").unwrap()
}

/// Does what a real AppImage does when the host runs `--appimage-extract`: unpacks itself.
fn extract_appimages(spawner: FakeSpawner) {
    tokio::spawn(async move {
        let mut n = 0;
        loop {
            n += 1;
            let process = spawner.wait_for(n).await;
            if process.command.args == ["--appimage-extract"] {
                let root = process.command.cwd.join("squashfs-root");
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(root.join("AppRun"), b"\x7fELF lichii").unwrap();
                process.exit(0);
            }
        }
    });
}

/// A pre-release of `tag` with the assets Lichii publishes, whose AppImage download redirects
/// to GitHub's CDN.
fn release(http: &FakeHttp, tag: &str) -> serde_json::Value {
    let arch = std::env::consts::ARCH;
    let name = format!("lichii-linux-{arch}.AppImage");
    let download = format!("https://github.com/zortos293/Lichii/releases/download/{tag}/{name}");
    let cdn = format!("https://release-assets.githubusercontent.com/lichii/{tag}/{name}");
    let sha = http.serve(&cdn, b"\x7fELF AppImage".to_vec());
    http.redirect(&download, &cdn);
    let other = |n: &str| serde_json::json!({ "name": n, "browser_download_url": "https://github.com/x", "size": 1 });
    serde_json::json!({ "tag_name": tag, "prerelease": true, "published_at": "2026-11-06T21:04:53Z", "assets": [
        other(&format!("lichii-linux-{arch}-pgo.AppImage")),
        other(&format!("lichii-linux-{arch}.tar.gz")),
        other("lichii-windows-x86_64.zip"),
        { "name": name, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
    ]})
}

async fn boot() -> (TestCore, tempfile::TempDir) {
    boot_with_saved(None, None).await
}

/// [`boot`], with what an earlier run of Nexus saved: `saved` as Lichii's settings, and
/// `firmware` as the Firmware block's settings file.
async fn boot_with_saved(
    saved: Option<serde_json::Value>,
    firmware: Option<serde_json::Value>,
) -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    if let Some(saved) = saved {
        let config = serde_json::json!({ "plugins": { "emu-lichii": { "emulator": saved } } });
        std::fs::write(host.data_dir().join("config.json"), config.to_string()).unwrap();
    }
    if let Some(firmware) = firmware {
        let plugin = host.data_dir().join("plugins/emu-lichii");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("firmware-settings.json"), firmware.to_string()).unwrap();
    }
    let releases = serde_json::json!([release(host.http(), TAG)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    let switch = games.path().join("Switch");
    let file = |rel: &str, bytes: &[u8]| {
        let path = switch.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    // An install's folder: the game, its update and its DLC.
    file("Celeste/Celeste [01002B30028F6000][v0].nsp", b"PFS0 game");
    file("Celeste/Celeste [01002B30028F6800][v262144].nsp", b"PFS0 update");
    file("Celeste/Celeste Farewell [01002B30028F7001][v0].nsp", b"PFS0 dlc");
    // A file name without a title id says it is an update.
    file("Kirby/Kirby Air Riders.nup", b"NUP game");
    file("Kirby/Kirby Air Riders Update 1.1.nsp", b"PFS0 update");
    // A game file of its own, beside the others.
    file("Loose.xci", b"HEAD cartridge");
    // An install still being written.
    file("Pending.part/Pending.nsp", b"PFS0 half");
    // As deep as the scan reads.
    file("Collection/Zelda/Zelda.nsp", b"PFS0 game");
    file("notes.txt", b"not a game");
    let test = host.built_in(nexus_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    (test, games)
}

/// Lichii keeps its data and settings in its own folders from build-150 on, and eden-cli
/// refuses every game while Eden's old folder is there without an import. So Nexus writes
/// firmware, reads settings and backs up there.
#[tokio::test]
async fn lichii_s_folders_default_to_lichii_s_own() {
    let home = tempfile::tempdir().unwrap();
    let host = FakeHost::new().home(home.path()).platform(Some(Platform::Linux));
    let test = host.built_in(nexus_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Plugin(lichii()))).await.unwrap();
    let folders = test.core.view().plugin_page.expect("Lichii's page").folders;
    let default = |name: &str| folders.iter().find(|f| f.name == name).map(|f| f.default.clone());
    assert_eq!(default("lichii-data"), Some(home.path().join(".local/share/lichii").display().to_string()));
    assert_eq!(default("lichii-config"), Some(home.path().join(".config/lichii").display().to_string()));
}

#[tokio::test]
async fn lichii_meets_every_contract() {
    let (test, _games) = boot().await;
    contract::emulator(&test, &lichii()).await;
}

#[tokio::test]
async fn switch_game_files_are_listed_under_the_catalog_s_console_and_updates_are_not() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    let home = test.core.view().home;
    let mut keys: Vec<&str> = home.games.iter().map(|g| g.key.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        ["emu-lichii:01002B30028F6000", "emu-lichii:Kirby Air Riders", "emu-lichii:Loose", "emu-lichii:Zelda"]
    );
    assert!(home.games.iter().all(|g| g.platform == "SWITCH"), "the catalog's console, so its offers match");
}

#[tokio::test]
async fn the_appimage_installs_without_a_test_run_and_starts_the_game_file_in_full_screen() {
    let (test, games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let app_run = test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun");
    assert!(app_run.is_file());
    assert_eq!(test.spawner.spawned().len(), 1, "only the extract ran: Lichii's --help never ends");

    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();

    let process = test.spawner.wait_for(2).await;
    assert_eq!(process.command.program, app_run);
    let game = games.path().join("Switch/Celeste/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(process.command.args, ["-f".to_owned(), "-g".to_owned(), game]);
    // Lichii's split GPU thread leaves Switch 2 games black on some PCs: off unless turned on.
    assert_eq!(process.command.env.get("EDEN_NX2_SPLIT").map(String::as_str), Some("0"));
}

#[tokio::test]
async fn a_game_whose_settings_choose_another_gpu_split_starts_with_it() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let game = "emu-lichii:01002B30028F6000";
    test.core
        .dispatch(Command::SetConfig {
            plugin: lichii(),
            // A game's settings are under its id in the plugin.
            scope: ConfigScope::Game("01002B30028F6000".into()),
            value: serde_json::json!({ "gpu_split": "2" }),
        })
        .await
        .unwrap();

    test.core.dispatch(Command::Session(SessionCmd::Launch(game.parse().unwrap()))).await.unwrap();

    let process = test.spawner.wait_for(2).await;
    assert_eq!(process.command.env.get("EDEN_NX2_SPLIT").map(String::as_str), Some("2"));
}

#[tokio::test]
async fn uninstalling_a_game_deletes_its_folder_with_its_update_and_dlc_and_a_loose_file_alone() {
    let (test, games) = boot().await;
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    for key in ["emu-lichii:01002B30028F6000", "emu-lichii:Loose"] {
        let game: GameKey = key.parse().unwrap();
        test.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
        test.core.dispatch(Command::UninstallGame(game)).await.unwrap();
    }
    let switch = games.path().join("Switch");
    assert!(!switch.join("Celeste").exists(), "the game's folder, with its update and DLC");
    assert!(!switch.join("Loose.xci").exists(), "the loose game file");
    assert!(switch.join("Kirby/Kirby Air Riders.nup").is_file(), "other games stay");
    assert!(switch.join("Collection/Zelda/Zelda.nsp").is_file(), "other games stay");
}

#[tokio::test]
async fn a_game_file_in_the_packages_folder_is_offered_and_installs_into_its_own_folder() {
    let (test, games) = boot().await;
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("Switch")).unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974000][v0].nsz"), b"NCZ game").unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974800][v65536].nsp"), b"PFS0 update").unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii()).unwrap();
    let offers: Vec<&str> =
        emu.panels.iter().filter(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).map(|p| p.text.as_str()).collect();
    assert_eq!(offers, ["Install Hades [0100535012974000][v0].nsz"], "the game, not its update");
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).unwrap();

    let start =
        Command::RunAction { plugin: lichii(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    test.core.dispatch(start).await.unwrap();
    test.until(nexus_core::Area::Installs, |core| {
        core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!(job.state, "done", "{}", job.error);
    let installed = games.path().join("Switch/Hades/Hades [0100535012974000][v0].nsz");
    assert_eq!(std::fs::read(installed).unwrap(), b"NCZ game");
}

/// The Emulators screen's button that opens Lichii's own window, if its page has one.
fn open_button(test: &TestCore) -> Option<nexus_ui_api::SlotVm> {
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii())?.clone();
    emu.panels.into_iter().find(|p| p.action.as_deref() == Some(HOST_OPEN_EMULATOR))
}

#[tokio::test]
async fn open_lichii_starts_its_own_window_without_a_game_and_no_game_starts_meanwhile() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let button = open_button(&test).expect("an Open Lichii button");
    assert_eq!(button.text, "Open Lichii");

    let open =
        Command::RunAction { plugin: lichii(), action: HOST_OPEN_EMULATOR.into(), game: None, args: button.args };
    test.core.dispatch(open).await.unwrap();

    let process = test.spawner.wait_for(2).await;
    let app_run = test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun");
    assert_eq!(process.command.program, app_run);
    assert!(process.command.args.is_empty(), "no game: {:?}", process.command.args);
    let running = test.core.view().running.expect("the running screen");
    assert_eq!(running.title, "Lichii");

    let play = Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap()));
    let refused = test.core.dispatch(play).await.unwrap_err().to_string();
    assert!(refused.contains("Lichii is running"), "{refused}");

    process.exit(0);
    test.until(nexus_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert!(test.core.view().home.games.iter().all(|g| g.last_played.is_empty()), "the window is not play time");
}

/// Where the build of `tag` keeps Lichii's program.
fn app_run(test: &TestCore) -> std::path::PathBuf {
    test.data_dir().join("plugins/emu-lichii/builds").join(TAG).join("squashfs-root/AppRun")
}

/// An Xbox 360 pad as SDL maps it on Linux, from SDL's controller database.
fn xbox(port: u32) -> Controller {
    Controller {
        name: "Xbox 360 Controller".into(),
        guid: "030000005e0400008e02000014010000".into(),
        port,
        mapping: "a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,leftshoulder:b4,\
                  leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b10,righttrigger:a5,\
                  rightx:a3,righty:a4,start:b7,x:b2,y:b3"
            .into(),
    }
}

/// Lichii's own settings file, as its window writes it: the keyboard for player 1.
const QT_CONFIG: &str = "[Renderer]\nbackend\\default=false\nbackend=1\n\n[Controls]\n\
player_0_connected\\default=true\nplayer_0_connected=true\n\
player_0_button_a\\default=true\nplayer_0_button_a=\"engine:keyboard,code:67,toggle:0\"\n";

/// A core with Lichii's official build, its eden-cli, Lichii's `qt-config.ini` ([`QT_CONFIG`])
/// and Celeste, and `controllers` connected.
async fn with_eden_cli(controllers: Vec<Controller>) -> (TestCore, tempfile::TempDir, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux)).controllers(controllers);
    let releases = serde_json::json!([release(host.http(), TAG)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("Switch")).unwrap();
    std::fs::write(games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp"), b"PFS0 game").unwrap();
    let eden = tempfile::tempdir().unwrap();
    std::fs::write(eden.path().join("qt-config.ini"), QT_CONFIG).unwrap();
    let test = host.built_in(nexus_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    for (name, path) in [("games", games.path()), ("lichii-config", eden.path())] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![path.into()] }).await.unwrap();
    }
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    std::fs::create_dir_all(app_run(&test).parent().unwrap().join("usr/bin")).unwrap();
    std::fs::write(app_run(&test).parent().unwrap().join("usr/bin/eden-cli"), b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    (test, games, eden)
}

/// Starts Celeste and returns its eden-cli command: the second process, after the AppImage's
/// unpacking.
async fn launch_celeste(test: &TestCore) -> nexus_testkit::SpawnedProcess {
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();
    test.spawner.wait_for(2).await
}

#[tokio::test]
async fn games_start_in_eden_cli_with_a_settings_file_that_gives_each_controller_a_player() {
    let (test, games, _eden) = with_eden_cli(vec![xbox(0), xbox(1)]).await;

    let process = launch_celeste(&test).await;
    assert_eq!(process.command.program, app_run(&test).parent().unwrap().join("usr/bin/eden-cli"));
    let args = &process.command.args;
    assert_eq!(args[0], "-c");
    let game = games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(args[2..], ["-f".to_owned(), "-g".to_owned(), game]);
    let file = std::fs::read_to_string(&args[1]).unwrap();
    let guid = "030000005e0400008e02000014010000";
    for line in [
        "[Renderer]".to_owned(),
        "backend=1".to_owned(),
        "airplane_mode\\default=false".to_owned(),
        "airplane_mode=false".to_owned(),
        "player_0_connected=true".to_owned(),
        "player_0_button_a\\default=false".to_owned(),
        format!("player_0_button_a=\"engine:sdl,port:0,guid:{guid},button:1\""),
        format!("player_0_button_b=\"engine:sdl,port:0,guid:{guid},button:0\""),
        format!("player_0_button_x=\"engine:sdl,port:0,guid:{guid},button:3\""),
        format!("player_0_button_zl=\"engine:sdl,port:0,guid:{guid},axis:2,threshold:0.5,invert:+\""),
        format!("player_0_button_dup=\"engine:sdl,port:0,guid:{guid},hat:0,direction:up\""),
        format!(
            "player_0_lstick=\"engine:sdl,port:0,guid:{guid},axis_x:0,axis_y:1,offset_x:0.000000,offset_y:0.000000,invert_x:+,invert_y:+\""
        ),
        "player_1_connected\\default=false".to_owned(),
        "player_1_connected=true".to_owned(),
        format!("player_1_button_a=\"engine:sdl,port:1,guid:{guid},button:1\""),
    ] {
        assert!(file.lines().any(|l| l == line), "{line} in:\n{file}");
    }
    assert!(!file.contains("engine:keyboard,code:67"), "player 1's keyboard keys give way to the controller");
    assert!(!file.contains("player_2_button_a="), "no third controller: player 3 stays as it was");
}

#[tokio::test]
async fn settings_show_airplane_mode_as_a_switch_that_is_off_and_no_firmware_setting_twice() {
    let (test, _games, _eden) = with_eden_cli(Vec::new()).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let view = test.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == lichii()).expect("the Lichii panel");
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["own_build", "fullscreen", "gpu_split", "airplane_mode"], "the Firmware block has the rest");
    let field = panel.fields.iter().find(|f| f.key == "airplane_mode").expect("an airplane mode field");
    assert_eq!(field.label, "Airplane mode");
    assert_eq!(field.kind, nexus_ui_api::FieldKind::Toggle);
    assert_eq!(field.value, "false");
}

/// Lichii's settings file that can't be read stops the game, and says why: before, the game
/// started with Lichii's defaults, airplane mode on among them.
#[tokio::test]
async fn an_unreadable_lichii_settings_file_stops_the_launch_and_says_why() {
    let (test, _games, eden) = with_eden_cli(Vec::new()).await;
    std::fs::remove_file(eden.path().join("qt-config.ini")).unwrap();
    std::fs::create_dir(eden.path().join("qt-config.ini")).unwrap();

    let launch = Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap()));
    let refused = test.core.dispatch(launch).await;

    let shown = format!("{refused:?} {:?}", test.core.view().notices);
    assert!(shown.contains("qt-config.ini"), "the reason names the file: {shown}");
    assert_eq!(test.spawner.spawned().len(), 1, "only the AppImage's unpacking ran, no game");
}

/// No Lichii settings file yet (Lichii's window never saved one): the game starts with
/// Nexus's settings over Lichii's defaults.
#[tokio::test]
async fn a_game_starts_without_a_lichii_settings_file() {
    let (test, _games, eden) = with_eden_cli(Vec::new()).await;
    std::fs::remove_file(eden.path().join("qt-config.ini")).unwrap();

    let process = launch_celeste(&test).await;

    let file = std::fs::read_to_string(&process.command.args[1]).unwrap();
    assert!(file.lines().any(|l| l == "airplane_mode=false"), "{file}");
}

#[tokio::test]
async fn a_game_starts_in_airplane_mode_once_the_player_turns_it_on() {
    let (test, _games, _eden) = with_eden_cli(Vec::new()).await;
    let on = Command::SetConfigField {
        plugin: lichii(),
        scope: ConfigScope::Emulator,
        key: "airplane_mode".into(),
        value: "true".into(),
    };
    test.core.dispatch(on).await.unwrap();

    let process = launch_celeste(&test).await;

    let file = std::fs::read_to_string(&process.command.args[1]).unwrap();
    assert!(file.lines().any(|l| l == "airplane_mode=true"), "airplane mode on in:\n{file}");
    assert!(!file.lines().any(|l| l == "airplane_mode=false"), "{file}");
}

/// A PFS0 of `files`, as an NSP or an NSZ holds them.
fn pfs0(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (mut names, mut entries, mut offset) = (Vec::new(), Vec::new(), 0u64);
    for (name, data) in files {
        entries.extend_from_slice(&offset.to_le_bytes());
        entries.extend_from_slice(&(data.len() as u64).to_le_bytes());
        entries.extend_from_slice(&(names.len() as u32).to_le_bytes());
        entries.extend_from_slice(&[0; 4]);
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        offset += data.len() as u64;
    }
    let mut out = b"PFS0".to_vec();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend(entries);
    out.extend(names);
    for (_, data) in files {
        out.extend_from_slice(data);
    }
    out
}

#[tokio::test]
async fn a_solid_nsz_installs_as_the_nsp_lichii_can_play() {
    let (test, games) = boot().await;
    // An NCA of a header and one section that isn't encrypted, and its solid NCZ, as nsz makes it.
    let header = vec![0x5Au8; 0x4000];
    let body: Vec<u8> = (0..0x2000u32).map(|i| (i % 256) as u8).collect();
    let mut ncz = header.clone();
    ncz.extend_from_slice(b"NCZSECTN");
    ncz.extend_from_slice(&1u64.to_le_bytes());
    for field in [0x4000u64, body.len() as u64, 1, 0] {
        ncz.extend_from_slice(&field.to_le_bytes());
    }
    ncz.extend_from_slice(&[0; 32]);
    ncz.extend(zstd::encode_all(body.as_slice(), 3).unwrap());
    let nsz = pfs0(&[("program.ncz", ncz), ("game.tik", b"ticket".to_vec())]);
    let packages = tempfile::tempdir().unwrap();
    std::fs::create_dir(packages.path().join("Switch")).unwrap();
    std::fs::write(packages.path().join("Switch/Hades [0100535012974000][v0].nsz"), nsz).unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![packages.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii()).unwrap();
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME)).unwrap();

    let start =
        Command::RunAction { plugin: lichii(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    test.core.dispatch(start).await.unwrap();
    test.until(nexus_core::Area::Installs, |core| {
        core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing")
    })
    .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!(job.state, "done", "{}", job.error);

    let folder = games.path().join("Switch/Hades");
    assert!(!folder.join("Hades [0100535012974000][v0].nsz").exists(), "no solid NSZ, which Lichii can't play");
    let nsp = std::fs::read(folder.join("Hades [0100535012974000][v0].nsp")).unwrap();
    let nca = [header, body].concat();
    assert_eq!(&nsp[..4], b"PFS0");
    assert!(nsp.windows(b"program.nca".len()).any(|w| w == b"program.nca"), "the NCZ is an NCA again");
    assert_eq!(&nsp[nsp.len() - nca.len() - 6..nsp.len() - 6], nca.as_slice(), "the NCA, then the ticket");
    assert_eq!(&nsp[nsp.len() - 6..], b"ticket");
}

#[tokio::test]
async fn a_game_that_starts_another_of_its_programs_is_started_again_in_the_same_session() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let cli = app_run(&test).parent().unwrap().join("usr/bin/eden-cli");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(&cli, b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();

    // eden-cli exits with 75 when the game starts another of its programs (Sonic x Shadow
    // Generations' two games): its launcher starts it again with EDEN_CLI_RESTART_HANDOFF set.
    let first = test.spawner.wait_for(2).await;
    assert_eq!(first.command.env.get("EDEN_CLI_RESTART_HANDOFF"), None);
    first.exit(75);
    let again = test.spawner.wait_for(3).await;
    assert_eq!(again.command.program, first.command.program);
    assert_eq!(again.command.args, first.command.args, "the same game");
    assert_eq!(again.command.env.get("EDEN_CLI_RESTART_HANDOFF").map(String::as_str), Some("1"));
    let running = test.core.view().running.expect("still running, in the same session");
    assert_eq!(running.key, "emu-lichii:01002B30028F6000");
    assert!(test.core.view().notices.iter().all(|n| !n.contains("code 75")), "no error for a restart");

    again.exit(0);
    test.until(nexus_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert_eq!(test.spawner.spawned().len(), 3, "a normal exit ends the game");
}

#[tokio::test]
async fn after_stop_a_game_asking_to_start_again_is_not_started() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: lichii(), version: None }).await.unwrap();
    let cli = app_run(&test).parent().unwrap().join("usr/bin/eden-cli");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(&cli, b"\x7fELF eden-cli").unwrap();
    test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    test.core
        .dispatch(Command::Session(SessionCmd::Launch("emu-lichii:01002B30028F6000".parse().unwrap())))
        .await
        .unwrap();
    let first = test.spawner.wait_for(2).await;
    let session = nexus_api::SessionId(test.core.view().running.expect("running").session);

    test.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();
    first.exit(75);
    test.until(nexus_core::Area::Sessions, |core| core.view().running.is_none()).await;
    assert_eq!(test.spawner.spawned().len(), 2, "Stop ends the game, whatever its code");
}

// The firmware: Nexus keeps the Switch firmware in Eden's folder up to date, from the releases
// of THZoria/NX_Firmware, when the player turns it on or asks for it.

use emu_lichii::release_url;

/// The firmware release Lichii starts with, and so the keys Nexus installs.
const FIRMWARE: &str = "22.5.0";
const KEYS_VERSION: &str = FIRMWARE;

/// A zip that stores `files` as they are (no compression), as the firmware releases do.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut index) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = sig.to_le_bytes().to_vec();
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            for size in [data.len() as u32; 2] {
                h.extend_from_slice(&size.to_le_bytes());
            }
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                // Comment length, disk, internal and external attributes.
                h.extend_from_slice(&[0; 10]);
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend_from_slice(data);
        index.extend(header(0x0201_4b50, true));
    }
    let at = out.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&index);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    for n in [count, count] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Serves the firmware release `tag` with `zip` as its asset, whose download redirects to
/// GitHub's CDN, and the keys of the same version. `digest` is the SHA-256 GitHub gives;
/// `None` for the zip's own.
fn serve_firmware(http: &FakeHttp, tag: &str, zip: Vec<u8>, digest: Option<&str>) {
    use std::io::Write;
    let mut keys = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("ProdKeys.NET/prod.keys", "test production keys"), ("title.keys", "test title keys")] {
        keys.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        keys.write_all(bytes.as_bytes()).unwrap();
    }
    http.serve(&format!("https://files.prodkeys.net/ProdKeys.NET-v{tag}.zip"), keys.finish().unwrap().into_inner());
    let name = format!("Firmware.{tag}.zip");
    let download = format!("https://github.com/THZoria/NX_Firmware/releases/download/{tag}/{name}");
    let cdn = format!("https://release-assets.githubusercontent.com/nx/{tag}/{name}");
    let size = zip.len();
    let sha = http.serve(&cdn, zip);
    http.redirect(&download, &cdn);
    let digest = digest.map(str::to_owned).unwrap_or(sha);
    let release = serde_json::json!({ "tag_name": tag, "published_at": "2026-11-10T01:55:37Z", "assets": [
        { "name": name, "browser_download_url": download, "size": size, "digest": format!("sha256:{digest}") },
    ]});
    http.serve(&release_url(tag), serde_json::to_vec(&release).unwrap());
}

/// Lichii with Eden's folder in a temporary folder that has the firmware `old` in it.
async fn with_eden(old: &[&str]) -> (TestCore, tempfile::TempDir, tempfile::TempDir) {
    with_eden_saved(old, None, None).await
}

/// [`with_eden`], with what an earlier run of Nexus saved (see [`boot_with_saved`]).
async fn with_eden_saved(
    old: &[&str],
    saved: Option<serde_json::Value>,
    firmware: Option<serde_json::Value>,
) -> (TestCore, tempfile::TempDir, tempfile::TempDir) {
    let (test, games) = boot_with_saved(saved, firmware).await;
    let eden = tempfile::tempdir().unwrap();
    let registered = eden.path().join("nand/system/Contents/registered");
    std::fs::create_dir_all(&registered).unwrap();
    for name in old {
        std::fs::write(registered.join(name), b"old firmware").unwrap();
    }
    test.core
        .dispatch(Command::SetFolders { name: "lichii-data".into(), paths: vec![eden.path().into()] })
        .await
        .unwrap();
    (test, games, eden)
}

/// The rows of the Firmware block on Lichii's panel.
fn firmware_rows(core: &nexus_core::Core) -> Option<Vec<nexus_ui_api::RowVm>> {
    let view = core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii())?.clone();
    emu.panels.into_iter().find(|p| p.id == "firmware").map(|p| p.rows)
}

fn row<'a>(rows: &'a [nexus_ui_api::RowVm], label: &str) -> Option<&'a nexus_ui_api::RowVm> {
    rows.iter().find(|r| r.label == label)
}

/// The Firmware block in a few words: "Firmware 22.5.0", "No firmware", or why the last update
/// failed.
fn firmware_badge(test: &TestCore) -> Option<String> {
    let rows = firmware_rows(&test.core)?;
    if let Some(failed) = row(&rows, "The last update failed") {
        return Some(format!("The firmware update failed: {}", failed.hint));
    }
    Some(match row(&rows, "Installed")?.value.as_str() {
        "None" => "No firmware".into(),
        "Not from Nexus" => "Firmware: not from Nexus".into(),
        version => format!("Firmware {version}"),
    })
}

/// Waits until no update runs: Update now is back.
async fn firmware_settles(test: &TestCore) -> String {
    test.until(nexus_core::Area::Plugins, |core| {
        firmware_rows(core).is_some_and(|rows| row(&rows, "Update now").is_some())
    })
    .await;
    firmware_badge(test).unwrap()
}

/// Presses Update the firmware on Lichii's panel, and waits for it to end.
async fn update_firmware(test: &TestCore) -> String {
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let action = "update-firmware".to_owned();
    test.core
        .dispatch(Command::RunAction { plugin: lichii(), action, game: None, args: serde_json::Value::Null })
        .await
        .unwrap();
    firmware_settles(test).await
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_newer_firmware_replaces_the_one_in_eden_s_folder_and_the_panel_names_it() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    let zip = stored_zip(&[("0a.cnmt.nca", b"meta"), ("1b.nca", &[7u8; 5000])]);
    serve_firmware(&test.http, FIRMWARE, zip, None);

    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");

    assert_eq!(std::fs::read(eden.path().join("keys/prod.keys")).unwrap(), b"test production keys");
    assert_eq!(std::fs::read(eden.path().join("keys/title.keys")).unwrap(), b"test title keys");
    let system = eden.path().join("nand/system");
    assert_eq!(files_in(&system.join("Contents/registered")), ["0a.cnmt.nca", "1b.nca"]);
    assert_eq!(std::fs::read(system.join("Contents/registered/1b.nca")).unwrap(), vec![7u8; 5000]);
    assert_eq!(files_in(&system), ["Contents"], "no download or old firmware left behind");
}

#[tokio::test]
async fn the_firmware_nexus_installed_last_downloads_nothing_again() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    update_firmware(&test).await;
    let downloads = || test.http.sent().iter().filter(|u| u.ends_with(".zip")).count();
    let before = downloads();

    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");
    assert_eq!(downloads(), before, "the zip isn't asked for again");
}

#[tokio::test]
async fn a_zip_with_a_folder_or_another_file_is_refused_and_the_old_firmware_stays() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    let registered = eden.path().join("nand/system/Contents/registered");
    let nca: &[u8] = b"nca";
    let in_a_folder: &[(&str, &[u8])] = &[("sub/1b.nca", nca)];
    let another_file: &[(&str, &[u8])] = &[("1b.nca", nca), ("readme.txt", b"hi")];
    for files in [in_a_folder, another_file] {
        serve_firmware(&test.http, FIRMWARE, stored_zip(files), None);

        let badge = update_firmware(&test).await;

        assert!(badge.starts_with("The firmware update failed"), "{badge}");
        assert_eq!(files_in(&registered), ["old.nca"]);
        assert_eq!(files_in(&eden.path().join("nand/system")), ["Contents"]);
    }
}

#[tokio::test]
async fn a_download_that_doesn_t_match_github_s_sha_256_leaves_the_old_firmware() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    let wrong = "0".repeat(64);
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), Some(&wrong));

    let badge = update_firmware(&test).await;

    assert!(badge.starts_with("The firmware update failed") && badge.contains("SHA-256"), "{badge}");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
    assert_eq!(files_in(&eden.path().join("nand/system")), ["Contents"]);
}

#[tokio::test]
async fn nexus_checks_for_firmware_on_its_own_only_once_the_player_turns_it_on() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    assert!(!test.http.sent().iter().any(|u| u.contains("NX_Firmware/releases/tags")), "off by default");
    assert_eq!(firmware_badge(&test).as_deref(), Some("No firmware"));

    turn_on_firmware_updates(&test).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();

    assert_eq!(firmware_settles(&test).await, "Firmware 22.5.0");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["1b.nca"]);
}

#[tokio::test]
async fn a_server_that_sends_the_whole_zip_for_a_piece_fails_the_update_and_the_old_firmware_stays() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    test.http.ignore_ranges();

    let badge = update_firmware(&test).await;

    assert!(badge.starts_with("The firmware update failed") && badge.contains("in pieces"), "{badge}");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
    assert_eq!(files_in(&eden.path().join("nand/system")), ["Contents"]);
}

#[tokio::test]
async fn the_old_firmware_of_an_update_that_stopped_between_its_renames_goes_back_first() {
    let (test, _games, eden) = with_eden(&[]).await;
    let system = eden.path().join("nand/system");
    std::fs::remove_dir(system.join("Contents/registered")).unwrap();
    std::fs::create_dir_all(system.join("registered.old")).unwrap();
    std::fs::write(system.join("registered.old/old.nca"), b"old firmware").unwrap();
    let wrong = "0".repeat(64);
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), Some(&wrong));

    update_firmware(&test).await;

    assert_eq!(files_in(&system.join("Contents/registered")), ["old.nca"]);
    assert_eq!(files_in(&system), ["Contents"]);
}

#[tokio::test]
async fn missing_keys_are_downloaded_again_for_the_installed_firmware() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");
    std::fs::remove_file(eden.path().join("keys/prod.keys")).unwrap();
    let before = test.http.sent().iter().filter(|u| u.contains("/nx/")).count();
    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");
    assert_eq!(std::fs::read(eden.path().join("keys/prod.keys")).unwrap(), b"test production keys");
    assert_eq!(test.http.sent().iter().filter(|u| u.contains("/nx/")).count(), before);
}

#[tokio::test]
async fn invalid_keys_leave_the_old_firmware_and_keys_in_place() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    std::fs::create_dir_all(eden.path().join("keys")).unwrap();
    std::fs::write(eden.path().join("keys/prod.keys"), b"old keys").unwrap();
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    test.http.serve(&format!("https://files.prodkeys.net/ProdKeys.NET-v{KEYS_VERSION}.zip"), b"invalid zip".to_vec());
    assert!(update_firmware(&test).await.contains("invalid keys zip"));
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
    assert_eq!(std::fs::read(eden.path().join("keys/prod.keys")).unwrap(), b"old keys");
}

#[tokio::test]
async fn a_keys_write_failure_restores_the_previous_firmware() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    std::fs::create_dir_all(eden.path().join("keys/prod.keys.part")).unwrap();
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    assert!(update_firmware(&test).await.starts_with("The firmware update failed"));
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
}

#[tokio::test]
async fn unavailable_matching_keys_name_the_version_and_preserve_existing_files() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    std::fs::create_dir_all(eden.path().join("keys")).unwrap();
    std::fs::write(eden.path().join("keys/prod.keys"), b"old keys").unwrap();
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    let url = format!("https://files.prodkeys.net/ProdKeys.NET-v{KEYS_VERSION}.zip");
    test.http.redirect(&url, "https://files.prodkeys.net/unavailable.zip");
    let badge = update_firmware(&test).await;
    assert!(badge.contains("keys archive version") && badge.contains(KEYS_VERSION) && badge.contains(&url), "{badge}");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
    assert_eq!(std::fs::read(eden.path().join("keys/prod.keys")).unwrap(), b"old keys");
    assert!(!test.http.sent().iter().any(|u| u.contains("/nx/")), "no firmware download without matching keys");
}

/// Turns on automatic firmware updates with the Firmware block's switch, the one place that
/// shows the setting.
async fn turn_on_firmware_updates(test: &TestCore) {
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let rows = firmware_rows(&test.core).unwrap();
    let switch = row(&rows, "Automatic updates").unwrap();
    assert_eq!(switch.switch, "off");
    let flip =
        Command::RunAction { plugin: lichii(), action: switch.action.clone(), game: None, args: switch.args.clone() };
    test.core.dispatch(flip).await.unwrap();
}

/// Types `version` into the Firmware block's Firmware release row, the one place that sets it.
async fn type_firmware_release(test: &TestCore, version: &str) -> Result<(), nexus_core::CoreError> {
    let edit = Command::RunAction {
        plugin: lichii(),
        action: "set-firmware-release".into(),
        game: None,
        args: serde_json::Value::String(version.into()),
    };
    test.core.dispatch(edit).await
}

/// Sets Lichii's firmware release in the Firmware block.
async fn set_firmware_release(test: &TestCore, version: &str) {
    type_firmware_release(test, version).await.unwrap();
}

/// With Automatic updates on, a read of the panel checks for the firmware release again once
/// 6 hours have passed, and not before.
#[tokio::test]
async fn automatic_updates_check_again_after_6_hours_and_not_before() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    turn_on_firmware_updates(&test).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    firmware_settles(&test).await;
    // The first check installs the firmware; a later one finds it up to date.
    assert!(firmware_status(&test).starts_with("Updated firmware"), "{}", firmware_status(&test));

    test.host.clock.advance(std::time::Duration::from_secs(5 * 60 * 60)).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    firmware_settles(&test).await;
    assert!(firmware_status(&test).starts_with("Updated firmware"), "no check before 6 hours");

    test.host.clock.advance(std::time::Duration::from_secs(2 * 60 * 60)).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    firmware_settles(&test).await;
    assert!(firmware_status(&test).contains("is up to date"), "a check after 6 hours: {}", firmware_status(&test));
}

/// A firmware settings file that doesn't read gives the defaults, and the Firmware block says
/// so: the player would otherwise not know why the release changed.
#[tokio::test]
async fn a_firmware_settings_file_that_doesn_t_read_is_shown_in_the_block() {
    let (test, _games, _eden) = with_eden(&[]).await;
    let plugin = test.data_dir().join("plugins/emu-lichii");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("firmware-settings.json"), b"not json").unwrap();
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let rows = firmware_rows(&test.core).unwrap();
    let warning = row(&rows, "Firmware settings").expect("a row about the settings file");
    assert!(warning.hint.contains("firmware-settings.json"), "{}", warning.hint);
    assert_eq!(row(&rows, "Firmware release").unwrap().value, "22.5.0", "the defaults apply");
}

/// Nexus 0.16.4 and older named Lichii's folders after Eden: a data folder the player set
/// under the old name is still Lichii's data folder.
#[tokio::test]
async fn lichii_s_data_folder_set_under_its_old_name_still_counts() {
    let eden = tempfile::tempdir().unwrap();
    let registered = eden.path().join("nand/system/Contents/registered");
    std::fs::create_dir_all(&registered).unwrap();
    std::fs::write(registered.join("old.nca"), b"old firmware").unwrap();
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let config = serde_json::json!({ "app": { "folders": { "eden-data": [eden.path()] } } });
    std::fs::write(host.data_dir().join("config.json"), config.to_string()).unwrap();
    let test = host.built_in(nexus_api::PluginBox::code(LichiiPlugin)).boot_core().await;

    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();

    let rows = firmware_rows(&test.core).unwrap();
    assert_eq!(row(&rows, "Installed").unwrap().value, "Not from Nexus", "the firmware in that folder");
}

/// Lichii lists no keyboard keys (its own window sets them), so its panel has no Keyboard
/// controls block: an empty one would say nothing.
#[tokio::test]
async fn lichii_s_panel_has_no_empty_keyboard_block() {
    let (test, _games, _eden) = with_eden(&[]).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == lichii()).expect("Lichii's panel");
    let ids: Vec<_> = emu.panels.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"firmware"), "{ids:?}");
    assert!(!ids.contains(&"keyboard"), "{ids:?}");
}

/// The Firmware block has a Firmware release row: it shows the set version, and typing a new
/// one into it saves the setting, so the next Update now installs that release.
#[tokio::test]
async fn the_firmware_block_s_release_row_edits_the_version_and_the_next_update_follows_it() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("22.nca", b"nca")]), None);
    serve_firmware(&test.http, "22.1.0", stored_zip(&[("21.nca", b"nca")]), None);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();

    let rows = firmware_rows(&test.core).unwrap();
    let release = row(&rows, "Firmware release").expect("a Firmware release row");
    assert_eq!(release.value, "22.5.0", "it shows the set version");
    assert!(release.input, "it takes a typed line");
    assert!(!release.action.is_empty(), "it is a button");
    assert!(!row(&rows, "Automatic updates").unwrap().input, "a switch row isn't typed into");

    let edit = Command::RunAction {
        plugin: lichii(),
        action: release.action.clone(),
        game: None,
        args: serde_json::Value::String("22.1.0".into()),
    };
    test.core.dispatch(edit).await.unwrap();
    test.until(nexus_core::Area::Plugins, |core| {
        firmware_rows(core).and_then(|r| row(&r, "Firmware release").map(|r| r.value.clone())) == Some("22.1.0".into())
    })
    .await;

    assert_eq!(update_firmware(&test).await, "Firmware 22.1.0");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["21.nca"]);
}

/// A typed release that isn't a version, like "../latest", is refused, and the set version stays.
#[tokio::test]
async fn a_typed_release_that_isn_t_a_version_is_refused_and_the_version_stays() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("22.nca", b"nca")]), None);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let action = row(&firmware_rows(&test.core).unwrap(), "Firmware release").unwrap().action.clone();

    let bad = Command::RunAction {
        plugin: lichii(),
        action,
        game: None,
        args: serde_json::Value::String("../latest".into()),
    };
    let result = test.core.dispatch(bad).await;

    assert!(result.is_err(), "a release that isn't a version is refused: {result:?}");
    assert_eq!(row(&firmware_rows(&test.core).unwrap(), "Firmware release").unwrap().value, "22.5.0");
}

/// Nexus 0.16.4 and older kept the firmware settings among Lichii's settings. The Firmware
/// block takes them over once, and the settings list no longer has them.
#[tokio::test]
async fn firmware_settings_saved_by_an_older_nexus_move_into_the_firmware_block() {
    let saved = serde_json::json!({ "firmware_updates": true, "firmware_release": "22.1.0", "fullscreen": false });
    let (test, _games, _eden) = with_eden_saved(&[], Some(saved), None).await;
    serve_firmware(&test.http, "22.1.0", stored_zip(&[("21.nca", b"nca")]), None);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();

    assert_eq!(firmware_settles(&test).await, "Firmware 22.1.0", "automatic updates stayed on");
    let rows = firmware_rows(&test.core).unwrap();
    assert_eq!(row(&rows, "Firmware release").unwrap().value, "22.1.0");
    assert_eq!(row(&rows, "Automatic updates").unwrap().switch, "on");
    test.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let view = test.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == lichii()).expect("the Lichii panel");
    assert!(panel.fields.iter().all(|f| !f.key.starts_with("firmware")), "the settings list has no firmware row");
}

/// Firmware settings that are there already stay: the older Nexus's settings move only once.
#[tokio::test]
async fn the_firmware_settings_there_already_are_kept_over_older_saved_ones() {
    let saved = serde_json::json!({ "firmware_updates": true, "firmware_release": "22.1.0" });
    let ours = serde_json::json!({ "automatic": false, "release": "21.0.0" });
    let (test, _games, _eden) = with_eden_saved(&[], Some(saved), Some(ours)).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let rows = firmware_rows(&test.core).unwrap();
    assert_eq!(row(&rows, "Firmware release").unwrap().value, "21.0.0");
    assert_eq!(row(&rows, "Automatic updates").unwrap().switch, "off");
}

/// A release that isn't a version, in the firmware settings file itself (written by hand, or
/// by a restore), is refused at the update too, and nothing is fetched for it.
#[tokio::test]
async fn a_bad_release_in_the_firmware_settings_file_is_refused_at_the_update() {
    let ours = serde_json::json!({ "automatic": false, "release": "../latest" });
    let (test, _games, eden) = with_eden_saved(&["old.nca"], None, Some(ours)).await;

    let badge = update_firmware(&test).await;

    assert!(badge.starts_with("The firmware update failed") && badge.contains("../latest"), "{badge}");
    assert!(test.http.sent().iter().all(|u| !u.contains("latest")), "nothing is fetched for it");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
}

/// A saved firmware release that isn't a version doesn't move into the Firmware block: the
/// block starts from its default.
#[tokio::test]
async fn a_saved_firmware_release_that_isn_t_a_version_is_left_behind() {
    let saved = serde_json::json!({ "firmware_release": "../latest" });
    let (test, _games, _eden) = with_eden_saved(&[], Some(saved), None).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    assert_eq!(row(&firmware_rows(&test.core).unwrap(), "Firmware release").unwrap().value, "22.5.0");
}

/// Settings saved by Nexus 0.16 and older hold a keys version: they still load, and the
/// firmware release replaces it.
#[tokio::test]
async fn settings_with_the_old_keys_version_still_load() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("22.nca", b"nca")]), None);
    let old = Command::SetConfigField {
        plugin: lichii(),
        scope: ConfigScope::Emulator,
        key: "keys_version".into(),
        value: "22.1.0".into(),
    };
    test.core.dispatch(old).await.unwrap();

    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");
}

/// Firmware newer than its keys can't be read (Lichii loops on "keys for gen … required"), so
/// Nexus installs the firmware release the player set, not the newest one, and its keys.
#[tokio::test]
async fn the_firmware_and_keys_are_of_the_firmware_release() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("22.nca", b"nca")]), None);
    serve_firmware(&test.http, "22.1.0", stored_zip(&[("21.nca", b"nca")]), None);
    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");

    set_firmware_release(&test, "22.1.0").await;

    assert_eq!(update_firmware(&test).await, "Firmware 22.1.0");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["21.nca"]);
    assert!(test.http.sent().iter().any(|u| u == "https://files.prodkeys.net/ProdKeys.NET-v22.1.0.zip"));
}

#[tokio::test]
async fn a_firmware_release_github_doesn_t_have_fails_and_the_old_firmware_stays() {
    let (test, _games, eden) = with_eden(&["old.nca"]).await;
    set_firmware_release(&test, "99.0.0").await;

    let badge = update_firmware(&test).await;

    assert!(badge.starts_with("The firmware update failed") && badge.contains("99.0.0"), "{badge}");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["old.nca"]);
    assert!(!test.http.sent().iter().any(|u| u.contains("prodkeys")), "no keys without their firmware");
}

#[tokio::test]
async fn a_typed_firmware_release_may_start_with_v_and_anything_else_but_a_version_is_refused() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, "22.1.0", stored_zip(&[("21.nca", b"nca")]), None);
    set_firmware_release(&test, "v22.1.0").await;
    assert_eq!(update_firmware(&test).await, "Firmware 22.1.0");

    for bad in ["../latest", "..", ".", "1..2"] {
        let refused = type_firmware_release(&test, bad).await.unwrap_err();
        assert!(refused.to_string().contains(bad), "{bad}: {refused}");
        test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
        let release = row(&firmware_rows(&test.core).unwrap(), "Firmware release").unwrap().value.clone();
        assert_eq!(release, "22.1.0", "{bad}: the release stays as it was");
    }
}

/// What the Firmware block says of the update: the step that runs, or the last check's result.
fn firmware_status(test: &TestCore) -> String {
    let rows = firmware_rows(&test.core).unwrap();
    match row(&rows, "Last check") {
        Some(last) => last.hint.clone(),
        None => rows[1].label.clone(),
    }
}

#[tokio::test]
async fn a_firmware_check_shows_progress_then_reports_updated_or_up_to_date() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    let gate = test.http.hold_next(&release_url(FIRMWARE));
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    test.core
        .dispatch(Command::RunAction {
            plugin: lichii(),
            action: "update-firmware".into(),
            game: None,
            args: serde_json::Value::Null,
        })
        .await
        .unwrap();
    assert_eq!(firmware_status(&test), "Checking for firmware updates…");
    gate.release();
    firmware_settles(&test).await;
    assert_eq!(firmware_status(&test), "Updated firmware to 22.5.0. Installed keys 22.5.0.");
    update_firmware(&test).await;
    assert_eq!(firmware_status(&test), "Firmware 22.5.0 is up to date. Keys 22.5.0 are installed. No updates needed.");
}

#[tokio::test]
async fn deleted_firmware_is_restored_by_a_manual_update_despite_the_saved_version() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    update_firmware(&test).await;
    let folder = eden.path().join("nand/system/Contents/registered");
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(update_firmware(&test).await, "Firmware 22.5.0");
    assert_eq!(std::fs::read(folder.join("1b.nca")).unwrap(), b"nca");
}

#[tokio::test]
async fn deleted_or_empty_firmware_bypasses_the_automatic_check_interval() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    turn_on_firmware_updates(&test).await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    firmware_settles(&test).await;
    let folder = eden.path().join("nand/system/Contents/registered");
    for empty in [false, true] {
        std::fs::remove_dir_all(&folder).unwrap();
        if empty {
            std::fs::create_dir_all(&folder).unwrap();
        }
        test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
        firmware_settles(&test).await;
        assert_eq!(std::fs::read(folder.join("1b.nca")).unwrap(), b"nca");
    }
}

#[tokio::test]
async fn the_firmware_block_s_switch_turns_automatic_updates_on_and_a_check_runs() {
    let (test, _games, eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let rows = firmware_rows(&test.core).unwrap();
    let switch = row(&rows, "Automatic updates").unwrap();
    assert_eq!(switch.switch, "off", "off by default");

    let flip =
        Command::RunAction { plugin: lichii(), action: switch.action.clone(), game: None, args: switch.args.clone() };
    test.core.dispatch(flip).await.unwrap();

    assert_eq!(firmware_settles(&test).await, "Firmware 22.5.0");
    assert_eq!(row(&firmware_rows(&test.core).unwrap(), "Automatic updates").unwrap().switch, "on");
    assert_eq!(files_in(&eden.path().join("nand/system/Contents/registered")), ["1b.nca"]);
}

#[tokio::test]
async fn the_firmware_block_shows_the_download_as_a_bar_without_update_now() {
    let (test, _games, _eden) = with_eden(&[]).await;
    serve_firmware(&test.http, FIRMWARE, stored_zip(&[("1b.nca", b"nca")]), None);
    let cdn = format!("https://release-assets.githubusercontent.com/nx/{FIRMWARE}/Firmware.{FIRMWARE}.zip");
    let gate = test.http.hold_next(&cdn);
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let action = "update-firmware".to_owned();
    test.core
        .dispatch(Command::RunAction { plugin: lichii(), action, game: None, args: serde_json::Value::Null })
        .await
        .unwrap();

    test.until(nexus_core::Area::Plugins, |core| {
        firmware_rows(core).is_some_and(|rows| row(&rows, "Downloading the firmware").is_some())
    })
    .await;
    let rows = firmware_rows(&test.core).unwrap();
    let download = row(&rows, "Downloading the firmware").unwrap();
    assert_eq!(download.progress, 0.0);
    assert_eq!(download.value, "0%");
    assert!(download.hint.starts_with("0.0 MB of "), "{}", download.hint);
    assert!(row(&rows, "Update now").is_none(), "no second update while one runs");

    gate.release();
    assert_eq!(firmware_settles(&test).await, "Firmware 22.5.0");
}
