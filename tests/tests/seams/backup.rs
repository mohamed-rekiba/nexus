//! Settings → Backup: a backup made on one computer, restored on another whose folders are
//! elsewhere, and what a restore keeps of the computer it runs on.

use std::path::{Path, PathBuf};

use nexus_api::*;
use nexus_core::{BackupCmd, Command, Pref};
use nexus_testkit::{FakeBackup, FakeHost, StubPlugin, TestCore};

const PLUGIN: &str = "emu-b";
const PERMISSIONS: &str = "[[permissions.filesystem]]\nfolder = \"emu-saves\"\nwrite = true\n\n\
                           [[permissions.filesystem]]\nfolder = \"emu-keys\"\n";

/// The plugin's items: its settings file in its data folder, its saves in a host folder (without
/// their cache), its keys in a folder it may only read, and large Wine prefixes.
fn items() -> Vec<BackupItem> {
    vec![
        BackupItem::new("settings", "Settings", BackupPlace::Data(RelPath::new("settings.ini").unwrap())),
        BackupItem::new("saves", "Saves", BackupPlace::Folder(HostPath::new("emu-saves", 0, RelPath::root())))
            .skip(&["cache"]),
        BackupItem::new("keys", "Keys", BackupPlace::Folder(HostPath::new("emu-keys", 0, RelPath::root()))),
        BackupItem::new("prefixes", "Wine prefixes", BackupPlace::Data(RelPath::new("prefixes").unwrap())).large(),
    ]
}

/// One computer: the plugin, its saves and keys folders, and the shared backup folder.
struct Pc {
    test: TestCore,
    saves: tempfile::TempDir,
    keys: tempfile::TempDir,
}

impl Pc {
    async fn new(backups: &Path) -> Pc {
        let plugin = StubPlugin::backs_up(PLUGIN, PERMISSIONS, FakeBackup::new(items()));
        let test = FakeHost::new().built_in(plugin.boxed()).boot_core().await;
        let (saves, keys) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for (name, dir) in [("emu-saves", &saves), ("emu-keys", &keys)] {
            let folders = Command::SetFolders { name: name.into(), paths: vec![dir.path().into()] };
            test.core.dispatch(folders).await.unwrap();
        }
        let folder = serde_json::json!(backups.display().to_string());
        test.core.dispatch(Command::Pref(Pref::BackupFolder, folder)).await.unwrap();
        Pc { test, saves, keys }
    }

    fn data(&self) -> PathBuf {
        self.test.data_dir().join("plugins").join(PLUGIN)
    }

    fn config(&self, keys: &[&str]) -> serde_json::Value {
        self.test.core.kernel().config().get(keys)
    }

    async fn run(&self, command: BackupCmd) -> Result<(), nexus_core::CoreError> {
        self.test.core.dispatch(Command::Backup(command)).await
    }
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn a_backup_from_one_computer_restores_into_another_s_folders_and_keeps_its_folder_settings() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    write(&a.data().join("settings.ini"), b"fullscreen=1");
    write(&a.saves.path().join("GAME/slot.dat"), b"progress");
    write(&a.saves.path().join("cache/shader.bin"), b"cache");
    write(&a.keys.path().join("prod.keys"), b"keys");
    write(&a.data().join("prefixes/G0/drive_c/save"), b"pc save");
    a.test.core.kernel().config().set(&["app", "playtime"], serde_json::json!({ "emu-b:G0": { "secs": 60 } })).unwrap();

    a.run(BackupCmd::BackUp { large: false }).await.unwrap();
    let made = files(backups.path());
    assert_eq!(made.len(), 1, "{made:?}");

    let b = Pc::new(backups.path()).await;
    write(&b.saves.path().join("OTHER/slot.dat"), b"b's own");
    let b_folders = b.config(&["app", "folders"]);
    b.run(BackupCmd::Restore(made[0].clone())).await.unwrap();

    assert_eq!(std::fs::read(b.saves.path().join("GAME/slot.dat")).unwrap(), b"progress", "in b's saves folder");
    assert!(!b.saves.path().join("cache").exists(), "the cache stayed out");
    assert_eq!(std::fs::read(b.saves.path().join("OTHER/slot.dat")).unwrap(), b"b's own", "b's other saves stay");
    assert_eq!(std::fs::read(b.data().join("settings.ini")).unwrap(), b"fullscreen=1");
    assert!(!b.keys.path().join("prod.keys").exists(), "a folder the plugin may only read isn't written");
    assert!(!b.data().join("prefixes").exists(), "large items are in a backup only when asked for");
    assert_eq!(b.config(&["app", "folders"]), b_folders, "b keeps its own folders");
    assert_eq!(b.config(&["app", "playtime"]), serde_json::Value::Null, "the settings wait for the next start");
    let pending: serde_json::Value =
        serde_json::from_slice(&std::fs::read(b.test.data_dir().join("restored-settings.json")).unwrap()).unwrap();
    assert_eq!(pending["app"]["playtime"], serde_json::json!({ "emu-b:G0": { "secs": 60 } }));
    assert!(pending["app"].get("folders").is_none(), "with no folder of a's");
    let now = files(backups.path());
    assert!(now.iter().any(|f| f.contains("-before-restore")), "what b had is backed up first: {now:?}");
}

#[tokio::test]
async fn large_items_go_in_when_asked_for() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    write(&a.data().join("prefixes/G0/drive_c/save"), b"pc save");
    a.run(BackupCmd::BackUp { large: true }).await.unwrap();

    let b = Pc::new(backups.path()).await;
    b.run(BackupCmd::Restore(files(backups.path())[0].clone())).await.unwrap();
    assert_eq!(std::fs::read(b.data().join("prefixes/G0/drive_c/save")).unwrap(), b"pc save");
}

#[tokio::test]
async fn a_restore_takes_only_a_backup_file_of_the_backup_folder() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    assert!(a.run(BackupCmd::Restore("../config.json".into())).await.is_err());
    assert!(a.run(BackupCmd::Restore("nexus-backup-missing.tar.gz".into())).await.is_err());
    assert!(files(backups.path()).iter().all(|f| f.contains("-before-restore")), "nothing else written");
}

/// A computer with the real Lichii, shadPS4, KytyPS5 and PC plugins, each of their folders in
/// a temporary folder of its own.
async fn emulators(backups: &Path) -> (TestCore, Vec<(&'static str, tempfile::TempDir)>) {
    let test = FakeHost::new()
        .platform(Some(Platform::Linux))
        .built_in(PluginBox::code(emu_lichii::LichiiPlugin))
        .built_in(PluginBox::code(emu_shadps4::ShadPlugin))
        .built_in(PluginBox::code(emu_kyty::KytyPlugin))
        .built_in(PluginBox::code(emu_pc::PcPlugin))
        .boot_core()
        .await;
    let mut folders = Vec::new();
    for name in ["eden-data", "eden-config", "shadps4-data", "kyty-data"] {
        let dir = tempfile::tempdir().unwrap();
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![dir.path().into()] }).await.unwrap();
        folders.push((name, dir));
    }
    let folder = serde_json::json!(backups.display().to_string());
    test.core.dispatch(Command::Pref(Pref::BackupFolder, folder)).await.unwrap();
    (test, folders)
}

fn at<'a>(folders: &'a [(&str, tempfile::TempDir)], name: &str) -> &'a Path {
    folders.iter().find(|(n, _)| *n == name).unwrap().1.path()
}

#[tokio::test]
async fn the_emulators_keys_firmware_settings_and_saves_go_to_another_computer() {
    let backups = tempfile::tempdir().unwrap();
    let (a, a_folders) = emulators(backups.path()).await;
    write(&at(&a_folders, "eden-data").join("keys/prod.keys"), b"keys");
    write(&at(&a_folders, "eden-data").join("nand/system/Contents/registered/fw.nca"), b"firmware");
    write(&at(&a_folders, "eden-data").join("log/eden_log.txt"), b"not an item");
    write(&at(&a_folders, "eden-config").join("qt-config.ini"), b"[UI]");
    write(&at(&a_folders, "shadps4-data").join("config.json"), b"{}");
    write(&at(&a_folders, "shadps4-data").join("savedata/1/CUSA1/SAVE.DAT"), b"ps4 save");
    write(&at(&a_folders, "shadps4-data").join("shader/big.bin"), b"cache");
    write(&at(&a_folders, "kyty-data").join("_SaveData/PPSA1/slot/save.dat"), b"ps5 save");
    a.core.dispatch(Command::Backup(BackupCmd::BackUp { large: false })).await.unwrap();
    let file = files(backups.path()).remove(0);

    let (b, b_folders) = emulators(backups.path()).await;
    b.core.dispatch(Command::Backup(BackupCmd::Restore(file))).await.unwrap();
    let read = |folder: &str, path: &str| std::fs::read(at(&b_folders, folder).join(path)).ok();
    assert_eq!(read("eden-data", "keys/prod.keys").as_deref(), Some(&b"keys"[..]));
    assert_eq!(read("eden-data", "nand/system/Contents/registered/fw.nca").as_deref(), Some(&b"firmware"[..]));
    assert_eq!(read("eden-data", "log/eden_log.txt"), None, "Eden's log isn't an item");
    assert_eq!(read("eden-config", "qt-config.ini").as_deref(), Some(&b"[UI]"[..]));
    assert_eq!(read("shadps4-data", "savedata/1/CUSA1/SAVE.DAT").as_deref(), Some(&b"ps4 save"[..]));
    assert_eq!(read("shadps4-data", "config.json").as_deref(), Some(&b"{}"[..]));
    assert_eq!(read("shadps4-data", "shader/big.bin"), None, "shadPS4's caches stay out");
    assert_eq!(read("kyty-data", "_SaveData/PPSA1/slot/save.dat").as_deref(), Some(&b"ps5 save"[..]));
}

#[cfg(unix)]
#[tokio::test]
async fn nothing_is_restored_when_the_backup_of_what_is_here_fails() {
    use std::os::unix::fs::PermissionsExt;
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    write(&a.saves.path().join("GAME/slot.dat"), b"a's");
    a.run(BackupCmd::BackUp { large: false }).await.unwrap();
    let file = files(backups.path()).remove(0);

    let b = Pc::new(backups.path()).await;
    write(&b.saves.path().join("GAME/slot.dat"), b"b's");
    // The backup folder takes no new file: the backup of b's saves can't be written.
    std::fs::set_permissions(backups.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let restored = b.run(BackupCmd::Restore(file)).await;
    std::fs::set_permissions(backups.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = restored.unwrap_err().to_string();
    assert!(error.contains("Nothing was restored"), "{error}");
    assert_eq!(std::fs::read(b.saves.path().join("GAME/slot.dat")).unwrap(), b"b's", "b's save stays");
}

#[tokio::test]
async fn a_second_job_waits_for_the_first() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    write(&a.saves.path().join("GAME/slot.dat"), b"progress");
    let (one, two) = tokio::join!(a.run(BackupCmd::BackUp { large: false }), a.run(BackupCmd::BackUp { large: false }));
    assert!(one.is_ok() != two.is_ok(), "one runs and one is refused: {one:?} {two:?}");
}

fn view(pc: &Pc) -> nexus_ui_api::BackupVm {
    pc.test.core.view().settings.backup
}

#[tokio::test]
async fn a_backup_in_the_list_can_be_deleted() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    a.run(BackupCmd::BackUp { large: false }).await.unwrap();
    let file = view(&a).backups[0].file.clone();
    a.run(BackupCmd::Delete(file.clone())).await.unwrap();
    assert!(files(backups.path()).is_empty());
    assert!(view(&a).backups.is_empty());
    assert!(a.run(BackupCmd::Delete("../settings.ini".into())).await.is_err(), "only a backup of the backup folder");
}

#[tokio::test]
async fn the_browser_shows_folders_and_backups_and_restores_one_from_another_folder() {
    let (backups, elsewhere) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = Pc::new(elsewhere.path()).await;
    write(&a.saves.path().join("GAME/slot.dat"), b"from the stick");
    a.run(BackupCmd::BackUp { large: false }).await.unwrap();
    std::fs::create_dir_all(elsewhere.path().join("old-pc")).unwrap();
    std::fs::create_dir_all(elsewhere.path().join(".hidden")).unwrap();
    std::fs::write(elsewhere.path().join("notes.txt"), b"not a backup").unwrap();

    let b = Pc::new(backups.path()).await;
    b.run(BackupCmd::Browse(Some(elsewhere.path().into()))).await.unwrap();
    let browser = view(&b).browser;
    assert!(browser.open);
    let names: Vec<(&str, bool)> = browser.entries.iter().map(|e| (e.name.as_str(), e.folder)).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(names[0], ("old-pc", true), "folders first, no hidden ones");
    assert!(!names[1].1 && names[1].0.starts_with("nexus-backup-"), "then backups, no other files: {names:?}");
    assert_eq!(browser.parent, elsewhere.path().parent().unwrap().display().to_string());
    assert!(browser.places.iter().any(|p| p.name == "Backup folder"), "{:?}", browser.places);

    let path = std::path::PathBuf::from(&browser.entries[1].path);
    b.run(BackupCmd::RestoreFile(path)).await.unwrap();
    assert_eq!(std::fs::read(b.saves.path().join("GAME/slot.dat")).unwrap(), b"from the stick");
    assert!(!view(&b).browser.open, "a restore closes the browser");
    assert!(
        files(backups.path()).iter().any(|f| f.contains("-before-restore")),
        "the backup of what was here is in b's backup folder"
    );
    assert!(b.run(BackupCmd::RestoreFile(elsewhere.path().join("notes.txt"))).await.is_err());
}

#[tokio::test]
async fn the_tab_says_what_a_backup_holds_here_and_which_items_are_large() {
    let backups = tempfile::tempdir().unwrap();
    let a = Pc::new(backups.path()).await;
    a.run(BackupCmd::Refresh).await.unwrap();
    let tab = view(&a);
    assert_eq!(tab.holds, "Nexus settings, Settings, Saves, Keys");
    assert_eq!(tab.large, "Wine prefixes");

    // Turned off, the plugin's items aren't in a backup, nor in the words.
    a.test
        .core
        .dispatch(Command::SetPluginEnabled { plugin: PluginId::new(PLUGIN).unwrap(), enabled: false })
        .await
        .unwrap();
    a.run(BackupCmd::Refresh).await.unwrap();
    let tab = view(&a);
    assert_eq!(tab.holds, "Nexus settings");
    assert_eq!(tab.large, "", "no large items: no switch");
}
