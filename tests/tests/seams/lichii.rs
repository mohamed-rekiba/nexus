//! The Lichii plugin on a fake Linux host: the bare AppImage of a pre-release, Switch game
//! files (but not their updates) from the player's folder, and the launch.

use emu_lichii::{LichiiPlugin, RELEASES};
use nexus_api::*;
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
    let host = FakeHost::new().platform(Some(Platform::Linux));
    let releases = serde_json::json!([release(host.http(), TAG)]);
    host.http().serve(RELEASES, serde_json::to_vec(&releases).unwrap());
    let games = tempfile::tempdir().unwrap();
    let switch = games.path().join("Switch");
    std::fs::create_dir_all(switch.join("Kirby")).unwrap();
    std::fs::write(switch.join("Celeste [01002B30028F6000][v0].nsp"), b"PFS0 game").unwrap();
    std::fs::write(switch.join("Celeste [01002B30028F6800][v262144].nsp"), b"PFS0 update").unwrap();
    std::fs::write(switch.join("Kirby/Kirby Air Riders.nup"), b"NUP game").unwrap();
    std::fs::write(switch.join("notes.txt"), b"not a game").unwrap();
    let test = host.built_in(nexus_api::PluginBox::code(LichiiPlugin)).boot_core().await;
    extract_appimages(test.spawner.clone());
    test.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();
    (test, games)
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
    assert_eq!(keys, ["emu-lichii:01002B30028F6000", "emu-lichii:Kirby Air Riders"]);
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
    let game = games.path().join("Switch/Celeste [01002B30028F6000][v0].nsp").display().to_string();
    assert_eq!(process.command.args, ["-f".to_owned(), "-g".to_owned(), game]);
}
