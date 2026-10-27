//! Seam 1, core app API: a store plugin describes the games other plugins find.

use std::time::Duration;

use crate::common::*;
use nexus_core::Command;
use nexus_testkit::FakeHost;
use nexus_ui_api::ImageRef;

#[tokio::test]
async fn the_first_library_load_waits_for_the_stores_descriptions() {
    let t = FakeHost::new()
        .plugin(games("emu-a", vec!["Alpha", "Beta"]))
        .plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            Some("https://art.example/alpha.png".into()),
            Duration::from_millis(300),
        ))
        .boot_core()
        .await;

    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let view = t.core.view();
    assert!(view.boot.ready);
    let alpha = view.home.games.iter().find(|g| g.title == "Alpha").unwrap();
    assert_eq!(alpha.rating, Some(4.5), "described before the boot is ready");
    assert_eq!(
        alpha.cover,
        Some(ImageRef::Url { plugin: id("store"), url: "https://art.example/alpha.png".into() }),
        "the store's art in place of the plugin's"
    );
}

#[tokio::test]
async fn a_game_the_store_does_not_know_keeps_its_own_details() {
    let t = FakeHost::new()
        .plugin(games("emu-a", vec!["Alpha", "Beta"]))
        .plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            Some("https://art.example/alpha.png".into()),
            Duration::ZERO,
        ))
        .boot_core()
        .await;

    t.core.dispatch(Command::RefreshLibrary).await.unwrap();

    let view = t.core.view();
    let beta = view.home.games.iter().find(|g| g.title == "Beta").unwrap();
    assert_eq!(beta.rating, None);
    assert_eq!(beta.cover, None, "the plugin's own cover, which is none");
}

#[tokio::test]
async fn the_library_has_no_in_game_filter_and_sorts_by_rating_and_release() {
    let t = FakeHost::new()
        .plugin(games("emu-a", vec!["Alpha", "Beta", "Gamma"]))
        .plugin(store("store", vec![("CUSA00000", 3.0, ""), ("CUSA00001", 4.8, "")], None, Duration::ZERO))
        .boot_core()
        .await;
    t.core.dispatch(Command::RefreshLibrary).await.unwrap();
    let titles = |t: &nexus_testkit::TestCore| -> Vec<String> {
        t.core.view().home.library.games.iter().map(|g| g.title.clone()).collect()
    };

    // The player's own result: Beta reaches gameplay. The Library has no "In-game" chip, and
    // the filter, if it was on before, doesn't hide the player's games.
    t.core.kernel().config().set(&["app", "results", "emu-a:G1"], "ingame".into()).unwrap();
    t.core.dispatch(Command::LibrarySystem(Some(nexus_core::INGAME_FILTER.into()))).await.unwrap();
    let chips = t.core.view().home.library.systems;
    assert!(chips.iter().all(|f| f.id != nexus_core::INGAME_FILTER), "{chips:?}");
    assert_eq!(titles(&t).len(), 3);

    // Top rated first; a game the store doesn't rate goes last.
    t.core.dispatch(Command::LibrarySort("rating".into())).await.unwrap();
    assert_eq!(titles(&t), ["Beta", "Alpha", "Gamma"]);
    t.core.dispatch(Command::LibrarySort("compat".into())).await.unwrap();
    assert_eq!(titles(&t)[0], "Beta", "the game that runs best first");
}

#[tokio::test]
async fn a_slow_store_holds_the_first_load_ten_seconds_on_the_clock_and_no_more() {
    let t = FakeHost::new()
        // Longer than the boot's own limit, so that limit is the one that ends the wait.
        .call_timeout(Duration::from_secs(60))
        .plugin(games("emu-a", vec!["Alpha"]))
        .plugin(store("store", vec![("CUSA00000", 4.5, "")], None, Duration::from_secs(30)))
        .boot_core()
        .await;
    let core = t.core.clone();
    let load = tokio::spawn(async move { core.dispatch(Command::RefreshLibrary).await });

    t.clock.advance(Duration::from_secs(9)).await;
    assert!(!t.core.view().boot.ready, "the first load waits for the store");
    t.clock.advance(Duration::from_secs(2)).await;
    assert!(t.core.view().boot.ready, "but no more than ten seconds");
    load.await.unwrap().unwrap();
}
