//! Emulators: installing before the first play, updates, and their lists.

use super::*;

#[tokio::test]
async fn playing_a_game_whose_emulator_isn_t_installed_asks_first_then_installs_and_plays() {
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_size = Some(20_000);
    let (t, _) = core_with(emu).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!((prompt.emulator.as_str(), prompt.version.as_str()), ("emu-a", "1.0"));
    assert_eq!((prompt.game.as_str(), prompt.size_bytes), ("A", Some(20_000)));
    assert!(installed(&t.core).await.is_empty() && t.spawner.spawned().is_empty(), "nothing happens yet");

    t.core.dispatch(Command::ConfirmInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    let process = t.spawner.wait_for(1).await;
    assert!(process.command.program.ends_with("builds/1.0/emu"), "{:?}", process.command.program);
}

#[tokio::test]
async fn a_plugin_named_by_its_console_names_its_program_where_a_sentence_means_the_program() {
    let host = FakeHost::new().first_run();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    let names = "name = \"PS5\"\nemulator = \"KytyPS5\"";
    let t = host.plugin(emu.plugin_named("emu-a", names, FAKE_EMULATOR_PERMISSIONS)).boot_core().await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let steps: Vec<String> = t.core.view().boot.steps.into_iter().map(|s| s.label).collect();
    assert!(steps.contains(&"KytyPS5 emulator".to_owned()), "the first start's step: {steps:?}");
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.expect("the Game Hub");
    assert_eq!(detail.launch_hint, "Play installs KytyPS5 first");

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let prompt = t.core.view().install_prompt.expect("a prompt before anything downloads");
    assert_eq!(prompt.emulator, "KytyPS5", "the program downloads, not the console");
    t.core.dispatch(Command::CancelInstall).await.unwrap();

    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!((row(&t).name, row(&t).build_label), ("PS5".to_owned(), String::new()), "the list names the console");
    let builds = t.core.view().emulators.emulators.into_iter().find(|e| e.plugin == id("emu-a")).unwrap();
    assert_eq!(builds.name, "KytyPS5", "the builds are the program's");
    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!((row(&t).build, row(&t).build_label), ("1.0".to_owned(), "KytyPS5 1.0".to_owned()));

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.spawner.wait_for(1).await;
    let running = t.core.view().running.expect("the game runs");
    assert_eq!(
        (running.emulator.as_str(), running.page.as_str()),
        ("KytyPS5", "PS5"),
        "the keys' page is the plugin's"
    );
}

#[tokio::test]
async fn not_now_leaves_the_emulator_uninstalled() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.core.dispatch(Command::CancelInstall).await.unwrap();
    assert!(t.core.view().install_prompt.is_none());
    assert!(installed(&t.core).await.is_empty());
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_newer_build_installs_by_itself_when_no_game_runs() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()));
}

#[tokio::test]
async fn an_update_waits_until_the_emulator_s_game_closes() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"], "not while the game runs");

    process.exit(0);
    for _ in 0..100 {
        if installed(&t.core).await.contains(&"2.0".to_owned()) {
            return;
        }
        t.clock.advance(WATCH_EVERY).await;
    }
    panic!("the update didn't run after the game closed");
}

#[tokio::test]
async fn automatic_updates_can_be_turned_off() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    assert!(t.core.view().settings.auto_update, "on by default, as in the old launcher");
    t.core.dispatch(Command::Pref(Pref::AutoUpdate, false.into())).await.unwrap();
    emu.release("2.0");

    t.core.dispatch(Command::CheckUpdates).await.unwrap();
    assert_eq!(installed(&t.core).await, ["1.0"]);
    assert!(!t.core.view().settings.auto_update);
}

#[tokio::test]
async fn a_game_doesn_t_start_while_its_emulator_updates() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    emu.release("2.0");
    let gate = t.http.hold_after(&emu.build_url, 10);

    let core = t.core.clone();
    let update = tokio::spawn(async move { core.dispatch(Command::UpdateEmulator { plugin: id("emu-a") }).await });
    let busy = |core: &Core| core.view().emulators.emulators.iter().any(|e| e.plugin == id("emu-a") && e.busy);
    t.until(Area::Downloads, busy).await;
    let err = t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap_err();
    assert!(err.to_string().contains("updating"), "{err}");
    assert!(t.spawner.spawned().is_empty());

    gate.release();
    update.await.unwrap().unwrap();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    assert_eq!(t.spawner.wait_for(1).await.command.args, ["G0"]);
}

#[tokio::test]
async fn opening_another_plugin_replaces_the_open_one_so_back_goes_to_the_list() {
    let t = FakeHost::new()
        .plugin(FakeEmulator::new(vec!["A"]).plugin("emu-a"))
        .plugin(FakeEmulator::new(vec!["B"]).plugin("emu-b"))
        .boot_core()
        .await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-a")))).await.unwrap();
    t.core.dispatch(Command::Navigate(Route::Plugin(id("emu-b")))).await.unwrap();
    assert_eq!(t.core.view().plugin_page.map(|p| p.plugin.id), Some(id("emu-b")));

    t.core.dispatch(Command::Back).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn emulators_and_themes_each_have_their_own_list() {
    let t = FakeHost::new().plugin(FakeEmulator::new(vec!["A"]).plugin("emu-a")).boot_core().await;
    t.core.dispatch(Command::Navigate(Route::Themes)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Themes);
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(t.core.view().screen, ScreenId::Emulators);
}

#[tokio::test]
async fn an_emulator_s_row_shows_its_installed_build_not_the_plugin_s_version() {
    let (t, _emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let row = |t: &TestCore| t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("emu-a")).unwrap();
    assert_eq!(row(&t).build, "", "not installed yet");

    install(&t.core).await;
    t.core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    assert_eq!(row(&t).build, "1.0");
}

#[tokio::test]
async fn emulator_updates_are_looked_for_a_minute_after_the_start_on_the_clock() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let started = t.clock.now();
    install(&t.core).await;
    emu.release("2.0");

    t.clock.advance(Duration::from_secs(59) - (t.clock.now() - started)).await;
    assert_eq!(installed(&t.core).await, ["1.0"], "not before the minute");
    t.clock.advance(Duration::from_secs(2)).await;
    assert!(installed(&t.core).await.contains(&"2.0".to_owned()), "the check found and installed 2.0");
}
