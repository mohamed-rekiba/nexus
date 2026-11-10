//! Navigation: the stack of screens, each with the game selected on it. A running game is no
//! screen: starting or ending one leaves the screens as they are.

use super::*;

/// The emulator with three games, installed, its library read.
async fn three_games() -> TestCore {
    let (t, _) = core_with(FakeEmulator::new(vec!["A", "B", "C"])).await;
    install(&t.core).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t
}

/// The title of the game selected in the Library's grid.
fn library_selected(t: &TestCore) -> Option<String> {
    let grid = t.core.view().home.library;
    grid.selected.map(|i| grid.games[i].title.clone())
}

#[tokio::test]
async fn a_game_played_from_its_game_hub_ends_on_the_hub_and_back_comes_to_the_library_on_it() {
    let t = three_games().await;
    let mut events = t.core.events();
    t.core.dispatch(Command::Navigate(Route::Library)).await.unwrap();
    t.core.dispatch(Command::Select(key("emu-a:G0"))).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G1"))).await.unwrap();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G1")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    assert_eq!(t.core.view().screen, ScreenId::GameDetail, "the Hub stays, with Resume and Stop");
    assert!(t.core.view().detail.unwrap().running);
    process.exit(0);
    closed(&mut events).await;
    assert_eq!(t.core.view().screen, ScreenId::GameDetail, "still the Hub once it ends");

    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Library);
    assert_eq!(library_selected(&t).as_deref(), Some("B"), "on the game whose Hub it was");
    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Home);
    assert!(!t.core.view().can_go_back);
}

#[tokio::test]
async fn a_game_played_from_a_grid_leaves_the_grid_on_screen() {
    let t = three_games().await;
    let mut events = t.core.events();
    t.core.dispatch(Command::Navigate(Route::Library)).await.unwrap();
    t.core.dispatch(Command::Select(key("emu-a:G2"))).await.unwrap();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G2")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    assert_eq!(t.core.view().screen, ScreenId::Library);
    process.exit(0);
    closed(&mut events).await;

    assert_eq!(t.core.view().screen, ScreenId::Library);
    assert_eq!(library_selected(&t).as_deref(), Some("C"));
}

#[tokio::test]
async fn home_and_the_library_each_keep_their_own_selection() {
    let t = three_games().await;
    t.core.dispatch(Command::Select(key("emu-a:G1"))).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Library)).await.unwrap();
    assert_eq!(library_selected(&t).as_deref(), Some("B"), "the Library opens on Home's game");
    t.core.dispatch(Command::Select(key("emu-a:G2"))).await.unwrap();

    t.core.dispatch(Command::Back).await.unwrap();

    let home = t.core.view().home;
    assert_eq!(home.selected.map(|i| home.games[i].title.clone()).as_deref(), Some("B"));
}

#[tokio::test]
async fn home_from_any_screen_goes_back_to_the_bottom() {
    let t = three_games().await;
    t.core.dispatch(Command::Navigate(Route::Library)).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Settings)).await.unwrap();

    t.core.dispatch(Command::Navigate(Route::Home)).await.unwrap();

    assert_eq!(t.core.view().screen, ScreenId::Home);
    assert!(!t.core.view().can_go_back);
}

#[tokio::test]
async fn a_running_game_brings_its_own_art_whatever_is_selected() {
    let t = three_games().await;
    t.core.dispatch(Command::Select(key("emu-a:G0"))).await.unwrap();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G2")))).await.unwrap();
    t.spawner.wait_for(1).await;

    let view = t.core.view();
    let running = view.running.expect("a running game");
    assert_eq!(running.title, "C");
    assert!(running.cover.is_some(), "its cover, for the launch splash");
    assert_eq!(running.cover, view.home.games[2].cover, "C's, not the selected A's");
}
