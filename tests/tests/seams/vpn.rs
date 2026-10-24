//! Seam 1, core app API: "Download only through a VPN". The torrent port is bound to the VPN's
//! interface, and a kill switch pauses the downloads when the VPN drops or changes.

use std::time::Duration;

use nexus_api::*;
use nexus_core::{Command, TorrentAction};
use nexus_testkit::{FakeHost, StubPlugin, TestCore};

use crate::common::{catalogs_fetched, full_path};

const ASTRO: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";
const BOLT: &str = "d23fe1c06bba254a9dc9f519b335aa7c1367a88b";

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

/// An emulator with no games of its own: it only takes the downloads' packages folder.
fn emulator() -> StubPlugin {
    struct Emu;
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
    StubPlugin::from_toml(
        "id = \"emu-kyty\"\nkind = \"emulator\"\nname = \"KytyPS5\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         console = \"PS5\"\n\
         capabilities = [\"catalog\"]\n\
         [[permissions.filesystem]]\nfolder = \"packages\"\n",
    )
    .provides(|_| Ok(Provided { catalog: Some(std::sync::Arc::new(Emu)), ..Provided::default() }))
}

/// A catalog with two games, Astro and Bolt, one release each.
fn catalog() -> String {
    let topic = |n: &str, name: &str, title: &str, hash: &str| {
        serde_json::json!({ "id": n, "title": format!("[PS5] {name} [EUR / v01.00]"), "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{hash}"),
            "game_info": { "name": name, "title_id": title, "region": "EUR", "version": "v01.00" } })
    };
    serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 2,
        "topics": [topic("1", "Astro", "PPSA01234", ASTRO), topic("2", "Bolt", "PPSA05678", BOLT)] })
    .to_string()
}

struct Setup {
    t: TestCore,
    _home: tempfile::TempDir,
}

async fn setup() -> Setup {
    let home = tempfile::tempdir().unwrap();
    let t = FakeHost::new()
        .home(home.path())
        .plugin(emulator())
        .built_in(PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    t.http.serve("https://xyz.dev/c/ps5.json", catalog());
    t.torrents.has(ASTRO, "Astro", &[("Astro/astro.pkg", b"astro")]);
    t.torrents.has(BOLT, "Bolt", &[("Bolt/bolt.pkg", b"bolt")]);
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    Setup { t, _home: home }
}

fn offer(hash: &str) -> GameKey {
    let title = if hash == ASTRO { "PPSA01234" } else { "PPSA05678" };
    format!("release-catalog:emu-kyty/{title}").parse().unwrap()
}

fn row(t: &TestCore, hash: &str) -> nexus_ui_api::TorrentVm {
    t.core.view().downloads.torrents.into_iter().find(|r| r.id == hash).expect("a download")
}

/// Waits until `hash`'s row passes `check`.
async fn until(t: &TestCore, hash: &str, what: &str, check: impl Fn(&nexus_ui_api::TorrentVm) -> bool) {
    for _ in 0..300 {
        if t.core.view().downloads.torrents.iter().any(|r| r.id == hash && check(r)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{hash} never {what}: {:?}", t.core.view().downloads.torrents);
}

async fn download(t: &TestCore, hash: &str) {
    let release = hash.to_owned();
    t.core.dispatch(Command::DownloadRelease { offer: offer(hash), release, folder: None }).await.unwrap();
}

async fn act(t: &TestCore, hash: &str, action: TorrentAction) {
    t.core.dispatch(Command::Torrent { id: hash.into(), action }).await.unwrap();
}

/// `hash` read, started and downloading.
async fn running(t: &TestCore, hash: &str) {
    download(t, hash).await;
    until(t, hash, "ready", |r| r.state == "ready").await;
    act(t, hash, TorrentAction::Start).await;
    t.torrents.progress(hash, 2);
    until(t, hash, "downloading", |r| r.state == "downloading").await;
}

#[tokio::test]
async fn downloads_go_through_the_vpn_and_pause_when_it_drops() {
    let s = setup().await;
    let t = &s.t;
    t.network.up("wg0");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    running(t, ASTRO).await;
    assert_eq!(t.torrents.routes(), [Some("wg0".to_owned())], "bound to the VPN before it starts");
    assert_eq!(t.core.view().settings.vpn.route, "wg0");

    t.network.down("wg0");
    until(t, ASTRO, "waiting", |r| r.waiting_vpn).await;
    let waiting = row(t, ASTRO);
    assert_eq!(waiting.state, "paused");
    assert!(t.torrents.running().is_empty(), "nothing runs without the VPN");
    assert_eq!(t.torrents.stopped_all(), 1, "the connections close too");
    assert_eq!(t.core.view().settings.vpn.route, "");
}

#[tokio::test]
async fn another_vpn_stops_the_downloads_and_starts_them_on_its_interface() {
    let s = setup().await;
    let t = &s.t;
    t.network.up("wg0");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    running(t, ASTRO).await;

    t.network.down("wg0");
    t.network.up("wg1");
    until(t, ASTRO, "started again", |r| r.state == "checking").await;
    assert_eq!(t.torrents.routes(), [Some("wg0".to_owned()), Some("wg1".to_owned())]);
    assert_eq!(t.torrents.started().len(), 2);
}

#[tokio::test]
async fn the_vpn_back_starts_only_the_downloads_it_stopped() {
    let s = setup().await;
    let t = &s.t;
    t.network.up("wg0");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    running(t, ASTRO).await;
    running(t, BOLT).await;
    // The player pauses Bolt: it stays paused.
    act(t, BOLT, TorrentAction::Pause).await;

    t.network.down("wg0");
    until(t, ASTRO, "waiting", |r| r.waiting_vpn).await;
    assert!(!row(t, BOLT).waiting_vpn);
    t.network.up("wg0");
    until(t, ASTRO, "started again", |r| !r.waiting_vpn && r.state == "checking").await;

    let started: Vec<String> = t.torrents.started().into_iter().map(|(hash, _)| hash).collect();
    assert_eq!(started, [ASTRO, BOLT, ASTRO]);
    assert_eq!(row(t, BOLT).state, "paused");
    assert_eq!(t.torrents.running(), [ASTRO]);
}

#[tokio::test]
async fn with_no_vpn_a_new_download_waits_and_reads_its_files_once_the_vpn_connects() {
    let s = setup().await;
    let t = &s.t;
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    download(t, ASTRO).await;

    let waiting = row(t, ASTRO);
    assert!(waiting.waiting_vpn, "{waiting:?}");
    assert_eq!((waiting.state.as_str(), waiting.file_count), ("paused", 0), "not read from the swarm");
    assert!(t.torrents.routes().is_empty());
    // Start waits too.
    act(t, ASTRO, TorrentAction::Start).await;
    assert!(t.torrents.started().is_empty());
    assert!(row(t, ASTRO).waiting_vpn);

    t.network.up("wg0");
    until(t, ASTRO, "ready", |r| r.state == "ready").await;
    assert_eq!(row(t, ASTRO).file_count, 1);
    assert!(t.torrents.started().is_empty(), "it waits for the player, as always");
    assert_eq!(t.torrents.routes(), [Some("wg0".to_owned())]);
}

#[tokio::test]
async fn with_the_setting_off_the_vpn_changes_nothing() {
    let s = setup().await;
    let t = &s.t;
    t.network.up("wg0");
    running(t, ASTRO).await;

    t.network.down("wg0");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(row(t, ASTRO).state, "downloading");
    assert!(!row(t, ASTRO).waiting_vpn);
    assert!(t.torrents.routes().is_empty());
    assert_eq!(t.torrents.stopped_all(), 0);
    assert_eq!(t.torrents.running(), [ASTRO]);
}

#[tokio::test]
async fn turning_the_setting_on_moves_running_downloads_onto_the_vpn_and_off_lets_waiting_ones_go() {
    let s = setup().await;
    let t = &s.t;
    running(t, ASTRO).await;
    t.network.up("wg0");

    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    until(t, ASTRO, "started on the VPN", |r| r.state == "checking" && !r.waiting_vpn).await;
    assert_eq!(t.torrents.routes(), [Some("wg0".to_owned())]);
    assert_eq!(t.torrents.stopped_all(), 1);

    t.network.down("wg0");
    until(t, ASTRO, "waiting", |r| r.waiting_vpn).await;
    t.core.dispatch(Command::SetVpnOnly(false)).await.unwrap();
    until(t, ASTRO, "started without the VPN", |r| r.state == "checking" && !r.waiting_vpn).await;
    assert_eq!(t.torrents.routes(), [Some("wg0".to_owned()), None]);
}

#[tokio::test]
async fn where_the_system_can_t_tell_a_vpn_the_player_picks_one() {
    let s = setup().await;
    let t = &s.t;
    t.network.picks_only();
    t.network.up("en0");
    t.network.up("utun4");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    download(t, ASTRO).await;
    assert!(row(t, ASTRO).waiting_vpn, "no network counts as the VPN until the player picks one");

    t.core.dispatch(Command::SetVpn("utun4".into())).await.unwrap();
    until(t, ASTRO, "ready", |r| r.state == "ready").await;
    assert_eq!(t.torrents.routes(), [Some("utun4".to_owned())]);
    let vpn = t.core.view().settings.vpn;
    assert_eq!((vpn.chosen.as_str(), vpn.route.as_str(), vpn.finds), ("utun4", "utun4", false));
}

#[tokio::test]
async fn the_setting_lists_the_vpns_and_connects_disconnects_and_adds_them() {
    let s = setup().await;
    let t = &s.t;
    t.network.up("wg0");
    t.network.down("work");
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let vpn = t.core.view().settings.vpn;
    let listed: Vec<(String, bool)> = vpn.vpns.iter().map(|v| (v.name.clone(), v.connected)).collect();
    assert_eq!(listed, [("wg0".to_owned(), true), ("work".to_owned(), false)]);
    assert!(vpn.can_import && vpn.problem.is_empty());

    t.core.dispatch(Command::ConnectVpn("work".into())).await.unwrap();
    t.core.dispatch(Command::DisconnectVpn("wg0".into())).await.unwrap();
    let file = full_path("/home/me/se-1.conf");
    t.core.dispatch(Command::ImportVpn(file.clone().into())).await.unwrap();
    assert_eq!(t.network.calls(), ["connect work".to_owned(), "disconnect wg0".into(), format!("import {file}")]);
    let connected: Vec<String> =
        t.core.view().settings.vpn.vpns.into_iter().filter(|v| v.connected).map(|v| v.name).collect();
    assert_eq!(connected, ["work", "se-1"]);

    // A refusal says why, on the setting.
    t.network.refuse("Error: Connection activation failed");
    assert!(t.core.dispatch(Command::ConnectVpn("wg0".into())).await.is_err());
    assert_eq!(t.core.view().settings.vpn.error, "Error: Connection activation failed");
}

#[tokio::test]
async fn where_downloads_can_t_be_bound_the_setting_stays_off_and_says_why() {
    let s = setup().await;
    let t = &s.t;
    t.network.unavailable("Not available on Windows");
    assert!(t.core.dispatch(Command::SetVpnOnly(true)).await.is_err());
    let vpn = t.core.view().settings.vpn;
    assert!(!vpn.only);
    assert_eq!(vpn.problem, "Not available on Windows");
}
