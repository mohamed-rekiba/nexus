//! The catalog's copies: bundled, fetched, kept for when a fetch fails, and their meta files.

use super::*;

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
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .plugin(FakeCatalog::new("emu-kyty").system("PS4").games([]).build().console("PS5"))
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
    t.clock.advance(std::time::Duration::from_millis(600)).await;
    assert!(t.core.view().notices.is_empty(), "{:?}", t.core.view().notices);

    // The fetch ends in the background, and the core reads the catalogs again.
    gate.release();
    let fetched = list_once(&t, |l| l.source == ReleaseSource::Fetched).await;
    assert_eq!(fetched.updated, "2026-10-18T00:00:00Z");
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    assert_eq!(offers_once(&t, &["release-catalog:emu-kyty/PPSA01234"]).await, ["release-catalog:emu-kyty/PPSA01234"]);
    assert_eq!(t.http.sent(), [URL], "fetched once");
}

/// The PS5 catalog once `done` says it is the one the test waits for, or after a second of the
/// clock: the Game Catalog fetches in its own task, which says only that the catalogs changed.
async fn list_once(t: &TestCore, done: impl Fn(&ReleaseList) -> bool) -> ReleaseList {
    for _ in 0..100 {
        let got = list(t).await;
        if got.as_ref().is_some_and(&done) {
            break;
        }
        t.clock.advance(std::time::Duration::from_millis(10)).await;
    }
    list(t).await.expect("a PS5 catalog")
}

/// The keys of the Catalog's offers, sorted, once they are `keys`.
async fn offers_once(t: &TestCore, keys: &[&str]) -> Vec<String> {
    t.until(Area::Catalogs, |_| offer_keys(t) == keys).await;
    offer_keys(t)
}

#[tokio::test]
async fn an_address_on_a_host_the_manifest_does_not_name_is_refused() {
    let t = boot().await;
    let refused = set_address(&t, "https://other.example/catalog").await.unwrap_err();
    assert!(refused.to_string().contains("other.example"), "{refused}");
    assert!(set_address(&t, "http://xyz.dev/catalog").await.is_err(), "https only");
}
