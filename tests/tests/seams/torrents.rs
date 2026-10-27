//! Seam 1, core app API: downloading a release a catalog offers, through the host's torrent
//! port (a fake swarm), and handing the finished download to the emulator's game installer.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_api::*;
use nexus_core::{Area, Command, GameInstallCmd, TorrentAction};
use nexus_testkit::{FakeHost, StubPlugin, TestCore, until as until_in};

use crate::common::{catalogs_fetched, native};

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
const URL: &str = "https://xyz.dev/c/ps5.json";
const PACKAGE: &[u8] = b"a game package";

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
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         console = \"PS5\"\n\
         install_note = \"Installs from a folder.\"\n\
         capabilities = [\"catalog\", \"game_installer\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n\
         [[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n",
    )
    .provides(move |_| {
        let emu = Arc::new(Emu(sources.clone()));
        Ok(Provided { catalog: Some(emu.clone()), game_installer: Some(emu), ..Provided::default() })
    })
}

fn catalog(magnet: &str) -> String {
    serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB", "magnet": magnet,
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }] })
    .to_string()
}

struct Setup {
    t: TestCore,
    home: tempfile::TempDir,
    sources: Arc<Mutex<Vec<HostPath>>>,
}

async fn setup() -> Setup {
    setup_with(HASH).await
}

/// The catalog offers one release, of torrent `hash`; the swarm has only `HASH`.
async fn setup_with(hash: &str) -> Setup {
    setup_on(hash, |h| h).await
}

/// [`setup_with`] on a host that `build` changes first.
async fn setup_on(hash: &str, build: impl FnOnce(FakeHost) -> FakeHost) -> Setup {
    let sources = Arc::new(Mutex::new(Vec::new()));
    setup_plugin(hash, emulator(sources.clone()), sources, build).await
}

/// [`setup_on`] with `plugin` as the emulator.
async fn setup_plugin(
    hash: &str,
    plugin: StubPlugin,
    sources: Arc<Mutex<Vec<HostPath>>>,
    build: impl FnOnce(FakeHost) -> FakeHost,
) -> Setup {
    let home = tempfile::tempdir().unwrap();
    let t = build(FakeHost::new())
        .home(home.path())
        .plugin(plugin)
        .built_in(PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    t.http.serve(URL, catalog(&format!("magnet:?xt=urn:btih:{hash}")));
    t.torrents.has(HASH, "Astro", &[("Astro/astro.pkg", PACKAGE)]);
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

fn offer() -> GameKey {
    "release-catalog:emu-kyty/PPSA01234".parse().unwrap()
}

fn row(t: &TestCore) -> nexus_ui_api::TorrentVm {
    t.core.view().downloads.torrents.into_iter().next().expect("a download")
}

/// Waits until the download's row says `state`.
async fn until(t: &TestCore, state: &str) -> nexus_ui_api::TorrentVm {
    t.until(Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(|r| r.state == state)).await;
    row(t)
}

async fn act(t: &TestCore, action: TorrentAction) {
    t.core.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act { id: HASH.into(), action })).await.unwrap();
}

/// Installs the download into the game folder the install sheet shows.
async fn install(t: &TestCore) {
    t.core.dispatch(Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: None })).await.unwrap();
}

#[tokio::test]
async fn a_release_lists_its_files_and_downloads_only_when_the_player_starts_it() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();

    let ready = until(t, "ready").await;
    assert_eq!(ready.files, ["Astro/astro.pkg · 0 MB"]);
    assert_eq!((ready.file_count, ready.total), (1, PACKAGE.len() as u64));
    assert_eq!(ready.actions, ["start", "cancel"]);
    assert!(t.torrents.started().is_empty(), "nothing downloads before Start");
    assert_eq!(ready.folder, native(&format!("~/Downloads/PS5/torrent-{HASH}")), "the home folder as ~");

    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    let downloading = until(t, "downloading").await;
    assert_eq!(downloading.actions, ["pause", "cancel"]);
    assert!(downloading.progress > 0.0 && downloading.peers > 0);

    t.torrents.complete(HASH);
    let complete = until(t, "complete").await;
    assert!(complete.seeding, "finished downloads seed while the setting is on");
    assert_eq!(complete.actions, ["install", "stop-seeding"]);
    let file = s.home.path().join(format!("Downloads/PS5/torrent-{HASH}/Astro/astro.pkg"));
    assert_eq!(std::fs::read(file).unwrap(), PACKAGE);
    assert!(t.core.view().notices.iter().any(|n| n.contains("Download complete: Astro")));
    let done = t.core.view().notice_cards.into_iter().find(|n| n.text.contains("Download complete")).unwrap();
    assert_eq!(done.kind, nexus_ui_api::NoticeKind::Success, "a finished download is good news");
    assert_eq!(done.opens, "Open Downloads");
}

#[tokio::test]
async fn a_download_says_what_its_emulator_installs_from_and_that_archives_unpack_first() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    let ready = until(t, "ready").await;
    assert_eq!(
        ready.install_note,
        "Installs from a folder. Nexus first unpacks the RAR, ZIP, 7z and TAR archives in the download, but not one \
         with a password."
    );
}

#[tokio::test]
async fn shutting_down_stops_the_torrents_and_a_download_stays_one_to_resume() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;

    t.core.shutdown().await;

    assert!(t.torrents.running().is_empty());
    assert_eq!(t.torrents.stopped_all(), 1, "their connections close too");
    assert_eq!(row(t).state, "downloading", "the next launch starts it again");
}

#[tokio::test]
async fn pause_cancel_and_remove_keep_the_files_and_stop_the_torrent() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;

    act(t, TorrentAction::Pause).await;
    assert_eq!(row(t).state, "paused");
    assert!(t.torrents.running().is_empty());
    act(t, TorrentAction::Start).await;
    act(t, TorrentAction::Cancel).await;
    assert_eq!(row(t).state, "cancelled");
    assert_eq!(row(t).actions, ["start", "remove"]);
    act(t, TorrentAction::Remove).await;
    assert!(t.core.view().downloads.torrents.is_empty());
    assert!(t.kernel.config().get(&["app", "torrents", HASH]).is_null(), "forgotten");
}

#[tokio::test]
async fn a_finished_download_installs_through_the_emulator_s_game_installer() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(t, "complete").await;

    install(t).await;
    let sources = s.sources.lock().unwrap().clone();
    assert_eq!(sources, [HostPath::new("packages", 0, RelPath::new(format!("PS5/torrent-{HASH}")).unwrap())]);
    t.until(Area::Installs, |_| row(t).install == "done").await;
    assert_eq!(row(t).install, "done");
}

#[tokio::test]
async fn sharing_turned_off_stops_seeding_and_a_restart_brings_downloads_back() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(t, "complete").await;

    t.core.dispatch(Command::Torrent(nexus_core::TorrentCmd::SeedDownloads(false))).await.unwrap();
    assert!(!row(t).seeding && !t.core.view().downloads.seeding);
    assert!(t.torrents.running().is_empty());

    // The next start: with sharing on, the finished download checks its files and seeds.
    t.core.dispatch(Command::Torrent(nexus_core::TorrentCmd::SeedDownloads(true))).await.unwrap();
    let again = nexus_core::Core::start(t.kernel.clone()).await;
    until_in(&again, Area::Torrents, |core| core.view().downloads.torrents.first().is_some_and(|r| r.seeding)).await;
    let restored = again.view().downloads.torrents.into_iter().next().unwrap();
    assert!(restored.seeding, "seeding again after its check: {restored:?}");
    assert!(t.torrents.started().last().unwrap().1, "it starts paused, to check its files without downloading");
}

#[tokio::test]
async fn a_download_cut_by_a_restart_comes_back_paused() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;

    let again = nexus_core::Core::start(t.kernel.clone()).await;
    let restored = again.view().downloads.torrents.into_iter().next().unwrap();
    assert_eq!(restored.state, "paused", "not started by itself");
    assert_eq!(restored.actions, ["start", "cancel"]);
}

#[tokio::test]
async fn a_torrent_no_peer_has_fails_with_why() {
    let other = "aa".repeat(20);
    let s = setup_with(&other).await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: other.clone(),
            folder: None,
        }))
        .await
        .unwrap();
    let failed = until(t, "failed").await;
    assert!(failed.error.contains("no peers"), "{}", failed.error);
    assert_eq!(failed.actions, ["start", "cancel"]);
}

#[tokio::test]
async fn a_finished_download_whose_files_moved_does_not_seed_after_a_restart() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(t, "complete").await;
    std::fs::remove_dir_all(s.home.path().join(format!("Downloads/PS5/torrent-{HASH}"))).unwrap();

    let again = nexus_core::Core::start(t.kernel.clone()).await;
    let failed = |core: &nexus_core::Core| core.view().downloads.torrents.first().is_some_and(|r| !r.error.is_empty());
    until_in(&again, Area::Torrents, failed).await;
    let restored = again.view().downloads.torrents.into_iter().next().unwrap();
    assert!(!restored.seeding, "{restored:?}");
    assert!(restored.error.contains("moved, changed or deleted"), "{}", restored.error);
    assert_eq!(restored.state, "complete");
}

#[tokio::test]
async fn a_download_that_does_not_fit_in_the_free_space_does_not_start() {
    let s = setup_on(HASH, |h| h.free_space(4)).await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;

    act(t, TorrentAction::Start).await;
    let failed = until(t, "failed").await;
    assert_eq!(failed.error, "Not enough free space in the download folder: 0.0 GB needed, 0.0 GB free");
    assert!(t.torrents.started().is_empty(), "nothing downloads");
}

#[tokio::test]
async fn a_download_that_fits_in_the_free_space_starts() {
    let s = setup_on(HASH, |h| h.free_space(PACKAGE.len() as u64)).await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;

    act(t, TorrentAction::Start).await;
    assert_eq!(t.torrents.started().len(), 1);
}

#[tokio::test]
async fn a_download_shows_its_time_left_while_bytes_arrive() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    t.torrents.rate(HASH, 2);
    t.until(Area::Torrents, |_| row(t).seconds_left == 5).await;
    assert_eq!(row(t).seconds_left, 5, "10 bytes left at 2 bytes a second");

    t.torrents.rate(HASH, 0);
    t.until(Area::Torrents, |_| row(t).download_rate == 0).await;
    assert_eq!(row(t).seconds_left, 0, "no time left with no bytes arriving");
}

#[tokio::test]
async fn a_download_that_stops_with_an_error_sends_one_notice() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;

    t.torrents.fail(HASH, "the disk is full");
    let failed = until(t, "failed").await;
    assert_eq!(failed.error, "the disk is full");
    // A few more polls see the same state.
    t.clock.advance(Duration::from_millis(50)).await;
    let stopped: Vec<String> =
        t.core.view().notices.into_iter().filter(|n| n.starts_with("Download stopped")).collect();
    assert_eq!(stopped, ["Download stopped: Astro. Open Downloads to see why"]);
    let kinds: Vec<_> = t.core.view().notice_cards.iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&nexus_ui_api::NoticeKind::Error), "a stop is an error: {kinds:?}");
}

#[tokio::test]
async fn a_download_s_folder_opens_once_it_is_on_disk() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    assert_eq!(t.core.download_folder(HASH), None, "no folder before it starts");

    act(t, TorrentAction::Start).await;
    let folder = s.home.path().join(format!("Downloads/PS5/torrent-{HASH}"));
    assert_eq!(t.core.download_folder(HASH), Some(folder));
    assert_eq!(t.core.download_folder("no-such-download"), None);
}

#[tokio::test]
async fn a_download_goes_to_the_folder_the_player_picks_inside_a_packages_folder() {
    let s = setup().await;
    let t = &s.t;
    let home = s.home.path();
    let paths = vec![home.join("Downloads"), home.join("Other")];
    t.core.dispatch(Command::SetFolders { name: "packages".into(), paths }).await.unwrap();

    // A folder outside every packages folder is refused, and the player hears why.
    let outside = Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer(),
        release: HASH.into(),
        folder: Some(home.join("Elsewhere")),
    });
    let refused = t.core.dispatch(outside).await.unwrap_err().to_string();
    assert!(refused.contains("Pick a folder inside a packages folder"), "{refused}");
    assert!(t.core.view().notices.iter().any(|n| n.contains("Pick a folder inside a packages folder")));
    assert!(t.core.view().downloads.torrents.is_empty());

    let picked = Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer(),
        release: HASH.into(),
        folder: Some("~/Other/PS5".into()),
    });
    t.core.dispatch(picked).await.unwrap();
    let ready = until(t, "ready").await;
    assert_eq!(ready.folder, native(&format!("~/Other/PS5/torrent-{HASH}")));
    let saved = t.kernel.config().get(&["app", "download_folder", "PS5"]);
    assert_eq!(saved.as_str(), home.join("Other/PS5").to_str(), "the last folder, for this console");

    // The next download of a PS5 game offers the same folder.
    t.core.dispatch(Command::OpenGame(offer())).await.unwrap();
    assert_eq!(t.core.view().detail.expect("the offer's Game Hub").download_folder, "~/Other/PS5");

    act(t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(t, "complete").await;
    assert!(home.join(format!("Other/PS5/torrent-{HASH}/Astro/astro.pkg")).is_file());
    install(t).await;
    let sources = s.sources.lock().unwrap().clone();
    assert_eq!(sources, [HostPath::new("packages", 1, RelPath::new(format!("PS5/torrent-{HASH}")).unwrap())]);
}

/// Each console's emulator on a host that `build` changes, with the catalog and a fake swarm
/// that has `files` as torrent `HASH`: the player downloads the console's one release, and it
/// finishes.
async fn downloaded(
    console: &str,
    topic: serde_json::Value,
    files: &[(&str, &[u8])],
    build: impl FnOnce(FakeHost, &std::path::Path) -> FakeHost,
) -> (TestCore, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let t = build(FakeHost::new(), home.path())
        .home(home.path())
        .platform(Some(Platform::Linux))
        .built_in(PluginByKind::emulator(emu_shadps4::ShadPlugin))
        .built_in(PluginByKind::emulator(emu_kyty::KytyPlugin))
        .built_in(PluginByKind::emulator(emu_pc::PcPlugin))
        .built_in(PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [topic] });
    t.http.serve(&format!("https://xyz.dev/c/{}.json", console.to_ascii_lowercase()), catalog.to_string());
    // The other consoles' catalogs are empty: else the bundled ones show, and their newer
    // releases push this one off the Catalog's first page.
    let empty = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 0,
        "topics": [] });
    for other in ["pc", "ps4", "ps5"].into_iter().filter(|c| *c != console.to_ascii_lowercase()) {
        t.http.serve(&format!("https://xyz.dev/c/{other}.json"), empty.to_string());
    }
    t.torrents.has(HASH, "Game", files);
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    let offers = t.core.view().catalog.games.games;
    let offer =
        offers.iter().find(|o| o.platform == console).unwrap_or_else(|| panic!("a {console} offer: {offers:?}"));
    let offer: GameKey = offer.key.parse().unwrap();

    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download { offer, release: HASH.into(), folder: None }))
        .await
        .unwrap();
    let ready = until(&t, "ready").await;
    assert!(ready.install_note.starts_with(&format!("Installs a {console} game from")), "{}", ready.install_note);
    act(&t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(&t, "complete").await;
    // A finished download unpacks first; its card offers Install once that is done.
    t.until(Area::Torrents, |_| row(&t).actions.contains(&"install".to_owned())).await;
    (t, home)
}

/// Waits until the download's install ends, and gives its row.
async fn installed(t: &TestCore) -> nexus_ui_api::TorrentVm {
    t.until(Area::Installs, |_| !matches!(row(t).install.as_str(), "" | "installing")).await;
    row(t)
}

/// [`downloaded`], then Install: the game shows in the library. Says what the Library holds
/// once the install ends.
async fn download_and_install(console: &str, topic: serde_json::Value, files: &[(&str, &[u8])]) -> Vec<String> {
    let (t, _home) = downloaded(console, topic, files, |h, _| h).await;
    install(&t).await;
    let ended = installed(&t).await;
    assert_eq!(ended.install, "done", "the {console} install: {ended:?}");
    // The download's card plays the game it installed, found by title id or else by the
    // installed game's title.
    assert!(!ended.game.is_empty() && ended.actions.contains(&"play".to_owned()), "the {console} card: {ended:?}");
    t.core.view().home.games.into_iter().map(|g| format!("{} {}", g.platform, g.title)).collect()
}

fn topic(title: &str, info: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": "1", "title": title, "size": "1 GB", "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
        "game_info": info })
}

#[tokio::test]
async fn a_finished_ps4_download_installs_and_shows_in_the_library() {
    use emu_shadps4::pkg4::{Kind, build};
    let package = build::package(&build::Spec {
        files: &[("eboot.bin", b"\x4f\x15\x3d\x1dfixture")],
        kind: Kind::Game,
        ..Default::default()
    });
    let info = serde_json::json!({ "name": "Fixture", "title_id": "CUSA12345", "region": "EUR", "version": "v01.00" });
    let games =
        download_and_install("PS4", topic("[PS4] Fixture [EUR / v01.00]", info), &[("Fixture/game.pkg", &package)])
            .await;
    assert!(games.contains(&"PS4 Fixture".to_owned()), "{games:?}");
}

#[tokio::test]
async fn a_finished_ps5_download_installs_and_shows_in_the_library() {
    let param: &[u8] = br#"{"titleId":"PPSA33333","localizedParameters":{"en-US":{"titleName":"Unpacked"}}}"#;
    let files: &[(&str, &[u8])] =
        &[("Release/PPSA33333-app0/sce_sys/param.json", param), ("Release/PPSA33333-app0/eboot.bin", b"elf")];
    let info = serde_json::json!({ "name": "Unpacked", "title_id": "PPSA33333", "region": "EUR", "version": "v01.00" });
    let games = download_and_install("PS5", topic("[PS5] Unpacked [EUR / v01.00]", info), files).await;
    assert!(games.contains(&"PS5 Unpacked".to_owned()), "{games:?}");
}

#[tokio::test]
async fn a_finished_pc_download_installs_and_shows_in_the_library() {
    let mut elf = vec![0u8; 0x200];
    elf[..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
    let files: &[(&str, &[u8])] = &[("Penguins/penguins.x86_64", &elf)];
    let games =
        download_and_install("PC", topic("Penguins (v1.2) [amd64] [Native]", serde_json::json!({})), files).await;
    assert!(games.contains(&"PC Penguins".to_owned()), "{games:?}");
}

// Unix only: links.
#[cfg(unix)]
#[tokio::test]
async fn an_unpacked_pc_game_moves_into_the_games_folder_with_its_links_and_empty_folders() {
    use std::os::unix::fs::MetadataExt;
    let mut installer = b"#!/bin/sh\narcsz=16\n".to_vec();
    installer.resize(2048, b'#');
    let (t, home) = downloaded(
        "PC",
        topic("Sonic (v1.4) [amd64] [Native]", serde_json::json!({})),
        &[("sonic.run", &installer)],
        |h, _| {
            // A repack with its own Proton: links between its libraries, and a Wine prefix's empty
            // `dosdevices`, which Proton makes its drive links in.
            let archives = h.archives();
            archives.holds("sonic.run", &[("Sonic/start", b"#!/bin/sh\n"), ("Sonic/wine/lib/libav.so.58", b"lib")]);
            archives.also_holds(
                "sonic.run",
                &[("Sonic/wine/lib/libav.so", "libav.so.58")],
                &["Sonic/wine/default_pfx/dosdevices"],
            );
            h
        },
    )
    .await;
    install(&t).await;
    let ended = installed(&t).await;
    assert_eq!(ended.install, "done", "{ended:?}");

    let game = home.path().join("Games/PC/Sonic");
    let link = game.join("wine/lib/libav.so");
    assert_eq!(std::fs::read_link(&link).unwrap(), std::path::Path::new("libav.so.58"), "still a link");
    assert!(game.join("wine/default_pfx/dosdevices").is_dir(), "the empty folder too");
    let left = |dir: &std::path::Path| -> bool {
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.file_name() == "start" {
                    return true;
                }
                if e.file_type().unwrap().is_dir() {
                    dirs.push(e.path());
                }
            }
        }
        false
    };
    assert!(!left(&home.path().join("Downloads")), "moved, not copied: the unpacked copy is gone");
    assert!(std::fs::metadata(game.join("start")).unwrap().mode() & 0o100 != 0, "the start script runs");
}

#[tokio::test]
async fn after_a_restart_a_pc_download_s_card_still_plays_the_game_it_installed() {
    let (topic, elf) = penguins();
    let (t, _home) = downloaded("PC", topic, &[("Penguins/penguins.x86_64", &elf)], |h, _| h).await;
    install(&t).await;
    assert_eq!(installed(&t).await.install, "done");

    // The next start: the install's record is gone, and a PC game has no title id.
    let again = nexus_core::Core::start(t.kernel.clone()).await;
    again.dispatch(Command::RefreshLibrary).await.unwrap();
    catalogs_fetched(&again).await;
    let card = |core: &nexus_core::Core| core.view().downloads.torrents.into_iter().next().unwrap();
    until_in(&again, Area::Library, |core| !card(core).game.is_empty()).await;
    let card = card(&again);
    assert!(!card.game.is_empty(), "the installed game: {card:?}");
    assert!(card.actions.contains(&"play".to_owned()), "Play, not Install: {:?}", card.actions);
}

fn penguins() -> (serde_json::Value, Vec<u8>) {
    let mut elf = vec![0u8; 0x200];
    elf[..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
    (topic("Penguins (v1.2) [amd64] [Native]", serde_json::json!({})), elf)
}

#[tokio::test]
async fn a_download_installs_into_the_game_folder_the_player_picks_and_the_next_install_offers_it() {
    let (topic, elf) = penguins();
    let (t, home) = downloaded("PC", topic, &[("Penguins/penguins.x86_64", &elf)], |h, _| h).await;
    let home = home.path();
    let paths = vec![home.join("Games"), home.join("Other")];
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths }).await.unwrap();
    assert_eq!(row(&t).install_folder, native("~/Games/PC"), "the console's folder in the first game folder");

    // A folder outside every game folder is refused, and the sheet hears why.
    let outside =
        Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: Some(home.join("Elsewhere")) });
    let refused = t.core.dispatch(outside).await.unwrap_err().to_string();
    assert!(refused.contains("Pick a folder inside a game folder"), "{refused}");
    assert!(row(&t).install_error.contains("Pick a folder inside a game folder"), "{:?}", row(&t));
    assert_eq!(row(&t).install, "", "nothing installs");

    let picked =
        Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: Some("~/Other/Mine".into()) });
    t.core.dispatch(picked).await.unwrap();
    assert_eq!(row(&t).install_error, "", "the error goes once an install starts");
    assert_eq!(installed(&t).await.install, "done");
    assert!(home.join("Other/Mine/Penguins").is_dir(), "the game is in the folder the player picked");
    assert_eq!(row(&t).install_folder, "~/Other/Mine", "the next install of a PC game offers it");
}

#[tokio::test]
async fn an_install_checks_the_free_space_in_the_game_folder_the_player_picks() {
    let (topic, elf) = penguins();
    let files: &[(&str, &[u8])] = &[("Penguins/penguins.x86_64", &elf)];
    let (t, home) = downloaded("PC", topic, files, |h, home| h.disk(home.join("Small"), 4)).await;
    let home = home.path();
    std::fs::create_dir_all(home.join("Small")).unwrap();
    let paths = vec![home.join("Games"), home.join("Small")];
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths }).await.unwrap();

    let small = Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: Some("~/Small/PC".into()) });
    let refused = t.core.dispatch(small).await.unwrap_err();
    let why = "Not enough free space in the game folder: 0.0 GB needed, 0.0 GB free";
    assert_eq!(refused.to_string(), why);
    assert_eq!(row(&t).install_error, why);
    assert_eq!(row(&t).install, "", "nothing installs");

    // The folder the sheet offers is on another disk, with room.
    t.core.dispatch(Command::GameInstall(GameInstallCmd::Download { id: HASH.into(), folder: None })).await.unwrap();
    assert_eq!(installed(&t).await.install, "done");
    assert!(home.join("Games/PC/Penguins").is_dir());
}

#[tokio::test]
async fn a_refused_download_says_why_on_its_game_hub_until_the_next_try() {
    let s = setup().await;
    let t = &s.t;
    let home = s.home.path();
    let paths = vec![home.join("Downloads"), home.join("Other")];
    t.core.dispatch(Command::SetFolders { name: "packages".into(), paths }).await.unwrap();
    t.core.dispatch(Command::OpenGame(offer())).await.unwrap();

    let outside = Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer(),
        release: HASH.into(),
        folder: Some(home.join("Elsewhere")),
    });
    assert!(t.core.dispatch(outside).await.is_err());
    let why = t.core.view().detail.expect("the Game Hub").download_error;
    assert!(why.contains("Pick a folder inside a packages folder"), "{why}");

    let picked = Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer(),
        release: HASH.into(),
        folder: Some("~/Other/PS5".into()),
    });
    t.core.dispatch(picked).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().download_error, "", "gone once a download starts");
}

#[tokio::test]
async fn a_download_checks_the_free_space_in_the_folder_the_player_picks() {
    let home = tempfile::tempdir().unwrap();
    let small = home.path().join("Small");
    std::fs::create_dir_all(&small).unwrap();
    let s = setup_on(HASH, |h| h.disk(small.clone(), 4)).await;
    let t = &s.t;
    let paths = vec![s.home.path().join("Downloads"), small.clone()];
    t.core.dispatch(Command::SetFolders { name: "packages".into(), paths }).await.unwrap();
    t.core.dispatch(Command::OpenGame(offer())).await.unwrap();

    let there = Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer(),
        release: HASH.into(),
        folder: Some(small.join("PS5")),
    });
    let refused = t.core.dispatch(there).await.unwrap_err().to_string();
    assert!(refused.starts_with("Not enough free space in the download folder: "), "{refused}");
    assert_eq!(t.core.view().detail.unwrap().download_error, refused, "the sheet says why");
    assert!(t.core.view().downloads.torrents.is_empty(), "nothing downloads");
}

#[tokio::test]
async fn downloads_count_what_downloads_and_what_shares() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    let counts = |t: &TestCore| (t.core.view().downloads.downloading, t.core.view().downloads.sharing);
    assert_eq!(counts(t), (0, 0), "a download that waits for Start doesn't download");
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;
    assert_eq!(counts(t), (1, 0));
    t.torrents.complete(HASH);
    until(t, "complete").await;
    assert_eq!(counts(t), (0, 1));
}

#[tokio::test]
async fn an_install_shows_on_its_download_s_card_which_cancels_it() {
    let s = setup_plugin(HASH, crate::common::slow_installer(), Default::default(), |h| h).await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.complete(HASH);
    until(t, "complete").await;

    install(t).await;
    // The install's progress reaches the card a moment later: the plugin never waits for it.
    t.until(Area::Installs, |_| row(t).install_progress > 0.0).await;
    let installing = row(t);
    assert_eq!((installing.install.as_str(), installing.install_progress), ("installing", 0.25));
    assert_eq!(installing.actions, ["stop-seeding", "cancel-install"], "no Remove while it installs");
    t.core.dispatch(Command::GameInstall(GameInstallCmd::Cancel { id: HASH.into() })).await.unwrap();
    t.until(Area::Installs, |_| row(t).install == "cancelled").await;
    assert_eq!(row(t).install, "cancelled");
    assert!(row(t).actions.contains(&"install".to_owned()), "it installs again: {:?}", row(t).actions);
}

#[tokio::test]
async fn once_installed_a_download_s_folder_is_the_game_s() {
    let (topic, elf) = penguins();
    let (t, home) = downloaded("PC", topic, &[("Penguins/penguins.x86_64", &elf)], |h, _| h).await;
    let download = home.path().join(format!("Downloads/PC/torrent-{HASH}"));
    assert_eq!(t.core.download_folder(HASH), Some(download), "the download's files, before the install");
    install(&t).await;
    assert_eq!(installed(&t).await.install, "done");
    assert_eq!(t.core.download_folder(HASH), Some(home.path().join("Games/PC/Penguins")), "the installed game");
}

#[tokio::test]
async fn a_download_reads_up_to_100_of_its_files_and_counts_them_all() {
    let s = setup().await;
    let t = &s.t;
    let names: Vec<String> = (0..120).map(|i| format!("Astro/part{i:03}.bin")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    t.torrents.has(HASH, "Astro", &files);
    let held = t.torrents.hold();
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    assert_eq!(row(t).state, "resolving", "the lookup waits");
    held.release();
    let ready = until(t, "ready").await;
    assert_eq!((ready.files.len(), ready.file_count), (100, 120));
}

#[tokio::test]
async fn a_running_download_is_read_every_poll_on_the_clock_and_not_between() {
    let s = setup().await;
    let t = &s.t;
    t.core
        .dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
            offer: offer(),
            release: HASH.into(),
            folder: None,
        }))
        .await
        .unwrap();
    until(t, "ready").await;
    act(t, TorrentAction::Start).await;
    t.torrents.progress(HASH, 4);
    until(t, "downloading").await;

    // A poll just read 4 bytes: the next is `torrent_poll` (10 ms on the fake host) away.
    t.torrents.progress(HASH, 8);
    t.clock.advance(Duration::from_millis(5)).await;
    assert_eq!(row(t).done, 4, "not read between two polls");
    t.clock.advance(Duration::from_millis(7)).await;
    assert_eq!(row(t).done, 8);
}
