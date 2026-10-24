//! Seam 1, core app API: the PC plugin on a fake host. A release installs from its download
//! folder (as the host unpacked it), shows in the Library, and starts on each system with what
//! that system can run.

use std::path::Path;
use std::time::Duration;

use crate::common::native;
use emu_pc::{GAME_FILE, GameManifest, PcPlugin};
use nexus_api::*;
use nexus_core::Command;
use nexus_testkit::{FakeHost, TestCore, contract};

fn pc() -> PluginId {
    PluginId::new("emu-pc").unwrap()
}

/// The first bytes of a Linux program.
fn elf() -> Vec<u8> {
    let mut h = vec![0u8; 0x200];
    h[..4].copy_from_slice(b"\x7fELF");
    h[4] = 2;
    h[5] = 1;
    h[16..18].copy_from_slice(&2u16.to_le_bytes());
    h[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
    h
}

/// The first bytes of a Windows program.
fn exe() -> Vec<u8> {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    pe[0x80..0x84].copy_from_slice(b"PE\0\0");
    pe
}

struct Pc {
    test: TestCore,
    games: tempfile::TempDir,
    packages: tempfile::TempDir,
}

async fn boot(platform: Platform) -> Pc {
    let test = FakeHost::new().platform(Some(platform)).built_in(PluginByKind::emulator(PcPlugin)).boot_core().await;
    with_folders(test).await
}

/// `test` with a games folder and a packages folder.
async fn with_folders(test: TestCore) -> Pc {
    let games = tempfile::tempdir().unwrap();
    let packages = tempfile::tempdir().unwrap();
    for (name, dir) in [("games", games.path()), ("packages", packages.path())] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![dir.into()] }).await.unwrap();
    }
    Pc { test, games, packages }
}

impl Pc {
    /// Writes `files` into the download folder `PC/torrent-abc`.
    fn download(&self, files: &[(&str, &[u8])]) {
        for (name, bytes) in files {
            let path = self.packages.path().join("PC/torrent-abc").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    /// Installs the download, and says how it ended.
    async fn install(&self) -> String {
        let source = serde_json::json!({ "folder": "packages", "root": 0, "path": "PC/torrent-abc" });
        let start = Command::RunAction { plugin: pc(), action: HOST_INSTALL_GAME.into(), game: None, args: source };
        let before = self.test.core.view().downloads.jobs.len();
        if let Err(e) = self.test.core.dispatch(start).await {
            return format!("failed {e}");
        }
        for _ in 0..500 {
            let jobs = self.test.core.view().downloads.jobs;
            // The newest job is first.
            if jobs.len() > before
                && let Some(job) = jobs.first()
                && job.state != "installing"
            {
                return format!("{} {}", job.state, job.error).trim().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the install never ended");
    }

    fn manifest(&self, folder: &str) -> GameManifest {
        let bytes = std::fs::read(self.games.path().join("PC").join(folder).join(GAME_FILE)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The Game Hub's "Start with <file>" label for the game `emu-pc:Game`.
    async fn next_start(&self) -> String {
        self.test.core.dispatch(Command::OpenGame("emu-pc:Game".parse().unwrap())).await.unwrap();
        let detail = self.test.core.view().detail.expect("the Game Hub is open");
        detail.actions.iter().find(|a| a.id == "start-something-else").map(|a| a.text.clone()).unwrap_or_default()
    }

    fn titles(&self) -> Vec<(String, String)> {
        self.test.core.view().home.games.into_iter().map(|g| (g.title, g.platform)).collect()
    }
}

#[tokio::test]
async fn pc_meets_every_contract() {
    let p = boot(Platform::Linux).await;
    p.download(&[("Game/game.x86_64", &elf())]);
    assert_eq!(p.install().await, "done");
    contract::emulator(&p.test, &pc()).await;
}

#[tokio::test]
async fn a_release_installs_from_its_download_with_its_start_script_and_shows_in_the_library() {
    let p = boot(Platform::Linux).await;
    p.download(&[
        ("Celeste (v1.4.0.0) [GOG]/start.sh", b"#!/bin/sh\n./game/Celeste.x86_64\n"),
        ("Celeste (v1.4.0.0) [GOG]/game/Celeste.x86_64", &elf()),
        ("Celeste (v1.4.0.0) [GOG]/game/Celeste.exe", &exe()),
        ("Celeste (v1.4.0.0) [GOG]/game/Content/levels.pak", b"levels"),
    ]);

    assert_eq!(p.install().await, "done");
    let game = p.games.path().join("PC/Celeste");
    assert_eq!(std::fs::read(game.join("game/Content/levels.pak")).unwrap(), b"levels");
    let manifest = p.manifest("Celeste");
    assert_eq!((manifest.name.as_str(), manifest.launch.as_str()), ("Celeste", "start.sh"));
    let others: Vec<_> = manifest.alternatives.iter().map(|s| s.launch.as_str()).collect();
    assert_eq!(others, ["game/Celeste.x86_64", "game/Celeste.exe"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(game.join("game/Celeste.x86_64")).unwrap().permissions().mode();
        assert_eq!(mode & 0o100, 0o100, "the program flag is back");
    }
    assert!(p.titles().contains(&("Celeste".into(), "PC".into())), "{:?}", p.titles());
}

#[tokio::test]
async fn play_starts_the_script_in_its_folder_and_start_something_else_moves_to_the_next_choice() {
    let p = boot(Platform::Linux).await;
    p.download(&[("Game/start.sh", b"#!/bin/sh\n"), ("Game/bin/game.x86_64", &elf())]);
    assert_eq!(p.install().await, "done");
    let game = p.games.path().join("PC/Game");
    assert_eq!(p.next_start().await, "Start with game.x86_64", "it names the next file");

    p.test.core.dispatch(Command::Launch("emu-pc:Game".parse().unwrap())).await.unwrap();
    let first = p.test.spawner.wait_for(1).await;
    assert_eq!(first.command.program, game.join("start.sh"));
    assert_eq!(first.command.cwd, game);
    first.exit(0);

    let next = Command::RunAction {
        plugin: pc(),
        action: "start-something-else".into(),
        game: Some("emu-pc:Game".parse().unwrap()),
        args: serde_json::Value::Null,
    };
    p.test.core.dispatch(next).await.unwrap();
    assert_eq!(p.manifest("Game").launch, "bin/game.x86_64");
    assert_eq!(p.manifest("Game").alternatives[0].launch, "start.sh", "the script goes last");
    assert_eq!(p.next_start().await, "Start with start.sh");
    tokio::time::sleep(Duration::from_millis(50)).await;
    p.test.core.dispatch(Command::Launch("emu-pc:Game".parse().unwrap())).await.unwrap();
    let second = p.test.spawner.wait_for(2).await;
    assert_eq!(second.command.program, game.join("bin/game.x86_64"));
    assert_eq!(second.command.cwd, game.join("bin"));
}

#[tokio::test]
async fn a_windows_game_runs_through_proton_on_linux_and_says_what_to_install_without_it() {
    let p = boot(Platform::Linux).await;
    p.download(&[("Hollow Knight/hollow_knight.exe", &exe())]);
    assert_eq!(p.install().await, "done");
    let launch = Command::Launch("emu-pc:Hollow-Knight".parse().unwrap());

    let refused = p.test.core.dispatch(launch.clone()).await.unwrap_err();
    assert!(refused.to_string().contains("install umu-launcher"), "{refused}");

    p.test.spawner.on_path("umu-run", Path::new("/usr/bin/umu-run"));
    p.test.core.dispatch(launch).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("umu-run"));
    let exe = p.games.path().join(native("PC/Hollow-Knight/hollow_knight.exe"));
    assert_eq!(run.args, [exe.display().to_string()]);
    assert_eq!(run.env.get("PROTONPATH").map(String::as_str), Some("GE-Proton"));
    assert_eq!(run.env.get("PROTON_USE_WOW64"), None, "this system runs 32-bit programs");
    let prefix = p.test.data_dir().join("plugins/emu-pc/prefixes/Hollow-Knight");
    assert_eq!(run.env.get("WINEPREFIX").map(String::as_str), Some(prefix.display().to_string().as_str()));
}

const UMU_LATEST: &str = "https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest";
const UMU_TAR: &str =
    "https://github.com/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar";

/// umu-launcher's zipapp archive: `umu/umu-run` in a tar.
fn umu_zipapp() -> Vec<u8> {
    let program = b"#!/usr/bin/env python3\n";
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(program.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "umu/umu-run", &program[..]).unwrap();
    tar.into_inner().unwrap()
}

/// Serves umu-launcher's latest release, 1.4.4, whose zipapp GitHub says has `sha256`.
fn serve_umu(p: &Pc, tar: Vec<u8>, sha256: &str) {
    p.test.http.serve(UMU_TAR, tar.clone());
    let release = serde_json::json!({ "tag_name": "1.4.4", "published_at": "2026-07-25T00:25:27Z",
        "assets": [{ "name": "umu-launcher-1.4.4-zipapp.tar", "browser_download_url": UMU_TAR,
            "size": tar.len(), "digest": format!("sha256:{sha256}") }] });
    p.test.http.serve(UMU_LATEST, release.to_string());
}

/// The rows of the PC page's "Windows games" section, as `label: value`, and its button.
async fn windows_section(p: &Pc) -> (Vec<String>, String) {
    p.test.core.dispatch(Command::Navigate(nexus_core::Route::Plugin(pc()))).await.unwrap();
    let page = p.test.core.view().plugin_page.unwrap();
    let section = page.sections.iter().find(|s| s.id == "windows").expect("a Windows games section");
    let rows = section.rows.iter().map(|r| format!("{}: {}", r.label, r.value)).collect();
    let button = page.sections.iter().find(|s| s.id == "umu").map(|s| s.text.clone()).unwrap_or_default();
    (rows, button)
}

#[tokio::test]
async fn umu_launcher_installs_from_its_latest_release_and_runs_windows_games_with_proton() {
    let p = boot(Platform::Linux).await;
    let (rows, button) = windows_section(&p).await;
    assert_eq!(rows, ["Run with: Nothing yet: install umu-launcher (Proton) or Wine"]);
    assert_eq!(button, "Install umu-launcher");

    let tar = umu_zipapp();
    let sha = p.test.http.serve("https://unused.example/umu", tar.clone());
    serve_umu(&p, tar, &sha);
    let install =
        Command::RunAction { plugin: pc(), action: "install-umu".into(), game: None, args: serde_json::Value::Null };
    p.test.core.dispatch(install).await.unwrap();
    let program = p.test.data_dir().join("plugins/emu-pc/umu/umu-run");
    assert_eq!(std::fs::read(&program).unwrap(), b"#!/usr/bin/env python3\n");
    let (rows, button) = windows_section(&p).await;
    assert_eq!(rows, ["Run with: Proton, through umu-launcher 1.4.4"]);
    assert_eq!(button, "Update umu-launcher");

    // A Windows game starts through it, with nothing on the PATH.
    p.download(&[("Hollow Knight/hollow_knight.exe", &exe())]);
    assert_eq!(p.install().await, "done");
    p.test.core.dispatch(Command::Launch("emu-pc:Hollow-Knight".parse().unwrap())).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, program);
    assert_eq!(run.env.get("PROTONPATH").map(String::as_str), Some("GE-Proton"));
}

#[tokio::test]
async fn a_umu_launcher_download_that_isn_t_the_one_github_lists_is_refused() {
    let p = boot(Platform::Linux).await;
    serve_umu(&p, umu_zipapp(), &"0".repeat(64));
    let install =
        Command::RunAction { plugin: pc(), action: "install-umu".into(), game: None, args: serde_json::Value::Null };
    let refused = p.test.core.dispatch(install).await.unwrap_err().to_string();
    assert!(refused.contains("checksum"), "{refused}");
    assert!(!p.test.data_dir().join("plugins/emu-pc/umu/umu-run").exists());
}

#[tokio::test]
async fn a_game_a_catalog_download_installed_has_the_offer_s_art_and_the_repack_s_name() {
    const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
    const ART: &str = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1";
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .built_in(PluginByKind::emulator(PcPlugin))
        .built_in(PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let p = with_folders(test).await;
    let t = &p.test;
    let topics = serde_json::json!([{ "id": "1", "title": "Pocket Tanks Deluxe [x86] [ENG] [Wine]",
        "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Pocket Tanks Deluxe" },
        "steam": { "appid": 1, "name": "Pocket Tanks", "header": format!("{ART}/header.jpg"),
            "assets": { "portrait": format!("{ART}/portrait.jpg"), "hero": format!("{ART}/hero.jpg") } } }]);
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": topics });
    t.http.serve("https://xyz.dev/c/pc.json", catalog.to_string());
    // A portable Wine repack: its folder is "Pocket Tanks", its game_info.txt names the game.
    let info = b"Pocket Tanks\n1.6\npockettanks.exe\n\n\nPocket Tanks Deluxe\n";
    t.torrents.has(
        HASH,
        "Pocket Tanks",
        &[
            ("Pocket Tanks/start.sh", b"#!/usr/bin/env bash\n"),
            ("Pocket Tanks/game_info/game_info.txt", info),
            ("Pocket Tanks/game_info/data/pockettanks.exe", &exe()),
        ],
    );
    let address = serde_json::json!({ "catalog_url": "https://xyz.dev/c" });
    let set = Command::SetConfig {
        plugin: PluginId::new("release-catalog").unwrap(),
        scope: ConfigScope::Emulator,
        value: address,
    };
    t.core.dispatch(set).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    // The Catalog shows the bundled copy first: wait for the one fetched from the address.
    for _ in 0..500 {
        let catalog = t.core.view().catalog;
        let fetched = !catalog.sources.is_empty() && catalog.sources.iter().all(|s| s.contains(": fetched from "));
        if fetched && !catalog.games.games.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let offer: GameKey = t.core.view().catalog.games.games[0].key.parse().unwrap();
    t.core.dispatch(Command::DownloadRelease { offer, release: HASH.into(), folder: None }).await.unwrap();
    let row = |t: &TestCore| t.core.view().downloads.torrents.into_iter().next();
    for (step, state) in [("ready", "ready"), ("complete", "complete")] {
        for _ in 0..300 {
            if row(t).is_some_and(|r| r.state == state) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if step == "ready" {
            let start = Command::Torrent { id: HASH.into(), action: nexus_core::TorrentAction::Start };
            t.core.dispatch(start).await.unwrap();
            t.torrents.complete(HASH);
        }
    }
    t.core.dispatch(Command::Torrent { id: HASH.into(), action: nexus_core::TorrentAction::Install }).await.unwrap();
    for _ in 0..300 {
        if row(t).is_some_and(|r| r.install == "done") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(p.titles(), [("Pocket Tanks Deluxe".to_owned(), "PC".to_owned())], "the repack's name for it");
    let portrait = nexus_ui_api::ImageRef::Url {
        plugin: PluginId::new("release-catalog").unwrap(),
        url: format!("{ART}/portrait.jpg"),
    };
    let card = t.core.view().home.games.first().cloned().unwrap();
    assert_eq!(card.cover, Some(portrait.clone()), "the offer's art");
    t.core.dispatch(Command::OpenGame(card.key.parse().unwrap())).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().cover, Some(portrait));
}

/// The first bytes of a 32-bit Linux program.
fn elf32() -> Vec<u8> {
    let mut h = elf();
    h[4] = 1;
    h[18..20].copy_from_slice(&3u16.to_le_bytes());
    h
}

#[tokio::test]
async fn a_portable_wine_game_this_system_can_t_run_starts_its_exe_through_proton() {
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .runs_32bit(false)
        .built_in(PluginByKind::emulator(PcPlugin))
        .boot_core()
        .await;
    let p = with_folders(test).await;
    // A portable Wine wrapper: its script starts the 32-bit Wine beside it.
    p.download(&[
        ("Pocket Tanks/start.sh", b"#!/usr/bin/env bash\n"),
        ("Pocket Tanks/wine/bin/wine", &elf32()),
        ("Pocket Tanks/game_info/data/pockettanks.exe", &exe()),
    ]);
    assert_eq!(p.install().await, "done");
    assert_eq!(p.manifest("Pocket-Tanks").launch, "start.sh");
    let launch = Command::Launch("emu-pc:Pocket-Tanks".parse().unwrap());

    let refused = p.test.core.dispatch(launch.clone()).await.unwrap_err().to_string();
    assert!(refused.contains("32-bit") && refused.contains("install umu-launcher"), "{refused}");

    p.test.spawner.on_path("umu-run", Path::new("/usr/bin/umu-run"));
    p.test.core.dispatch(launch).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("umu-run"));
    let exe = p.games.path().join(native("PC/Pocket-Tanks/game_info/data/pockettanks.exe"));
    assert_eq!(run.args, [exe.display().to_string()]);
    // With no 32-bit libraries, a 32-bit game's graphics come from the 64-bit ones.
    assert_eq!(run.env.get("PROTON_USE_WOW64").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn a_windows_game_at_a_resolution_of_its_own_fills_the_screen_through_gamescope() {
    let p = boot(Platform::Linux).await;
    p.test.spawner.on_path("umu-run", Path::new("/usr/bin/umu-run"));
    p.download(&[("Pocket Tanks/pockettanks.exe", &exe())]);
    assert_eq!(p.install().await, "done");
    let game: nexus_api::GameKey = "emu-pc:Pocket-Tanks".parse().unwrap();

    // Its Game Hub has the setting, at the screen's resolution.
    p.test.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
    let detail = p.test.core.view().detail.unwrap();
    let panel = detail.settings.first().expect("the game's settings");
    let field = panel.fields.iter().find(|f| f.key == "resolution").expect("a Resolution setting");
    assert_eq!(field.value, "screen");
    assert_eq!(field.choices[..3], ["screen", "640x480", "800x600"]);

    let set = |value: &str| Command::SetConfigField {
        plugin: pc(),
        scope: nexus_api::ConfigScope::Game("Pocket-Tanks".into()),
        key: "resolution".into(),
        value: value.into(),
    };
    p.test.core.dispatch(set("800x600")).await.unwrap();
    let launch = Command::Launch(game.clone());
    let refused = p.test.core.dispatch(launch.clone()).await.unwrap_err().to_string();
    assert!(refused.contains("install gamescope"), "{refused}");

    p.test.spawner.on_path("gamescope", Path::new("/usr/bin/gamescope"));
    p.test.core.dispatch(launch.clone()).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("gamescope"));
    let exe = p.games.path().join(native("PC/Pocket-Tanks/pockettanks.exe")).display().to_string();
    let args: Vec<&str> = run.args.iter().map(String::as_str).collect();
    assert_eq!(args, ["-f", "-w", "800", "-h", "600", "--", "umu-run", &exe]);
    assert_eq!(run.env.get("PROTONPATH").map(String::as_str), Some("GE-Proton"));
}

#[tokio::test]
async fn a_native_game_has_no_resolution_setting() {
    let p = boot(Platform::Linux).await;
    p.download(&[("Tux/tux.x86_64", &elf())]);
    assert_eq!(p.install().await, "done");
    p.test.core.dispatch(Command::OpenGame("emu-pc:Tux".parse().unwrap())).await.unwrap();
    assert!(p.test.core.view().detail.unwrap().settings.is_empty());
}

#[tokio::test]
async fn on_windows_the_exe_runs_as_it_is_and_a_linux_game_says_it_can_t_run_here() {
    let p = boot(Platform::Windows).await;
    p.download(&[("Game/start.sh", b"#!/bin/sh\n"), ("Game/Game.exe", &exe())]);
    assert_eq!(p.install().await, "done");
    assert_eq!(p.manifest("Game").launch, "Game.exe", "the one Windows can start");
    p.test.core.dispatch(Command::Launch("emu-pc:Game".parse().unwrap())).await.unwrap();
    let run = p.test.spawner.wait_for(1).await;
    assert_eq!(run.command.program, p.games.path().join("PC/Game/Game.exe"));
    run.exit(0);
    tokio::time::sleep(Duration::from_millis(50)).await;

    std::fs::remove_dir_all(p.packages.path().join("PC/torrent-abc")).unwrap();
    p.download(&[("Penguins/penguins.x86_64", &elf())]);
    assert_eq!(p.install().await, "done", "it installs anyway");
    let refused = p.test.core.dispatch(Command::Launch("emu-pc:Penguins".parse().unwrap())).await.unwrap_err();
    assert!(refused.to_string().contains("This is a Linux game: it can't run on Windows"), "{refused}");
}

#[tokio::test]
async fn on_a_mac_a_release_with_a_script_and_an_exe_runs_the_exe_through_wine() {
    let p = boot(Platform::Macos).await;
    p.download(&[("Game/start.sh", b"#!/bin/sh\n"), ("Game/Game.exe", &exe())]);
    assert_eq!(p.install().await, "done");
    assert_eq!(p.manifest("Game").launch, "Game.exe");
    p.test.spawner.on_path("wine", Path::new("/opt/homebrew/bin/wine"));
    p.test.core.dispatch(Command::Launch("emu-pc:Game".parse().unwrap())).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("wine"));
    assert!(!run.env.contains_key("PROTONPATH"), "Proton is Linux's");
}

#[tokio::test]
async fn the_release_s_packaging_comes_from_its_unpacked_folder_and_a_data_archive_stays_packed() {
    let p = boot(Platform::Linux).await;
    // A repack: an archive with nothing beside it, which the host unpacked.
    p.download(&[
        ("Repack/game.part1.rar", b"one"),
        ("Repack/game.part2.rar", b"two"),
        ("unpacked/Repack/game/Hollow/hollow.x86_64", &elf()),
        ("unpacked/Repack/game/Hollow/data.bin", b"data"),
    ]);
    assert_eq!(p.install().await, "done");
    let game = p.games.path().join("PC/Repack");
    assert_eq!(std::fs::read(game.join("Hollow/data.bin")).unwrap(), b"data");
    assert!(!game.join("game.part1.rar").exists(), "the packaging isn't copied");
    assert_eq!(p.manifest("Repack").launch, "Hollow/hollow.x86_64");

    // A game whose data archive sits beside its program: the archive stays as it is.
    std::fs::remove_dir_all(p.packages.path().join("PC/torrent-abc")).unwrap();
    p.download(&[("Tux/tux.x86_64", &elf()), ("Tux/data.tar", b"tar"), ("unpacked/Tux/data/levels.bin", b"levels")]);
    assert_eq!(p.install().await, "done");
    let tux = p.games.path().join("PC/Tux");
    assert_eq!(std::fs::read(tux.join("data.tar")).unwrap(), b"tar");
    assert!(!tux.join("levels.bin").exists(), "its unpacked files aren't used");
}

#[tokio::test]
async fn a_gog_installer_s_files_install_however_deep_they_are() {
    let p = boot(Platform::Linux).await;
    // World of Goo's levels are nine folders down in the unpacked installer.
    let level = "data/noarch/game/game/res/levels/GoingUp/GoingUp.scene.bin";
    p.download(&[
        ("world_of_goo.sh", b"#!/bin/sh\n"),
        ("unpacked/world_of_goo/data/noarch/start.sh", b"#!/bin/bash\n"),
        ("unpacked/world_of_goo/data/noarch/game/WorldOfGoo.bin.x86_64", &elf()),
        ("unpacked/world_of_goo/data/noarch/gameinfo", b"World of Goo\n1.6.538\n87464\n"),
        (&format!("unpacked/world_of_goo/{level}"), b"scene"),
    ]);
    assert_eq!(p.install().await, "done");
    assert_eq!(p.titles(), [("World of Goo".to_owned(), "PC".to_owned())], "the name GOG gives it");
    let game = p.games.path().join("PC/World-of-Goo");
    assert_eq!(std::fs::read(game.join(level)).unwrap(), b"scene", "the deepest file too");
}

#[tokio::test]
async fn a_gog_game_installed_before_shows_the_name_gog_gives_it() {
    let p = boot(Platform::Linux).await;
    let game = p.games.path().join("PC/world-of-goo-2-1-0-13120-81588");
    std::fs::create_dir_all(game.join("data/noarch")).unwrap();
    std::fs::write(game.join("data/noarch/start.sh"), b"#!/bin/bash\n").unwrap();
    std::fs::write(game.join("data/noarch/gameinfo"), b"World of Goo 2\n1.0.13120\n81588\n").unwrap();
    let manifest = serde_json::json!({ "version": 1, "name": "world of goo 2 1 0 13120 81588",
        "launch": "data/noarch/start.sh", "kind": "script" });
    std::fs::write(game.join(GAME_FILE), manifest.to_string()).unwrap();
    p.test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    assert_eq!(p.titles(), [("World of Goo 2".to_owned(), "PC".to_owned())]);
}

#[tokio::test]
async fn parts_spelled_in_another_case_belong_to_their_set() {
    let p = boot(Platform::Linux).await;
    // The host names a set's folder as its first part spells it.
    p.download(&[
        ("Repack/Game.part1.rar", b"one"),
        ("Repack/game.part2.rar", b"two"),
        ("unpacked/Repack/Game/game.x86_64", &elf()),
    ]);
    assert_eq!(p.install().await, "done");
    let game = p.games.path().join("PC/Repack");
    assert!(game.join("game.x86_64").is_file());
    assert!(!game.join("game.part2.rar").exists(), "every part is the packaging");
}

#[tokio::test]
async fn an_appimage_runs_without_fuse_and_a_mac_app_runs_its_program() {
    let p = boot(Platform::Linux).await;
    let mut appimage = elf();
    appimage[8..11].copy_from_slice(b"AI\x02");
    p.download(&[("Tux/Tux.AppImage", &appimage)]);
    assert_eq!(p.install().await, "done");
    p.test.core.dispatch(Command::Launch("emu-pc:Tux".parse().unwrap())).await.unwrap();
    let run = p.test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, p.games.path().join("PC/Tux/Tux.AppImage"));
    assert_eq!(run.env.get("APPIMAGE_EXTRACT_AND_RUN").map(String::as_str), Some("1"));

    let mac = boot(Platform::Macos).await;
    mac.download(&[("Tux/Tux.app/Contents/MacOS/Tux", &[0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1])]);
    assert_eq!(mac.install().await, "done");
    mac.test.core.dispatch(Command::Launch("emu-pc:Tux".parse().unwrap())).await.unwrap();
    let run = mac.test.spawner.wait_for(1).await.command;
    let program = mac.games.path().join("PC/Tux/Tux.app/Contents/MacOS/Tux");
    assert_eq!((run.program, run.cwd), (program.clone(), program.parent().unwrap().to_path_buf()));
}

#[tokio::test]
async fn a_download_with_nothing_that_starts_a_game_is_refused() {
    let p = boot(Platform::Linux).await;
    p.download(&[("Docs/readme.txt", b"hi")]);
    let refused = p.install().await;
    assert!(refused.starts_with("failed") && refused.contains("Nothing in this download starts a game"), "{refused}");
}
