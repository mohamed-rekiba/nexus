//! Seam 1, core app API: a finished download's archives are unpacked through the host's
//! archives port (a fake) before the emulator's game installer takes the download.

use std::sync::{Arc, Mutex};

use nexus_api::*;
use nexus_core::{ArchiveKind, Area, Command, TorrentAction};
use nexus_testkit::{FakeHost, StubPlugin, TestCore};

use crate::common::catalogs_fetched;

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
const URL: &str = "https://xyz.dev/c/ps5.json";

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

/// An emulator with no games of its own, whose game installer says which source it got.
fn emulator(sources: Arc<Mutex<Vec<HostPath>>>) -> StubPlugin {
    struct Emu(Arc<Mutex<Vec<HostPath>>>);
    #[async_trait]
    impl Catalog for Emu {
        async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
            Ok(Page { items: vec![], total: None })
        }
        async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
            Ok(vec![])
        }
        async fn details(&self, game: &str) -> PluginResult<GameDetails> {
            Err(PluginError::NotFound(game.into()))
        }
        async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
            Ok(None)
        }
    }
    #[async_trait]
    impl GameInstaller for Emu {
        async fn candidates(&self) -> PluginResult<Vec<Installable>> {
            Ok(vec![])
        }
        async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
            self.0.lock().unwrap().push(source.clone());
            job.finished(Ok(InstalledGame { dest: source.clone(), title: "Astro".into() }));
            Ok(HostPath::new("games", 0, RelPath::new("PS5/astro.part").unwrap()))
        }
    }
    StubPlugin::from_toml(
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"PS5\"\nemulator = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         console = \"PS5\"\n\
         capabilities = [\"catalog\", \"game_installer\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n",
    )
    .provides(move |_| {
        let emu = Arc::new(Emu(sources.clone()));
        Ok(Provided { catalog: Some(emu.clone()), game_installer: Some(emu), ..Provided::default() })
    })
}

struct Setup {
    t: TestCore,
    home: tempfile::TempDir,
    sources: Arc<Mutex<Vec<HostPath>>>,
}

/// The catalog offers one release, whose torrent holds `files`.
async fn setup(files: &[(&str, &[u8])]) -> Setup {
    let sources = Arc::new(Mutex::new(Vec::new()));
    setup_with(|h| h.plugin(emulator(sources.clone())), files, sources.clone()).await
}

/// [`setup`] with the PS5 emulator `add` puts in.
async fn setup_with(
    add: impl FnOnce(FakeHost) -> FakeHost,
    files: &[(&str, &[u8])],
    sources: Arc<Mutex<Vec<HostPath>>>,
) -> Setup {
    let home = tempfile::tempdir().unwrap();
    let t = add(FakeHost::new().home(home.path()))
        .built_in(PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }] });
    t.http.serve(URL, catalog.to_string());
    t.torrents.has(HASH, "Astro", files);
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    Setup { t, home, sources }
}

impl Setup {
    fn folder(&self) -> std::path::PathBuf {
        self.home.path().join(format!("Downloads/PS5/torrent-{HASH}"))
    }

    /// Downloads the release to the end.
    async fn finish(&self) {
        let offer: GameKey = "release-catalog:emu-kyty/PPSA01234".parse().unwrap();
        self.t
            .core
            .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download { offer, release: HASH.into(), folder: None }))
            .await
            .unwrap();
        until(&self.t.core, Area::Torrents, |r| r.state == "ready").await;
        act(&self.t.core, TorrentAction::Start).await;
        self.t.torrents.complete(HASH);
        until(&self.t.core, Area::Torrents, |r| r.state == "complete").await;
    }
}

fn row(core: &nexus_core::Core) -> nexus_ui_api::TorrentVm {
    core.view().downloads.torrents.into_iter().next().expect("a download")
}

/// Waits until the download's row passes `test`, looking after each change in `area`.
async fn until(
    core: &nexus_core::Core,
    area: Area,
    test: impl Fn(&nexus_ui_api::TorrentVm) -> bool,
) -> nexus_ui_api::TorrentVm {
    nexus_testkit::until(core, area, |core| core.view().downloads.torrents.first().is_some_and(&test)).await;
    row(core)
}

async fn act(core: &nexus_core::Core, action: TorrentAction) {
    core.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act { id: HASH.into(), action })).await.unwrap();
}

#[tokio::test]
async fn a_finished_download_unpacks_its_archive_and_install_takes_the_unpacked_files() {
    let s =
        setup(&[("Astro/game.part1.rar", b"one"), ("Astro/game.part2.rar", b"two"), ("Astro/readme.txt", b"hi")]).await;
    s.t.archives.holds("game.part1.rar", &[("Astro-app0/eboot.bin", b"elf")]);
    s.finish().await;

    let done = until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    assert_eq!(s.t.archives.unpacked(), [(ArchiveKind::Rar, vec!["game.part1.rar".into(), "game.part2.rar".into()])]);
    let file = s.folder().join("unpacked/Astro/game/Astro-app0/eboot.bin");
    assert_eq!(std::fs::read(file).unwrap(), b"elf");
    assert_eq!(done.actions, ["install", "stop-seeding", "remove-unpacked"]);
    assert_eq!(done.unpack_progress, 1.0);

    act(&s.t.core, TorrentAction::Install).await;
    until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    let sources = s.sources.lock().unwrap().clone();
    assert_eq!(sources, [HostPath::new("packages", 0, RelPath::new(format!("PS5/torrent-{HASH}")).unwrap())]);
}

#[tokio::test]
async fn several_archives_each_unpack_into_their_own_folder() {
    let s = setup(&[("Astro/game.rar", b"g"), ("Astro/update.rar", b"u"), ("dlc/game.7z", b"d")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    s.t.archives.holds("update.rar", &[("patch.pkg", b"update")]);
    s.t.archives.holds("game.7z", &[("addon.pkg", b"dlc")]);
    s.finish().await;

    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    let unpacked = s.folder().join("unpacked");
    assert_eq!(std::fs::read(unpacked.join("Astro/game/game.pkg")).unwrap(), b"game");
    assert_eq!(std::fs::read(unpacked.join("Astro/update/patch.pkg")).unwrap(), b"update");
    assert_eq!(std::fs::read(unpacked.join("dlc/game/addon.pkg")).unwrap(), b"dlc");
}

#[tokio::test]
async fn a_set_with_a_missing_part_fails_before_anything_is_unpacked() {
    let s = setup(&[("Astro/game.part1.rar", b"one"), ("Astro/game.part3.rar", b"three")]).await;
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("part 2 of Astro/game is missing"), "{}", failed.error);
    assert!(s.t.archives.unpacked().is_empty(), "nothing unpacked");
    assert!(!s.folder().join("unpacked").exists());
}

#[tokio::test]
async fn an_unpack_that_fails_says_why_and_leaves_no_half_folder() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.fails("game.rar", "This archive needs a password");
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("needs a password"), "{}", failed.error);
    assert_eq!(failed.actions, ["install", "stop-seeding"], "Install tries again");
    let left: Vec<_> = walk(&s.folder().join("unpacked"));
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_failed_unpack_removes_the_sets_it_already_unpacked() {
    let s = setup(&[("Astro/a.rar", b"a"), ("Astro/b.rar", b"b")]).await;
    s.t.archives.holds("a.rar", &[("a.pkg", b"a")]);
    s.t.archives.fails("b.rar", "b is damaged");
    s.finish().await;

    let failed = until(&s.t.core, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(failed.error.contains("b is damaged"), "{}", failed.error);
    assert!(walk(&s.folder().join("unpacked")).is_empty(), "{:?}", walk(&s.folder().join("unpacked")));
}

#[tokio::test]
async fn cancel_stops_the_unpacking_and_install_starts_it_again() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    let held = s.t.archives.hold();
    s.finish().await;

    let unpacking = until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacking").await;
    assert_eq!(unpacking.actions, ["stop-seeding", "cancel"], "Cancel stops the unpacking");
    act(&s.t.core, TorrentAction::Cancel).await;
    let cancelled = until(&s.t.core, Area::Torrents, |r| r.unpack == "cancelled").await;
    assert_eq!(cancelled.state, "complete", "the download stays");
    assert!(walk(&s.folder().join("unpacked")).is_empty());

    held.release();
    act(&s.t.core, TorrentAction::Install).await;
    until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    assert_eq!(std::fs::read(s.folder().join("unpacked/Astro/game/game.pkg")).unwrap(), b"game");
    assert_eq!(s.sources.lock().unwrap().len(), 1, "installed once unpacked");
}

#[tokio::test]
async fn an_unpack_cut_by_a_restart_comes_back_failed_and_install_unpacks_first() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    let _held = s.t.archives.hold();
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacking").await;
    // The row says it unpacks before the unpacker is asked: let it get there.
    s.t.clock.run_until_idle().await;
    assert_eq!(s.t.archives.unpacked().len(), 1, "the first app's unpack runs");

    let again = nexus_core::Core::start(s.t.kernel.clone()).await;
    let restored = until(&again, Area::Torrents, |r| r.unpack == "failed").await;
    assert!(restored.error.contains("stopped when Nexus closed"), "{}", restored.error);
    // The download checks its files first, and seeds once they check out.
    until(&again, Area::Torrents, |r| r.seeding).await;
    // The first app closed: its unpack never ends, so only the new one writes the folder.
    s.t.archives.cut();
    act(&again, TorrentAction::Install).await;
    until(&again, Area::Installs, |r| r.install == "done").await;
    assert_eq!(std::fs::read(s.folder().join("unpacked/Astro/game/game.pkg")).unwrap(), b"game");
}

#[tokio::test]
async fn remove_unpacked_files_keeps_the_download_and_its_seeding() {
    let s = setup(&[("Astro/game.rar", b"g")]).await;
    s.t.archives.holds("game.rar", &[("game.pkg", b"game")]);
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;

    act(&s.t.core, TorrentAction::RemoveUnpacked).await;
    let after = row(&s.t.core);
    assert_eq!(after.unpack, "waiting");
    assert!(!s.folder().join("unpacked").exists());
    assert_eq!(std::fs::read(s.folder().join("Astro/game.rar")).unwrap(), b"g", "the torrent's files stay");
    assert!(after.seeding);
}

#[tokio::test]
async fn a_download_with_no_archives_installs_as_before() {
    let s = setup(&[("Astro/astro.pkg", b"package")]).await;
    s.finish().await;
    let done = row(&s.t.core);
    assert_eq!(done.unpack, "");
    assert_eq!(done.actions, ["install", "stop-seeding"]);
}

/// Every file under `dir`.
fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

// Unix only: "moved, not copied" compares inode numbers.
#[cfg(unix)]
#[tokio::test]
async fn installing_the_unpacked_game_moves_it_into_the_games_folder_and_the_archive_stays() {
    use std::os::unix::fs::MetadataExt;
    let s = setup_with(
        |h| h.built_in(PluginByKind::emulator(emu_kyty::KytyPlugin)),
        &[("Astro/game.7z", b"seven")],
        Arc::default(),
    )
    .await;
    let param = br#"{"titleId":"PPSA01234","localizedParameters":{"en-US":{"titleName":"Astro"}}}"#;
    s.t.archives
        .holds("game.7z", &[("PPSA01234-app0/sce_sys/param.json", param), ("PPSA01234-app0/eboot.bin", b"elf")]);
    s.finish().await;
    until(&s.t.core, Area::Torrents, |r| r.unpack == "unpacked").await;
    let unpacked = s.folder().join("unpacked/Astro/game/PPSA01234-app0");
    let inode = std::fs::metadata(unpacked.join("eboot.bin")).unwrap().ino();

    act(&s.t.core, TorrentAction::Install).await;
    let done = until(&s.t.core, Area::Installs, |r| r.install == "done").await;
    let installed = s.home.path().join("Games/PS5/PPSA01234-app0/eboot.bin");
    assert_eq!(std::fs::read(&installed).unwrap(), b"elf");
    assert_eq!(std::fs::metadata(&installed).unwrap().ino(), inode, "moved, not copied");
    assert!(!unpacked.exists(), "the unpacked copy moved");
    assert_eq!(std::fs::read(s.folder().join("Astro/game.7z")).unwrap(), b"seven", "the download stays, for sharing");
    assert!(!done.actions.contains(&"remove-unpacked".to_owned()), "nothing unpacked is left: {:?}", done.actions);
}
