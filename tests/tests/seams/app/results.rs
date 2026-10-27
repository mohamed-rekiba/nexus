//! After a run: how far the game got, asked once for each build.

use super::*;

#[tokio::test]
async fn after_playing_the_player_says_how_far_it_got_and_it_is_kept() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"));

    t.core.dispatch(Command::ReportResult(Some(nexus_api::CompatLevel::Menus))).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus");
}

#[tokio::test]
async fn not_now_keeps_no_result() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    process.exit(0);
    closed(&mut events).await;

    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    assert_eq!(t.core.view().result_prompt, None);
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "");
}

/// Plays `emu-a:G0` once more, as the `n`th run, and waits until it closes.
async fn play_run(t: &TestCore, n: usize) {
    let mut events = t.core.events();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.spawner.wait_for(n).await.exit(0);
    closed(&mut events).await;
}

#[tokio::test]
async fn how_far_it_got_is_asked_once_for_each_build() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(Some(nexus_api::CompatLevel::Menus))).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "this build has a result");

    emu.release("2.0");
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().detail.unwrap().your_result, "menus", "the old result stays until then");
}

#[tokio::test]
async fn not_now_is_kept_for_the_build() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    install(&t.core).await;
    play_run(&t, 1).await;
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    play_run(&t, 2).await;
    assert_eq!(t.core.view().result_prompt, None, "not again for this build");
    t.core.dispatch(Command::AskResult(key("emu-a:G0"))).await.unwrap();
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "Rate how it runs still asks");
    t.core.dispatch(Command::ReportResult(None)).await.unwrap();

    emu.release("2.0");
    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    play_run(&t, 3).await;
    assert_eq!(t.core.view().result_prompt.as_deref(), Some("A"), "a new build asks again");
}
