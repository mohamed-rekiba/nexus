//! Seam 1, core app API: the game catalog service refreshes a console's catalog from its forums'
//! Atom feed and forum list, when **Refresh from the forums** is on.

mod common;

use common::*;
use nexus_api::{ConfigScope, ReleaseList};
use nexus_core::Command;

use nexus_testkit::{FakeHost, TestCore};

const CATALOG: &str = "https://xyz.dev/catalog/ps5.json";
/// The catalog sources the tests serve, so they don't depend on the project's own file.
const SOURCES: &str = "https://xyz.dev/catalog-sources.json";
const FEED_ANSWER: &str = include_str!("../../plugins/services/release-catalog/fixtures/forum-546.atom");
const PVC_ANSWER: &str = include_str!("../../plugins/services/release-catalog/fixtures/pvc-546.json");
const FORUM: &str = "https://xyz.net/forum/viewforum.php?f=546";
const KEPT: &str = "plugins/release-catalog/refreshed/ps5";

/// The PS5 forum's feed, at the plugin's fixed address with the forum's number.
fn feed_url() -> String {
    release_catalog::refresh::feed_url("546")
}

/// The PS5 forum's forum list, at the plugin's fixed address with the forum's number.
fn pvc_url() -> String {
    release_catalog::refresh::list_url("546")
}

fn topic(id: &str, name: &str, title_id: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id, "title": format!("[PS5] {name} [EUR / v1.01]"), "size": "1.00 GB", "seeders": 1,
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": "2026-09-01",
        "source": FORUM, "platform": "PS5",
        "game_info": { "name": name, "title_id": title_id, "region": "EUR", "version": "v1.01", "genre": "Action",
            "developer": "Team Asobi", "release_summary": format!("{name}.") } })
}

/// The catalog at the catalog address: five releases of the PS5 forum.
fn base() -> String {
    let mut astro = topic("6900001", "Astro", "PPSA01234", 'x');
    astro["magnet"] = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a".into();
    let topics = serde_json::json!([
        astro,
        topic("6900002", "Bolt", "PPSA02222", '2'),
        topic("6900003", "Comet", "PPSA03333", '3'),
        topic("6900004", "Drift", "PPSA04444", '4'),
        topic("6900005", "Echo", "PPSA05555", '5'),
    ]);
    serde_json::json!({ "source": FORUM, "complete": true, "collected_at": "2026-10-01T00:00:00Z",
        "translation": { "language": "en" }, "topic_count": 5, "topics": topics })
    .to_string()
}

/// A core with the catalog at its address, the feed and this forum list answer, and the refresh
/// on or off.
async fn boot_with(refresh: bool, pvc: String) -> TestCore {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    serve(&t, refresh, pvc).await;
    t
}

/// Serves the catalog, its sources, the feed and the forum list `pvc`, and sets the catalog
/// address with the refresh on or off.
async fn serve(t: &TestCore, refresh: bool, pvc: String) {
    let sources =
        serde_json::json!({ "ps5": { "platform": "PS5", "forums": [{ "key": "ps5", "url": FORUM, "tag": "PS5" }] } });
    t.http.serve(SOURCES, sources.to_string());
    t.http.serve(CATALOG, base());
    t.http.serve(&feed_url(), FEED_ANSWER);
    t.http.serve(&pvc_url(), pvc);
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/catalog", "sources_url": SOURCES,
                "refresh_from_forums": refresh }),
        })
        .await
        .unwrap();
}

async fn boot(refresh: bool) -> TestCore {
    boot_with(refresh, PVC_ANSWER.to_owned()).await
}

async fn list(t: &TestCore) -> ReleaseList {
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    provided.releases.unwrap().releases(&nexus_api::Console::new("PS5").unwrap()).await.unwrap().expect("a PS5 catalog")
}

/// The PS5 catalog once the copy fetched from the catalog address is in, or after a second:
/// the first ask gets the bundled copy, and the fetch runs in the background.
async fn fetched(t: &TestCore) -> ReleaseList {
    for _ in 0..100 {
        if list(t).await.source == nexus_api::ReleaseSource::Fetched {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    list(t).await
}

/// Waits until `done`, or a second passes. `true` when it is done.
async fn until(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if done() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    false
}

/// The next time the Game Catalog says its catalogs changed, or after a second: `true` when
/// it did.
async fn changed(events: &mut futures::stream::BoxStream<'static, nexus_api::Event>) -> bool {
    use futures::StreamExt;
    let next = async {
        while let Some(event) = events.next().await {
            if matches!(&event, nexus_api::Event::Custom { topic, .. } if topic == nexus_api::RELEASES_CHANGED) {
                return true;
            }
        }
        false
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), next).await.unwrap_or(false)
}

/// The PS5 catalog once the refresh its first ask starts has kept its copy, or after a second.
async fn refreshed(t: &TestCore) -> ReleaseList {
    list(t).await;
    until(|| t.data_dir().join(format!("{KEPT}.meta.json")).exists()).await;
    list(t).await
}

fn ids(list: &ReleaseList) -> Vec<String> {
    let mut ids: Vec<String> = list.releases.iter().map(|r| r.id.clone()).collect();
    ids.sort();
    ids
}

/// The refreshed PS5 copy's meta file (`.meta`), or the copy it names (``).
fn kept(t: &TestCore, part: &str) -> serde_json::Value {
    let read = |path: String| serde_json::from_slice(&std::fs::read(t.data_dir().join(path)).unwrap()).unwrap();
    let meta: serde_json::Value = read(format!("{KEPT}.meta.json"));
    match part {
        ".meta" => meta,
        _ => read(format!(
            "plugins/release-catalog/refreshed/{}",
            meta["body"].as_str().expect("the meta names its copy")
        )),
    }
}

#[tokio::test]
async fn with_the_refresh_off_no_forum_is_asked() {
    let t = boot(false).await;
    assert_eq!(fetched(&t).await.releases.len(), 5);
    assert_eq!(t.http.sent(), [CATALOG], "only the catalog address");
    assert!(!t.data_dir().join(format!("{KEPT}.meta.json")).exists());
}

#[tokio::test]
async fn a_refresh_adds_new_releases_drops_removed_ones_and_updates_the_rest() {
    let t = boot(true).await;
    let refreshed = refreshed(&t).await;
    let mut sent = t.http.sent();
    sent.sort();
    assert_eq!(sent, [pvc_url().as_str(), feed_url().as_str(), SOURCES, CATALOG]);

    // Echo (6900005) is gone from the forum list. Moss is VR only. Bolt has a new torrent.
    assert_eq!(
        ids(&refreshed),
        [
            "3".repeat(40),
            "4".repeat(40),
            "ab12".repeat(10),
            "c12fe1c06bba254a9dc9f519b335aa7c1367a88a".to_owned(),
            "d2".repeat(20)
        ]
    );
    // The new release of Astro takes the game's facts from the release the catalog has.
    let new = refreshed.releases.iter().find(|r| r.id == "ab12".repeat(10)).unwrap();
    assert_eq!((new.title.as_str(), new.title_id.as_str()), ("Astro", "PPSA01234"));
    assert_eq!((new.description.as_str(), new.version.as_str()), ("Astro.", ""), "the game's facts, not the upload's");
    assert_eq!(new.size_bytes, Some(13_249_974_886));
    assert_eq!(new.added, "2026-10-07T13:55:12+00:00");
    assert_ne!(refreshed.updated, "2026-10-01T00:00:00Z", "the time of the refresh");

    let file = kept(&t, "");
    let topic = |id: &str| file["topics"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
    assert_eq!(topic("6900001")["seeders"], 42, "peers from the forum list");
    assert_eq!(topic("6900001")["leechers"], 3);
    assert_eq!(topic("6900002")["size"], "2.00 GB", "a re-upload of a new size gets a new label");
    assert_eq!(topic("6900002")["title"], "[PS5] Bolt [EUR / v1.01]", "the English title stays");
    let added = topic("6912345");
    assert_eq!(
        added["magnet"],
        format!("magnet:?xt=urn:btih:{}&tr=http%3A%2F%2Fbt3.t-ru.org%2Fann%3Fmagnet", "AB12".repeat(10))
    );
    assert_eq!(added["game_info_status"], "borrowed");
    assert_eq!(added["game_info"]["borrowed_from"], "6900001");
    assert_eq!((&added["source"], &added["platform"]), (&FORUM.into(), &"PS5".into()));
    assert_eq!(added["url"], "https://xyz.net/forum/viewtopic.php?t=6912345");

    let meta = kept(&t, ".meta");
    assert_eq!((&meta["new"], &meta["removed"], &meta["borrowed"]), (&1.into(), &1.into(), &1.into()));

    // The refreshed copy is used for six hours, with no new request.
    assert_eq!(ids(&list(&t).await), ids(&refreshed));
    assert_eq!(t.http.sent().len(), 4);
}

#[tokio::test]
async fn a_forum_list_that_answers_for_under_80_percent_of_the_catalog_drops_nothing() {
    let mut answer: serde_json::Value = serde_json::from_str(PVC_ANSWER).unwrap();
    answer["result"].as_object_mut().unwrap().retain(|id, _| id == "6900001" || id == "6912345");
    let t = boot_with(true, answer.to_string()).await;
    // Two of six releases listed: a bad answer, not a mass deletion.
    assert_eq!(refreshed(&t).await.releases.len(), 6);
    assert_eq!(kept(&t, ".meta")["removed"], 0);
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_copy_refreshed_before() {
    let t = boot(true).await;
    let mut events = t.core.events();
    list(&t).await;
    assert!(changed(&mut events).await, "the fetch of the catalog address ends");
    assert!(changed(&mut events).await, "the first refresh ends");

    // Six hours on, the forum list answers with something that doesn't read.
    let path = t.data_dir().join(format!("{KEPT}.meta.json"));
    let mut meta = kept(&t, ".meta");
    meta["refreshed"] = 0.into();
    std::fs::write(&path, meta.to_string()).unwrap();
    t.http.serve(&pvc_url(), b"{ not json".to_vec());
    let refreshed = ids(&list(&t).await);
    assert_eq!(refreshed.len(), 5, "the refreshed copy: {refreshed:?}");
    assert!(refreshed.contains(&"ab12".repeat(10)), "with the new release: {refreshed:?}");
    assert!(changed(&mut events).await, "the second refresh fails, and the page's reason changes");
    assert_eq!(t.http.sent().iter().filter(|u| **u == pvc_url()).count(), 2, "it was asked again");
    assert_eq!(ids(&list(&t).await), refreshed);
}

#[tokio::test]
async fn a_failed_refresh_tries_again_after_a_back_off_not_every_second() {
    let t = boot(true).await;
    let mut events = t.core.events();
    list(&t).await;
    assert!(changed(&mut events).await, "the fetch of the catalog address ends");
    assert!(changed(&mut events).await, "the first refresh ends");

    // The copy is old, and the forum list answers with something that doesn't read.
    let path = t.data_dir().join(format!("{KEPT}.meta.json"));
    let mut meta = kept(&t, ".meta");
    meta["refreshed"] = 0.into();
    std::fs::write(&path, meta.to_string()).unwrap();
    t.http.serve(&pvc_url(), b"{ not json".to_vec());
    list(&t).await;
    assert!(changed(&mut events).await, "the second refresh fails");
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    list(&t).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let asked = t.http.sent().iter().filter(|u| **u == pvc_url()).count();
    assert_eq!(asked, 2, "no new try before the back-off ends, also on an ask: {:?}", t.http.sent());
}

#[tokio::test]
async fn a_refresh_that_can_t_keep_its_copy_is_no_change_and_backs_off() {
    // KytyPS5, so the core itself reads the catalogs again on each change.
    let t = FakeHost::new()
        .plugin(offering_emulator(""))
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    // A folder where the meta file goes: the copy is written, its meta file is not.
    std::fs::create_dir_all(t.data_dir().join(format!("{KEPT}.meta.json"))).unwrap();
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    assert!(feed_asked(&t).await, "the refresh runs");
    let row = refresh_row(&t, "PS5: Failed").await;
    assert!(row.starts_with("PS5: Failed: could not keep the refreshed ps5 catalog"), "{row}");
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let asked = t.http.sent().iter().filter(|u| **u == feed_url()).count();
    assert_eq!(asked, 1, "no new refresh before the back-off ends: {:?}", t.http.sent());
}

#[tokio::test]
async fn a_new_base_copy_keeps_the_refreshed_releases_until_its_own_refresh_works() {
    // A failed refresh tries again after a second here, not five minutes.
    let plugin = release_catalog::ReleaseCatalog::default().retry_after(std::time::Duration::from_secs(1));
    let t = FakeHost::new().built_in(nexus_api::PluginByKind::service(plugin)).boot_core().await;
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    let mut events = t.core.events();
    list(&t).await;
    assert!(changed(&mut events).await, "the fetch of the catalog address ends");
    assert!(changed(&mut events).await, "the first refresh ends");
    let new_release = "ab12".repeat(10);

    // A new catalog address has a copy with Flux, and the forum list fails.
    let mut flux: serde_json::Value = serde_json::from_str(&base()).unwrap();
    flux["topics"].as_array_mut().unwrap().push(topic("6900006", "Flux", "PPSA06666", '6'));
    flux["topic_count"] = 6.into();
    t.http.serve("https://xyz.dev/new/ps5.json", flux.to_string());
    t.http.serve(&pvc_url(), b"{ not json".to_vec());
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/new", "sources_url": SOURCES,
                "refresh_from_forums": true }),
        })
        .await
        .unwrap();
    assert!(ids(&list(&t).await).contains(&new_release), "the refreshed releases stay");
    assert!(changed(&mut events).await, "the fetch of the new copy ends");
    assert!(changed(&mut events).await, "the refresh of the new copy fails");
    assert!(ids(&list(&t).await).contains(&new_release), "the refreshed releases stay");

    // The forum list answers again: the next refresh, after the back-off, is of the new copy.
    t.http.serve(&pvc_url(), PVC_ANSWER);
    list(&t).await;
    let mut again = false;
    for _ in 0..3 {
        again = changed(&mut events).await;
        if again {
            break;
        }
    }
    assert!(again, "the refresh of the new copy ends: {:?}", t.http.sent());
    let now = ids(&list(&t).await);
    assert!(now.contains(&new_release) && now.contains(&"6".repeat(40)), "{now:?}");
}

#[tokio::test]
async fn a_console_named_like_the_kept_catalog_sources_keeps_its_own_copy() {
    let t = boot(false).await;
    let (sections, _) = page(&t).await;
    assert_eq!(rows(&sections, "release-catalog:sources")[0], "List: fetched from xyz.dev");

    t.http.serve("https://xyz.dev/catalog/catalog-sources.json", base());
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    let console = nexus_api::Console::new("catalog-sources").unwrap();
    let releases = provided.releases.unwrap();
    // No bundled copy: the first ask gets none, and the fetch runs in the background.
    let mut list = None;
    for _ in 0..100 {
        list = releases.releases(&console).await.unwrap();
        if list.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(list.expect("its catalog").releases.len(), 5);
    let (sections, _) = page(&t).await;
    let sources = rows(&sections, "release-catalog:sources");
    assert_eq!(sources[..2], ["List: fetched from xyz.dev".to_owned(), "PS5: ps5".to_owned()]);
    assert_eq!(t.http.sent().iter().filter(|u| *u == SOURCES).count(), 1, "the kept list is still in use");
}

fn refresh_now() -> Command {
    Command::RunAction {
        plugin: id("release-catalog"),
        action: "refresh-now".into(),
        game: None,
        args: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn one_refresh_runs_at_a_time() {
    let t = boot(true).await;
    let mut events = t.core.events();
    let gate = t.http.hold_next(&pvc_url());
    list(&t).await;
    assert!(feed_asked(&t).await, "the first refresh runs");
    // Refresh now while the first refresh waits for the forum list.
    t.core.dispatch(refresh_now()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(t.http.sent().iter().filter(|u| **u == feed_url()).count(), 1, "no second refresh while one runs");
    gate.release();
    assert!(changed(&mut events).await, "the fetch of the catalog address ended before");
    assert!(changed(&mut events).await, "the first refresh ends");
    assert!(changed(&mut events).await, "then the second one ends");
    assert_eq!(t.http.sent().iter().filter(|u| **u == feed_url()).count(), 2);
}

#[tokio::test]
async fn the_meta_file_names_the_refreshed_copy_it_goes_with_and_older_copies_go() {
    let t = boot(true).await;
    let mut events = t.core.events();
    list(&t).await;
    assert!(changed(&mut events).await, "the fetch of the catalog address ends");
    assert!(changed(&mut events).await);
    for _ in 0..2 {
        t.core.dispatch(refresh_now()).await.unwrap();
        assert!(changed(&mut events).await);
    }
    let folder = t.data_dir().join("plugins/release-catalog/refreshed");
    let mut names: Vec<String> =
        std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names.len(), 3, "the meta file, the copy it names and the one before: {names:?}");
    let body = kept(&t, ".meta")["body"].as_str().unwrap().to_owned();
    assert_eq!(names.iter().filter(|n| **n != "ps5.meta.json").max(), Some(&body), "the newest copy");
    assert_eq!(kept(&t, "")["topic_count"], 5);
}

#[tokio::test]
async fn in_a_long_session_the_refresh_runs_again_once_the_copy_is_old_with_no_ask() {
    // A copy is fresh for a second here, not six hours.
    let plugin = release_catalog::ReleaseCatalog::fresh_for(std::time::Duration::from_secs(1));
    let t = FakeHost::new().built_in(nexus_api::PluginByKind::service(plugin)).boot_core().await;
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    let mut events = t.core.events();
    list(&t).await;
    assert!(changed(&mut events).await, "the fetch of the catalog address ends");
    assert!(changed(&mut events).await, "the first refresh ends");
    // The fetched copy also gets old each second, and its fetch publishes a change of its own:
    // wait for the second feed ask, not for a change.
    let mut again = false;
    for _ in 0..300 {
        again = t.http.sent().iter().filter(|u| **u == feed_url()).count() >= 2;
        if again {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(again, "the refresh ran again: {:?}", t.http.sent());
    assert_eq!(t.http.sent().iter().filter(|u| **u == feed_url()).count(), 2);
}

#[tokio::test]
async fn the_refresh_task_ends_when_the_plugin_is_turned_off() {
    // A copy is fresh for a second here, so the task would refresh each second.
    let plugin = release_catalog::ReleaseCatalog::fresh_for(std::time::Duration::from_secs(1));
    let t = FakeHost::new().built_in(nexus_api::PluginByKind::service(plugin)).boot_core().await;
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    // A reference to the catalogs that outlives the plugin.
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    let releases = provided.releases.unwrap();
    releases.releases(&nexus_api::Console::new("PS5").unwrap()).await.unwrap();
    assert!(feed_asked(&t).await, "the first refresh runs");

    t.core.kernel().deactivate(&id("release-catalog")).await.unwrap();
    let asked = t.http.sent().len();
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(t.http.sent().len(), asked, "nothing is fetched once the plugin is off: {:?}", t.http.sent());
    drop(releases);
}

/// The Game Catalog page's blocks, by their label (`release-catalog:refresh`), with their rows as
/// `label: value`, and its actions.
async fn page(t: &TestCore) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    t.core.dispatch(Command::Navigate(nexus_core::Route::Plugin(id("release-catalog")))).await.unwrap();
    let page = t.core.view().plugin_page.expect("the Game Catalog page");
    let sections = page
        .sections
        .iter()
        .filter(|s| s.kind == nexus_ui_api::SlotKind::Section)
        .map(|s| (s.label.clone(), s.rows.iter().map(|r| format!("{}: {}", r.label, r.value)).collect()))
        .collect();
    let actions = page.sections.iter().filter_map(|s| s.action.clone()).collect();
    (sections, actions)
}

fn rows<'a>(sections: &'a [(String, Vec<String>)], label: &str) -> &'a [String] {
    &sections.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label} in {sections:?}")).1
}

#[tokio::test]
async fn the_catalog_page_shows_the_sources_and_the_last_refresh() {
    let t = boot(true).await;
    list(&t).await;
    refresh_row(&t, "PS5: Refreshed").await;
    let (sections, actions) = page(&t).await;
    assert_eq!(rows(&sections, "release-catalog:sources")[0], "List: fetched from xyz.dev");
    let refresh = rows(&sections, "release-catalog:refresh");
    assert_eq!(refresh.len(), 1, "{refresh:?}");
    // A date to read, as "Oct 23, 2026 at 06:06 UTC", not an ISO time.
    let time = refresh[0].strip_prefix("PS5: Refreshed ").and_then(|r| r.split(": ").next()).unwrap_or_default();
    assert!(time.contains(", 20") && time.ends_with(" UTC") && !time.contains('-'), "{refresh:?}");
    assert!(refresh[0].ends_with(": 1 new, 1 removed"), "{refresh:?}");
    assert_eq!(actions, ["refresh-now"]);
}

#[tokio::test]
async fn with_the_refresh_off_the_page_says_so_and_has_no_action() {
    let t = boot(false).await;
    list(&t).await;
    let (sections, actions) = page(&t).await;
    assert_eq!(rows(&sections, "release-catalog:refresh"), ["Refresh from the forums: Off"]);
    assert!(actions.is_empty());
}

#[tokio::test]
async fn refresh_now_asks_the_forums_again_while_the_copy_is_fresh() {
    let t = boot(true).await;
    refreshed(&t).await;
    let action = Command::RunAction {
        plugin: id("release-catalog"),
        action: "refresh-now".into(),
        game: None,
        args: serde_json::Value::Null,
    };
    t.core.dispatch(action).await.unwrap();
    let twice = || t.http.sent().iter().filter(|u| **u == feed_url()).count() == 2;
    assert!(until(twice).await, "the feed was asked again: {:?}", t.http.sent());
}

#[tokio::test]
async fn a_failed_refresh_shows_its_reason() {
    let t = boot_with(true, "{ not json".to_owned()).await;
    list(&t).await;
    let row = refresh_row(&t, "PS5: Failed").await;
    assert!(row.starts_with(&format!("PS5: Failed: {}", pvc_url())), "{row}");
    assert!(row.ends_with("Not refreshed yet"), "{row}");
}

#[tokio::test]
async fn with_downloads_only_through_a_vpn_the_refresh_waits_for_the_vpn() {
    let t = boot(true).await;
    t.network.down("wg-se");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    assert_eq!(fetched(&t).await.releases.len(), 5, "the copy with no refresh");
    let row = refresh_row(&t, "PS5: Waiting for the VPN").await;
    assert!(row.starts_with("PS5: Waiting for the VPN"), "{row}");
    assert!(!t.http.sent().iter().any(|u| **u == feed_url() || *u == pvc_url()), "no forum is asked");

    // The VPN connects: Refresh now runs it.
    t.network.up("wg-se");
    let action = Command::RunAction {
        plugin: id("release-catalog"),
        action: "refresh-now".into(),
        game: None,
        args: serde_json::Value::Null,
    };
    t.core.dispatch(action).await.unwrap();
    assert!(feed_asked(&t).await, "the forums are asked once the VPN is there");
    let row = refresh_row(&t, "PS5: Refreshed").await;
    assert!(row.starts_with("PS5: Refreshed"), "{row}");
}

#[tokio::test]
async fn with_downloads_only_through_a_vpn_and_no_vpn_at_all_the_refresh_is_skipped() {
    let t = boot(true).await;
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    list(&t).await;
    let row = refresh_row(&t, "PS5: Skipped").await;
    assert!(row.starts_with("PS5: Skipped: there is no VPN"), "{row}");
    assert!(!t.http.sent().iter().any(|u| **u == feed_url() || *u == pvc_url()), "no forum is asked");
}

#[tokio::test]
async fn where_downloads_can_t_be_bound_to_a_vpn_the_page_says_why_the_refresh_ran_without_one() {
    let t = boot(true).await;
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    // The setting is on, and then this computer can't bind traffic to the VPN.
    t.network.unavailable("Not available on Windows");
    list(&t).await;
    let row = refresh_row(&t, "PS5: Refreshed").await;
    assert!(row.starts_with("PS5: Refreshed"), "{row}");
    assert!(row.ends_with(". Ran without the VPN: Not available on Windows"), "{row}");
}

/// A core with KytyPS5, so the core itself asks for the PS5 catalog, and "Download only through
/// a VPN" on while the VPN `wg-se` is down. Then the catalog is served with the refresh on or off.
async fn boot_waiting_for_the_vpn(refresh: bool) -> TestCore {
    let t = FakeHost::new()
        .plugin(offering_emulator(""))
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    t.network.down("wg-se");
    t.core.dispatch(Command::SetVpnOnly(true)).await.unwrap();
    serve(&t, refresh, PVC_ANSWER.to_owned()).await;
    t
}

/// Waits until the feed was asked, or a second passes. `true` when it was asked.
async fn feed_asked(t: &TestCore) -> bool {
    for _ in 0..100 {
        if t.http.sent().iter().any(|u| **u == feed_url()) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    false
}

/// The PS5 row of the "Catalog refresh" section, once it starts with `start`, or after a second.
async fn refresh_row(t: &TestCore, start: &str) -> String {
    let mut row = String::new();
    for _ in 0..100 {
        let (sections, _) = page(t).await;
        row = rows(&sections, "release-catalog:refresh")[0].clone();
        if row.starts_with(start) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    row
}

#[tokio::test]
async fn a_refresh_that_waits_for_the_vpn_runs_when_the_vpn_connects() {
    let t = boot_waiting_for_the_vpn(true).await;
    let row = refresh_row(&t, "PS5: Waiting for the VPN").await;
    assert!(row.starts_with("PS5: Waiting for the VPN"), "{row}");
    assert!(!t.http.sent().iter().any(|u| **u == feed_url() || *u == pvc_url()), "no forum is asked");

    // No command: the VPN check sees the VPN connect, and the core reads the catalogs again.
    t.network.up("wg-se");
    assert!(feed_asked(&t).await, "the forums are asked once the VPN is there: {:?}", t.http.sent());
    let row = refresh_row(&t, "PS5: Refreshed").await;
    assert!(row.starts_with("PS5: Refreshed"), "{row}");
}

#[tokio::test]
async fn with_the_refresh_off_the_vpn_connecting_asks_no_forum() {
    let t = boot_waiting_for_the_vpn(false).await;
    t.network.up("wg-se");
    assert!(!feed_asked(&t).await, "{:?}", t.http.sent());
}

/// The titles of the Catalog's games, sorted.
fn offers(t: &TestCore) -> Vec<String> {
    let mut titles: Vec<String> = t.core.view().catalog.games.games.iter().map(|g| g.title.clone()).collect();
    titles.sort();
    titles
}

/// The Catalog's games once there are `count`, or after a second.
async fn offers_once(t: &TestCore, count: usize) -> Vec<String> {
    for _ in 0..100 {
        if offers(t).len() == count {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    offers(t)
}

#[tokio::test]
async fn a_slow_forum_still_gives_the_catalog_at_once_and_the_refresh_comes_after() {
    let t = FakeHost::new()
        .plugin(offering_emulator(""))
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    // The grid is current while the Catalog is the screen.
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    // The forum list answers only when the test lets it, after the core's call timeout.
    let gate = t.http.hold_next(&pvc_url());
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    let fetched = offers_once(&t, 5).await;
    assert_eq!(fetched.len(), 5, "the catalog with no refresh, while the forum list is held: {fetched:?}");

    // Past the call timeout, the held refresh is no error.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(t.core.view().notices.is_empty(), "{:?}", t.core.view().notices);
    assert_eq!(offers(&t).len(), 5);

    // The refresh ends in the background, and the core reads the catalogs again.
    gate.release();
    let refreshed = offers_once(&t, 4).await;
    assert_eq!(refreshed.len(), 4, "Echo is gone from the forum list: {refreshed:?}");
}

const LINUX: &str = "https://xyz.net/forum/viewforum.php?f=1992";
const WINE: &str = "https://xyz.net/forum/viewforum.php?f=2059";
const PC_KEPT: &str = "plugins/release-catalog/refreshed/pc";

/// A PC topic of `forum`, with one seeder.
fn pc_topic(id: &str, name: &str, forum: &str, hash: char) -> serde_json::Value {
    serde_json::json!({ "id": id, "title": format!("{name} [Native]"), "size": "1.00 GB", "seeders": 1,
        "magnet": format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40)), "published_at": "2026-09-01",
        "source": forum, "platform": "PC", "system": "linux", "wine_packed": forum == WINE,
        "game_info": { "name": name, "genre": "Platformer" } })
}

/// A forum list with `seeders` for each topic.
fn pc_list(rows: &[(&str, u32, char)]) -> String {
    let result: serde_json::Map<String, serde_json::Value> = rows
        .iter()
        .map(|(id, seeders, hash)| ((*id).to_owned(), serde_json::json!([seeders, hash.to_string().repeat(40)])))
        .collect();
    serde_json::json!({ "format": { "topic_id": ["seeders", "info_hash"] }, "update_time": 0, "result": result })
        .to_string()
}

/// A core with the PC catalog of two forums, Linux and Wine, at the catalog address, the refresh on,
/// and each forum's feed and forum list. The Wine forum list is served only with `wine`.
async fn boot_pc(wine: bool) -> TestCore {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let sources = serde_json::json!({ "pc": { "platform": "PC", "forums": [
        { "key": "linux", "url": LINUX, "system": "linux" },
        { "key": "wine", "url": WINE, "system": "linux", "wine": true }] } });
    t.http.serve(SOURCES, sources.to_string());
    let topics = serde_json::json!([pc_topic("1", "Celeste", LINUX, 'a'), pc_topic("2", "Quake", WINE, 'b')]);
    let catalog = serde_json::json!({ "complete": true, "collected_at": "2026-10-01T00:00:00Z",
        "translation": { "language": "en" }, "topic_count": 2, "topics": topics });
    t.http.serve("https://xyz.dev/catalog/pc.json", catalog.to_string());
    let empty = "<feed></feed>";
    t.http.serve(&release_catalog::refresh::feed_url("1992"), empty);
    t.http.serve(&release_catalog::refresh::feed_url("2059"), empty);
    t.http.serve(&release_catalog::refresh::list_url("1992"), pc_list(&[("1", 7, 'a')]));
    if wine {
        t.http.serve(&release_catalog::refresh::list_url("2059"), pc_list(&[("2", 9, 'b')]));
    }
    t.core
        .dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/catalog", "sources_url": SOURCES,
                "refresh_from_forums": true }),
        })
        .await
        .unwrap();
    let provided = t.core.kernel().provided(&id("release-catalog")).await.unwrap();
    provided.releases.unwrap().releases(&nexus_api::Console::new("PC").unwrap()).await.unwrap();
    until(|| t.data_dir().join(format!("{PC_KEPT}.meta.json")).exists()).await;
    t
}

/// The seeders of each topic in the refreshed PC catalog, by id.
fn pc_seeders(t: &TestCore) -> Vec<(String, serde_json::Value)> {
    let read = |path: String| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(t.data_dir().join(path)).unwrap()).unwrap()
    };
    let meta = read(format!("{PC_KEPT}.meta.json"));
    let file =
        read(format!("plugins/release-catalog/refreshed/{}", meta["body"].as_str().expect("the meta names its copy")));
    file["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["id"].as_str().unwrap().to_owned(), t["seeders"].clone()))
        .collect()
}

#[tokio::test]
async fn each_forum_of_a_catalog_updates_its_own_topics() {
    let t = boot_pc(true).await;
    assert_eq!(pc_seeders(&t), [("1".to_owned(), 7.into()), ("2".to_owned(), 9.into())]);
}

#[tokio::test]
async fn a_forum_that_fails_leaves_the_other_forum_s_update_in_place_and_the_page_names_it() {
    let t = boot_pc(false).await;
    // Celeste's forum answered; Quake's forum list is not there, and Quake stays as it was.
    assert_eq!(pc_seeders(&t), [("1".to_owned(), 7.into()), ("2".to_owned(), 1.into())]);
    let (sections, _) = page(&t).await;
    let row = &rows(&sections, "release-catalog:refresh")[0];
    assert!(row.starts_with("PC: Refreshed"), "{row}");
    let missing = release_catalog::refresh::list_url("2059");
    assert!(row.contains(&format!("wine failed: HTTP 404 from {missing}")), "{row}");
}

/// The Catalog's notices, as (text, what clicking one opens).
fn notices(t: &TestCore) -> Vec<(String, String)> {
    t.core.view().notice_cards.into_iter().map(|n| (n.text, n.opens)).collect()
}

#[tokio::test]
async fn a_refresh_that_adds_releases_says_how_many_and_the_first_copy_says_nothing() {
    let t = FakeHost::new()
        .plugin(offering_emulator(""))
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    // The grid is current while the Catalog is the screen.
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    let gate = t.http.hold_next(&pvc_url());
    serve(&t, true, PVC_ANSWER.to_owned()).await;
    assert_eq!(offers_once(&t, 5).await.len(), 5);
    assert!(notices(&t).is_empty(), "the fetched copy takes the bundled one's place: {:?}", notices(&t));

    // The refresh adds Astro's new release, drops Echo's and puts Bolt's torrent uploaded again
    // in place of its old one: one new release.
    gate.release();
    assert!(until(|| !notices(&t).is_empty()).await, "a notice");
    assert_eq!(notices(&t), [("1 new release in the Catalog".to_owned(), "Open the Catalog".to_owned())]);
    let kind = t.core.view().notice_cards[0].kind;
    assert_eq!(kind, nexus_ui_api::NoticeKind::Info);
}

/// A core with a PS5 emulator, so the core asks for the PS5 catalog, on the Catalog, with the
/// refresh on or off.
async fn on_the_catalog(refresh: bool) -> TestCore {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-kyty", "PS5", vec![]).console("PS5"))
        .boot_core()
        .await;
    serve(&t, refresh, PVC_ANSWER.to_owned()).await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    t
}

/// The Catalog's buttons: their words and their action.
fn catalog_buttons(t: &TestCore) -> Vec<(String, String)> {
    let view = t.core.view().catalog;
    view.actions.iter().map(|a| (a.text.clone(), a.action.clone().unwrap_or_default())).collect()
}

/// What the Catalog says of its refresh.
fn catalog_status(t: &TestCore) -> Vec<String> {
    t.core.view().catalog.status.iter().map(|s| s.text.clone()).collect()
}

#[tokio::test]
async fn with_the_refresh_off_the_catalog_s_button_says_to_turn_it_on_in_settings_and_opens_them() {
    let t = on_the_catalog(false).await;
    assert_eq!(
        catalog_buttons(&t),
        [("Turn on Refresh from the forums in Settings".to_owned(), nexus_api::HOST_OPEN_SETTINGS.to_owned())]
    );
    assert!(catalog_status(&t).is_empty());
    let open = Command::RunAction {
        plugin: id("release-catalog"),
        action: nexus_api::HOST_OPEN_SETTINGS.into(),
        game: None,
        args: serde_json::Value::Null,
    };
    t.core.dispatch(open).await.unwrap();
    assert_eq!(t.core.view().screen, nexus_ui_api::ScreenId::Settings);
}

#[tokio::test]
async fn the_catalog_s_refresh_button_runs_refresh_now_and_says_refreshing_then_when_it_ran() {
    let t = on_the_catalog(true).await;
    let refreshed = || catalog_status(&t).first().is_some_and(|s| s.starts_with("Refreshed "));
    assert!(until(refreshed).await, "the first refresh ran: {:?}", catalog_status(&t));
    assert_eq!(catalog_buttons(&t), [("Refresh".to_owned(), "refresh-now".to_owned())]);

    let gate = t.http.hold_next(&pvc_url());
    t.core.dispatch(refresh_now()).await.unwrap();
    assert_eq!(catalog_status(&t), ["Refreshing…"]);
    gate.release();
    let refreshed = || catalog_status(&t).first().is_some_and(|s| s.starts_with("Refreshed "));
    assert!(until(refreshed).await, "it says when it ran: {:?}", catalog_status(&t));
}

#[tokio::test]
async fn the_catalog_says_why_its_refresh_did_not_run() {
    let t = FakeHost::new()
        .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        .plugin(games_on("emu-kyty", "PS5", vec![]).console("PS5"))
        .boot_core()
        .await;
    serve(&t, true, "{ not json".to_owned()).await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    let failed = || catalog_status(&t).first().is_some_and(|s| s.starts_with("PS5: Failed: "));
    assert!(until(failed).await, "{:?}", catalog_status(&t));
}
