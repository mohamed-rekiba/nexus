//! Seam 1, core app API: the game catalog service, its copies, and the releases it lists, one
//! module per area. The catalog fixtures more than one area uses are here.

mod consoles;
mod copies;
mod grid;
mod offers;
mod sources;

use crate::common::*;
use nexus_api::{ConfigScope, ReleaseList, ReleaseSource};
use nexus_core::{Area, Command};
use std::sync::{Arc, Mutex};

use nexus_testkit::{FakeHost, StubPlugin, TestCore};

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

const URL: &str = "https://xyz.dev/catalog/ps5.json";

/// A catalog file in the old launcher's format, with these topics.
fn catalog_of(collected_at: &str, topics: serde_json::Value) -> String {
    serde_json::json!({ "source": "test", "complete": true, "collected_at": collected_at,
        "translation": { "language": "en" }, "topic_count": topics.as_array().unwrap().len(), "topics": topics })
    .to_string()
}

fn catalog(collected_at: &str) -> String {
    catalog_of(
        collected_at,
        serde_json::json!([{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "1 GB",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "published_at": "2026-10-01",
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05",
                "genre": "Action", "release_summary": "A robot." } }]),
    )
}

async fn boot() -> TestCore {
    FakeHost::new().built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default())).boot_core().await
}

async fn set_address(t: &TestCore, url: &str) -> nexus_core::CoreResult<()> {
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": url }),
        })
        .await
}

async fn list(t: &TestCore) -> Option<ReleaseList> {
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    provided.releases.unwrap().releases(&nexus_api::Console::new("PS5").unwrap()).await.unwrap()
}

/// The keys of the Catalog's offers, sorted.
fn offer_keys(t: &TestCore) -> Vec<String> {
    let mut keys: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.key.clone()).collect();
    keys.sort();
    keys
}

/// `days` ago, as `YYYY-MM-DD` (Howard Hinnant's civil_from_days).
pub(crate) fn days_ago(days: i64) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let z = secs / 86_400 - days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn topic(id: u32, title_id: &str, title: &str, version: &str, added: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[PS4] {title} [EUR / {version}]"),
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": added,
        "game_info": { "name": title, "title_id": title_id, "region": "EUR", "version": version, "genre": "Action",
            "release_summary": format!("{title} {version}.") } })
}

/// A catalog with two versions of one game, a game added long ago, and a game the emulator has.
async fn with_offers() -> TestCore {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS4").games(["Alpha"]).build().console("PS5"))
        .boot_core()
        .await;
    let topics = serde_json::json!([
        topic(1, "CUSA00001", "Racer", "1.05", &days_ago(3), 'a'),
        topic(2, "CUSA00001", "Racer", "1.00", &days_ago(60), 'b'),
        topic(3, "CUSA00002", "Quest", "2.00", &days_ago(30), 'c'),
        topic(4, "CUSA00000", "Alpha", "1.00", &days_ago(1), 'd'),
    ]);
    let file = catalog_of("2026-10-18T00:00:00Z", topics);
    t.http.serve(URL, file.to_string());
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    t
}

const STEAM_ART: &str = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/504230";

/// The `steam` object the old launcher's catalog script writes for a game Steam has.
fn steam_block() -> serde_json::Value {
    serde_json::json!({ "appid": 504230, "name": "Celeste", "short": "Help Madeline survive.",
        "genres": ["Action", "Indie"], "release": "Jan 25, 2018", "developers": ["Maddy Makes Games"],
        "publishers": ["Maddy Makes Games Inc."], "header": format!("{STEAM_ART}/header.jpg"),
        "shots": [format!("{STEAM_ART}/ss_1.jpg")],
        "assets": { "portrait": format!("{STEAM_ART}/portrait.jpg"), "hero": format!("{STEAM_ART}/hero.jpg"),
            "logo": format!("{STEAM_ART}/logo.png") },
        "protondb": "gold", "checked_at": "2026-10-14T00:00:00Z" })
}

/// A catalog for `emulator` with these topics, served and read, on the Catalog.
async fn offering(emulator: &str, console: &'static str, topics: serde_json::Value, store: bool) -> TestCore {
    let mut host = FakeHost::new()
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new(emulator).system(console).games([]).build().console(console));
    if store {
        host = host.plugin(FakeStore::new("store").knows("PPSA07777", 4.0, "").build());
    }
    let t = host.boot_core().await;
    let file = console.to_ascii_lowercase();
    t.http.serve(&format!("https://xyz.dev/catalog/{file}.json"), catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    catalogs_fetched(&t.core).await;
    t
}

async fn open_first(t: &TestCore) -> nexus_ui_api::GameDetailVm {
    let offer = t.core.view().catalog.games.games.first().cloned().expect("an offer");
    t.core.dispatch(Command::OpenGame(offer.key.parse().unwrap())).await.unwrap();
    t.core.view().detail.unwrap()
}

fn steam_image(name: &str) -> Option<nexus_ui_api::ImageRef> {
    Some(nexus_ui_api::ImageRef::Url { plugin: id("release-catalog"), url: format!("{STEAM_ART}/{name}") })
}

fn titles(t: &TestCore) -> Vec<String> {
    t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect()
}
