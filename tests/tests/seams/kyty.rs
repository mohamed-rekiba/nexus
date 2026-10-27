//! The KytyPS5 plugin on a fake host: GitHub releases, a build unpacked from a redirect, PS5
//! games found in the player's folder, saves that survive an update, and a rollback.

use std::path::Path;
use std::time::Duration;

use emu_kyty::{COMPATIBILITY, KytyPlugin, RELEASES};
use futures::StreamExt;
use nexus_api::*;
use nexus_core::{Area, Command, Route, SessionCmd};
use nexus_testkit::{FakeHost, FakeHttp, TestCore, contract};

#[test]
fn the_manifest_s_key_map_is_the_keyboard_panel_s() {
    let manifest = KytyPlugin.manifest();
    let keys: Vec<_> = manifest.keymap.iter().map(|k| (k.button.as_str(), k.keys.as_str())).collect();
    assert_eq!(keys, emu_kyty::KEYS);
}

fn kyty() -> PluginId {
    PluginId::new("emu-kyty").unwrap()
}

/// A Linux build archive: `KytyPS5/kyty_emulator`, and `extra` files.
fn build_archive(marker: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    let script = format!("#!/bin/sh\necho {marker}\n");
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o644);
    tar.append_data(&mut header, "KytyPS5/kyty_emulator", script.as_bytes()).unwrap();
    for (path, data) in extra {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        tar.append_data(&mut header, path, *data).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// A GitHub release with one Linux build, whose download redirects to GitHub's CDN.
fn release(http: &FakeHttp, tag: &str, published: &str) -> serde_json::Value {
    release_shipping(http, tag, published, &[])
}

/// Like [`release`], with more files in the archive.
fn release_shipping(http: &FakeHttp, tag: &str, published: &str, extra: &[(&str, &[u8])]) -> serde_json::Value {
    let name = format!("{tag}-Linux-x86_64.tar.gz");
    let download = format!("https://github.com/KytyPS5/KytyPS5/releases/download/{tag}/{name}");
    let cdn = format!("https://objects.githubusercontent.com/kyty/{name}");
    let sha = http.serve(&cdn, build_archive(tag, extra));
    http.redirect(&download, &cdn);
    serde_json::json!({
        "tag_name": tag, "published_at": published,
        "assets": [
            { "name": format!("{tag}-macOS-x86_64.zip"), "browser_download_url": "https://github.com/x.zip", "size": 1 },
            { "name": name, "browser_download_url": download, "size": 0, "digest": format!("sha256:{sha}") },
        ],
    })
}

fn ps5_game(root: &Path, folder: &str, title_id: &str, title: &str) {
    let sce = root.join(folder).join("sce_sys");
    std::fs::create_dir_all(&sce).unwrap();
    let param = serde_json::json!({
        "titleId": title_id, "contentVersion": "01.000.000",
        "localizedParameters": { "defaultLanguage": "en-US", "en-US": { "titleName": title } },
    });
    std::fs::write(sce.join("param.json"), param.to_string()).unwrap();
    std::fs::write(sce.join("icon0.png"), b"icon").unwrap();
}

struct Kyty {
    test: TestCore,
    games: tempfile::TempDir,
}

const V1: &str = "KytyPS5-2026-10-01-aaaaaaa";
const V2: &str = "KytyPS5-2026-10-12-bbbbbbb";

/// What a KytyPS5 build prints for `--help`: its banner first.
const BANNER: &[u8] = b"KytyPS5 git = aaaaaaa, date = 2026.10.01\nusage: kyty_emulator [options]\n";

/// The build of `tag` prints its banner when the plugin runs it with `--help`.
fn runs(t: &TestCore, tag: &str) {
    t.spawner.prints(&work_dir(t, tag).join("kyty_emulator"), BANNER);
}

async fn boot(releases: &[&str]) -> Kyty {
    boot_on(FakeHost::new(), releases).await
}

async fn boot_on(host: FakeHost, releases: &[&str]) -> Kyty {
    let host = host.platform(Some(Platform::Linux));
    let list: Vec<_> = releases
        .iter()
        .map(|tag| release(host.http(), tag, if *tag == V1 { "2026-10-01T00:00:00Z" } else { "2026-10-12T00:00:00Z" }))
        .collect();
    host.http().serve(RELEASES, serde_json::to_vec(&list).unwrap());
    let compat =
        serde_json::json!({ "PPSA01234": { "status": "InGame", "platforms": { "linux": { "status": "MainMenu" } } } });
    host.http().serve(COMPATIBILITY, compat.to_string().into_bytes());
    let games = tempfile::tempdir().unwrap();
    ps5_game(games.path(), "PS5/Astro", "PPSA01234", "Astro Bot");
    // Three folders down: the console folder, then a folder of the player's.
    ps5_game(games.path(), "PS5/Demon/PPSA05678-app0", "PPSA05678", "Demon's Souls");
    let test = host.built_in(nexus_api::PluginBox::code(KytyPlugin)).boot_core().await;
    for tag in [V1, V2] {
        runs(&test, tag);
    }
    let folders = Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] };
    test.core.dispatch(folders).await.unwrap();
    Kyty { test, games }
}

fn work_dir(t: &TestCore, tag: &str) -> std::path::PathBuf {
    t.data_dir().join("plugins/emu-kyty/builds").join(tag).join("KytyPS5")
}

#[tokio::test]
async fn kyty_meets_every_contract() {
    let k = boot(&[V1]).await;
    contract::emulator(&k.test, &kyty()).await;
}

#[tokio::test]
async fn games_come_from_the_player_s_folder_with_their_icons_and_community_status() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    let home = k.test.core.view().home;
    let titles: Vec<_> = home.games.iter().map(|g| (g.key.as_str(), g.title.as_str())).collect();
    assert_eq!(titles, [("emu-kyty:PPSA01234", "Astro Bot"), ("emu-kyty:PPSA05678", "Demon's Souls")]);
    assert_eq!(home.games[0].badges[0].text, "Menus on Linux", "the Linux result, not the best elsewhere");
    let cover = home.games[0].cover.clone().expect("the game's icon");
    assert_eq!(k.test.core.image_bytes(&cover).await.unwrap(), b"icon");
}

#[tokio::test]
async fn a_build_installs_from_github_and_a_game_launches_with_its_folder() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();

    let binary = work_dir(&k.test, V1).join("kyty_emulator");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&binary).unwrap().permissions().mode() & 0o111, 0o111, "made executable");
    }
    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA05678".parse().unwrap()))).await.unwrap();

    let process = k.test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, binary);
    assert_eq!(process.command.cwd, work_dir(&k.test, V1), "KytyPS5 writes its data next to itself");
    let game = nexus_testkit::native(k.games.path(), "PS5/Demon/PPSA05678-app0");
    assert_eq!(process.command.args[..2], ["--game".to_owned(), game]);
    assert!(process.command.args.contains(&"--fullscreen".to_owned()));
}

#[tokio::test]
async fn saves_and_caches_move_to_the_new_build_on_an_update_and_back_on_a_rollback() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    std::fs::create_dir_all(work_dir(&k.test, V1).join("_SaveData/PPSA01234")).unwrap();
    std::fs::write(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat"), b"progress").unwrap();

    // A new build appears; "Check for new builds" asks GitHub again.
    let http = &k.test.http;
    let list =
        serde_json::json!([release(http, V2, "2026-10-12T00:00:00Z"), release(http, V1, "2026-10-01T00:00:00Z")]);
    k.test.http.serve(RELEASES, serde_json::to_vec(&list).unwrap());
    let check =
        Command::RunAction { plugin: kyty(), action: "check".into(), game: None, args: serde_json::Value::Null };
    core.dispatch(check).await.unwrap();
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: Some(V2.into()) }).await.unwrap();

    assert_eq!(std::fs::read(work_dir(&k.test, V2).join("_SaveData/PPSA01234/save.dat")).unwrap(), b"progress");
    assert!(!work_dir(&k.test, V1).join("_SaveData").exists());
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let panels = &core.view().emulators.emulators[0].panels;
    assert!(panels.iter().any(|p| p.text.contains("2026-10-01")), "a rollback to the build before: {panels:?}");

    let rollback =
        Command::RunAction { plugin: kyty(), action: "rollback".into(), game: None, args: serde_json::Value::Null };
    core.dispatch(rollback).await.unwrap();
    assert_eq!(std::fs::read(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat")).unwrap(), b"progress");
    core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap();
    assert_eq!(k.test.spawner.wait_for(1).await.command.cwd, work_dir(&k.test, V1));

    // Removing the current build parks the data; the next install picks it up.
    core.dispatch(Command::UninstallEmulator { plugin: kyty(), version: V1.into() }).await.unwrap();
    core.dispatch(Command::UninstallEmulator { plugin: kyty(), version: V2.into() }).await.unwrap();
    assert!(k.test.data_dir().join("plugins/emu-kyty/data/_SaveData/PPSA01234/save.dat").is_file());
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    assert_eq!(std::fs::read(work_dir(&k.test, V2).join("_SaveData/PPSA01234/save.dat")).unwrap(), b"progress");
}

#[tokio::test]
async fn a_new_build_that_does_not_run_here_is_removed_and_the_saves_stay_with_the_build_before() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    std::fs::create_dir_all(work_dir(&k.test, V1).join("_SaveData/PPSA01234")).unwrap();
    std::fs::write(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat"), b"progress").unwrap();
    let http = &k.test.http;
    let list =
        serde_json::json!([release(http, V2, "2026-10-12T00:00:00Z"), release(http, V1, "2026-10-01T00:00:00Z")]);
    k.test.http.serve(RELEASES, serde_json::to_vec(&list).unwrap());
    core.dispatch(action("check")).await.unwrap();
    // The new build starts, but prints no banner.
    k.test.spawner.prints(&work_dir(&k.test, V2).join("kyty_emulator"), b"Illegal instruction\n");

    let err = core.dispatch(Command::UpdateEmulator { plugin: kyty() }).await.unwrap_err();

    assert!(err.to_string().contains("does not run on this system"), "{err}");
    assert!(!k.test.data_dir().join("plugins/emu-kyty/builds").join(V2).exists(), "the broken build goes");
    assert_eq!(core.view().emulators.emulators[0].installed, [V1]);
    assert_eq!(std::fs::read(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat")).unwrap(), b"progress");
    core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap();
    let process = k.test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, work_dir(&k.test, V1).join("kyty_emulator"), "games start on the build before");
}

#[tokio::test]
async fn the_release_list_is_fetched_once_for_hours_not_on_every_screen() {
    let k = boot(&[V1]).await;
    let mut events = k.test.core.events();
    for _ in 0..3 {
        k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
        k.test.core.dispatch(Command::Back).await.unwrap();
    }
    let _ = tokio::time::timeout(Duration::from_millis(50), events.next()).await;

    let api_calls = k.test.http.sent().iter().filter(|u| u.as_str() == RELEASES).count();
    assert_eq!(api_calls, 1, "{:?}", k.test.http.sent());
}

#[tokio::test]
async fn the_update_button_keeps_the_build_before_for_a_rollback_and_prunes_older_ones() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    let publish = |tags: &[(&str, &str)]| {
        let list: Vec<_> = tags.iter().map(|(tag, at)| release(&k.test.http, tag, at)).collect();
        k.test.http.serve(RELEASES, serde_json::to_vec(&list).unwrap());
    };
    let check =
        || Command::RunAction { plugin: kyty(), action: "check".into(), game: None, args: serde_json::Value::Null };
    publish(&[(V2, "2026-10-12T00:00:00Z"), (V1, "2026-10-01T00:00:00Z")]);
    core.dispatch(check()).await.unwrap();

    core.dispatch(Command::UpdateEmulator { plugin: kyty() }).await.unwrap();

    assert_eq!(core.view().emulators.emulators[0].installed, [V1, V2], "the one before stays, the current one last");
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let panels = &core.view().emulators.emulators[0].panels;
    assert!(panels.iter().any(|p| p.label == "emu-kyty:rollback"), "{panels:?}");

    const V3: &str = "KytyPS5-2026-10-15-ccccccc";
    runs(&k.test, V3);
    publish(&[(V3, "2026-10-15T00:00:00Z"), (V2, "2026-10-12T00:00:00Z"), (V1, "2026-10-01T00:00:00Z")]);
    core.dispatch(check()).await.unwrap();
    core.dispatch(Command::UpdateEmulator { plugin: kyty() }).await.unwrap();

    assert_eq!(core.view().emulators.emulators[0].installed, [V2, V3], "the oldest is gone");
    assert!(!k.test.data_dir().join("plugins/emu-kyty/builds").join(V1).exists());
}

#[tokio::test]
async fn saves_merge_into_a_build_that_ships_its_own_data_folder() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    std::fs::create_dir_all(work_dir(&k.test, V1).join("_SaveData/PPSA01234")).unwrap();
    std::fs::write(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat"), b"progress").unwrap();
    // The next build arrives with a _SaveData folder of its own.
    let v2 = release_shipping(&k.test.http, V2, "2026-10-12T00:00:00Z", &[("KytyPS5/_SaveData/README", b"shipped")]);
    k.test.http.serve(RELEASES, serde_json::to_vec(&serde_json::json!([v2])).unwrap());
    let check =
        Command::RunAction { plugin: kyty(), action: "check".into(), game: None, args: serde_json::Value::Null };
    core.dispatch(check).await.unwrap();

    core.dispatch(Command::UpdateEmulator { plugin: kyty() }).await.unwrap();

    let saves = work_dir(&k.test, V2).join("_SaveData");
    assert_eq!(std::fs::read(saves.join("PPSA01234/save.dat")).unwrap(), b"progress", "moved into the shipped folder");
    assert_eq!(std::fs::read(saves.join("README")).unwrap(), b"shipped");
}

#[tokio::test]
async fn the_emulators_screen_names_the_controller_and_lists_the_keyboard_keys() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).expect("KytyPS5 on the Emulators screen");
    assert_eq!(emu.controllers, ["Controller (DualSense, DualShock 4, Xbox and most others)", "Keyboard"]);

    let keys = emu.panels.iter().find(|p| p.label == "emu-kyty:keyboard").expect("the keyboard keys");
    let key = |what: &str| keys.rows.iter().find(|r| r.label == what).map(|r| r.value.as_str());
    assert_eq!(key("Cross"), Some("J"));
    assert_eq!(key("Right stick"), Some("T F G H"));
    assert_eq!(key("Touchpad"), Some("Backspace / Tab"));
}

/// A self-built KytyPS5 in a temporary folder, mapped as `kyty-build`, that prints its banner.
async fn own_build(k: &Kyty) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let release = dir.path().join("_Build/release-linux");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(release.join("kyty_emulator"), b"#!/bin/sh\n").unwrap();
    k.test.spawner.prints(&release.join("kyty_emulator"), b"KytyPS5 git = 6799ecb, date = 2026.09.29\nusage: ...\n");
    let folders = Command::SetFolders { name: "kyty-build".into(), paths: vec![dir.path().into()] };
    k.test.core.dispatch(folders).await.unwrap();
    dir
}

fn set(key: &str, value: &str) -> Command {
    Command::SetConfigField { plugin: kyty(), scope: ConfigScope::Emulator, key: key.into(), value: value.into() }
}

fn action(name: &str) -> Command {
    Command::RunAction { plugin: kyty(), action: name.into(), game: None, args: serde_json::Value::Null }
}

#[tokio::test]
async fn your_own_build_runs_games_from_its_folder_and_shows_its_version() {
    let k = boot(&[V1]).await;
    let own = own_build(&k).await;
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();

    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    assert_eq!(emu.installed, ["Your own build · 2026-09-29 (6799ecb)"]);

    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA05678".parse().unwrap()))).await.unwrap();
    let process = k.test.spawner.wait_for(1).await;
    let release = own.path().join("_Build/release-linux");
    assert_eq!(process.command.program, release.join("kyty_emulator"));
    assert_eq!(process.command.cwd, release, "your build keeps its data next to itself");
}

#[tokio::test]
async fn your_own_build_can_be_mapped_as_the_program_itself() {
    let k = boot(&[V1]).await;
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("kyty_emulator");
    std::fs::write(&program, b"#!/bin/sh\n").unwrap();
    k.test.spawner.prints(&program, b"KytyPS5 git = 6799ecb, date = 2026.09.29\nusage: ...\n");
    let folders = Command::SetFolders { name: "kyty-build".into(), paths: vec![program.clone()] };
    k.test.core.dispatch(folders).await.unwrap();
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();

    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA05678".parse().unwrap()))).await.unwrap();
    let process = k.test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, program);
    assert_eq!(process.command.cwd, dir.path(), "its data next to it");
}

#[tokio::test]
async fn a_kyty_on_the_path_is_found_and_can_be_chosen() {
    let k = boot(&[V1]).await;
    k.test.spawner.on_path("kyty_emulator", Path::new("/opt/kyty/kyty_emulator"));
    k.test.spawner.prints(Path::new("/opt/kyty/kyty_emulator"), b"git = aaaaaaa, date = 2026.10.01\n");

    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some("use-own")).expect("an offer to use it");
    assert!(offer.text.contains("/opt/kyty/kyty_emulator"), "{}", offer.text);

    k.test.core.dispatch(action("use-own")).await.unwrap();
    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap();
    let process = k.test.spawner.wait_for(1).await;
    assert_eq!(process.command.program, Path::new("kyty_emulator"), "run from PATH, by name");
}

#[tokio::test]
async fn own_build_on_without_one_says_to_turn_it_off_when_a_build_is_downloaded() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();

    let err = k
        .test
        .core
        .dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap())))
        .await
        .unwrap_err();

    // The plugin says what is missing; the core words it, with the emulator's page.
    let notices = k.test.core.view().notices;
    assert!(notices.iter().any(|n| n.contains("turn off Own build on the PS5 page")), "{err}: {notices:?}");
    assert!(k.test.spawner.spawned().is_empty());
}

#[tokio::test]
async fn switching_to_official_builds_brings_your_saves_along() {
    let k = boot(&[V1]).await;
    let own = own_build(&k).await;
    let saves = own.path().join("_Build/release-linux/_SaveData/PPSA01234");
    std::fs::create_dir_all(&saves).unwrap();
    std::fs::write(saves.join("save.dat"), b"own progress").unwrap();
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();

    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();

    let copied = work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat");
    assert_eq!(std::fs::read(copied).unwrap(), b"own progress");
    assert!(saves.join("save.dat").is_file(), "your build's saves stay where they were");
    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap();
    assert_eq!(k.test.spawner.wait_for(1).await.command.program, work_dir(&k.test, V1).join("kyty_emulator"));
}

#[tokio::test]
async fn a_kyty_you_built_in_your_home_folder_is_found_and_its_saves_and_caches_come_along() {
    let home = tempfile::tempdir().unwrap();
    let k = boot_on(FakeHost::new().home(home.path()), &[V1]).await;
    let build = home.path().join("projects/KytyPS5/_Build/release-linux");
    std::fs::create_dir_all(build.join("_SaveData/PPSA01234")).unwrap();
    std::fs::create_dir_all(build.join("_PipelineCache")).unwrap();
    std::fs::write(build.join("kyty_emulator"), b"#!/bin/sh\n").unwrap();
    std::fs::write(build.join("_SaveData/PPSA01234/save.dat"), b"own progress").unwrap();
    std::fs::write(build.join("_PipelineCache/shaders.bin"), b"cache").unwrap();
    k.test.spawner.prints(&build.join("kyty_emulator"), b"KytyPS5 git = 6799ecb, date = 2026.09.29\n");

    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    let offer = emu.panels.iter().find(|p| p.action.as_deref() == Some("use-own")).expect("an offer to use it");
    assert!(offer.text.contains("projects/KytyPS5"), "{}", offer.text);
    k.test.core.dispatch(action("use-own")).await.unwrap();

    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();

    let data = work_dir(&k.test, V1);
    assert_eq!(std::fs::read(data.join("_SaveData/PPSA01234/save.dat")).unwrap(), b"own progress");
    assert_eq!(std::fs::read(data.join("_PipelineCache/shaders.bin")).unwrap(), b"cache", "the shader cache too");
    assert!(build.join("_PipelineCache/shaders.bin").is_file(), "your build keeps its own copy");
}

#[tokio::test]
async fn kyty_settings_come_in_the_order_the_old_launcher_showed_them_with_where_builds_come_from_first() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::Navigate(Route::Settings)).await.unwrap();
    let view = k.test.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == kyty()).expect("the KytyPS5 panel");
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "own_build",
            "release_repo",
            "prereleases",
            "fullscreen",
            "resolution",
            "video_out",
            "present_mode",
            "amd_cpu",
            "extra_args"
        ]
    );
}

#[tokio::test]
async fn a_choice_setting_is_saved_and_reaches_the_command_line() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    k.test.core.dispatch(set("present_mode", "Fifo")).await.unwrap();
    k.test.core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap();
    let args = k.test.spawner.wait_for(1).await.command.args;
    let at = args.iter().position(|a| a == "--present-mode").expect("the present mode");
    assert_eq!(args[at + 1], "Fifo");
}

/// Maps a `packages` folder with `files` in its `PS5` folder, and returns it.
async fn packages(k: &Kyty, files: &[(&str, Vec<u8>)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("PS5")).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.path().join("PS5").join(name), bytes).unwrap();
    }
    k.test
        .core
        .dispatch(Command::SetFolders { name: "packages".into(), paths: vec![dir.path().into()] })
        .await
        .unwrap();
    dir
}

/// Starts the install the Emulators screen offers for `file`, and waits for it to end.
async fn install_game(k: &Kyty, file: &str) -> String {
    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    let offer = emu
        .panels
        .iter()
        .find(|p| p.action.as_deref() == Some(HOST_INSTALL_GAME) && p.text.contains(file))
        .unwrap_or_else(|| panic!("an offer to install {file}: {:?}", emu.panels));
    let start =
        Command::RunAction { plugin: kyty(), action: HOST_INSTALL_GAME.into(), game: None, args: offer.args.clone() };
    k.test.core.dispatch(start).await.unwrap();
    k.test
        .until(Area::Installs, |core| core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing"))
        .await;
    let job = k.test.core.view().downloads.jobs.into_iter().next().unwrap();
    format!("{} {}", job.state, job.error)
}

#[tokio::test]
async fn a_ps5_package_installs_into_the_games_folder_and_shows_in_the_library() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    let pkg = build::package(&build::Spec {
        files: &[("eboot.bin", b"fixture executable"), ("sce_module/a.prx", b"module")],
        entropy: true,
        ..Default::default()
    });
    let _p = packages(&k, &[("game.pkg", pkg), ("notes.txt", b"not a package".to_vec())]).await;

    assert_eq!(install_game(&k, "game.pkg").await.trim(), "done");
    let game = k.games.path().join("PS5/PPSA12345-app0");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"fixture executable");
    assert_eq!(std::fs::read(game.join("sce_module/a.prx")).unwrap(), b"module");
    assert!(game.join("sce_sys/param.json").is_file());
    assert!(!k.games.path().join("PS5/game.part").exists());
    let titles: Vec<_> = k.test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Generated fixture".to_owned()), "{titles:?}");
}

#[tokio::test]
async fn a_download_folder_installs_from_the_package_inside_it() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    let pkg = build::package(&build::Spec {
        files: &[("eboot.bin", b"fixture executable")],
        entropy: true,
        ..Default::default()
    });
    // A torrent's folder: the package one folder down, beside a bigger file that isn't one.
    let dir = packages(&k, &[]).await;
    let download = dir.path().join("PS5/torrent-abc/Game");
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(download.join("game.pkg"), &pkg).unwrap();
    std::fs::write(download.join("../trailer.mkv"), vec![0u8; pkg.len() * 2]).unwrap();

    let source = serde_json::json!({ "folder": "packages", "root": 0, "path": "PS5/torrent-abc" });
    let start = Command::RunAction { plugin: kyty(), action: HOST_INSTALL_GAME.into(), game: None, args: source };
    k.test.core.dispatch(start).await.unwrap();
    k.test
        .until(Area::Installs, |core| core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing"))
        .await;
    let job = k.test.core.view().downloads.jobs.into_iter().next().unwrap();
    assert_eq!((job.state.as_str(), job.error.as_str()), ("done", ""));
    let game = k.games.path().join("PS5/PPSA12345-app0");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"fixture executable");
}

#[tokio::test]
async fn a_download_folder_with_no_package_says_so() {
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    std::fs::create_dir_all(dir.path().join("PS5/torrent-abc")).unwrap();
    std::fs::write(dir.path().join("PS5/torrent-abc/game.rar"), b"Rar!").unwrap();
    let source = serde_json::json!({ "folder": "packages", "root": 0, "path": "PS5/torrent-abc" });
    let start = Command::RunAction { plugin: kyty(), action: HOST_INSTALL_GAME.into(), game: None, args: source };
    let refused = k.test.core.dispatch(start).await.unwrap_err();
    assert!(refused.to_string().contains("holds no package, disc image or unpacked game"), "{refused}");
}

#[tokio::test]
async fn a_game_in_an_exfat_image_installs_from_its_folder_inside() {
    let k = boot(&[V1]).await;
    let param = br#"{"titleId":"PPSA22222","localizedParameters":{"en-US":{"titleName":"Image game"}}}"#;
    let image = emulator_kit::exfat::build::image(
        &[("PPSA22222-app0/sce_sys/param.json", param), ("PPSA22222-app0/eboot.bin", b"elf")],
        true,
    );
    let _p = packages(&k, &[("disc.exfat", image)]).await;

    assert_eq!(install_game(&k, "disc.exfat").await.trim(), "done");
    let game = k.games.path().join("PS5/PPSA22222-app0");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"elf");
    assert_eq!(std::fs::read(game.join("sce_sys/param.json")).unwrap(), param);
}

#[tokio::test]
async fn a_game_that_is_already_installed_is_not_installed_twice() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    std::fs::create_dir_all(k.games.path().join("PS5/PPSA12345-app0")).unwrap();
    let pkg = build::package(&build::Spec { files: &[("eboot.bin", b"x")], ..Default::default() });
    let _p = packages(&k, &[("game.pkg", pkg)]).await;

    let end = install_game(&k, "game.pkg").await;
    assert!(end.starts_with("failed") && end.contains("already"), "{end}");
    assert!(!k.games.path().join("PS5/game.part").exists());
}

#[tokio::test]
async fn automatic_updates_leave_your_own_build_alone() {
    let k = boot(&[V1, V2]).await;
    let _own = own_build(&k).await;
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();

    k.test.core.dispatch(Command::CheckUpdates).await.unwrap();
    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    assert_eq!(emu.installed, ["Your own build · 2026-09-29 (6799ecb)"], "still your own build");
    assert!(emu.update.is_some(), "the switch to an official build is still offered");
}

#[tokio::test]
async fn switching_away_from_a_kyty_on_the_path_brings_its_saves_along() {
    let k = boot(&[V1]).await;
    k.test.spawner.on_path("kyty_emulator", Path::new("/opt/kyty/kyty_emulator"));
    k.test.spawner.prints(Path::new("/opt/kyty/kyty_emulator"), b"git = aaaaaaa, date = 2026.10.01\n");
    k.test.core.dispatch(set("own_build", "true")).await.unwrap();
    // From PATH, KytyPS5 runs in the plugin's own folder and keeps its saves there.
    let saves = k.test.data_dir().join("plugins/emu-kyty/own/_SaveData/PPSA01234");
    std::fs::create_dir_all(&saves).unwrap();
    std::fs::write(saves.join("save.dat"), b"path progress").unwrap();

    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    assert_eq!(std::fs::read(work_dir(&k.test, V1).join("_SaveData/PPSA01234/save.dat")).unwrap(), b"path progress");
}

const FORK_RELEASES: &str = "https://api.github.com/repos/mohamed-rekiba/KytyPS5/releases?per_page=10";

#[tokio::test]
async fn builds_come_from_the_release_repo_you_set_with_its_prereleases() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    // The upstream list is fetched and cached first.
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let mut test_build = release(&k.test.http, "KytyPS5-2026-10-13-c9013e6", "2026-10-13T00:00:00Z");
    test_build["prerelease"] = true.into();
    runs(&k.test, "KytyPS5-2026-10-13-c9013e6");
    k.test.http.serve(FORK_RELEASES, serde_json::to_vec(&serde_json::json!([test_build])).unwrap());

    core.dispatch(set("release_repo", "mohamed-rekiba/KytyPS5")).await.unwrap();
    core.dispatch(set("prereleases", "true")).await.unwrap();
    core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();

    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    assert_eq!(emu.installed, ["KytyPS5-2026-10-13-c9013e6"], "not the cached upstream list");
}

#[tokio::test]
async fn on_macos_builds_come_from_the_macos_test_builds_by_default() {
    let host = FakeHost::new().platform(Some(Platform::Macos));
    host.http().serve(FORK_RELEASES, b"[]".to_vec());
    let test = host.built_in(nexus_api::PluginBox::code(KytyPlugin)).boot_core().await;
    test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let sent = test.http.sent();
    assert!(sent.iter().any(|u| u == FORK_RELEASES), "{sent:?}");
    assert!(!sent.iter().any(|u| u == RELEASES), "{sent:?}");
}

#[tokio::test]
async fn a_release_repo_that_is_not_owner_slash_repo_is_never_asked() {
    let k = boot(&[V1]).await;
    let refused = k.test.core.dispatch(set("release_repo", "a/../b")).await;
    assert!(refused.is_err(), "the setting is refused");
    k.test.core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await.unwrap();
    let sent = k.test.http.sent();
    assert!(!sent.iter().any(|u| u.contains("/repos/a/")), "{sent:?}");
    k.test.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = k.test.core.view();
    let emu = view.emulators.emulators.iter().find(|e| e.plugin == kyty()).unwrap();
    assert_eq!(emu.installed, [V1], "still the upstream build");
}

#[tokio::test]
async fn on_macos_pre_releases_are_on_by_default_and_turning_them_off_hides_the_test_builds() {
    let host = FakeHost::new().platform(Some(Platform::Macos));
    let test_build = serde_json::json!([{
        "tag_name": "KytyPS5-2026-10-13-c9013e6", "published_at": "2026-10-13T00:00:00Z", "prerelease": true,
        "assets": [{ "name": "KytyPS5-macOS-x86_64.zip", "browser_download_url": "https://github.com/x.zip", "size": 1 }],
    }]);
    host.http().serve(FORK_RELEASES, serde_json::to_vec(&test_build).unwrap());
    let test = host.built_in(nexus_api::PluginBox::code(KytyPlugin)).boot_core().await;
    let core = &test.core;
    let shown = |key: &str| {
        let view = core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == kyty()).expect("the KytyPS5 panel");
        panel.fields.iter().find(|f| f.key == key).map(|f| f.value.clone()).unwrap()
    };
    // The plugin writes its settings itself here, before anything is stored.
    core.dispatch(action("use-own")).await.unwrap();
    core.dispatch(Command::Navigate(Route::Settings)).await.unwrap();
    assert_eq!(shown("prereleases"), "true", "the form shows what the plugin does");

    core.dispatch(set("prereleases", "false")).await.unwrap();
    let installed = core.dispatch(Command::InstallEmulator { plugin: kyty(), version: None }).await;
    assert!(installed.is_err(), "no build without pre-releases");
}

#[tokio::test]
async fn a_stored_window_size_becomes_the_nearest_resolution_and_settings_still_change() {
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let old = serde_json::json!({ "plugins": { "emu-kyty": { "emulator": {
        "width": 2560, "height": 1440, "present_mode": "Fifo", "fullscreen": true,
    } } } });
    std::fs::write(host.data_dir().join("config.json"), old.to_string()).unwrap();
    let test = host.built_in(nexus_api::PluginBox::code(KytyPlugin)).boot_core().await;
    let core = &test.core;
    core.dispatch(Command::Navigate(Route::Settings)).await.unwrap();
    let shown = |key: &str| {
        let view = core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == kyty()).expect("the KytyPS5 panel");
        panel.fields.iter().find(|f| f.key == key).map(|f| f.value.clone())
    };
    assert_eq!(shown("resolution").as_deref(), Some("2560x1440"));
    assert_eq!(shown("present_mode").as_deref(), Some("Fifo"), "the other settings stay");
    assert_eq!(shown("width"), None, "no window size fields any more");

    core.dispatch(set("fullscreen", "false")).await.unwrap();
    assert_eq!(shown("fullscreen").as_deref(), Some("false"));
}

#[tokio::test]
async fn a_turned_off_plugin_has_no_games_and_cannot_launch_until_turned_on_again() {
    let k = boot(&[V1]).await;
    let core = &k.test.core;
    core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    assert_eq!(core.view().home.games.len(), 2);

    core.dispatch(Command::SetPluginEnabled { plugin: kyty(), enabled: false }).await.unwrap();
    assert!(core.view().home.games.is_empty(), "its games leave the library");
    let refused =
        core.dispatch(Command::Session(SessionCmd::Launch("emu-kyty:PPSA01234".parse().unwrap()))).await.unwrap_err();
    assert!(refused.to_string().contains("turned off"), "{refused}");
    let row = core.view().emulators.plugins.into_iter().find(|p| p.id == kyty()).expect("still listed");
    assert!(!row.enabled);

    core.dispatch(Command::SetPluginEnabled { plugin: kyty(), enabled: true }).await.unwrap();
    assert_eq!(core.view().home.games.len(), 2, "back without a restart");
}

#[tokio::test]
async fn a_theme_cannot_be_turned_off() {
    let test = FakeHost::new().built_in(nexus_api::PluginBox::theme(theme_default::DefaultTheme)).boot_core().await;
    let theme = PluginId::new("default").unwrap();
    let off = test.core.dispatch(Command::SetPluginEnabled { plugin: theme, enabled: false }).await;
    assert!(matches!(off, Err(nexus_core::CoreError::Invalid(_))), "{off:?}");
}

#[tokio::test]
async fn a_game_s_menu_finds_its_folder_and_log_and_names_the_community_list() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let game: nexus_api::GameKey = "emu-kyty:PPSA01234".parse().unwrap();
    k.test.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();

    let folder = k.test.core.game_folder(&game).expect("the game's folder");
    assert_eq!(folder.canonicalize().unwrap(), k.games.path().join("PS5/Astro").canonicalize().unwrap());
    assert_eq!(k.test.core.game_log(&game), None, "no log before it ran");
    let detail = k.test.core.view().detail.unwrap();
    assert!(detail.has_folder);
    assert!(!detail.has_log);
    assert_eq!(detail.compat_page, "https://kytyps5.github.io/");

    // "Rate how it runs": the answer is kept for this game.
    k.test.core.dispatch(Command::AskResult(game.clone())).await.unwrap();
    k.test.core.dispatch(Command::ReportResult(Some("menus".into()))).await.unwrap();
    assert_eq!(k.test.core.view().detail.unwrap().your_result, "menus");
}

#[tokio::test]
async fn a_game_has_its_own_settings_and_a_change_shows_on_its_page() {
    let k = boot(&[V1]).await;
    k.test.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let game: nexus_api::GameKey = "emu-kyty:PPSA01234".parse().unwrap();
    k.test.core.dispatch(Command::Navigate(Route::GameSettings(game.clone()))).await.unwrap();

    let view = k.test.core.view();
    assert_eq!(view.screen, nexus_ui_api::ScreenId::GameSettings);
    let detail = view.detail.expect("the game's details");
    let [panel] = detail.settings.as_slice() else { panic!("one panel: {:?}", detail.settings) };
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.as_str()).collect();
    assert!(keys.contains(&"extra_args"), "{keys:?}");

    let scope = ConfigScope::Game(game.local.clone());
    let set =
        Command::SetConfigField { plugin: game.plugin.clone(), scope, key: "extra_args".into(), value: "--fps".into() };
    k.test.core.dispatch(set).await.unwrap();
    let panel = &k.test.core.view().detail.unwrap().settings[0];
    let extra = panel.fields.iter().find(|f| f.key == "extra_args").unwrap();
    assert_eq!(extra.value, "--fps", "the page shows the new value");

    // An optional choice: first "As the emulator", which clears it again.
    let video = panel.fields.iter().find(|f| f.key == "video_out").expect("video_out");
    assert_eq!((video.choices[0].as_str(), video.choice_labels[0].as_str()), ("", "As the emulator"));
    let scope = ConfigScope::Game(game.local.clone());
    let set = Command::SetConfigField {
        plugin: game.plugin.clone(),
        scope: scope.clone(),
        key: "video_out".into(),
        value: video.choices[1].clone(),
    };
    k.test.core.dispatch(set).await.unwrap();
    let set =
        Command::SetConfigField { plugin: game.plugin.clone(), scope, key: "video_out".into(), value: String::new() };
    k.test.core.dispatch(set).await.unwrap();
    let stored = k.test.core.kernel().config().get(&["plugins", "emu-kyty", "game:PPSA01234"]);
    assert_eq!(stored["video_out"], serde_json::Value::Null, "{stored}");
}

/// Installs the download at `PS5/torrent-abc` in the packages folder, and waits for it to end.
async fn install_download(k: &Kyty) -> String {
    let source = serde_json::json!({ "folder": "packages", "root": 0, "path": "PS5/torrent-abc" });
    let start = Command::RunAction { plugin: kyty(), action: HOST_INSTALL_GAME.into(), game: None, args: source };
    if let Err(e) = k.test.core.dispatch(start).await {
        return format!("failed {e}");
    }
    k.test
        .until(Area::Installs, |core| core.view().downloads.jobs.first().is_some_and(|j| j.state != "installing"))
        .await;
    let job = k.test.core.view().downloads.jobs.into_iter().next().unwrap();
    format!("{} {}", job.state, job.error)
}

#[tokio::test]
async fn an_unpacked_game_folder_in_a_download_installs_as_it_is() {
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    let game = dir.path().join("PS5/torrent-abc/Release/PPSA33333-app0");
    std::fs::create_dir_all(game.join("sce_sys")).unwrap();
    std::fs::create_dir_all(game.join("sce_module")).unwrap();
    let param = br#"{"titleId":"PPSA33333","localizedParameters":{"en-US":{"titleName":"Unpacked"}}}"#;
    std::fs::write(game.join("sce_sys/param.json"), param).unwrap();
    std::fs::write(game.join("eboot.bin"), b"elf").unwrap();
    std::fs::write(game.join("sce_module/a.prx"), b"module").unwrap();
    std::fs::write(game.join("unlocker.pkg"), b"copied, not unpacked").unwrap();

    assert_eq!(install_download(&k).await.trim(), "done");
    let installed = k.games.path().join("PS5/PPSA33333-app0");
    assert_eq!(std::fs::read(installed.join("eboot.bin")).unwrap(), b"elf");
    assert_eq!(std::fs::read(installed.join("sce_module/a.prx")).unwrap(), b"module");
    assert_eq!(std::fs::read(installed.join("unlocker.pkg")).unwrap(), b"copied, not unpacked");
    assert!(game.join("eboot.bin").is_file(), "a download's own files are copied, never moved: it shares them");
    let titles: Vec<_> = k.test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Unpacked".to_owned()), "{titles:?}");
}

#[tokio::test]
async fn the_package_that_holds_the_game_nearest_the_top_installs_and_others_are_skipped() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    let game = build::package(&build::Spec { files: &[("eboot.bin", b"the game")], ..Default::default() });
    // An add-on package holds no eboot.bin, and is the biggest file.
    let big = vec![7u8; 4096];
    let addon = build::package(&build::Spec { files: &[("data.bin", &big)], ..Default::default() });
    let base = dir.path().join("PS5/torrent-abc");
    std::fs::create_dir_all(base.join("Game/Overlay")).unwrap();
    std::fs::write(base.join("Game/game.pkg"), &game).unwrap();
    std::fs::write(base.join("Game/addon.pkg"), &addon).unwrap();
    let overlay = build::package(&build::Spec { files: &[("eboot.bin", b"an overlay")], ..Default::default() });
    std::fs::write(base.join("Game/Overlay/overlay.pkg"), &overlay).unwrap();

    assert_eq!(install_download(&k).await.trim(), "done");
    let installed = k.games.path().join("PS5/PPSA12345-app0");
    assert_eq!(std::fs::read(installed.join("eboot.bin")).unwrap(), b"the game");
}

#[tokio::test]
async fn two_game_packages_at_the_same_depth_are_refused() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    let game = build::package(&build::Spec { files: &[("eboot.bin", b"x")], ..Default::default() });
    let base = dir.path().join("PS5/torrent-abc");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("one.pkg"), &game).unwrap();
    std::fs::write(base.join("two.pkg"), &game).unwrap();

    let refused = install_download(&k).await;
    assert!(refused.starts_with("failed") && refused.contains("Several packages hold a game"), "{refused}");
}

#[tokio::test]
async fn two_unpacked_game_folders_at_the_same_depth_are_refused() {
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    for id in ["PPSA33333", "PPSA44444"] {
        let game = dir.path().join(format!("PS5/torrent-abc/{id}-app0"));
        std::fs::create_dir_all(game.join("sce_sys")).unwrap();
        let param = format!(r#"{{"titleId":"{id}","localizedParameters":{{"en-US":{{"titleName":"{id}"}}}}}}"#);
        std::fs::write(game.join("sce_sys/param.json"), param).unwrap();
        std::fs::write(game.join("eboot.bin"), b"elf").unwrap();
    }
    let refused = install_download(&k).await;
    assert!(refused.starts_with("failed") && refused.contains("Several folders hold a game"), "{refused}");
}

#[tokio::test]
async fn a_package_the_host_unpacked_from_an_archive_installs() {
    use emulator_kit::pkg5::build;
    let k = boot(&[V1]).await;
    let dir = packages(&k, &[]).await;
    let pkg = build::package(&build::Spec { files: &[("eboot.bin", b"from the archive")], ..Default::default() });
    let base = dir.path().join("PS5/torrent-abc");
    std::fs::create_dir_all(base.join("Astro")).unwrap();
    std::fs::write(base.join("Astro/game.rar"), b"Rar!").unwrap();
    std::fs::create_dir_all(base.join("unpacked/Astro/game/Astro")).unwrap();
    std::fs::write(base.join("unpacked/Astro/game/Astro/game.pkg"), &pkg).unwrap();

    assert_eq!(install_download(&k).await.trim(), "done");
    let game = k.games.path().join("PS5/PPSA12345-app0");
    assert_eq!(std::fs::read(game.join("eboot.bin")).unwrap(), b"from the archive");
}
