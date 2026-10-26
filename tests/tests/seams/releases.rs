//! Seam 1, core app API: the game catalog service, its copies, and the releases it lists.

use crate::common::*;
use nexus_api::{ConfigScope, ReleaseList, ReleaseSource};
use nexus_core::Command;
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
    FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await
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

#[tokio::test]
async fn without_an_address_the_bundled_catalog_is_used() {
    let t = boot().await;
    let list = list(&t).await.expect("a bundled catalog for the PS5");
    assert_eq!(list.source, ReleaseSource::Bundled);
    assert_eq!(list.problem, None);
    assert!(t.http.sent().is_empty(), "nothing is fetched");
}

#[tokio::test]
async fn a_fetched_catalog_is_used_and_kept_for_when_the_fetch_fails() {
    let t = boot().await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog/").await.unwrap();

    // The first ask gets the bundled copy; the fetch runs in the background.
    let fetched = list_once(&t, |l| l.source == ReleaseSource::Fetched).await;
    assert_eq!(fetched.source, ReleaseSource::Fetched);
    assert_eq!((fetched.updated.as_str(), fetched.host.as_str()), ("2026-10-18T00:00:00Z", "xyz.dev"));
    assert_eq!(fetched.releases[0].id, HASH);

    // Six hours on, the address answers with a file that doesn't read: the kept copy stays.
    let kept = t.data_dir().join("plugins/release-catalog/fetched/ps5.meta.json");
    let mut stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&kept).unwrap()).unwrap();
    stored["fetched"] = 0.into();
    std::fs::write(&kept, stored.to_string()).unwrap();
    t.http.serve(URL, b"{ not json".to_vec());
    let cached = list_once(&t, |l| l.source == ReleaseSource::Cached).await;
    assert_eq!(cached.source, ReleaseSource::Cached);
    assert_eq!(cached.updated, "2026-10-18T00:00:00Z");
    assert!(cached.problem.unwrap().contains("not a catalog file"));
}

/// The fetched PS5 copy's meta file.
fn fetched_meta(t: &TestCore) -> serde_json::Value {
    let path = t.data_dir().join("plugins/release-catalog/fetched/ps5.meta.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Makes the fetched PS5 copy six hours old.
fn age_the_fetched_copy(t: &TestCore) {
    let mut meta = fetched_meta(t);
    meta["fetched"] = 0.into();
    std::fs::write(t.data_dir().join("plugins/release-catalog/fetched/ps5.meta.json"), meta.to_string()).unwrap();
}

#[tokio::test]
async fn the_meta_file_names_the_fetched_copy_it_goes_with_and_older_copies_go() {
    let t = boot().await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    list_once(&t, |l| l.source == ReleaseSource::Fetched).await;
    let folder = t.data_dir().join("plugins/release-catalog/fetched");
    let names = || {
        let mut names: Vec<String> =
            std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    };
    let first = fetched_meta(&t)["body"].as_str().expect("the meta names its copy").to_owned();
    assert!(first.starts_with("ps5.") && first != "ps5.json", "a new name for each copy: {first}");
    assert_eq!(std::fs::read(folder.join(&first)).unwrap(), catalog("2026-10-18T00:00:00Z").into_bytes());

    // Six hours on, the same file: the copy keeps its name.
    age_the_fetched_copy(&t);
    list_once(&t, |_| fetched_meta(&t)["fetched"] != 0).await;
    assert_eq!(fetched_meta(&t)["body"], first.as_str());

    // Two new files: the copy before stays for a reader that read the meta file before.
    for day in ["2026-10-19T00:00:00Z", "2026-10-20T00:00:00Z"] {
        t.http.serve(URL, catalog(day));
        age_the_fetched_copy(&t);
        list_once(&t, |l| l.updated == day).await;
    }
    let body = fetched_meta(&t)["body"].as_str().unwrap().to_owned();
    assert_eq!(names().len(), 3, "the meta file, the copy it names and the one before: {:?}", names());
    assert_eq!(names().iter().filter(|n| **n != "ps5.meta.json").max(), Some(&body), "the newest copy");
    assert!(!names().contains(&first), "the oldest copy is gone");
}

#[tokio::test]
async fn with_no_copy_kept_a_failed_fetch_falls_back_to_the_bundled_catalog() {
    let t = boot().await;
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    let list = list_once(&t, |l| l.problem.is_some()).await;
    assert_eq!(list.source, ReleaseSource::Bundled);
    assert!(list.problem.is_some(), "the page says why");
}

#[tokio::test]
async fn a_slow_catalog_address_still_gives_the_bundled_catalog_at_once_and_the_fetched_copy_after() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-kyty", vec![]).console("PS5"))
        .boot_core()
        .await;
    // The catalog address answers only when the test lets it, after the core's call timeout.
    let gate = t.http.hold_next(URL);
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_millis(200), list(&t)).await;
    let first = first.expect("an answer at once").expect("the bundled PS5 catalog");
    assert_eq!((first.source, first.problem), (ReleaseSource::Bundled, None));

    // Past the call timeout, the held fetch is no error.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(t.core.view().notices.is_empty(), "{:?}", t.core.view().notices);

    // The fetch ends in the background, and the core reads the catalogs again.
    gate.release();
    let fetched = list_once(&t, |l| l.source == ReleaseSource::Fetched).await;
    assert_eq!(fetched.updated, "2026-10-18T00:00:00Z");
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    assert_eq!(offers_once(&t, &["release-catalog:emu-kyty/PPSA01234"]).await, ["release-catalog:emu-kyty/PPSA01234"]);
    assert_eq!(t.http.sent(), [URL], "fetched once");
}

/// The PS5 catalog once `done` says it is the one the test waits for, or after a second.
async fn list_once(t: &TestCore, done: impl Fn(&ReleaseList) -> bool) -> ReleaseList {
    for _ in 0..100 {
        let got = list(t).await;
        if got.as_ref().is_some_and(&done) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    list(t).await.expect("a PS5 catalog")
}

/// The keys of the Catalog's offers, sorted, once they are `keys`, or after a second.
async fn offers_once(t: &TestCore, keys: &[&str]) -> Vec<String> {
    for _ in 0..100 {
        if offer_keys(t) == keys {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    offer_keys(t)
}

#[tokio::test]
async fn an_address_on_a_host_the_manifest_does_not_name_is_refused() {
    let t = boot().await;
    let refused = set_address(&t, "https://other.example/catalog").await.unwrap_err();
    assert!(refused.to_string().contains("other.example"), "{refused}");
    assert!(set_address(&t, "http://xyz.dev/catalog").await.is_err(), "https only");
}

/// A PS4 catalog with one game, Racer.
fn ps4_catalog() -> String {
    catalog_of("2026-10-18T00:00:00Z", serde_json::json!([topic(1, "CUSA00001", "Racer", "1.00", "2026-10-01", 'a')]))
}

/// Waits until each catalog on the Catalog is the copy fetched from the catalog address, or a
/// second passes: the first read gets the bundled copy, and the fetch runs in the background.
async fn fetched_on_the_catalog(t: &TestCore) {
    for _ in 0..100 {
        let sources = t.core.view().catalog.sources;
        if !sources.is_empty() && sources.iter().all(|s| s.contains(": fetched from ")) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The keys of the Catalog's offers, sorted.
fn offer_keys(t: &TestCore) -> Vec<String> {
    let mut keys: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.key.clone()).collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn an_emulator_gets_the_catalog_of_the_console_it_plays_and_keeps_its_own_id_in_the_keys() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-a", vec![]).console("PS5"))
        .plugin(games("emu-b", vec![]).console("PS4"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    t.http.serve("https://xyz.dev/catalog/ps4.json", ps4_catalog());
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    assert_eq!(offer_keys(&t), ["release-catalog:emu-a/PPSA01234", "release-catalog:emu-b/CUSA00001"]);
    let mut sent = t.http.sent();
    sent.sort();
    assert_eq!(sent, ["https://xyz.dev/catalog/ps4.json", URL], "one file for each console");
}

#[tokio::test]
async fn a_console_turned_off_in_the_catalog_s_settings_offers_nothing_and_its_games_stay() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-a", "PS5", vec!["Alpha"]).console("PS5"))
        .plugin(games("emu-b", vec![]).console("PS4"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    t.http.serve("https://xyz.dev/catalog/ps4.json", ps4_catalog());
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    // A toggle for each console an emulator plays, on.
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let toggles = |t: &TestCore| -> Vec<(String, String, String)> {
        let view = t.core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
        panel
            .fields
            .iter()
            .filter(|f| f.key.starts_with("show_p"))
            .map(|f| (f.key.clone(), f.label.clone(), f.value.clone()))
            .collect()
    };
    let shown = |on: &str| -> Vec<(String, String, String)> {
        vec![
            ("show_ps4".into(), "Show PS4 games".into(), "true".into()),
            ("show_ps5".into(), "Show PS5 games".into(), on.into()),
        ]
    };
    assert_eq!(toggles(&t), shown("true"));

    let ps5 = |on: bool| Command::SetConfigField {
        plugin: id("release-catalog"),
        scope: ConfigScope::Emulator,
        key: "show_ps5".into(),
        value: on.to_string(),
    };
    t.core.dispatch(ps5(false)).await.unwrap();
    assert_eq!(toggles(&t), shown("false"));
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    assert_eq!(offer_keys(&t), ["release-catalog:emu-b/CUSA00001"], "no PS5 game to download");
    let offer = "release-catalog:emu-a/PPSA01234".parse().unwrap();
    let download = Command::DownloadRelease { offer, release: HASH.into(), folder: None };
    assert!(t.core.dispatch(download).await.is_err(), "nor a PS5 release");
    let library: Vec<String> = t.core.view().home.games.iter().map(|g| g.title.clone()).collect();
    assert_eq!(library, ["Alpha"], "the installed PS5 game stays in the library");

    t.core.dispatch(ps5(true)).await.unwrap();
    assert_eq!(offer_keys(&t), ["release-catalog:emu-a/PPSA01234", "release-catalog:emu-b/CUSA00001"]);
}

#[tokio::test]
async fn a_console_whose_emulator_lists_its_own_releases_has_no_other_catalog() {
    // Steam's library comes from the Steam plugin, which lists its own console's releases: the
    // console catalog is asked nothing for it.
    let t = FakeHost::new()
        .platform(Some(nexus_api::Platform::Linux))
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .built_in(nexus_api::PluginByKind::emulator(emu_steam::SteamPlugin))
        .plugin(games("emu-a", vec![]).console("PS5"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let view = t.core.view();
    let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
    let keys: Vec<_> = panel.fields.iter().map(|f| f.key.clone()).collect();
    assert!(keys.contains(&"show_ps5".to_owned()) && !keys.contains(&"show_steam".to_owned()), "{keys:?}");
    assert!(t.http.sent().is_empty(), "nothing fetched for Steam: {:?}", t.http.sent());
}

#[tokio::test]
async fn the_catalog_s_console_toggles_follow_the_emulators_turned_on() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-a", vec![]).console("PS5"))
        .plugin(games("emu-b", vec![]).console("PS4"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Settings)).await.unwrap();
    let toggles = |t: &TestCore| -> Vec<String> {
        let view = t.core.view();
        let panel = view.settings.panels.iter().find(|p| p.slot.plugin == id("release-catalog")).unwrap();
        panel.fields.iter().filter(|f| f.key.starts_with("show_p")).map(|f| f.key.clone()).collect()
    };
    assert_eq!(toggles(&t), ["show_ps4", "show_ps5"]);

    let emu_b = |enabled: bool| Command::SetPluginEnabled { plugin: id("emu-b"), enabled };
    t.core.dispatch(emu_b(false)).await.unwrap();
    assert_eq!(toggles(&t), ["show_ps5"], "no PS4 emulator, no PS4 toggle");
    t.core.dispatch(emu_b(true)).await.unwrap();
    assert_eq!(toggles(&t), ["show_ps4", "show_ps5"]);
}

#[tokio::test]
async fn of_two_emulators_for_one_console_the_first_loaded_gets_the_catalog() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-a", vec![]).console("PS5"))
        .plugin(games("emu-c", vec![]).console("PS5"))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    let order = t.core.kernel().order();
    let first = order.iter().find(|p| ["emu-a", "emu-c"].contains(&p.as_str())).unwrap();
    assert_eq!(offer_keys(&t), [format!("release-catalog:{first}/PPSA01234")], "the other gets no offers");
    assert_eq!(t.core.view().catalog.sources.len(), 1);
    assert_eq!(t.http.sent(), [URL], "the PS5 catalog is fetched once");
}

#[tokio::test]
async fn an_emulator_with_no_console_gets_no_catalog() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-a", vec![]))
        .boot_core()
        .await;
    t.http.serve(URL, catalog("2026-10-18T00:00:00Z"));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();

    assert_eq!(offer_keys(&t), Vec::<String>::new());
    assert!(t.core.view().catalog.sources.is_empty());
    assert!(t.http.sent().is_empty(), "the catalog plugin is not asked");
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
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-kyty", vec!["Alpha"]).console("PS5"))
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
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;
    t
}

#[tokio::test]
async fn the_catalog_lists_one_game_per_title_id_newest_first_with_its_badges() {
    let t = with_offers().await;
    let view = t.core.view().catalog;
    assert_eq!(view.games.sort, "added", "newest first");
    let shown: Vec<(String, Vec<String>)> =
        view.games.games.iter().map(|c| (c.title.clone(), c.badges.iter().map(|b| b.text.clone()).collect())).collect();
    assert_eq!(
        shown,
        [
            ("Alpha".to_owned(), vec!["Installed".to_owned()]),
            ("Racer".to_owned(), vec!["New".to_owned()]),
            ("Quest".to_owned(), vec![]),
        ],
        "the emulator's own game is Installed, a release from this week is New"
    );
    assert_eq!(view.sources.len(), 1);
    assert!(
        view.sources[0].starts_with("emu-kyty: fetched from xyz.dev · updated 2026-10-18 · 4 releases"),
        "{}",
        view.sources[0]
    );

    t.core.dispatch(Command::CatalogSearch("rac".into())).await.unwrap();
    let titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    assert_eq!(titles, ["Racer"], "the search narrows the offers");
    assert_eq!(t.core.view().home.library.query, "", "the Library keeps its own search");

    t.core.dispatch(Command::CatalogSearch(String::new())).await.unwrap();
    t.core.dispatch(Command::CatalogSort("title".into())).await.unwrap();
    let titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    assert_eq!(titles, ["Alpha", "Quest", "Racer"]);
}

#[tokio::test]
async fn an_offer_opens_on_its_releases_newest_first() {
    let t = with_offers().await;
    let racer = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Racer").unwrap();
    t.core.dispatch(Command::OpenGame(racer.key.parse().unwrap())).await.unwrap();

    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.title, "Racer");
    assert_eq!(detail.emulator, "emu-kyty");
    assert_eq!(detail.description, "Racer 1.05.", "the newest release's words");
    let versions: Vec<(&str, bool)> = detail.releases.iter().map(|r| (r.version.as_str(), r.new)).collect();
    assert_eq!(versions, [("1.05", true), ("1.00", false)]);
    assert!(!detail.can.play, "a game to download doesn't play");
    assert_eq!((detail.can.primary.as_str(), detail.can.get.as_str()), ("get", "Download"), "it downloads");
    assert!(detail.facts.iter().any(|f| f.id == "region" && f.value == "EUR"));
}

/// A store that answers every title id, and says which it was asked about.
fn counting_store(asked: Arc<Mutex<Vec<String>>>) -> StubPlugin {
    struct Counting(Arc<Mutex<Vec<String>>>);
    #[nexus_api::async_trait]
    impl nexus_api::Metadata for Counting {
        async fn describe(
            &self,
            game: &nexus_api::GameRef,
        ) -> nexus_api::PluginResult<Option<nexus_api::GameMetadata>> {
            self.0.lock().unwrap().push(game.title_id.clone());
            Ok(Some(nexus_api::GameMetadata { title: game.title.clone(), ..Default::default() }))
        }
    }
    StubPlugin::from_toml(
        "id = \"store\"\nkind = \"service\"\nname = \"store\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"metadata\"]",
    )
    .provides(move |_| {
        Ok(nexus_api::Provided { metadata: Some(Arc::new(Counting(asked.clone()))), ..Default::default() })
    })
}

#[tokio::test]
async fn the_catalog_shows_a_page_at_a_time_and_describes_only_what_it_shows() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-kyty", vec![]).console("PS5"))
        .plugin(counting_store(asked.clone()))
        .boot_core()
        .await;
    // 130 games, one release each, a hash apiece.
    let topics: Vec<serde_json::Value> = (0..130u32)
        .map(|n| serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS4] Game {n:03}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": format!("Game {n:03}"), "title_id": format!("CUSA{:05}", n + 1), "genre": "Action" } }))
        .collect();
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", serde_json::Value::Array(topics)));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    let page = |t: &TestCore| {
        let g = t.core.view().catalog.games;
        (g.games.len(), g.total)
    };
    assert_eq!(page(&t), (nexus_core::GRID_PAGE, 130));
    // Until the fetch is in, the Catalog shows the bundled catalog, and the store is asked
    // about its first page too: only this catalog's games (CUSA…) count.
    let described = || async {
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        asked.lock().unwrap().iter().filter(|id| id.starts_with("CUSA")).count()
    };
    assert_eq!(described().await, nexus_core::GRID_PAGE, "the store is asked about the shown page only");

    t.core.dispatch(Command::CatalogMore).await.unwrap();
    assert_eq!(page(&t), (2 * nexus_core::GRID_PAGE, 130));
    t.core.dispatch(Command::CatalogMore).await.unwrap();
    assert_eq!(page(&t), (130, 130), "the last page is the rest");
    assert_eq!(described().await, 130, "each game once");

    t.core.dispatch(Command::CatalogSearch("Game 1".into())).await.unwrap();
    assert_eq!(page(&t), (30, 30), "Game 100 to Game 129");
    t.core.dispatch(Command::CatalogSearch("Game".into())).await.unwrap();
    assert_eq!(page(&t), (nexus_core::GRID_PAGE, 130), "a search starts from the first page");
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
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on(emulator, console, vec![]).console(console));
    if store {
        host =
            host.plugin(crate::common::store("store", vec![("PPSA07777", 4.0, "")], None, std::time::Duration::ZERO));
    }
    let t = host.boot_core().await;
    let file = console.to_ascii_lowercase();
    t.http.serve(&format!("https://xyz.dev/catalog/{file}.json"), catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;
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

#[tokio::test]
async fn a_pc_offer_shows_steam_s_words_and_art_and_its_protondb_tier() {
    let t = offering(
        "emu-pc",
        "PC",
        serde_json::json!([{ "id": "1", "title": "Celeste (v1.4.0.0) [amd64] [Wine]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": {}, "steam": steam_block() }]),
        false,
    )
    .await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert_eq!(card.cover, steam_image("portrait.jpg"), "the card has Steam's portrait");

    let detail = open_first(&t).await;
    assert_eq!(detail.description, "Help Madeline survive.");
    assert_eq!(detail.developer, "Maddy Makes Games");
    assert_eq!(detail.genres, ["Action", "Indie"]);
    assert_eq!(detail.release, "2018-01-25");
    assert_eq!(detail.cover, steam_image("portrait.jpg"));
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.logo, steam_image("logo.png"));
    assert_eq!(detail.screenshots, [steam_image("ss_1.jpg").unwrap()]);
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
    assert_eq!(detail.protondb, "gold", "a game packed with Wine has ProtonDB's tier");
    assert_eq!(detail.protondb_url, "https://www.protondb.com/app/504230");
}

#[tokio::test]
async fn an_installed_game_with_no_art_of_its_own_has_its_offer_s_and_keeps_it_with_the_catalog_off() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-pc", "PC", vec!["Celeste"]).console("PC"))
        .boot_core()
        .await;
    let topics = serde_json::json!([{ "id": "1", "title": "Celeste (v1.4.0.0) [amd64] [Wine]",
        "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Celeste" }, "steam": steam_block() }]);
    t.http.serve("https://xyz.dev/catalog/pc.json", catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert!(card.badges.iter().any(|b| b.text == "Installed"), "the offer is the installed game");

    let cover = |t: &TestCore| t.core.view().home.games.first().and_then(|g| g.cover.clone());
    assert_eq!(cover(&t), steam_image("portrait.jpg"), "the library card has the offer's art");
    // Its Game Hub has the offer's Steam art, and no releases: it is installed.
    let game: nexus_api::GameKey = t.core.view().home.games[0].key.parse().unwrap();
    t.core.dispatch(Command::OpenGame(game.clone())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.cover, steam_image("portrait.jpg"));
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.logo, steam_image("logo.png"));
    assert!(detail.releases.is_empty(), "the offer's releases are the Catalog's");

    t.core.dispatch(Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: false }).await.unwrap();
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    assert_eq!(cover(&t), steam_image("portrait.jpg"), "and keeps its cover with no catalog");
    t.core.dispatch(Command::OpenGame(game)).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().cover, steam_image("portrait.jpg"), "in its Game Hub too");
}

#[tokio::test]
async fn a_native_pc_offer_has_no_protondb_tier() {
    let t = offering(
        "emu-pc",
        "PC",
        serde_json::json!([{ "id": "1", "title": "Celeste [Native]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": {}, "steam": steam_block() }]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
    assert_eq!((detail.protondb.as_str(), detail.protondb_url.as_str()), ("", ""));
}

#[tokio::test]
async fn a_ps_offer_the_store_has_no_art_for_gets_steam_s_art() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([{ "id": "1", "title": "[PS5] Celeste [EUR]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Celeste" },
            "steam": steam_block() }]),
        true,
    )
    .await;
    let card = t.core.view().catalog.games.games.first().cloned().unwrap();
    assert_eq!(card.cover, steam_image("portrait.jpg"));
    let detail = open_first(&t).await;
    assert_eq!(detail.background, steam_image("hero.jpg"));
    assert_eq!(detail.protondb, "", "a console game has no ProtonDB tier");
}

#[tokio::test]
async fn the_store_s_data_comes_first_and_steam_fills_what_it_leaves_out() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([{ "id": "1", "title": "[PS5] Celeste [EUR]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
            "game_info": { "name": "Celeste", "title_id": "PPSA07777" }, "steam": steam_block() }]),
        true,
    )
    .await;
    let offer = t.core.view().catalog.games.games.first().cloned().unwrap();
    t.core.dispatch(Command::OpenGame(offer.key.parse().unwrap())).await.unwrap();
    let mut detail = t.core.view().detail.unwrap();
    for _ in 0..200 {
        if detail.developer == "MOJANG AB" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        detail = t.core.view().detail.unwrap();
    }
    assert_eq!(detail.developer, "MOJANG AB", "the store's publisher");
    assert_eq!(detail.genres, ["Adventure", "Arcade"], "the store's genres");
    assert_eq!(detail.screenshots.len(), 2, "the store's screenshots");
    assert_eq!(detail.description, "Help Madeline survive.", "the store has no words: Steam's");
    assert_eq!(detail.background, steam_image("hero.jpg"), "the store has no background: Steam's");
    assert_eq!(detail.store_url, "https://store.steampowered.com/app/504230/");
}

#[tokio::test]
async fn a_ps5_package_release_says_it_can_t_be_installed() {
    let package = "dddddddddddddddddddddddddddddddddddddddd";
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([
            { "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
              "published_at": "2026-10-01", "game_info": { "name": "Astro", "title_id": "PPSA01234", "format": "exfat" } },
            { "id": "2", "title": "[PS5] Astro [EUR / v01.04 / fpkg]", "magnet": format!("magnet:?xt=urn:btih:{package}"),
              "published_at": "2026-09-01", "game_info": { "name": "Astro", "title_id": "PPSA01234" } },
        ]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    let warnings: Vec<(&str, &str)> = detail.releases.iter().map(|r| (r.id.as_str(), r.warning.as_str())).collect();
    assert_eq!(
        warnings,
        [(HASH, ""), (package, "Retail PS5 packages can't be installed; pick a folder, an archive or an exFAT image"),],
        "the package stays listed, with why"
    );
}

#[tokio::test]
async fn a_ps4_package_release_has_no_warning() {
    let t = offering(
        "emu-shadps4",
        "PS4",
        serde_json::json!([{ "id": "1", "title": "[PS4] Racer [EUR / v1.00 / pkg]",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"), "game_info": { "name": "Racer", "title_id": "CUSA00001" } }]),
        false,
    )
    .await;
    let detail = open_first(&t).await;
    assert_eq!(detail.releases[0].warning, "", "shadPS4 unpacks PS4 packages");
}

#[tokio::test]
async fn an_edition_with_its_own_title_id_joins_its_base_game() {
    let t = offering(
        "emu-shadps4",
        "PS4",
        serde_json::json!([
            topic(1, "CUSA00001", "Racer", "1.00", "2026-09-01", 'a'),
            topic(2, "CUSA00009", "Racer Digital Deluxe Edition", "1.05", "2026-10-01", 'b'),
            topic(3, "CUSA00008", "Racer: Collector's Edition", "1.02", "2026-09-15", 'c'),
            topic(4, "CUSA00002", "Racer 2 Deluxe Edition", "1.00", "2026-08-01", 'd'),
        ]),
        false,
    )
    .await;
    let mut offers: Vec<(String, usize)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.releases)).collect();
    offers.sort();
    assert_eq!(
        offers,
        [("Racer 2 Deluxe Edition".to_owned(), 1), ("Racer Digital Deluxe Edition".to_owned(), 3)],
        "the editions join the base game; the sequel stays apart"
    );
}

const SOURCES_URL: &str = "https://xyz.dev/catalog-sources.json";

/// A catalog sources file with these forums for the PS5, by key and address.
fn sources_of(forums: &[(&str, &str)]) -> String {
    let forums: Vec<serde_json::Value> =
        forums.iter().map(|(key, url)| serde_json::json!({ "key": key, "url": url, "tag": "PS5" })).collect();
    serde_json::json!({ "ps5": { "platform": "PS5", "forums": forums } }).to_string()
}

async fn set_sources(t: &TestCore, url: &str) -> nexus_core::CoreResult<()> {
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "sources_url": url }),
        })
        .await
}

/// The rows of the catalog plugin's "Catalog sources" section, as `label: value`.
async fn sources_rows(t: &TestCore) -> Vec<String> {
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    let request = nexus_api::SlotRequest { slot: nexus_api::SlotId::EmulatorPanel, game: None };
    let items = provided.slots.unwrap().items(&request).await.unwrap();
    let section = items.into_iter().find(|i| i.id == "sources").expect("a sources section");
    let nexus_api::SlotContent::Section { title, rows } = section.content else { panic!("a section") };
    assert_eq!(title, "Catalog sources");
    rows.into_iter().map(|r| format!("{}: {}", r.label, r.value)).collect()
}

#[tokio::test]
async fn without_a_sources_address_the_bundled_sources_are_used() {
    let t = boot().await;
    let rows = sources_rows(&t).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert!(rows.contains(&"PS5: ps5".to_owned()), "{rows:?}");
    assert!(rows.contains(&"PC: linux, wine".to_owned()), "{rows:?}");
    assert!(t.http.sent().is_empty(), "nothing is fetched");
}

#[tokio::test]
async fn a_fetched_sources_list_is_used_and_kept_for_when_the_fetch_fails() {
    let t = boot().await;
    t.http.serve(SOURCES_URL, sources_of(&[("new", "https://xyz.net/forum/viewforum.php?f=1")]));
    set_sources(&t, SOURCES_URL).await.unwrap();

    assert_eq!(sources_rows(&t).await, ["List: fetched from xyz.dev", "PS5: new"]);
    assert!(t.data_dir().join("plugins/release-catalog/sources/catalog-sources.json").is_file());
    // A kept copy is used for six hours, with no new request.
    sources_rows(&t).await;
    assert_eq!(t.http.sent(), [SOURCES_URL]);

    // Six hours on, the address answers with a file that doesn't read: the kept copy stays.
    let kept = t.data_dir().join("plugins/release-catalog/sources/catalog-sources.meta.json");
    let mut stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&kept).unwrap()).unwrap();
    stored["fetched"] = 0.into();
    std::fs::write(&kept, stored.to_string()).unwrap();
    t.http.serve(SOURCES_URL, b"{ not json".to_vec());
    let rows = sources_rows(&t).await;
    assert_eq!(rows[0], "List: the copy fetched before from xyz.dev");
    assert!(
        rows[1].starts_with("Fetch failed: this is not a catalog sources file in the current format: "),
        "{rows:?}"
    );
    assert_eq!(rows[2], "PS5: new");
}

#[tokio::test]
async fn with_no_sources_copy_kept_a_failed_fetch_falls_back_to_the_bundled_sources() {
    let t = boot().await;
    set_sources(&t, SOURCES_URL).await.unwrap();
    let rows = sources_rows(&t).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert_eq!(rows[1], format!("Fetch failed: HTTP 404 from {SOURCES_URL}"));
    assert!(rows.contains(&"PS5: ps5".to_owned()), "{rows:?}");
}

#[tokio::test]
async fn a_forum_with_no_f_number_is_skipped_and_named() {
    let t = boot().await;
    t.http.serve(
        SOURCES_URL,
        sources_of(&[("ok", "https://xyz.net/forum/viewforum.php?f=1"), ("bad", "https://xyz.net/forum/index.php")]),
    );
    set_sources(&t, SOURCES_URL).await.unwrap();
    assert_eq!(
        sources_rows(&t).await,
        ["List: fetched from xyz.dev", "PS5: ok", "Skipped: No f= number: bad: https://xyz.net/forum/index.php"]
    );
}

#[tokio::test]
async fn a_sources_file_with_its_catalogs_under_a_key_does_not_read() {
    let t = boot().await;
    let nested = serde_json::json!({ "api": "https://api.xyz.net/f/", "feed": "https://feed.xyz.net/f/",
        "catalogs": { "ps5": { "platform": "PS5", "forums": [] } } });
    t.http.serve(SOURCES_URL, nested.to_string());
    set_sources(&t, SOURCES_URL).await.unwrap();
    let rows = sources_rows(&t).await;
    assert_eq!(rows[0], "List: the bundled list");
    assert!(
        rows[1].starts_with("Fetch failed: this is not a catalog sources file in the current format: "),
        "{rows:?}"
    );
}

#[tokio::test]
async fn a_sources_address_on_a_host_the_manifest_does_not_name_is_refused() {
    let t = boot().await;
    let refused = set_sources(&t, "https://other.example/catalog-sources.json").await.unwrap_err();
    assert!(refused.to_string().contains("other.example"), "{refused}");
    assert!(set_sources(&t, "http://xyz.dev/catalog-sources.json").await.is_err(), "https only");
}

/// A catalog's topic of `name` for `platform`, with this title id (empty for none).
fn named(id: u32, name: &str, platform: &str, title_id: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[{platform}] {name} [EUR]"), "platform": platform,
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": "2026-10-01",
        "game_info": { "name": name, "title_id": title_id, "genre": "Action" } })
}

/// The Catalog's cards, as (title, platform, badges), sorted.
fn cards(t: &TestCore) -> Vec<(String, String, Vec<String>)> {
    let mut cards: Vec<_> = t
        .core
        .view()
        .catalog
        .games
        .games
        .iter()
        .map(|c| (c.title.clone(), c.platform.clone(), c.badges.iter().map(|b| b.text.clone()).collect()))
        .collect();
    cards.sort();
    cards
}

#[tokio::test]
async fn releases_of_one_name_for_two_consoles_stay_two_games() {
    let t = offering(
        "emu-kyty",
        "PS5",
        serde_json::json!([
            named(1, "Astro", "PS5", "PPSA01234", 'a'),
            named(2, "Astro", "PS4", "CUSA01234", 'b'),
            named(3, "Bolt", "PS5", "", 'c'),
            named(4, "Bolt", "PS4", "", 'd'),
        ]),
        false,
    )
    .await;
    let mut games: Vec<(String, String, usize)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.key.clone(), c.releases)).collect();
    games.sort();
    assert_eq!(games.len(), 4, "{games:?}");
    let keys: std::collections::HashSet<&String> = games.iter().map(|(_, key, _)| key).collect();
    assert_eq!(keys.len(), 4, "each game has its own key: {games:?}");
    assert!(games.iter().all(|(_, _, releases)| *releases == 1), "{games:?}");
}

#[tokio::test]
async fn an_installed_game_matches_its_offer_by_name_on_the_same_console_and_has_a_chip() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-kyty", "PS5", vec!["Alpha"]).console("PS5"))
        .boot_core()
        .await;
    let topics = serde_json::json!([
        named(1, "Alpha", "PS5", "", 'a'),
        named(2, "Alpha", "PS4", "", 'b'),
        named(3, "Bolt", "PS5", "", 'c')
    ]);
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    let installed = |t: &TestCore| {
        cards(t).into_iter().filter(|(_, _, badges)| badges.contains(&"Installed".to_owned())).collect::<Vec<_>>()
    };
    assert_eq!(
        installed(&t),
        [("Alpha".to_owned(), "PS5".to_owned(), vec!["Installed".to_owned()])],
        "the PS4 game of the same name is another game"
    );
    let systems = t.core.view().catalog.games.systems;
    let chip = systems.iter().find(|s| s.id == nexus_core::INSTALLED_FILTER).expect("an Installed chip");
    assert_eq!((chip.label.as_str(), chip.count, chip.on), ("Installed", 1, false));

    t.core.dispatch(Command::CatalogSystem(Some(nexus_core::INSTALLED_FILTER.into()))).await.unwrap();
    let shown: Vec<(String, String)> = cards(&t).into_iter().map(|(title, platform, _)| (title, platform)).collect();
    assert_eq!(shown, [("Alpha".to_owned(), "PS5".to_owned())], "only the installed game");
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    assert_eq!(cards(&t).len(), 3);
}

/// A topic of `name` at `version`, `size` GB, with `seeders`, for the PS5.
fn sized(id: u32, name: &str, version: &str, size: u64, seeders: u64, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] {name} [EUR / {version}]"), "platform": "PS5",
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": format!("2026-10-{id:02}"),
        "size_bytes": size * 1_000_000_000, "seeders": seeders,
        "game_info": { "name": name, "version": version, "genre": "Action" } })
}

#[tokio::test]
async fn a_card_shows_its_best_release_and_the_game_hub_orders_them_by_version() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-kyty", "PS5", vec!["Alpha"]).console("PS5"))
        .boot_core()
        .await;
    let mut art = sized(5, "Quest", "1.00", 5, 1, 'e');
    art["steam"] = steam_block();
    let topics = serde_json::json!([
        // The highest version wins, not the newest upload.
        sized(1, "Racer", "1.05", 3, 1, 'a'),
        sized(2, "Racer", "1.02", 2, 1, 'b'),
        sized(3, "Racer", "1.00", 1, 1, 'c'),
        // A release with art wins over a higher version.
        sized(4, "Quest", "2.00", 6, 1, 'd'),
        art,
        // The version the player has wins over all.
        sized(6, "Alpha", "02.000.000", 8, 9, 'f'),
        sized(7, "Alpha", "01.000.002", 7, 0, '1'),
        // Of one version, the one more people share.
        sized(8, "Bolt", "1.00", 4, 1, '2'),
        sized(9, "Bolt", "1.00", 9, 30, '3'),
    ]);
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", topics));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let alpha = t.core.view().home.games.first().cloned().expect("the installed Alpha");
    t.core.dispatch(Command::OpenGame(alpha.key.parse().unwrap())).await.unwrap();
    t.core.dispatch(Command::Back).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;

    let mut sizes: Vec<(String, Option<u64>)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.size_bytes)).collect();
    sizes.sort();
    let gb = |n: u64| Some(n * 1_000_000_000);
    assert_eq!(
        sizes,
        [
            ("Alpha".to_owned(), gb(7)),
            ("Bolt".to_owned(), gb(9)),
            ("Quest".to_owned(), gb(5)),
            ("Racer".to_owned(), gb(3))
        ]
    );

    let racer = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Racer").unwrap();
    t.core.dispatch(Command::OpenGame(racer.key.parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    let releases: Vec<(&str, &str, bool)> =
        detail.releases.iter().map(|r| (r.version.as_str(), r.version_label.as_str(), r.best)).collect();
    assert_eq!(
        releases,
        [
            ("1.05", "Version 3 of 3 (newest)", true),
            ("1.02", "Version 2 of 3", false),
            ("1.00", "Version 1 of 3", false)
        ]
    );
}

#[tokio::test]
async fn a_card_and_the_game_hub_say_how_available_a_release_is() {
    let mut low = sized(2, "Bolt", "1.00", 1, 2, 'b');
    low["leechers"] = 4.into();
    low["peers_observed"] = "2026-10-07T14:07:32Z".into();
    let mut unknown = sized(4, "Drift", "1.00", 1, 0, 'd');
    unknown["seeders"] = serde_json::Value::Null;
    let topics =
        serde_json::json!([sized(1, "Astro", "1.00", 1, 3, 'a'), low, sized(3, "Comet", "1.00", 1, 0, 'c'), unknown]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let mut cards: Vec<(String, String)> =
        t.core.view().catalog.games.games.iter().map(|c| (c.title.clone(), c.availability.clone())).collect();
    cards.sort();
    let want = [("Astro", "good"), ("Bolt", "low"), ("Comet", "none"), ("Drift", "unknown")];
    assert_eq!(cards, want.map(|(a, b)| (a.to_owned(), b.to_owned())), "3 or more is good, 1 or 2 low");

    let bolt = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Bolt").unwrap();
    t.core.dispatch(Command::OpenGame(bolt.key.parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.availability, "low", "the best release's");
    let r = &detail.releases[0];
    assert_eq!((r.seeders, r.leechers, r.counted_at.as_str()), (Some(2), Some(4), "2026-10-07T14:07:32Z"));
    assert_eq!(r.availability, "low");
}

#[tokio::test]
async fn the_catalog_s_genre_chips_are_the_fixed_genres() {
    let genre = |id: u32, name: &str, genre: &str, hash: char| {
        let mut topic = named(id, name, "PS5", "", hash);
        topic["game_info"]["genre"] = genre.into();
        topic
    };
    let topics = serde_json::json!([
        genre(1, "Astro", "Hack and Slash, Role-Playing", 'a'),
        genre(2, "Bolt", "Action", 'b'),
        genre(3, "Comet", "Card game", 'c'),
        genre(4, "Drift", "", 'd'),
    ]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let chips: Vec<(String, usize)> =
        t.core.view().catalog.games.genres.iter().map(|g| (g.label.clone(), g.count)).collect();
    let want = [("All genres", 4), ("Action", 2), ("Other", 1), ("RPG", 1)];
    assert_eq!(chips, want.map(|(g, n)| (g.to_owned(), n)), "a game with no genre words has no chip");

    t.core.dispatch(Command::CatalogGenre(Some("Action".into()))).await.unwrap();
    let mut titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, ["Astro", "Bolt"]);
}

#[tokio::test]
async fn the_catalog_s_search_matches_each_word_in_the_title_title_id_publisher_region_and_version() {
    let release = |id: u32, name: &str, title_id: &str, publisher: &str, region: &str, version: &str, hash: char| {
        let mut topic = named(id, name, "PS5", title_id, hash);
        topic["game_info"]["publisher"] = publisher.into();
        topic["game_info"]["region"] = region.into();
        topic["game_info"]["version"] = version.into();
        topic
    };
    let topics = serde_json::json!([
        release(1, "Astro Bot", "PPSA01234", "Sony Interactive", "EUR", "v01.05", 'a'),
        release(2, "Astro Bot", "PPSA01234", "Sony Interactive", "USA", "v01.02", 'b'),
        release(3, "Bolt", "PPSA05555", "Ubisoft", "USA", "v2.00", 'c'),
    ]);
    let t = offering("emu-kyty", "PS5", topics, false).await;
    let found = |query: &'static str| {
        let t = &t;
        async move {
            t.core.dispatch(Command::CatalogSearch(query.into())).await.unwrap();
            let mut titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect();
            titles.sort();
            titles
        }
    };
    assert_eq!(found("ppsa05555").await, ["Bolt"], "the title id");
    assert_eq!(found("sony eur").await, ["Astro Bot"], "the publisher and the region");
    assert_eq!(found("bot astro").await, ["Astro Bot"], "each word, in any order");
    assert_eq!(found("usa v2").await, ["Bolt"], "the region and the version, of one release");
    assert!(found("ubisoft eur").await.is_empty(), "every word, in one release");
    assert_eq!(found("ast eu").await, ["Astro Bot"], "the start of each word");
    assert!(found("stro eur").await.is_empty(), "not the middle of one");
    assert_eq!(found("stro b").await, ["Astro Bot"], "the whole search inside the title");
    assert_eq!(found("usa").await, ["Astro Bot", "Bolt"]);
}

#[tokio::test]
async fn a_topic_s_first_picture_is_the_cover_when_there_is_no_steam_art() {
    let picture = |id: u32, name: &str, hash: char| {
        let mut topic = named(id, name, "PS5", "", hash);
        topic["game_info"]["image_urls"] =
            serde_json::json!(["https://i1.imageban.ru/out/cover.jpg", "https://i1.imageban.ru/out/shot.jpg"]);
        topic
    };
    let mut steam = picture(2, "Celeste", 'b');
    steam["steam"] = steam_block();
    let t = offering("emu-kyty", "PS5", serde_json::json!([picture(1, "Astro", 'a'), steam]), false).await;
    let cover = |title: &str| t.core.view().catalog.games.games.into_iter().find(|c| c.title == title).unwrap().cover;
    let topic_s = nexus_ui_api::ImageRef::Url {
        plugin: id("release-catalog"),
        url: "https://i1.imageban.ru/out/cover.jpg".into(),
    };
    assert_eq!(cover("Astro"), Some(topic_s.clone()), "the topic's first picture");
    assert_eq!(cover("Celeste"), steam_image("portrait.jpg"), "Steam's art comes first");

    let astro = t.core.view().catalog.games.games.into_iter().find(|c| c.title == "Astro").unwrap();
    t.core.dispatch(Command::OpenGame(astro.key.parse().unwrap())).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().cover, Some(topic_s), "the Game Hub's cover too");
}

fn titles(t: &TestCore) -> Vec<String> {
    t.core.view().catalog.games.games.iter().map(|c| c.title.clone()).collect()
}

#[tokio::test]
async fn the_catalog_sorts_by_size_smallest_first_by_compatibility_and_by_the_catalog_s_release_date() {
    let mut astro = sized(1, "Astro", "1.00", 5, 1, 'a');
    astro["game_info"]["release_date"] = "2021-03-04".into();
    let mut bolt = sized(2, "Bolt", "1.00", 1, 1, 'b');
    bolt["game_info"]["release_year"] = "2024".into();
    let mut comet = sized(3, "Comet", "1.00", 3, 1, 'c');
    comet["game_info"]["release_date"] = "2023-11-30".into();
    // No size and no date: last in both.
    let mut drift = sized(4, "Drift", "1.00", 0, 1, 'd');
    drift.as_object_mut().unwrap().remove("size_bytes");
    let t = offering("emu-kyty", "PS5", serde_json::json!([astro, bolt, comet, drift]), false).await;

    t.core.dispatch(Command::CatalogSort("smallest".into())).await.unwrap();
    assert_eq!(titles(&t), ["Bolt", "Comet", "Astro", "Drift"], "smallest first, no size last");
    t.core.dispatch(Command::CatalogSort("size".into())).await.unwrap();
    assert_eq!(titles(&t), ["Astro", "Comet", "Bolt", "Drift"], "largest first");
    t.core.dispatch(Command::CatalogSort("release".into())).await.unwrap();
    assert_eq!(titles(&t), ["Bolt", "Comet", "Astro", "Drift"], "newest release date first, a year counts");
    t.core.dispatch(Command::CatalogSort("compat".into())).await.unwrap();
    assert_eq!(t.core.view().catalog.games.sort, "compat");
}

#[tokio::test]
async fn the_catalog_says_when_its_list_was_updated_and_counts_its_games_and_releases() {
    let mut astro = sized(1, "Astro", "1.00", 1, 3, 'a');
    astro["peers_observed"] = "2026-10-07T14:07:32Z".into();
    let mut newer = sized(2, "Astro", "1.01", 1, 3, 'b');
    newer["peers_observed"] = "2026-10-19T21:07:34.000Z".into();
    let t = offering("emu-kyty", "PS5", serde_json::json!([astro, newer, sized(3, "Bolt", "1.00", 1, 3, 'c')]), false)
        .await;
    let view = t.core.view().catalog;
    assert_eq!(view.updated, "2026-10-19", "the newest count of seeders");
    assert_eq!((view.games.total, view.games.all, view.releases), (2, 2, 3));
    assert!(!view.loading);

    t.core.dispatch(Command::CatalogSearch("bolt".into())).await.unwrap();
    let view = t.core.view().catalog;
    assert_eq!((view.games.total, view.games.all, view.releases), (1, 2, 3), "the search leaves 1 of 2 games");

    // With no count of seeders, the date the list was collected.
    let t = with_offers().await;
    assert_eq!(t.core.view().catalog.updated, "2026-10-18");
}

/// A catalog service whose catalogs come only once `open` has permits.
fn slow_catalogs(open: Arc<tokio::sync::Semaphore>) -> StubPlugin {
    struct Slow(Arc<tokio::sync::Semaphore>);
    #[nexus_api::async_trait]
    impl nexus_api::Releases for Slow {
        async fn releases(&self, _: &nexus_api::Console) -> nexus_api::PluginResult<Option<ReleaseList>> {
            let _permit = self.0.acquire().await;
            Ok(None)
        }
    }
    StubPlugin::from_toml(
        "id = \"slow-catalog\"\nkind = \"service\"\nname = \"Slow\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"releases\"]",
    )
    .provides(move |_| Ok(nexus_api::Provided { releases: Some(Arc::new(Slow(open.clone()))), ..Default::default() }))
}

#[tokio::test]
async fn the_catalog_opens_at_once_and_says_it_is_loading_until_the_catalogs_are_read() {
    let open = Arc::new(tokio::sync::Semaphore::new(0));
    let t = FakeHost::new()
        .plugin(slow_catalogs(open.clone()))
        .plugin(games_on("emu-kyty", "PS5", vec![]).console("PS5"))
        .boot_core()
        .await;
    let core = t.core.clone();
    let navigate = tokio::spawn(async move { core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await });
    for _ in 0..100 {
        if t.core.view().screen == nexus_ui_api::ScreenId::Catalog {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(t.core.view().screen, nexus_ui_api::ScreenId::Catalog, "the Catalog opens before the read ends");
    assert!(t.core.view().catalog.loading, "the catalogs are not read yet");

    open.add_permits(10);
    navigate.await.unwrap().unwrap();
    assert!(!t.core.view().catalog.loading, "read");
}

#[tokio::test]
async fn the_game_hub_s_facts_come_from_the_best_release() {
    let mut best = sized(1, "Astro", "1.05", 2, 5, 'a');
    best["leechers"] = 2.into();
    best["peers_observed"] = "2026-10-07T14:07:32Z".into();
    best["game_info"] = serde_json::json!({ "name": "Astro", "version": "1.05", "region": "EUR",
        "release_summary": "The best release.", "publisher": "Sony", "developer": "Team Asobi",
        "release_date": "2024-09-06", "multiplayer": "No", "release_update": "v01.05", "format": "fpkg",
        "minimum_firmware": "5.xx", "interface_languages": "English, French", "audio_languages": "English" });
    // A newer upload of a lower version.
    let mut newest = sized(9, "Astro", "1.00", 1, 1, 'b');
    newest["game_info"]["release_summary"] = "The newest upload.".into();
    newest["game_info"]["region"] = "USA".into();
    let t = offering("emu-kyty", "PS5", serde_json::json!([best, newest]), false).await;
    let detail = open_first(&t).await;
    assert_eq!(detail.description, "The best release.");
    assert_eq!(detail.developer, "Team Asobi");
    let facts: Vec<(&str, &str, &str)> =
        detail.facts.iter().map(|f| (f.id.as_str(), f.label.as_str(), f.value.as_str())).collect();
    assert_eq!(
        facts,
        [
            ("region", "Region", "EUR"),
            ("version", "Version", "1.05"),
            ("publisher", "Publisher", "Sony"),
            ("developer", "Developer", "Team Asobi"),
            ("release-date", "Release date", "Sep 6, 2024"),
            ("players", "Players", "Single player"),
            ("update", "Update", "v01.05"),
            ("format", "Format", "fpkg"),
            ("firmware", "Minimum firmware", "5.xx"),
            ("interface-languages", "Interface languages", "English, French"),
            ("audio-languages", "Audio languages", "English"),
            ("seeders", "Sharing it (seeders)", "5"),
            ("leechers", "Getting it (leechers)", "2"),
            ("counted", "Counted", "Oct 7, 2026 · not live"),
        ]
    );
}

#[tokio::test]
async fn with_the_game_catalog_turned_off_there_is_no_catalog_to_open() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-kyty", vec![]).console("PS5"))
        .boot_core()
        .await;
    assert!(t.core.view().catalog.available);
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Downloads)).await.unwrap();

    let off = Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: false };
    t.core.dispatch(off).await.unwrap();

    assert!(!t.core.view().catalog.available);
    let opened = t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await;
    assert!(matches!(opened, Err(nexus_core::CoreError::Invalid(_))), "{opened:?}");
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, nexus_ui_api::ScreenId::Home, "back past where the Catalog was");

    let on = Command::SetPluginEnabled { plugin: id("release-catalog"), enabled: true };
    t.core.dispatch(on).await.unwrap();
    assert!(t.core.view().catalog.available);
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
}

#[tokio::test]
async fn a_hidden_game_leaves_the_catalog_and_the_hidden_filter_shows_it() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games("emu-kyty", vec![]).console("PS5"))
        .boot_core()
        .await;
    let topics: Vec<serde_json::Value> = ["Astro", "Bolt", "Comet"]
        .iter()
        .enumerate()
        .map(|(n, name)| {
            serde_json::json!({ "id": (n + 1).to_string(), "title": format!("[PS5] {name}"),
            "magnet": format!("magnet:?xt=urn:btih:{:040x}", n + 1), "published_at": "2026-01-01",
            "game_info": { "name": name } })
        })
        .collect();
    t.http.serve(URL, catalog_of("2026-10-18T00:00:00Z", serde_json::Value::Array(topics)));
    set_address(&t, "https://xyz.dev/catalog").await.unwrap();
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    fetched_on_the_catalog(&t).await;
    let titles =
        |t: &TestCore| -> Vec<String> { t.core.view().catalog.games.games.iter().map(|g| g.title.clone()).collect() };
    let hidden_chip =
        |t: &TestCore| t.core.view().catalog.games.systems.into_iter().find(|s| s.id == nexus_core::HIDDEN_FILTER);
    assert_eq!(titles(&t), ["Astro", "Bolt", "Comet"]);
    assert!(hidden_chip(&t).is_none(), "no Hidden chip while nothing is hidden");
    let bolt: nexus_api::GameKey =
        t.core.view().catalog.games.games.iter().find(|g| g.title == "Bolt").unwrap().key.parse().unwrap();

    t.core.dispatch(Command::HideGame { game: bolt.clone(), hidden: true }).await.unwrap();

    assert_eq!(titles(&t), ["Astro", "Comet"]);
    let grid = t.core.view().catalog;
    assert_eq!((grid.games.total, grid.games.all, grid.releases), (2, 2, 2), "a hidden game counts nowhere");
    let chip = hidden_chip(&t).expect("a Hidden chip");
    assert_eq!((chip.label.as_str(), chip.count, chip.on), ("Hidden", 1, false));
    assert_eq!(t.core.kernel().config().get(&["app", "hidden_games"]), serde_json::json!([bolt.to_string()]));

    t.core.dispatch(Command::CatalogSystem(Some(nexus_core::HIDDEN_FILTER.into()))).await.unwrap();
    assert_eq!(titles(&t), ["Bolt"], "the Hidden chip shows only the hidden games");
    assert!(t.core.view().catalog.games.games[0].hidden);

    // The Game Catalog's "Show the Hidden filter" off: no chip, and its filter no longer applies.
    let filter = |on: bool| Command::SetConfigField {
        plugin: id("release-catalog"),
        scope: ConfigScope::Emulator,
        key: "show_hidden_filter".into(),
        value: on.to_string(),
    };
    t.core.dispatch(filter(false)).await.unwrap();
    assert!(hidden_chip(&t).is_none(), "the chip is off");
    assert_eq!(titles(&t), ["Astro", "Comet"], "the hidden game stays hidden");
    t.core.dispatch(filter(true)).await.unwrap();
    assert_eq!(hidden_chip(&t).map(|c| c.count), Some(1));

    t.core.dispatch(Command::HideGame { game: bolt, hidden: false }).await.unwrap();
    t.core.dispatch(Command::CatalogSystem(None)).await.unwrap();
    assert_eq!(titles(&t), ["Astro", "Bolt", "Comet"]);
    assert!(hidden_chip(&t).is_none());
}
