//! The shadPS4 plugin on a fake Linux host: the official zip with its AppImage, PS4 games
//! (but not their updates) from the player's folder, and community compatibility.

use std::path::Path;

use emu_shadps4::{COMPATIBILITY, RELEASES, ShadPlugin};
use emulator_kit::sfo::{Value as Sfo, build};
use nexus_api::*;
use nexus_core::{Area, Command};
use nexus_testkit::{FakeHost, FakeHttp, FakeSpawner, TestCore, contract};

const TAG: &str = "v.0.18.0";

#[test]
fn the_manifest_s_key_map_is_the_keyboard_panel_s() {
    let manifest = ShadPlugin.manifest();
    let keys: Vec<_> = manifest.keymap.iter().map(|k| (k.button.as_str(), k.keys.as_str())).collect();
    assert_eq!(keys, emu_shadps4::KEYS);
}

fn shad() -> PluginId {
    PluginId::new("emu-shadps4").unwrap()
}

fn zip_with_appimage() -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("Shadps4-sdl.AppImage", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(b"\x7fELF AppImage").unwrap();
    zip.finish().unwrap().into_inner()
}

fn ps4_game(root: &Path, folder: &str, category: &str, title_id: &str, title: &str) {
    let sce = root.join(folder).join("sce_sys");
    std::fs::create_dir_all(&sce).unwrap();
    let sfo = build(&[
        ("CATEGORY", Sfo::Text(category.into())),
        ("TITLE", Sfo::Text(title.into())),
        ("TITLE_ID", Sfo::Text(title_id.into())),
        ("APP_VER", Sfo::Text("01.08".into())),
    ]);
    std::fs::write(sce.join("param.sfo"), sfo).unwrap();
    std::fs::write(sce.join("icon0.png"), b"icon").unwrap();
    std::fs::write(sce.join("pic1.png"), b"background").unwrap();
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
                std::fs::write(root.join("AppRun"), b"#!/bin/sh\n").unwrap();
                process.exit(0);
            }
        }
    });
}

/// A GitHub release of `tag` with a Windows and a Linux build, whose download redirects to
/// GitHub's CDN.
fn release(http: &FakeHttp, tag: &str, published: &str) -> serde_json::Value {
    let version = tag.trim_start_matches("v.");
    let name = format!("shadps4-linux-sdl-{version}.zip");
    let download = format!("https://github.com/shadps4-emu/shadPS4/releases/download/{tag}/{name}");
    let cdn = format!("https://release-assets.githubusercontent.com/shadps4/{tag}/linux.zip");
    let sha = http.serve(&cdn, zip_with_appimage());
    http.redirect(&download, &cdn);
    serde_json::json!({ "tag_name": tag, "published_at": published, "assets": [
        { "name": format!("shadps4-win64-sdl-{version}.zip"), "browser_download_url": "https://github.com/w.zip", "size": 1 },
        { "name": name, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
    ]})
}

/// Where the build of `tag` keeps the program games start.
fn app_run(test: &TestCore, tag: &str) -> std::path::PathBuf {
    test.data_dir().join("plugins/emu-shadps4/builds").join(tag).join("squashfs-root/AppRun")
}

/// What a shadPS4 build prints for `--help`.
const HELP: &[u8] = b"shadPS4 v0.18.0\nUsage: shadps4 [options] <elf or eboot.bin path>\n";

async fn boot() -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let releases = serde_json::json!([release(host.http(), TAG, "2026-10-01T00:00:00Z")]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let compat_cdn = "https://release-assets.githubusercontent.com/compat.json";
    host.http().redirect(COMPATIBILITY, compat_cdn);
    let compat = serde_json::json!({ "CUSA04118": { "os-linux": { "status": "status-playable", "last_tested": "2026-09-01" } } });
    host.http().serve(compat_cdn, compat.to_string().into_bytes());
    let games = tempfile::tempdir().unwrap();
    ps4_game(games.path(), "CUSA04118", "gd", "CUSA04118", "Firewatch");
    ps4_game(games.path(), "CUSA04118-patch", "gp", "CUSA04118", "Firewatch");
    let test = host.built_in(nexus_api::PluginBox::code(ShadPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    test.spawner.prints(&app_run(&test, TAG), HELP);
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    (test, games)
}

#[tokio::test]
async fn shadps4_meets_every_contract() {
    let (test, _games) = boot().await;
    contract::emulator(&test, &shad()).await;
}

#[tokio::test]
async fn full_games_are_listed_with_their_status_and_updates_are_not() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let home = test.core.view().home;
    assert_eq!(home.games.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["emu-shadps4:CUSA04118"]);
    assert_eq!(home.games[0].badges[0].text, "Playable on Linux");
    assert_eq!(home.games[0].platform, "PS4");
}

#[tokio::test]
async fn the_game_hub_says_when_the_result_was_tested() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    test.core.dispatch(Command::OpenGame("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();

    let compat = test.core.view().detail.unwrap().compat.expect("the community's result");
    assert_eq!((compat.label.as_str(), compat.tested.as_str()), ("Playable", "2026-09-01"));
    assert_eq!(compat.notes, "Tested on Linux", "the date shows once");
}

#[tokio::test]
async fn the_app_run_from_the_appimage_starts_the_game_folder() {
    let (test, games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: shad(), version: None }).await.unwrap();
    let app_run = test.data_dir().join("plugins/emu-shadps4/builds").join(TAG).join("squashfs-root/AppRun");
    assert!(app_run.is_file());

    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();

    let process = test.spawner.wait_for(2).await;
    assert_eq!(process.command.program, app_run);
    let game = games.path().join("CUSA04118").display().to_string();
    assert_eq!(process.command.args, ["-g".to_owned(), game, "-f".to_owned(), "true".to_owned()]);
}

#[tokio::test]
async fn a_new_build_that_does_not_run_here_is_removed_and_the_build_before_stays_current() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::InstallEmulator { plugin: shad(), version: None }).await.unwrap();
    const NEXT: &str = "v.0.19.0";
    let list = serde_json::json!([
        release(&test.http, NEXT, "2026-10-12T00:00:00Z"),
        release(&test.http, TAG, "2026-10-01T00:00:00Z"),
    ]);
    test.http.serve(RELEASES, serde_json::to_vec(&list).unwrap());
    let check =
        Command::RunAction { plugin: shad(), action: "check".into(), game: None, args: serde_json::Value::Null };
    test.core.dispatch(check).await.unwrap();
    // The new build starts, but it is not shadPS4 that answers.
    test.spawner.prints(&app_run(&test, NEXT), b"error while loading shared libraries: libSDL3.so.0\n");

    let err = test.core.dispatch(Command::UpdateEmulator { plugin: shad() }).await.unwrap_err();

    assert!(err.to_string().contains("does not run on this system"), "{err}");
    let ran = test.spawner.ran();
    assert_eq!(ran.last().map(|c| c.args.clone()), Some(vec!["--help".to_owned()]), "a test run with --help");
    assert!(!test.data_dir().join("plugins/emu-shadps4/builds").join(NEXT).exists(), "the broken build goes");
    assert_eq!(test.core.view().emulators.emulators[0].installed, [TAG]);
    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();
    let program = test.spawner.spawned().last().unwrap().command.program.clone();
    assert_eq!(program, app_run(&test, TAG), "games start on the build before");
}

#[tokio::test]
async fn the_emulators_screen_names_the_controller_and_lists_the_keyboard_keys() {
    let (test, _games) = boot().await;
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == shad()).expect("shadPS4 on the Emulators screen");
    assert_eq!(emu.controllers, ["Controller (DualSense, DualShock 4, Xbox and most others)", "Keyboard"]);

    let keys = emu.panels.iter().find(|p| p.label == "emu-shadps4:keyboard").expect("the keyboard keys");
    let key = |what: &str| keys.rows.iter().find(|r| r.label == what).map(|r| r.value.as_str());
    assert_eq!(key("Cross"), Some("N (or keypad 2)"));
    assert_eq!(key("Right stick"), Some("I J K L"));
    assert_eq!(key("Touchpad"), Some("Space"));
}

/// A shadPS4 you installed yourself, in a temporary folder mapped as `shadps4-build`.
async fn own_build(test: &TestCore) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Shadps4-sdl.AppImage"), b"\x7fELF AppImage").unwrap();
    let folders = Command::SetFolders { name: "shadps4-build".into(), paths: vec![dir.path().into()] };
    test.core.dispatch(folders).await.unwrap();
    dir
}

fn set(key: &str, value: &str) -> Command {
    Command::SetConfigField { plugin: shad(), scope: ConfigScope::Emulator, key: key.into(), value: value.into() }
}

#[tokio::test]
async fn your_own_build_runs_games_from_its_folder() {
    let (test, _games) = boot().await;
    let own = own_build(&test).await;
    test.core.dispatch(set("own_build", "true")).await.unwrap();

    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == shad()).unwrap();
    assert_eq!(emu.installed, ["Your own build"]);

    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();
    let process = test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, own.path().join("Shadps4-sdl.AppImage"));
    assert_eq!(process.command.cwd, own.path());
}

#[tokio::test]
async fn a_shadps4_on_the_path_is_found_and_can_be_chosen() {
    let (test, _games) = boot().await;
    test.spawner.on_path("shadps4", Path::new("/usr/local/bin/shadps4"));

    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == shad()).unwrap();
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some("use-own")).expect("an offer to use it");
    assert!(offer.text.contains("/usr/local/bin/shadps4"), "{}", offer.text);

    let use_own =
        Command::RunAction { plugin: shad(), action: "use-own".into(), game: None, args: serde_json::Value::Null };
    test.core.dispatch(use_own).await.unwrap();
    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();
    assert_eq!(test.spawner.wait_for(1).await.command.program, Path::new("shadps4"), "run from PATH, by name");
}

#[tokio::test]
async fn installing_an_official_build_switches_away_from_your_own() {
    let (test, _games) = boot().await;
    let _own = own_build(&test).await;
    test.core.dispatch(set("own_build", "true")).await.unwrap();

    test.core.dispatch(Command::InstallEmulator { plugin: shad(), version: None }).await.unwrap();
    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();
    let program = test.spawner.spawned().last().unwrap().command.program.clone();
    assert!(program.ends_with("squashfs-root/AppRun"), "the official build runs: {program:?}");
}

/// Maps a `packages` folder with `files` in its `PS4` folder, and shadPS4's data folder;
/// returns both.
async fn packages(test: &TestCore, files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("PS4")).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.path().join("PS4").join(name), bytes).unwrap();
    }
    let data = tempfile::tempdir().unwrap();
    for (name, path) in [("packages", dir.path()), ("shadps4-data", data.path())] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![path.into()] }).await.unwrap();
    }
    (dir, data)
}

/// Starts the install the Emulators screen offers for `file`, and waits for it to end.
async fn install_game(test: &TestCore, file: &str) -> String {
    test.core.dispatch(Command::Navigate(nexus_core::Route::Emulators)).await.unwrap();
    let view = test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == shad()).unwrap();
    let offer = emu
        .panels
        .iter()
        .find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME) && p.text.contains(file))
        .unwrap_or_else(|| panic!("an offer to install {file}: {:?}", emu.panels));
    let start =
        Command::RunAction { plugin: shad(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    let jobs_before = test.core.view().downloads.jobs.len();
    // Some things fail before any copying starts, such as an update without its game.
    if let Err(e) = test.core.dispatch(start).await {
        return format!("failed {e}");
    }
    let ended = |core: &nexus_core::Core| {
        let jobs = core.view().downloads.jobs;
        jobs.len() > jobs_before && jobs.first().is_some_and(|j| j.state != "installing")
    };
    test.until(Area::Installs, ended).await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    format!("{} {}", job.state, job.error)
}

fn ps4_package(kind: emu_shadps4::pkg4::Kind, files: &[(&str, &[u8])]) -> Vec<u8> {
    use emu_shadps4::pkg4::build::{Spec, package};
    package(&Spec { files, kind, ..Default::default() })
}

#[tokio::test]
async fn a_ps4_package_installs_into_the_games_folder_and_shows_in_the_library() {
    use emu_shadps4::pkg4::Kind;
    let (test, games) = boot().await;
    let _p =
        packages(&test, &[("game.pkg", ps4_package(Kind::Game, &[("eboot.bin", b"\x4f\x15\x3d\x1dfixture")]))]).await;

    assert_eq!(install_game(&test, "game.pkg").await.trim(), "done");
    let game = games.path().join("PS4/CUSA12345");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"\x4f\x15\x3d\x1dfixture");
    assert!(game.join("sce_sys/param.sfo").is_file());
    let titles: Vec<_> = test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Fixture".to_owned()), "{titles:?}");
}

#[tokio::test]
async fn an_update_goes_next_to_its_game_and_needs_the_game_first() {
    use emu_shadps4::pkg4::Kind;
    let (test, games) = boot().await;
    let _p = packages(
        &test,
        &[
            ("game.pkg", ps4_package(Kind::Game, &[("eboot.bin", b"v1")])),
            ("patch.pkg", ps4_package(Kind::Update, &[("eboot.bin", b"v2")])),
        ],
    )
    .await;

    let early = install_game(&test, "patch.pkg").await;
    assert!(early.starts_with("failed") && early.contains("first"), "{early}");
    assert_eq!(install_game(&test, "game.pkg").await.trim(), "done");
    assert_eq!(install_game(&test, "patch.pkg").await.trim(), "done");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345-patch/eboot.bin")).unwrap(), b"v2");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345/eboot.bin")).unwrap(), b"v1", "the game stays");
}

#[tokio::test]
async fn uninstalling_a_game_deletes_its_update_and_its_add_ons_too() {
    let (test, games) = boot().await;
    let data = tempfile::tempdir().unwrap();
    let addon = data.path().join("addcont/CUSA04118/ADDON00000000001");
    std::fs::create_dir_all(&addon).unwrap();
    std::fs::write(addon.join("music.bin"), b"song").unwrap();
    let other = data.path().join("addcont/CUSA99999");
    std::fs::create_dir_all(&other).unwrap();
    test.core
        .dispatch(Command::SetFolders { name: "shadps4-data".into(), paths: vec![data.path().into()] })
        .await
        .unwrap();
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let game: nexus_api::GameKey = "emu-shadps4:CUSA04118".parse().unwrap();
    test.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();

    test.core.dispatch(Command::UninstallGame(game)).await.unwrap();
    assert!(!games.path().join("CUSA04118").exists(), "the game");
    assert!(!games.path().join("CUSA04118-patch").exists(), "its update");
    assert!(!data.path().join("addcont/CUSA04118").exists(), "its add-ons");
    assert!(other.exists(), "another game's add-ons stay");
}

#[tokio::test]
async fn dlc_goes_into_shadps4_s_add_on_folder() {
    use emu_shadps4::pkg4::Kind;
    let (test, _games) = boot().await;
    let (_p, data) = packages(&test, &[("addon.pkg", ps4_package(Kind::Dlc, &[("music.bin", b"song")]))]).await;

    assert_eq!(install_game(&test, "addon.pkg").await.trim(), "done");
    assert_eq!(std::fs::read(data.path().join("addcont/CUSA12345/ADDON00000000001/music.bin")).unwrap(), b"song");
}

#[tokio::test]
async fn one_games_folder_holds_games_for_both_consoles_and_each_emulator_finds_its_own() {
    let games = tempfile::tempdir().unwrap();
    ps4_game(games.path(), "PS4/CUSA04118", "gd", "CUSA04118", "Firewatch");
    let sce = games.path().join("PS5/Astro/sce_sys");
    std::fs::create_dir_all(&sce).unwrap();
    let param =
        serde_json::json!({ "titleId": "PPSA01234", "localizedParameters": { "en-US": { "titleName": "Astro Bot" } } });
    std::fs::write(sce.join("param.json"), param.to_string()).unwrap();
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .built_in(nexus_api::PluginBox::code(ShadPlugin))
        .built_in(nexus_api::PluginBox::code(emu_kyty::KytyPlugin))
        .boot_core()
        .await;
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let mut keys: Vec<_> = test.core.view().home.games.into_iter().map(|g| g.key).collect();
    keys.sort();
    assert_eq!(keys, ["emu-kyty:PPSA01234", "emu-shadps4:CUSA04118"]);
    let folders: Vec<_> = test.core.view().settings.folders.into_iter().filter(|f| f.name == "games").collect();
    assert_eq!(folders.len(), 1, "one games folder for both");
    assert_eq!(folders[0].title, "Game folders");
}

#[tokio::test]
async fn on_macos_the_official_zip_installs_and_its_shadps4_starts_the_game() {
    use std::io::Write;
    // The real macOS zip: the program and its Vulkan libraries at the top.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [("shadps4", &b"\xcf\xfa\xed\xfe mach-o"[..]), ("libvulkan.dylib", b"lib")] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    let zip = zip.finish().unwrap().into_inner();
    let host = FakeHost::new().platform(Some(Platform::Macos));
    let name = "shadps4-macos-sdl-2026-0.20.0.zip";
    let download = format!("https://github.com/shadps4-emu/shadPS4/releases/download/v.0.20.0/{name}");
    let sha = host.http().serve(&download, zip);
    let releases = serde_json::json!([{ "tag_name": "v.0.20.0", "published_at": "2026-10-14T00:00:00Z", "assets": [
        { "name": "shadps4-linux-sdl-0.20.0.zip", "browser_download_url": "https://github.com/l.zip", "size": 1 },
        { "name": name, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
    ]}]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    ps4_game(games.path(), "CUSA04118", "gd", "CUSA04118", "Firewatch");
    let test = host.built_in(nexus_api::PluginBox::code(ShadPlugin)).boot_core().await;
    assert!(test.core.kernel().order().contains(&shad()), "loaded on macOS");
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    let program = test.data_dir().join("plugins/emu-shadps4/builds/v.0.20.0/shadps4");
    test.spawner.prints(&program, HELP);

    test.core.dispatch(Command::InstallEmulator { plugin: shad(), version: None }).await.unwrap();
    assert!(program.is_file());
    test.core.dispatch(Command::Launch("emu-shadps4:CUSA04118".parse().unwrap())).await.unwrap();
    assert_eq!(test.spawner.wait_for(1).await.command.program, program);
}

/// Installs the download at `PS4/<folder>` in the packages folder, and waits for it to end.
async fn install_download(test: &TestCore, folder: &str) -> String {
    let source = serde_json::json!({ "folder": "packages", "root": 0, "path": format!("PS4/{folder}") });
    let start = Command::RunAction { plugin: shad(), action: HOST_INSTALL_GAME.into(), game: None, args: source };
    if let Err(e) = test.core.dispatch(start).await {
        return format!("failed {e}");
    }
    test.until(Area::Installs, |core| core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing"))
        .await;
    let job = test.core.view().downloads.jobs.into_iter().next().unwrap();
    format!("{} {}", job.state, job.error)
}

/// Writes `files` into the download folder `PS4/torrent-abc`.
fn download(packages: &Path, files: &[(&str, Vec<u8>)]) {
    for (name, bytes) in files {
        let path = packages.join("PS4/torrent-abc").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

#[tokio::test]
async fn a_download_with_the_game_its_update_and_its_dlc_installs_all_three_in_one_job() {
    use emu_shadps4::pkg4::{
        Kind,
        build::{Spec, package},
    };
    let (test, games) = boot().await;
    let (p, data) = packages(&test, &[]).await;
    let dlc = |label: &'static str, song: &'static [u8]| {
        package(&Spec { files: &[("music.bin", song)], kind: Kind::Dlc, label, ..Default::default() })
    };
    download(
        p.path(),
        &[
            ("Game/game.pkg", ps4_package(Kind::Game, &[("eboot.bin", b"v1")])),
            ("Game/Update/patch.pkg", ps4_package(Kind::Update, &[("eboot.bin", b"v2")])),
            ("Game/DLC/one.pkg", dlc("ADDON00000000001", b"one")),
            ("Game/DLC/two.pkg", dlc("ADDON00000000002", b"two")),
        ],
    );

    assert_eq!(install_download(&test, "torrent-abc").await.trim(), "done");
    assert_eq!(test.core.view().downloads.jobs.len(), 1, "one job");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345/eboot.bin")).unwrap(), b"v1");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345-patch/eboot.bin")).unwrap(), b"v2");
    let addcont = data.path().join("addcont/CUSA12345");
    assert_eq!(std::fs::read(addcont.join("ADDON00000000001/music.bin")).unwrap(), b"one");
    assert_eq!(std::fs::read(addcont.join("ADDON00000000002/music.bin")).unwrap(), b"two");
    let left: Vec<_> = std::fs::read_dir(games.path().join("PS4")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!left.iter().any(|n| n.to_string_lossy().ends_with(".part")), "{left:?}");
}

#[tokio::test]
async fn a_download_with_two_games_or_two_updates_is_refused_before_anything_is_written() {
    use emu_shadps4::pkg4::Kind;
    let (test, games) = boot().await;
    let (p, _data) = packages(&test, &[]).await;
    let game = ps4_package(Kind::Game, &[("eboot.bin", b"v1")]);
    download(p.path(), &[("a/game.pkg", game.clone()), ("b/game.pkg", game.clone())]);
    let two_games = install_download(&test, "torrent-abc").await;
    assert!(two_games.starts_with("failed") && two_games.contains("2 games"), "{two_games}");

    std::fs::remove_dir_all(p.path().join("PS4/torrent-abc")).unwrap();
    let update = ps4_package(Kind::Update, &[("eboot.bin", b"v2")]);
    download(p.path(), &[("game.pkg", game), ("u1/patch.pkg", update.clone()), ("u2/patch.pkg", update)]);
    let two_updates = install_download(&test, "torrent-abc").await;
    assert!(two_updates.starts_with("failed") && two_updates.contains("2 updates"), "{two_updates}");
    assert!(!games.path().join("PS4/CUSA12345").exists(), "nothing written");
}

#[tokio::test]
async fn an_update_alone_needs_its_game_and_an_add_on_already_there_stays() {
    use emu_shadps4::pkg4::Kind;
    let (test, games) = boot().await;
    let (p, data) = packages(&test, &[]).await;
    download(p.path(), &[("patch.pkg", ps4_package(Kind::Update, &[("eboot.bin", b"v2")]))]);
    let early = install_download(&test, "torrent-abc").await;
    assert!(early.starts_with("failed") && early.contains("install the game first"), "{early}");

    let addon = data.path().join("addcont/CUSA12345/ADDON00000000001");
    std::fs::create_dir_all(&addon).unwrap();
    std::fs::write(addon.join("music.bin"), b"mine").unwrap();
    std::fs::remove_dir_all(p.path().join("PS4/torrent-abc")).unwrap();
    download(
        p.path(),
        &[
            ("game.pkg", ps4_package(Kind::Game, &[("eboot.bin", b"v1")])),
            ("addon.pkg", ps4_package(Kind::Dlc, &[("music.bin", b"new")])),
        ],
    );
    assert_eq!(install_download(&test, "torrent-abc").await.trim(), "done");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345/eboot.bin")).unwrap(), b"v1");
    assert_eq!(std::fs::read(addon.join("music.bin")).unwrap(), b"mine", "an installed add-on isn't replaced");
}

#[tokio::test]
async fn an_unpacked_ps4_game_folder_installs_as_it_is_and_an_update_folder_does_not() {
    let (test, games) = boot().await;
    let (p, _data) = packages(&test, &[]).await;
    let download_dir = p.path().join("PS4/torrent-abc/Release");
    ps4_game(&download_dir, "CUSA55555-patch", "gp", "CUSA55555", "Hollow");
    let refused = install_download(&test, "torrent-abc").await;
    assert!(refused.starts_with("failed") && refused.contains("holds no"), "{refused}");

    ps4_game(&download_dir, "CUSA55555", "gd", "CUSA55555", "Hollow");
    std::fs::write(download_dir.join("CUSA55555/eboot.bin"), b"\x4f\x15\x3d\x1dgame").unwrap();
    std::fs::write(download_dir.join("CUSA55555/unlocker.pkg"), b"not unpacked").unwrap();
    assert_eq!(install_download(&test, "torrent-abc").await.trim(), "done");
    let game = games.path().join("PS4/CUSA55555");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"\x4f\x15\x3d\x1dgame");
    assert_eq!(std::fs::read(game.join("unlocker.pkg")).unwrap(), b"not unpacked", "copied as it is");
    let titles: Vec<_> = test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Hollow".to_owned()), "{titles:?}");
}

#[tokio::test]
async fn packages_the_host_unpacked_from_archives_install_together() {
    use emu_shadps4::pkg4::Kind;
    let (test, games) = boot().await;
    let (p, _data) = packages(&test, &[]).await;
    download(
        p.path(),
        &[
            ("Game/game.rar", b"Rar!".to_vec()),
            ("Game/update.rar", b"Rar!".to_vec()),
            ("unpacked/Game/game/game.pkg", ps4_package(Kind::Game, &[("eboot.bin", b"v1")])),
            ("unpacked/Game/update/patch.pkg", ps4_package(Kind::Update, &[("eboot.bin", b"v2")])),
        ],
    );
    assert_eq!(install_download(&test, "torrent-abc").await.trim(), "done");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345/eboot.bin")).unwrap(), b"v1");
    assert_eq!(std::fs::read(games.path().join("PS4/CUSA12345-patch/eboot.bin")).unwrap(), b"v2");
}
