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
    assert_eq!(b.config(&["app", "playtime"]), serde_json::json!({ "emu-b:G0": { "secs": 60 } }));
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
