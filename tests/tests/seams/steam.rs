//! Seam 1, core app API: the Steam plugin on a fake host. The games Steam installed show in the
//! Library with Steam's art, and Play asks Steam to start them; the session then follows the
//! game Steam started (`LaunchSpec::game_process`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use emu_steam::SteamPlugin;
use nexus_api::*;
use nexus_core::Command;
use nexus_testkit::{FakeHost, TestCore, contract};
use nexus_ui_api::ImageRef;

fn steam() -> PluginId {
    PluginId::new("emu-steam").unwrap()
}

/// An app manifest as Steam writes it.
fn acf(id: u32, name: &str, flags: u32, dir: &str) -> String {
    format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\t\
         \"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{dir}\"\n\t\"SizeOnDisk\"\t\t\"2048\"\n}}\n"
    )
}

fn write(path: PathBuf, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

struct Steam {
    test: TestCore,
    /// Steam's own folder.
    root: tempfile::TempDir,
    /// A library on another drive.
    library: tempfile::TempDir,
}

/// Steam with Dota 2 (art in its cache) and Proton in its own folder, Portal 2 in a library on
/// another drive, and a game still downloading.
async fn boot() -> Steam {
    let test =
        FakeHost::new().platform(Some(Platform::Linux)).built_in(PluginByKind::emulator(SteamPlugin)).boot_core().await;
    let (root, library) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let apps = root.path().join("steamapps");
    write(apps.join("appmanifest_570.acf"), acf(570, "Dota 2", 4, "dota 2 beta").as_bytes());
    write(
        apps.join("appmanifest_1493710.acf"),
        acf(1493710, "Proton Experimental", 4, "Proton - Experimental").as_bytes(),
    );
    write(
        apps.join("appmanifest_1628350.acf"),
        acf(1628350, "Steam Linux Runtime 3.0 (sniper)", 4, "SteamLinuxRuntime_sniper").as_bytes(),
    );
    // Downloading: not fully installed (no 4 in its flags).
    write(apps.join("appmanifest_42.acf"), acf(42, "Half Here", 1026, "half").as_bytes());
    write(root.path().join("steamapps/common/dota 2 beta/game/dota.sh"), b"#!/bin/sh\n");
    write(root.path().join("appcache/librarycache/570/library_600x900.jpg"), b"jpg");
    write(root.path().join("appcache/librarycache/570/library_hero.jpg"), b"jpg");
    write(library.path().join("steamapps/appmanifest_620.acf"), acf(620, "Portal 2", 6, "Portal 2").as_bytes());
    let folders = format!(
        "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\
         \"2\"\n\t{{\n\t\t\"path\"\t\t\"/mnt/elsewhere/SteamLibrary\"\n\t}}\n}}\n",
        root.path().display(),
        library.path().display()
    );
    write(root.path().join("steamapps/libraryfolders.vdf"), folders.as_bytes());
    let paths = vec![root.path().into(), library.path().into()];
    test.core.dispatch(Command::SetFolders { name: "steam".into(), paths }).await.unwrap();
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    Steam { test, root, library }
}

impl Steam {
    fn titles(&self) -> Vec<String> {
        self.test.core.view().home.games.into_iter().map(|g| g.title).collect()
    }

    fn cover(&self, title: &str) -> Option<ImageRef> {
        self.test.core.view().home.games.into_iter().find(|g| g.title == title).and_then(|g| g.cover)
    }

    /// The rows of the plugin page's section `id`, as `label: value`.
    async fn panel(&self, id: &str) -> Vec<String> {
        let provided = self.test.core.kernel().provided(&steam()).await.unwrap();
        let request = SlotRequest { slot: SlotId::EmulatorPanel, game: None };
        let items = provided.slots.unwrap().items(&request).await.unwrap();
        let section = items.into_iter().find(|i| i.id == id).expect("the section");
        let SlotContent::Section { rows, .. } = section.content else { panic!("a section") };
        rows.into_iter().map(|r| format!("{}: {}", r.label, r.value)).collect()
    }
}

#[tokio::test]
async fn steam_meets_every_contract() {
    let s = boot().await;
    s.test.spawner.on_path("steam", Path::new("/usr/bin/steam"));
    contract::emulator(&s.test, &steam()).await;
}

#[tokio::test]
async fn the_games_steam_installed_show_with_steam_s_art_and_its_tools_don_t() {
    let s = boot().await;
    assert_eq!(s.titles(), ["Dota 2", "Portal 2"], "installed games, from every library; no Proton, no downloads");

    let cached = HostPath::new("steam", 0, RelPath::new("appcache/librarycache/570/library_600x900.jpg").unwrap());
    assert_eq!(s.cover("Dota 2"), Some(ImageRef::Folder { plugin: steam(), path: cached }), "from Steam's cache");
    let url = "https://cdn.cloudflare.steamstatic.com/steam/apps/620/library_600x900.jpg";
    assert_eq!(s.cover("Portal 2"), Some(ImageRef::Url { plugin: steam(), url: url.into() }), "from Steam's server");
}

#[tokio::test]
async fn the_old_cache_layout_still_gives_a_game_its_cover() {
    let s = boot().await;
    std::fs::remove_dir_all(s.root.path().join("appcache/librarycache/570")).unwrap();
    // Before 2024 Steam kept each file flat, named after the game.
    write(s.root.path().join("appcache/librarycache/570_library_600x900.jpg"), b"jpg");
    s.test.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let flat = HostPath::new("steam", 0, RelPath::new("appcache/librarycache/570_library_600x900.jpg").unwrap());
    assert_eq!(s.cover("Dota 2"), Some(ImageRef::Folder { plugin: steam(), path: flat }));
}

#[tokio::test]
async fn play_asks_steam_to_start_the_game_and_follows_the_game_steam_started() {
    let s = boot().await;
    s.test.spawner.on_path("steam", Path::new("/usr/bin/steam"));
    let mut events = s.test.core.events();
    s.test.core.dispatch(Command::Launch("emu-steam:570".parse().unwrap())).await.unwrap();
    let steam_command = s.test.spawner.wait_for(1).await;
    let run = &steam_command.command;
    assert_eq!(run.program, Path::new("steam"));
    assert_eq!(run.args, ["-silent", "steam://rungameid/570"]);
    assert_eq!(run.cwd, s.root.path().join("steamapps/common/dota 2 beta"), "in the game's folder");
    assert!(run.keep_running, "it may become Steam's client, which outlives Nexus");

    // Steam hands the game to its reaper and the command exits.
    steam_command.exit(0);
    s.test.processes.start(7001, "/home/p/.local/share/Steam/ubuntu12_32/reaper SteamLaunch AppId=570 -- dota");
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(s.test.core.view().running.is_some(), "Dota 2 runs");
    s.test.processes.exit(7001);
    let exit = loop {
        if let Event::ProcessExited { exit, .. } = futures::StreamExt::next(&mut events).await.unwrap() {
            break exit;
        }
    };
    assert!(exit.success, "{exit:?}");
}

#[tokio::test]
async fn without_steam_play_says_to_install_it() {
    let s = boot().await;
    let refused = s.test.core.dispatch(Command::Launch("emu-steam:570".parse().unwrap())).await.unwrap_err();
    assert!(refused.to_string().contains("Install Steam"), "{refused}");
}

#[tokio::test]
async fn the_game_hub_says_it_runs_through_steam_and_the_page_lists_steam_s_libraries() {
    let s = boot().await;
    let provided = s.test.core.kernel().provided(&steam()).await.unwrap();
    let request = SlotRequest { slot: SlotId::GameDetailSection, game: Some("570".into()) };
    let items = provided.slots.unwrap().items(&request).await.unwrap();
    let SlotContent::Section { title, rows } = items[0].content.clone() else { panic!("a section") };
    assert_eq!(title, "Steam");
    let rows: Vec<_> = rows.into_iter().map(|r| format!("{}: {}", r.label, r.value)).collect();
    assert_eq!(rows, ["Runs: Through Steam", "Steam app: 570"]);

    let libraries = s.panel("libraries").await;
    assert_eq!(
        libraries,
        [
            format!("{}: Read", s.root.path().display()),
            format!("{}: Read", s.library.path().display()),
            "/mnt/elsewhere/SteamLibrary: Add it to Steam libraries in Settings to see its games".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_game_s_details_have_its_size_folder_and_steam_s_hero_art() {
    let s = boot().await;
    let provided = s.test.core.kernel().provided(&steam()).await.unwrap();
    let details = provided.catalog.unwrap().details("570").await.unwrap();
    assert_eq!(details.size_bytes, Some(2048));
    assert_eq!(details.folder, Some(HostPath::new("steam", 0, RelPath::new("steamapps/common/dota 2 beta").unwrap())));
    let hero = HostPath::new("steam", 0, RelPath::new("appcache/librarycache/570/library_hero.jpg").unwrap());
    assert_eq!(details.background, Some(ImageSource::Folder(hero)));
}

#[tokio::test]
async fn a_flatpak_steam_s_game_starts_through_the_flatpak() {
    let test =
        FakeHost::new().platform(Some(Platform::Linux)).built_in(PluginByKind::emulator(SteamPlugin)).boot_core().await;
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    write(root.join("steamapps/appmanifest_570.acf"), acf(570, "Dota 2", 4, "dota 2 beta").as_bytes());
    std::fs::create_dir_all(root.join("steamapps/common/dota 2 beta")).unwrap();
    test.core.dispatch(Command::SetFolders { name: "steam".into(), paths: vec![root.clone()] }).await.unwrap();
    test.spawner.on_path("flatpak", Path::new("/usr/bin/flatpak"));

    test.core.dispatch(Command::Launch("emu-steam:570".parse().unwrap())).await.unwrap();
    let run = test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("flatpak"));
    assert_eq!(run.args, ["run", "com.valvesoftware.Steam", "steam://rungameid/570"]);
}

#[tokio::test]
async fn a_flatpak_steam_s_library_on_another_drive_starts_through_the_flatpak_too() {
    let test =
        FakeHost::new().platform(Some(Platform::Linux)).built_in(PluginByKind::emulator(SteamPlugin)).boot_core().await;
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    std::fs::create_dir_all(root.join("steamapps")).unwrap();
    let drive = tempfile::tempdir().unwrap();
    write(drive.path().join("steamapps/appmanifest_620.acf"), acf(620, "Portal 2", 4, "Portal 2").as_bytes());
    std::fs::create_dir_all(drive.path().join("steamapps/common/Portal 2")).unwrap();
    let paths = vec![root, drive.path().into()];
    test.core.dispatch(Command::SetFolders { name: "steam".into(), paths }).await.unwrap();
    test.spawner.on_path("flatpak", Path::new("/usr/bin/flatpak"));

    test.core.dispatch(Command::Launch("emu-steam:620".parse().unwrap())).await.unwrap();
    let run = test.spawner.wait_for(1).await.command;
    assert_eq!(run.program, Path::new("flatpak"), "{:?}", run.args);
}
