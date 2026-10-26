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
    // Steam's games are a system of their own in the Library, beside PC, PS5 and PS4, with the
    // name and color the plugin's manifest gives them.
    let chips: Vec<_> = s.test.core.view().home.library.systems.into_iter().map(|f| (f.label, f.count)).collect();
    assert!(chips.contains(&("Steam".to_owned(), 2)), "{chips:?}");
    let card = s.test.core.view().home.games.into_iter().next().unwrap();
    assert_eq!(card.system, nexus_ui_api::GameSystemVm { name: "Steam".into(), color: Some([0x66, 0xc0, 0xf4]) });

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

/// Steam at its default folder under `home`, listing `libraries` (and itself) as its libraries.
fn steam_at_home(home: &Path, libraries: &[&Path]) -> PathBuf {
    let root = home.join(".local/share/Steam");
    let mut list = format!("\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n", root.display());
    for (i, library) in libraries.iter().enumerate() {
        list.push_str(&format!("\t\"{}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n", i + 1, library.display()));
    }
    list.push_str("}\n");
    write(root.join("steamapps/libraryfolders.vdf"), list.as_bytes());
    root
}

#[tokio::test]
async fn the_libraries_steam_lists_are_read_with_nothing_set() {
    let home = tempfile::tempdir().unwrap();
    // Wherever the player put it: Steam's own list says.
    let drive = tempfile::tempdir().unwrap();
    write(
        drive.path().join("steamapps/appmanifest_440.acf"),
        acf(440, "Team Fortress 2", 4, "Team Fortress 2").as_bytes(),
    );
    // Listed, but not a Steam library: no `steamapps` in it.
    let other = tempfile::tempdir().unwrap();
    write(other.path().join("notes.txt"), b"x");
    let root = steam_at_home(home.path(), &[drive.path(), other.path(), Path::new("relative/path")]);
    write(root.join("steamapps/appmanifest_570.acf"), acf(570, "Dota 2", 4, "dota 2 beta").as_bytes());
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .home(home.path())
        .built_in(PluginByKind::emulator(SteamPlugin))
        .boot_core()
        .await;
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let titles: Vec<_> = test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert_eq!(titles, ["Dota 2", "Team Fortress 2"], "Steam's folder and the library on another drive");
}

#[tokio::test]
async fn the_player_s_own_steam_libraries_setting_replaces_steam_s_list() {
    let home = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    write(
        drive.path().join("steamapps/appmanifest_440.acf"),
        acf(440, "Team Fortress 2", 4, "Team Fortress 2").as_bytes(),
    );
    let root = steam_at_home(home.path(), &[drive.path()]);
    write(root.join("steamapps/appmanifest_570.acf"), acf(570, "Dota 2", 4, "dota 2 beta").as_bytes());
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .home(home.path())
        .built_in(PluginByKind::emulator(SteamPlugin))
        .boot_core()
        .await;
    test.core.dispatch(Command::SetFolders { name: "steam".into(), paths: vec![root] }).await.unwrap();
    test.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let titles: Vec<_> = test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert_eq!(titles, ["Dota 2"], "only the folders the player set");
}

/// Steam's app cache (`appcache/appinfo.vdf`, version 0x29) naming each app of `apps`.
fn app_cache(apps: &[(u32, &str)]) -> Vec<u8> {
    let table = ["appinfo", "common", "name"];
    let mut out = Vec::new();
    out.extend(0x0756_4429u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    for (id, name) in apps {
        // appinfo { common { name "…" } }, keys by their place in the table.
        let mut keys = vec![0];
        keys.extend(0u32.to_le_bytes());
        keys.push(0);
        keys.extend(1u32.to_le_bytes());
        keys.push(1);
        keys.extend(2u32.to_le_bytes());
        keys.extend(name.as_bytes());
        keys.extend([0, 8, 8, 8]);
        out.extend(id.to_le_bytes());
        out.extend((60 + keys.len() as u32).to_le_bytes());
        out.extend([0u8; 60]);
        out.extend(keys);
    }
    out.extend(0u32.to_le_bytes());
    let offset = out.len() as u64;
    out[8..16].copy_from_slice(&offset.to_le_bytes());
    out.extend((table.len() as u32).to_le_bytes());
    for t in table {
        out.extend(t.as_bytes());
        out.push(0);
    }
    out
}

/// The player's Steam collections in `root`: Favorites, Hidden and one of their own.
fn collections(root: &Path, favorite: &str, hidden: &str, own: &str) {
    let entry = |id: &str, added: &str| {
        let value = format!(r#"{{"id":"{id}","name":"{id}","added":{added},"removed":[]}}"#);
        serde_json::json!([format!("user-collections.{id}"), { "key": format!("user-collections.{id}"), "value": value }])
    };
    let list = serde_json::json!([entry("favorite", favorite), entry("hidden", hidden), entry("uc-coop", own)]);
    let dir = root.join("userdata/1234/config/cloudstorage");
    write(dir.join("cloud-storage-namespace-1.json"), list.to_string().as_bytes());
    write(dir.join("cloud-storage-namespaces.json"), b"[[1,\"1\"]]");
}

/// The Catalog's games: title and badges' text.
fn catalog(s: &Steam) -> Vec<(String, Vec<String>)> {
    let games = s.test.core.view().catalog.games.games;
    games.into_iter().map(|g| (g.title, g.badges.into_iter().map(|b| b.text).collect())).collect()
}

/// The Catalog open, once it shows `n` games.
async fn open_catalog(s: &Steam, n: usize) {
    s.test.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    for _ in 0..200 {
        if s.test.core.view().catalog.games.games.len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the Catalog never showed {n} games: {:?}", catalog(s));
}

#[tokio::test]
async fn the_catalog_shows_the_player_s_steam_library_with_hidden_games_under_hidden() {
    let s = boot().await;
    // Dota 2 and Portal 2 are installed; Team Fortress 2 isn't; 2500 is hidden, and Steam's
    // cache doesn't name it.
    collections(s.root.path(), "[570, 440]", "[2500]", "[620]");
    write(
        s.root.path().join("appcache/appinfo.vdf"),
        &app_cache(&[(440, "Team Fortress 2"), (570, "Dota 2"), (620, "Portal 2")]),
    );
    open_catalog(&s, 3).await;

    assert_eq!(
        catalog(&s),
        [
            ("Dota 2".to_owned(), vec!["Favorite".to_owned(), "Installed".to_owned()]),
            ("Portal 2".to_owned(), vec!["Installed".to_owned()]),
            ("Team Fortress 2".to_owned(), vec!["Favorite".to_owned()]),
        ],
        "the player's library, without what they hid"
    );
    let chips: Vec<_> = s.test.core.view().catalog.games.systems.into_iter().map(|f| f.label).collect();
    assert!(chips.contains(&"Steam".to_owned()), "{chips:?}");

    s.test.core.dispatch(Command::CatalogSystem(Some("hidden".into()))).await.unwrap();
    assert_eq!(catalog(&s), [("Steam app 2500".to_owned(), vec![])], "Hidden shows it, by its number");
}

#[tokio::test]
async fn install_asks_steam_to_install_the_game_and_downloads_nothing() {
    let s = boot().await;
    collections(s.root.path(), "[440]", "[]", "[]");
    write(s.root.path().join("appcache/appinfo.vdf"), &app_cache(&[(440, "Team Fortress 2")]));
    open_catalog(&s, 1).await;
    let key: GameKey = s.test.core.view().catalog.games.games[0].key.parse().unwrap();
    s.test.core.dispatch(Command::OpenGame(key.clone())).await.unwrap();
    let release = s.test.core.view().detail.unwrap().releases.remove(0);
    assert_eq!(release.store, "Steam", "its Game Hub says Steam installs it");

    s.test.core.dispatch(Command::DownloadRelease { offer: key, release: release.id, folder: None }).await.unwrap();
    assert_eq!(s.test.desktop.links(), ["steam://install/440"]);
    assert!(s.test.core.view().downloads.jobs.is_empty(), "nothing downloads");
    let notices = s.test.core.view().notices;
    assert!(notices.iter().any(|n| n.contains("Steam is installing Team Fortress 2")), "{notices:?}");
}

#[tokio::test]
async fn a_steam_game_s_hub_installs_it_until_it_is_installed_and_then_plays_it() {
    let s = boot().await;
    s.test.spawner.on_path("steam", Path::new("/usr/bin/steam"));
    // Dota 2 is installed, Team Fortress 2 isn't.
    collections(s.root.path(), "[570, 440]", "[]", "[]");
    write(s.root.path().join("appcache/appinfo.vdf"), &app_cache(&[(440, "Team Fortress 2"), (570, "Dota 2")]));
    open_catalog(&s, 2).await;
    let offer = |title: &str| -> GameKey {
        let games = s.test.core.view().catalog.games.games;
        games.into_iter().find(|g| g.title == title).unwrap().key.parse().unwrap()
    };

    s.test.core.dispatch(Command::OpenGame(offer("Team Fortress 2"))).await.unwrap();
    let hub = s.test.core.view().detail.unwrap();
    assert!(!hub.can.play, "not installed: no Play");
    assert_eq!(hub.releases.iter().map(|r| r.store.as_str()).collect::<Vec<_>>(), ["Steam"], "Install in Steam");

    let dota = offer("Dota 2");
    s.test.core.dispatch(Command::OpenGame(dota.clone())).await.unwrap();
    let hub = s.test.core.view().detail.unwrap();
    assert!(hub.can.play, "installed: Play");
    assert!(hub.releases.is_empty(), "nothing to install: {:?}", hub.releases);
    s.test.core.dispatch(Command::Launch(dota)).await.unwrap();
    let run = s.test.spawner.wait_for(1).await.command;
    assert_eq!(run.args, ["-silent", "steam://rungameid/570"], "Play starts the installed game");
}

#[tokio::test]
async fn every_game_s_actions_follow_one_rule_on_its_card_and_its_game_hub() {
    let s = boot().await;
    // Dota 2 is installed, Team Fortress 2 isn't.
    collections(s.root.path(), "[570, 440]", "[]", "[]");
    write(s.root.path().join("appcache/appinfo.vdf"), &app_cache(&[(440, "Team Fortress 2"), (570, "Dota 2")]));
    let dota: GameKey = "emu-steam:570".parse().unwrap();
    // Its details say how it uninstalls.
    s.test.core.dispatch(Command::OpenGame(dota.clone())).await.unwrap();
    open_catalog(&s, 2).await;
    let view = s.test.core.view();
    let card =
        |games: &[nexus_ui_api::GameCardVm], title: &str| games.iter().find(|g| g.title == title).unwrap().clone();
    let library = card(&view.home.games, "Dota 2");
    let installed = card(&view.catalog.games.games, "Dota 2");
    let not_installed = card(&view.catalog.games.games, "Team Fortress 2");

    let plays = nexus_ui_api::GameActionsVm {
        primary: "play".into(),
        play: true,
        uninstall: dota.to_string(),
        uninstall_store: "Steam".into(),
        ..Default::default()
    };
    let gets = nexus_ui_api::GameActionsVm {
        primary: "get".into(),
        get: "Install in Steam".into(),
        get_release: "steam-440".into(),
        get_store: "Steam".into(),
        ..Default::default()
    };
    assert_eq!(library.can, plays, "a Library game: Play, and Steam uninstalls it");
    assert_eq!(installed.can, plays, "its Catalog card: the same, for the installed game it is");
    assert_eq!(not_installed.can, gets, "not installed: get it from Steam");
    assert_eq!(not_installed.availability, "", "no torrent to be available");

    // Each Game Hub says the same as the game's card.
    for card in [library, installed, not_installed] {
        s.test.core.dispatch(Command::OpenGame(card.key.parse().unwrap())).await.unwrap();
        assert_eq!(s.test.core.view().detail.unwrap().can, card.can, "{}", card.key);
    }
}

#[tokio::test]
async fn uninstall_asks_steam_to_uninstall_its_game() {
    let s = boot().await;
    let dota: GameKey = "emu-steam:570".parse().unwrap();
    s.test.core.dispatch(Command::OpenGame(dota.clone())).await.unwrap();
    s.test.core.dispatch(Command::UninstallGame(dota)).await.unwrap();
    assert_eq!(s.test.desktop.links(), ["steam://uninstall/570"]);
    assert!(s.root.path().join("steamapps/common/dota 2 beta").is_dir(), "Steam removes it, not Nexus");
    let notices = s.test.core.view().notices;
    assert!(notices.iter().any(|n| n.contains("Steam is uninstalling Dota 2")), "{notices:?}");
}

#[tokio::test]
async fn a_game_steam_has_no_art_cached_for_gets_only_art_steam_s_server_has() {
    let s = boot().await;
    let provided = s.test.core.kernel().provided(&steam()).await.unwrap();
    let catalog = provided.catalog.unwrap();
    // Portal 2 has nothing in Steam's cache: an older game may have no hero or logo on the
    // server either, but every game has its cover and header there.
    let details = catalog.details("620").await.unwrap();
    let header = "https://cdn.cloudflare.steamstatic.com/steam/apps/620/header.jpg";
    assert_eq!(details.background, Some(ImageSource::Url(header.into())), "the header, not a guessed hero");
    assert_eq!(catalog.artwork("620", ArtworkKind::Logo).await.unwrap(), None, "no logo to guess at");

    // Its Catalog release says the same.
    collections(s.root.path(), "[620]", "[]", "[]");
    let releases = provided.releases.unwrap();
    let list = releases.releases(&Console::new("Steam").unwrap()).await.unwrap().unwrap();
    let steam = list.releases[0].steam.clone().unwrap();
    assert_eq!((steam.hero.as_str(), steam.logo.as_str(), steam.header.as_str()), ("", "", header));
}

/// Team Fortress 2's app file in `root`, as Steam writes it while it installs: `flags`, and
/// bytes downloaded and written into place of 1000 each.
fn installing_tf2(root: &Path, flags: u32, downloaded: u32, staged: u32) {
    let acf = format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"440\"\n\t\"name\"\t\t\"Team Fortress 2\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\
         \"installdir\"\t\t\"Team Fortress 2\"\n\t\"BytesToDownload\"\t\t\"1000\"\n\t\"BytesDownloaded\"\t\t\"{downloaded}\"\n\t\
         \"BytesToStage\"\t\t\"1000\"\n\t\"BytesStaged\"\t\t\"{staged}\"\n}}\n"
    );
    write(root.join("steamapps/appmanifest_440.acf"), acf.as_bytes());
}

/// The Downloads card of Steam's install of Team Fortress 2, once `done` says it is the one.
async fn steam_card(s: &Steam, done: impl Fn(&nexus_ui_api::TorrentVm) -> bool) -> nexus_ui_api::TorrentVm {
    for _ in 0..300 {
        let cards = s.test.core.view().downloads.torrents;
        if let Some(card) = cards.into_iter().find(|t| t.id == "store:emu-steam:steam-440")
            && done(&card)
        {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no such card: {:?}", s.test.core.view().downloads.torrents);
}

#[tokio::test]
async fn steam_s_install_shows_in_downloads_until_the_game_is_installed() {
    let s = boot().await;
    collections(s.root.path(), "[440]", "[]", "[]");
    write(s.root.path().join("appcache/appinfo.vdf"), &app_cache(&[(440, "Team Fortress 2")]));
    // Steam downloads it: a quarter there.
    installing_tf2(s.library.path(), 1026 | (1 << 20), 250, 0);

    let card = steam_card(&s, |t| t.state == "downloading" && t.install.is_empty()).await;
    assert_eq!((card.title.as_str(), card.store.as_str(), card.progress), ("Team Fortress 2", "Steam", 0.25));
    assert_eq!(card.actions, ["open-store"], "Steam pauses and cancels its own");
    s.test
        .core
        .dispatch(Command::Torrent { id: card.id.clone(), action: nexus_core::TorrentAction::OpenStore })
        .await
        .unwrap();
    assert_eq!(s.test.desktop.links(), ["steam://open/downloads"]);

    // Downloaded: Steam writes it into place.
    installing_tf2(s.library.path(), 1026 | (1 << 21), 1000, 500);
    let card = steam_card(&s, |t| t.install == "installing").await;
    assert_eq!(card.install_progress, 0.5);

    // Installed: the card goes, and the game is in the Library.
    installing_tf2(s.library.path(), 4, 1000, 1000);
    std::fs::create_dir_all(s.library.path().join("steamapps/common/Team Fortress 2")).unwrap();
    let tf2_card = |s: &Steam| s.test.core.view().downloads.torrents.into_iter().any(|t| t.id.ends_with("steam-440"));
    for _ in 0..300 {
        if !tf2_card(&s) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!tf2_card(&s), "{:?}", s.test.core.view().downloads.torrents);
    let titles: Vec<_> = s.test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert!(titles.contains(&"Team Fortress 2".to_owned()), "{titles:?}");
    let notices = s.test.core.view().notices;
    assert!(notices.iter().any(|n| n == "Team Fortress 2 is installed"), "{notices:?}");
}

#[tokio::test]
async fn a_steam_game_s_hub_has_steam_s_screenshots_words_and_trailer_kept_a_week() {
    let s = boot().await;
    let details = serde_json::json!({ "570": { "success": true, "data": {
        "name": "Dota 2", "short_description": "Every day, millions of players battle.",
        "developers": ["Valve"], "genres": [{"description": "Action"}, {"description": "Strategy"}],
        "release_date": {"date": "Jul 9, 2013"},
        "screenshots": [{"path_full": "https://shared.akamai.steamstatic.com/ss_1.jpg"},
                        {"path_full": "https://shared.akamai.steamstatic.com/ss_2.jpg"}],
        "movies": [{"hls_h264": "https://video.akamai.steamstatic.com/570/hls.m3u8"}] } } });
    let url = "https://store.steampowered.com/api/appdetails?appids=570&l=english";
    s.test.http.serve(url, details.to_string());
    // Booting asked before Steam answered: nothing was kept, and the next look asks again.
    s.test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let dota: GameKey = "emu-steam:570".parse().unwrap();

    let mut hub = None;
    for _ in 0..300 {
        s.test.core.dispatch(Command::OpenGame(dota.clone())).await.unwrap();
        hub = s.test.core.view().detail.filter(|d| !d.screenshots.is_empty());
        if hub.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let hub = hub.unwrap_or_else(|| panic!("no details; sent {:?}", s.test.http.sent()));
    assert_eq!(hub.screenshots.len(), 2);
    assert_eq!(hub.description, "Every day, millions of players battle.");
    assert_eq!(hub.genres, ["Action", "Strategy"]);
    assert_eq!(hub.trailer_url, "https://video.akamai.steamstatic.com/570/hls.m3u8");

    // Kept: once it answered, Steam isn't asked again.
    let asked = s.test.http.sent().iter().filter(|u| *u == url).count();
    s.test.core.dispatch(Command::RefreshLibrary).await.unwrap();
    s.test.core.dispatch(Command::OpenGame(dota)).await.unwrap();
    assert_eq!(s.test.http.sent().iter().filter(|u| *u == url).count(), asked, "{:?}", s.test.http.sent());
}
