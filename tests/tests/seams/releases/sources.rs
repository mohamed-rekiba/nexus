//! The sources list: bundled, fetched, and the forums it names.

use super::*;

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
