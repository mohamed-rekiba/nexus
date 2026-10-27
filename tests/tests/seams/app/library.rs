//! The library: catalogs merged, a broken one, search, and the first selection.

use super::*;

#[tokio::test]
async fn the_library_merges_every_catalog_sorted_by_title_with_badges() {
    let a = FakeEmulator::new(vec!["Zeta", "Alpha"]);
    let b = FakeEmulator::new(vec!["Mu"]);
    let t = FakeHost::new().plugin(a.plugin("emu-a")).plugin(b.plugin("emu-b")).boot_core().await;

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    let home = t.core.view().home;
    let titles: Vec<_> = home.games.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Alpha", "Mu", "Zeta"]);
    assert_eq!(home.games[0].key, "emu-a:G1");
    assert_eq!(home.games[1].key, "emu-b:G0");
    assert_eq!(home.games[0].badges[0].label, "emu-a:region");
    assert_eq!(home.games[0].badges[0].text, "EU");
    // No plugin here describes PS4: its code is its name, in the theme's neutral look.
    assert_eq!(home.games[0].system, nexus_ui_api::GameSystemVm { name: "PS4".into(), color: None });
}

#[tokio::test]
async fn a_broken_catalog_leaves_a_notice_and_the_other_catalogs_still_show() {
    let mut broken = FakeEmulator::new(vec!["Hidden"]);
    broken.broken_catalog = true;
    let ok = FakeEmulator::new(vec!["Shown"]);
    let t = FakeHost::new().plugin(broken.plugin("emu-a")).plugin(ok.plugin("emu-b")).boot_core().await;

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.games.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Shown"]);
    assert!(view.notices.iter().any(|n| n.contains("catalog server is down")), "{:?}", view.notices);
}

#[tokio::test]
async fn search_opens_the_catalog_on_its_text() {
    let a = FakeEmulator::new(vec!["The Last Road", "Road Rage"]);
    let t = FakeHost::new().plugin(a.plugin("emu-a")).boot_core().await;

    t.core.dispatch(Command::Search("road".into())).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::Catalog);
    assert_eq!(view.catalog.games.query, "road");
    assert_eq!(view.home.library.query, "", "the Library keeps its own search");
}

#[tokio::test]
async fn the_first_game_is_selected_with_its_details_once_the_library_loads() {
    let (t, _) = core_with(FakeEmulator::new(vec!["Beta", "Alpha"])).await;

    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    let view = t.core.view();
    assert_eq!(view.home.selected, Some(0), "the first game by title");
    assert_eq!(view.detail.map(|d| d.title), Some("Alpha".to_owned()), "its details are in, for the hero");
}

#[tokio::test]
async fn navigation_and_selection_live_in_the_core() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A", "B"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    t.core.dispatch(Command::Select(key("emu-a:G1"))).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G1"))).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::GameDetail);
    assert_eq!(view.detail.as_ref().unwrap().title, "B");
    assert!(view.can_go_back);

    t.core.dispatch(Command::Navigate(Route::Downloads)).await.unwrap();
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::GameDetail);
    t.core.dispatch(Command::Back).await.unwrap();
    let view = t.core.view();
    assert_eq!(view.screen, ScreenId::Home);
    assert_eq!(view.home.selected, Some(1));
    assert!(view.home.games[1].selected);
}
