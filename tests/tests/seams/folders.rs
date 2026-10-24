//! Seam 1, core app API: named host folders. The player maps a name to folders; a plugin
//! reaches only the names its manifest asks for, writes only with `write = true`, and can't
//! follow a symlink out.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::common::{full_path, native};
use nexus_api::*;
use nexus_core::{Command, CoreError};
use nexus_testkit::{FakeHost, StubPlugin, TestCore};

const MANIFEST: &str = "id = \"reader\"\nkind = \"emulator\"\nname = \"R\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
    [[permissions.filesystem]]\nfolder = \"games\"\n\
    [[permissions.filesystem]]\nfolder = \"saves\"\nwrite = true\ndefault = \"~/Saves\"\n";

/// Boots a plugin with `MANIFEST` and returns its folders facade.
async fn boot(host: FakeHost) -> (TestCore, Arc<dyn Folders>) {
    let folders: Arc<Mutex<Option<Arc<dyn Folders>>>> = Arc::default();
    let slot = folders.clone();
    let plugin = StubPlugin::from_toml(MANIFEST).provides(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.folders().clone());
        Ok(Provided::default())
    });
    let t = host.plugin(plugin).boot_core().await;
    t.core.kernel().provided(&PluginId::new("reader").unwrap()).await.unwrap();
    let folders = folders.lock().unwrap().clone().expect("init ran");
    (t, folders)
}

fn map(t: &TestCore, name: &str, roots: &[&Path]) {
    let paths: Vec<_> = roots.iter().map(|p| p.to_str().unwrap()).collect();
    t.core.kernel().config().set(&["app", "folders", name], serde_json::json!(paths)).unwrap();
}

fn at(folder: &str, root: u32, path: &str) -> HostPath {
    HostPath::new(folder, root, RelPath::new(path).unwrap())
}

#[tokio::test]
async fn a_mapped_folder_lists_and_reads_and_an_unmapped_one_is_empty() {
    let (t, folders) = boot(FakeHost::new()).await;
    assert!(folders.roots("games").await.unwrap().is_empty(), "nothing until the player maps it");

    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::create_dir(a.path().join("Game One")).unwrap();
    std::fs::write(a.path().join("Game One/eboot.bin"), b"0123456789").unwrap();
    map(&t, "games", &[a.path(), b.path()]);

    let roots = folders.roots("games").await.unwrap();
    assert_eq!(roots.iter().map(|r| r.root).collect::<Vec<_>>(), [0, 1]);
    let entries = folders.list(&at("games", 0, "Game One")).await.unwrap();
    assert_eq!(entries, [DirEntry { name: "eboot.bin".into(), is_dir: false, size: 10 }]);
    assert!(folders.list(&HostPath::new("games", 0, RelPath::root())).await.unwrap()[0].is_dir);
    assert_eq!(folders.read_range(&at("games", 0, "Game One/eboot.bin"), 2, 3).await.unwrap(), b"234");
    assert!(matches!(folders.list(&at("games", 2, "x")).await, Err(PluginError::NotFound(_))));
}

#[tokio::test]
async fn a_folder_s_size_counts_every_file_under_it_and_skips_linked_folders() {
    let (t, folders) = boot(FakeHost::new()).await;
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("Game/sce_sys")).unwrap();
    std::fs::write(games.path().join("Game/eboot.bin"), [0u8; 1000]).unwrap();
    std::fs::write(games.path().join("Game/sce_sys/param.json"), [0u8; 24]).unwrap();
    // A link back up: entered, it would loop.
    #[cfg(unix)]
    std::os::unix::fs::symlink(games.path().join("Game"), games.path().join("Game/sce_sys/loop")).unwrap();
    map(&t, "games", &[games.path()]);

    assert_eq!(folders.size(&at("games", 0, "Game")).await.unwrap(), 1024);
    assert_eq!(folders.size(&at("games", 0, "Missing")).await.unwrap(), 0);
}

#[tokio::test]
async fn names_the_manifest_did_not_ask_for_and_writes_without_a_write_grant_are_refused() {
    let (t, folders) = boot(FakeHost::new()).await;
    let games = tempfile::tempdir().unwrap();
    map(&t, "games", &[games.path()]);
    map(&t, "other", &[games.path()]);

    let err = folders.roots("other").await.unwrap_err();
    assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
    let err = folders.write(&at("games", 0, "new.txt"), b"x").await.unwrap_err();
    assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
    let err = folders.remove(&at("games", 0, "anything")).await.unwrap_err();
    assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
    assert!(!games.path().join("new.txt").exists());
}

#[tokio::test]
async fn a_write_grant_writes_and_its_default_is_under_the_home_folder() {
    let home = tempfile::tempdir().unwrap();
    let (_t, folders) = boot(FakeHost::new().home(home.path())).await;

    let roots = folders.roots("saves").await.unwrap();
    assert_eq!(roots[0].path, home.path().join("Saves").display().to_string());
    folders.write(&at("saves", 0, "slot1/data.bin"), b"save").await.unwrap();
    assert_eq!(std::fs::read(home.path().join("Saves/slot1/data.bin")).unwrap(), b"save");
    folders.remove(&at("saves", 0, "slot1")).await.unwrap();
    assert!(!home.path().join("Saves/slot1").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_out_of_a_folder_is_refused_but_a_symlinked_root_works() {
    let (t, folders) = boot(FakeHost::new()).await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"x").unwrap();
    let real = tempfile::tempdir().unwrap();
    std::fs::write(real.path().join("game.txt"), b"ok").unwrap();
    std::os::unix::fs::symlink(outside.path(), real.path().join("escape")).unwrap();
    // The player's folder is itself a link, as `~/Games` often is.
    let links = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(real.path(), links.path().join("Games")).unwrap();
    map(&t, "games", &[&links.path().join("Games")]);

    assert_eq!(folders.read_range(&at("games", 0, "game.txt"), 0, 10).await.unwrap(), b"ok");
    let err = folders.read_range(&at("games", 0, "escape/secret"), 0, 10).await.unwrap_err();
    assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
}

/// A launcher that prepares whatever spec the test gives it.
struct SpecLauncher(LaunchSpec);

#[async_trait]
impl Launcher for SpecLauncher {
    async fn prepare(&self, _: &str, _: &serde_json::Value) -> PluginResult<LaunchSpec> {
        Ok(self.0.clone())
    }
}

/// A PC-style plugin: games in `pc-games`, which it may run, and `wine` from the system.
async fn launcher_host(spec: LaunchSpec, process: &str) -> TestCore {
    let manifest = format!(
        "id = \"pc\"\nkind = \"emulator\"\nname = \"PC\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"launcher\"]\n[permissions]\nprocess = {process}\n\
         [[permissions.filesystem]]\nfolder = \"pc-games\"\n"
    );
    let plugin = StubPlugin::from_toml(&manifest).provides(move |_| {
        Ok(Provided { launcher: Some(Arc::new(SpecLauncher(spec.clone()))), ..Provided::default() })
    });
    FakeHost::new().plugin(plugin).boot_core().await
}

fn game_key() -> GameKey {
    "pc:game".parse().unwrap()
}

#[tokio::test]
async fn a_program_in_a_granted_folder_runs_from_its_real_path() {
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir(games.path().join("Game")).unwrap();
    std::fs::write(games.path().join("Game/start.sh"), b"#!/bin/sh\n").unwrap();
    let start = at("pc-games", 0, "Game/start.sh");
    let spec = LaunchSpec {
        program: start.clone().into(),
        args: vec!["--windowed".into(), LaunchArg::path(at("pc-games", 0, "Game/save"))],
        env: Default::default(),
        cwd: None,
    };
    let t = launcher_host(spec, "[\"folder:pc-games\"]").await;
    map(&t, "pc-games", &[games.path()]);

    t.core.dispatch(Command::Launch(game_key())).await.unwrap();

    let process = t.spawner.wait_for(1).await;
    assert_eq!(process.command.program, games.path().join("Game/start.sh"));
    assert_eq!(process.command.cwd, games.path().join("Game"), "the program's folder");
    let save = nexus_testkit::native(games.path(), "Game/save");
    assert_eq!(process.command.args, ["--windowed".to_owned(), save]);
}

#[tokio::test]
async fn a_folder_program_without_its_folder_rule_is_not_spawned() {
    let games = tempfile::tempdir().unwrap();
    let spec = LaunchSpec {
        program: at("pc-games", 0, "Game/start.sh").into(),
        args: vec![],
        env: Default::default(),
        cwd: None,
    };
    let t = launcher_host(spec, "[\"system:wine\"]").await;
    map(&t, "pc-games", &[games.path()]);

    let err = t.core.dispatch(Command::Launch(game_key())).await.unwrap_err();

    assert!(matches!(err, CoreError::PermissionDenied { .. }), "{err}");
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_system_program_runs_by_name_only_with_its_rule_and_a_cwd() {
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir(games.path().join("Game")).unwrap();
    let wine = |cwd: Option<Location>| LaunchSpec {
        program: Location::System("wine".into()),
        args: vec![LaunchArg::path(at("pc-games", 0, "Game/game.exe"))],
        env: Default::default(),
        cwd,
    };
    let in_game = Some(at("pc-games", 0, "Game").into());

    let t = launcher_host(wine(in_game.clone()), "[\"system:wine\"]").await;
    map(&t, "pc-games", &[games.path()]);
    t.core.dispatch(Command::Launch(game_key())).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    assert_eq!(process.command.program, Path::new("wine"), "the spawner looks it up on PATH");
    assert_eq!(process.command.cwd, games.path().join("Game"));
    assert_eq!(process.command.args, [nexus_testkit::native(games.path(), "Game/game.exe")]);

    let t = launcher_host(wine(in_game), "[\"system:umu-run\"]").await;
    let err = t.core.dispatch(Command::Launch(game_key())).await.unwrap_err();
    assert!(matches!(err, CoreError::PermissionDenied { .. }), "{err}");

    let t = launcher_host(wine(None), "[\"system:wine\"]").await;
    let err = t.core.dispatch(Command::Launch(game_key())).await.unwrap_err();
    assert!(matches!(&err, CoreError::Invalid(m) if m.contains("cwd")), "{err}");
}

#[tokio::test]
async fn the_player_maps_a_folder_and_settings_and_the_plugin_page_show_who_asked_for_it() {
    let home = tempfile::tempdir().unwrap();
    let (t, folders) = boot(FakeHost::new().home(home.path())).await;
    let games = tempfile::tempdir().unwrap();

    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] }).await.unwrap();

    assert_eq!(folders.roots("games").await.unwrap()[0].path, games.path().display().to_string());
    let view = t.core.view().settings.folders;
    let names: Vec<_> = view.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["games"], "Settings has the game folders");
    assert_eq!(view[0].paths, [games.path().display().to_string()]);
    assert_eq!(view[0].requested_by, ["R"]);
    assert!(!view[0].write);
    // The plugin's own folders are on its page.
    t.core.dispatch(Command::Navigate(nexus_core::Route::Plugin(PluginId::new("reader").unwrap()))).await.unwrap();
    let page = t.core.view().plugin_page.expect("the reader's page").folders;
    let names: Vec<_> = page.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["saves"]);
    assert_eq!(page[0].default, home.path().join("Saves").display().to_string());
    assert!(page[0].write);

    let err = t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec!["relative".into()] }).await;
    assert!(matches!(err, Err(CoreError::Invalid(_))), "{err:?}");
    // A path from the home folder, as a shell writes it.
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec!["~/Games".into()] }).await.unwrap();
    assert_eq!(folders.roots("games").await.unwrap()[0].path, home.path().join("Games").display().to_string());
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec!["~".into()] }).await.unwrap();
    assert_eq!(folders.roots("games").await.unwrap()[0].path, home.path().display().to_string());
    let err = t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec!["~other/Games".into()] }).await;
    assert!(matches!(err, Err(CoreError::Invalid(_))), "another user's home isn't read: {err:?}");
    t.core.dispatch(Command::SetFolders { name: "saves".into(), paths: vec![] }).await.unwrap();
    assert_eq!(folders.roots("saves").await.unwrap()[0].path, home.path().join("Saves").display().to_string());
}

#[cfg(unix)]
#[tokio::test]
async fn marking_a_program_executable_needs_a_write_grant() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let (t, folders) = boot(FakeHost::new().home(home.path())).await;
    let games = tempfile::tempdir().unwrap();
    std::fs::write(games.path().join("game.x86_64"), b"\x7fELF").unwrap();
    map(&t, "games", &[games.path()]);
    std::fs::create_dir_all(home.path().join("Saves")).unwrap();
    std::fs::write(home.path().join("Saves/tool"), b"#!/bin/sh\n").unwrap();

    let err = folders.set_executable(&at("games", 0, "game.x86_64")).await.unwrap_err();
    assert!(matches!(err, PluginError::PermissionDenied(_)), "{err:?}");
    folders.set_executable(&at("saves", 0, "tool")).await.unwrap();
    let mode = std::fs::metadata(home.path().join("Saves/tool")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[tokio::test]
async fn a_game_s_own_image_comes_from_its_folder_with_the_plugin_s_grant() {
    let (t, _) = boot(FakeHost::new()).await;
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("G/sce_sys")).unwrap();
    std::fs::write(games.path().join("G/sce_sys/icon0.png"), b"png bytes").unwrap();
    map(&t, "games", &[games.path()]);
    map(&t, "elsewhere", &[games.path()]);
    let image = |folder: &str| nexus_ui_api::ImageRef::Folder {
        plugin: PluginId::new("reader").unwrap(),
        path: at(folder, 0, "G/sce_sys/icon0.png"),
    };

    assert_eq!(t.core.image_bytes(&image("games")).await.unwrap(), b"png bytes");
    let err = t.core.image_bytes(&image("elsewhere")).await.unwrap_err();
    assert!(matches!(err, CoreError::Plugin { error: PluginError::PermissionDenied(_), .. }), "{err}");
}

#[tokio::test]
async fn an_environment_value_can_be_a_path_the_host_resolves() {
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir(games.path().join("Game")).unwrap();
    let spec = LaunchSpec {
        program: Location::System("wine".into()),
        args: vec![],
        env: [
            ("WINEPREFIX".to_owned(), LaunchArg::path(RelPath::new("prefixes/game").unwrap())),
            ("GAMEID".to_owned(), "umu-default".into()),
        ]
        .into(),
        cwd: Some(at("pc-games", 0, "Game").into()),
    };
    let t = launcher_host(spec, "[\"system:wine\"]").await;
    map(&t, "pc-games", &[games.path()]);

    t.core.dispatch(Command::Launch(game_key())).await.unwrap();

    let process = t.spawner.wait_for(1).await;
    let prefix = nexus_testkit::native(t.data_dir(), "plugins/pc/prefixes/game");
    assert_eq!(process.command.env.get("WINEPREFIX"), Some(&prefix));
    assert_eq!(process.command.env.get("GAMEID").map(String::as_str), Some("umu-default"));
}

#[tokio::test]
async fn a_big_file_is_written_in_pieces_then_renamed_into_place_and_only_with_a_write_grant() {
    let (t, folders) = boot(FakeHost::new()).await;
    let saves = tempfile::tempdir().unwrap();
    let games = tempfile::tempdir().unwrap();
    map(&t, "saves", &[saves.path()]);
    map(&t, "games", &[games.path()]);

    // Pieces arrive in any order; a gap fills with zeros until its piece comes.
    let part = at("saves", 0, "Game.part/eboot.bin");
    folders.write_at(&part, 4, b"4567").await.unwrap();
    folders.write_at(&part, 0, b"0123").await.unwrap();
    folders.write_at(&part, 8, b"89").await.unwrap();
    folders.rename(&at("saves", 0, "Game.part"), &at("saves", 0, "Game")).await.unwrap();
    assert_eq!(std::fs::read(saves.path().join("Game/eboot.bin")).unwrap(), b"0123456789");
    assert!(!saves.path().join("Game.part").exists());

    let denied = |r: PluginResult<()>| matches!(r, Err(PluginError::PermissionDenied(_)));
    assert!(denied(folders.write_at(&at("games", 0, "x.bin"), 0, b"x").await), "games is read-only");
    std::fs::write(games.path().join("a"), b"a").unwrap();
    assert!(denied(folders.rename(&at("games", 0, "a"), &at("games", 0, "b")).await));
    assert!(
        matches!(
            folders.rename(&at("saves", 0, "Game"), &at("games", 0, "Game")).await,
            Err(PluginError::InvalidInput(_)) | Err(PluginError::PermissionDenied(_))
        ),
        "a rename stays inside one folder"
    );
}

#[tokio::test]
async fn the_games_folder_is_games_in_the_home_folder_until_the_player_maps_it() {
    let home = tempfile::tempdir().unwrap();
    let (t, folders) = boot(FakeHost::new().home(home.path())).await;
    assert_eq!(folders.roots("games").await.unwrap()[0].path, home.path().join("Games").display().to_string());
    let view = t.core.view().settings.folders;
    assert_eq!(view[0].default, home.path().join("Games").display().to_string());
}

#[tokio::test]
async fn the_packages_folder_is_downloads_until_the_player_maps_it_and_settings_show_it_once() {
    let home = tempfile::tempdir().unwrap();
    let asks = |id: &str| {
        StubPlugin::from_toml(&format!(
            "id = \"{id}\"\nkind = \"emulator\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
             [[permissions.filesystem]]\nfolder = \"packages\"\ntitle = \"Its own title\"\n"
        ))
    };
    let t = FakeHost::new().home(home.path()).plugin(asks("emu-a")).plugin(asks("emu-b")).boot_core().await;

    let view = t.core.view().settings.folders;
    let packages: Vec<_> = view.iter().filter(|f| f.name == "packages").collect();
    assert_eq!(packages.len(), 1, "one row for both plugins");
    assert_eq!(packages[0].default, home.path().join("Downloads").display().to_string());
    assert_eq!(packages[0].title, "Packages to install", "the host's title, not a plugin's");
    assert_eq!(packages[0].requested_by.len(), 2);
}

#[tokio::test]
async fn a_folder_can_have_another_default_on_one_system() {
    let home = tempfile::tempdir().unwrap();
    let manifest = "id = \"data\"\nkind = \"emulator\"\nname = \"D\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
        [[permissions.filesystem]]\nfolder = \"emu-data\"\ndefault = \"~/.local/share/Emu\"\n\
        [permissions.filesystem.default_on]\nmacos = \"~/Library/Application Support/Emu\"\n";
    let folders: Arc<Mutex<Option<Arc<dyn Folders>>>> = Arc::default();
    let slot = folders.clone();
    let plugin = StubPlugin::from_toml(manifest).provides(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.folders().clone());
        Ok(Provided::default())
    });
    let t = FakeHost::new().home(home.path()).platform(Some(Platform::Macos)).plugin(plugin).boot_core().await;
    t.core.kernel().provided(&PluginId::new("data").unwrap()).await.unwrap();
    let folders = folders.lock().unwrap().clone().unwrap();
    let mac = home.path().join("Library/Application Support/Emu").display().to_string();
    assert_eq!(folders.roots("emu-data").await.unwrap()[0].path, mac);
}

#[tokio::test]
async fn settings_show_a_folder_in_the_home_folder_with_a_tilde() {
    let home = tempfile::tempdir().unwrap();
    let (t, _folders) = boot(FakeHost::new().home(home.path())).await;
    let big = full_path("/Volumes/Big/Games");
    map(&t, "games", &[&home.path().join("Games"), Path::new(&big)]);

    let games = t.core.view().settings.folders.into_iter().find(|f| f.name == "games").unwrap();
    assert_eq!(games.shown, format!("{}; {big}", native("~/Games")));
    assert!(games.text.starts_with(home.path().to_str().unwrap()), "the text to edit keeps the full path");
}
