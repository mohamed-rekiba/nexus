//! Seam 1, core app API: the game files. Each game's folder gets `nexus_game.json` and its art
//! in `nexus_art/`; the Library reads them before it asks any store.

use std::path::Path;

use crate::common::*;
use nexus_core::{Area, Command};
use nexus_testkit::{FakeHost, TestCore};
use nexus_ui_api::ImageRef;

const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 7, 7, 7];
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nshot";

/// Alpha and Beta, each a folder in a games folder of their own. The store knows Alpha
/// (`CUSA00000`), with its cover and two screenshots on the network.
async fn games(store_knows_alpha: bool) -> (TestCore, tempfile::TempDir) {
    let host = FakeHost::new();
    host.http().serve("https://art.example/alpha.jpg", JPG);
    host.http().serve("https://art.example/1.png", PNG);
    host.http().serve("https://art.example/2.png", PNG);
    let mut store = FakeStore::new("store").icon("https://art.example/alpha.jpg");
    if store_knows_alpha {
        store = store.knows("CUSA00000", 4.5, "Build anything");
    }
    let t = host
        .plugin(FakeCatalog::new("emu-a").system("PS4").in_folders().games(["Alpha", "Beta"]).build())
        .plugin(store.build())
        .boot_core()
        .await;
    let dir = tempfile::tempdir().unwrap();
    for game in ["Alpha", "Beta"] {
        std::fs::create_dir(dir.path().join(game)).unwrap();
    }
    t.core.dispatch(Command::SetFolders { name: "games".into(), paths: vec![dir.path().into()] }).await.unwrap();
    (t, dir)
}

fn game_file(dir: &Path) -> Option<serde_json::Value> {
    Some(serde_json::from_slice(&std::fs::read(dir.join("nexus_game.json")).ok()?).unwrap())
}

#[tokio::test]
async fn every_game_the_library_knows_gets_its_file_and_its_art_in_its_folder() {
    let (t, dir) = games(true).await;

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let alpha = dir.path().join("Alpha");
    t.until(Area::Library, |_| alpha.join("nexus_game.json").exists()).await;

    let file = game_file(&alpha).unwrap();
    assert_eq!(file["format"], 1);
    assert_eq!(file["title"], "Alpha");
    assert_eq!(file["title_id"], "CUSA00000");
    assert_eq!(file["publisher"], "MOJANG AB");
    assert_eq!(file["genres"], serde_json::json!(["Adventure", "Arcade"]));
    assert_eq!(file["rating"]["score"], 4.5);
    assert_eq!(file["art"]["cover"], "nexus_art/cover.jpg");
    assert_eq!(
        file["art"]["screenshots"],
        serde_json::json!(["nexus_art/screenshot_1.png", "nexus_art/screenshot_2.png"])
    );
    assert_eq!(std::fs::read(alpha.join("nexus_art/cover.jpg")).unwrap(), JPG);
    assert_eq!(std::fs::read(alpha.join("nexus_art/screenshot_2.png")).unwrap(), PNG);
    let names: Vec<String> =
        std::fs::read_dir(&alpha).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert!(names.iter().all(|n| n == "nexus_game.json" || n == "nexus_art"), "nothing else: {names:?}");

    let card = t.core.view().home.games.into_iter().find(|g| g.title == "Alpha").unwrap();
    assert!(
        matches!(&card.cover, Some(ImageRef::Folder { path, .. }) if path.path.as_str() == "Alpha/nexus_art/cover.jpg"),
        "the card shows the art in the game's folder: {:?}",
        card.cover
    );
    assert!(game_file(&dir.path().join("Beta")).is_none(), "nothing is known of Beta beyond its listing");
}

#[tokio::test]
async fn a_game_with_its_file_shows_whole_with_no_store_and_no_network() {
    let (t, dir) = games(false).await;
    let alpha = dir.path().join("Alpha");
    std::fs::create_dir(alpha.join("nexus_art")).unwrap();
    std::fs::write(alpha.join("nexus_art/cover.jpg"), JPG).unwrap();
    let file = serde_json::json!({
        "format": 1,
        "title": "Alpha",
        "publisher": "From The File",
        "genres": ["Racing"],
        "rating": { "score": 3.5, "total": 10 },
        "art": { "cover": "nexus_art/cover.jpg" }
    });
    std::fs::write(alpha.join("nexus_game.json"), file.to_string()).unwrap();

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.until(Area::Library, |core| core.view().home.games.iter().any(|g| g.title == "Alpha" && g.rating == Some(3.5)))
        .await;

    let card = t.core.view().home.games.into_iter().find(|g| g.title == "Alpha").unwrap();
    assert!(
        matches!(&card.cover, Some(ImageRef::Folder { path, .. }) if path.path.as_str() == "Alpha/nexus_art/cover.jpg")
    );
    t.core.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap())).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.genres, ["Racing"]);
    assert!(
        t.http.requests().iter().all(|(url, _)| !url.contains("art.example")),
        "no picture came from the network: {:?}",
        t.http.requests()
    );
}

#[tokio::test]
async fn a_game_file_on_disk_is_never_written_over() {
    let (t, dir) = games(true).await;
    let alpha = dir.path().join("Alpha");
    let own = r#"{"format": 1, "title": "Alpha", "publisher": "Mine"}"#;
    std::fs::write(alpha.join("nexus_game.json"), own).unwrap();

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.clock.run_until_idle().await;

    assert_eq!(std::fs::read_to_string(alpha.join("nexus_game.json")).unwrap(), own);
    assert!(!alpha.join("nexus_art").exists());
}
