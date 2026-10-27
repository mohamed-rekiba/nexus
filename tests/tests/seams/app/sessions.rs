//! Play sessions: launch, the window in front, stop, handed-over games and play time.

use super::*;

#[tokio::test]
async fn launching_spawns_the_program_from_the_plugin_folder_and_tracks_the_process() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    let plugin = id("emu-a");
    t.core
        .dispatch(Command::SetConfig {
            plugin: plugin.clone(),
            scope: ConfigScope::Emulator,
            value: serde_json::json!({"scale": 2}),
        })
        .await
        .unwrap();
    let mut events = t.core.events();

    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();

    let process = t.spawner.wait_for(1).await;
    assert_eq!(process.command.program, t.data_dir().join("plugins/emu-a/builds/1.0/emu"));
    assert_eq!(process.command.args, ["G0"]);
    assert_eq!(process.command.env.get("EMU_SCALE").map(String::as_str), Some("2"));
    assert!(matches!(events.next().await, Some(Event::ProcessStarted { game, .. }) if game == key("emu-a:G0")));
    assert_eq!(t.core.view().screen, ScreenId::Running);

    process.exit(0);
    loop {
        if let Event::ProcessExited { exit, .. } = events.next().await.expect("the events ended first") {
            assert!(exit.success);
            break;
        }
    }
    assert_eq!(emu.exits()[0].0, "G0", "the plugin hears about the exit");
    assert_ne!(t.core.view().screen, ScreenId::Running);
}

#[tokio::test]
async fn stop_kills_a_running_game() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    install(&t.core).await;
    let mut events = t.core.events();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let session = loop {
        if let Event::ProcessStarted { session, .. } = events.next().await.expect("the events ended first") {
            break session;
        }
    };

    t.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();

    loop {
        if let Event::ProcessExited { session: s, exit, .. } = events.next().await.expect("the events ended first") {
            assert_eq!(s, session);
            assert!(!exit.success);
            break;
        }
    }
}

#[tokio::test]
async fn a_started_game_comes_to_the_front_and_the_launcher_comes_back_when_it_closes() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;

    until(&t, "the game is in front", || t.core.game_in_front()).await;
    assert_eq!(t.windows.shown(), [Shown::Game(process.id)]);
    let running = t.core.view().running.unwrap();
    assert_eq!(running.state, "running", "its window showed");
    assert!(running.in_front);

    process.exit(0);
    closed(&mut events).await;

    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));
    assert!(!t.core.game_in_front());
    assert_eq!(process.signals(), [Signal::Kill], "what the game left running stops with it");
}

#[tokio::test]
async fn a_game_whose_window_closed_is_stopped_and_the_launcher_comes_back() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    // On macOS a program can keep running after its last window closed.
    t.windows.windows_end_games();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.windows.no_windows();
    closed(&mut events).await;

    assert_eq!(process.signals().first(), Some(&Signal::Terminate), "asked to close, as Stop does");
    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));
    assert!(t.core.view().notices.is_empty(), "the player closed it: no crash notice");
}

#[tokio::test]
async fn a_launcher_s_own_window_closing_before_the_game_s_opens_leaves_the_game_running() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the launcher's menu is in front", || t.core.game_in_front()).await;

    // The player picked "Start Game": the menu closed, and the game takes a while to open.
    t.windows.no_windows();
    t.clock.advance(Duration::from_millis(300)).await;
    assert!(process.signals().is_empty(), "nothing stopped it: {:?}", process.signals());
    assert!(t.core.view().running.is_some(), "the game keeps its session");
}

#[tokio::test]
async fn a_game_a_start_script_left_running_keeps_the_session_until_it_ends() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    // The script starts the game in the background and exits.
    let game = process.id + 1000;
    t.processes.add(process.id, game);
    process.exit(0);

    t.clock.advance(Duration::from_millis(200)).await;
    assert!(t.core.view().running.is_some(), "the game still runs");
    assert!(process.signals().is_empty(), "nothing was killed: {:?}", process.signals());

    t.processes.exit(game);
    let exit = closed(&mut events).await;
    assert!(exit.success, "the script's own exit");
}

#[tokio::test]
async fn the_window_in_front_is_the_game_s_when_a_process_below_it_owns_it() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;
    // A sandbox's process, out of the game's group, draws the game.
    let sandboxed = process.id + 1000;
    t.processes.add(process.id, sandboxed);

    t.windows.set_front(Front::Launcher);
    until(&t, "the launcher is in front", || !t.core.game_in_front()).await;
    t.windows.set_front(Front::Other(sandboxed));
    until(&t, "the sandboxed window counts as the game's", || t.core.game_in_front()).await;
}

#[tokio::test]
async fn stop_reaches_the_processes_a_sandbox_moved_out_of_the_game_s_group() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();
    let (session, process) = play(&t).await;
    let sandboxed = process.id + 1000;
    t.processes.add(process.id, sandboxed);
    t.processes.stubborn();

    t.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();
    closed(&mut events).await;

    let ended = t.processes.ended();
    assert!(ended.first().is_some_and(|(ids, force)| ids.contains(&sandboxed) && !force), "asked first: {ended:?}");
    assert!(ended.iter().any(|(ids, force)| ids.contains(&sandboxed) && *force), "then killed: {ended:?}");
}

/// The command line Steam gives game 7's first process.
const STEAM_GAME: &str = "reaper SteamLaunch AppId=7 -- /games/seven/run";

#[tokio::test]
async fn a_game_another_program_starts_runs_until_its_own_process_ends() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    let (_, launcher) = play(&t).await;
    // `steam steam://rungameid/7` hands the game to Steam and exits.
    launcher.exit(0);
    t.clock.advance(Duration::from_millis(100)).await;
    assert!(t.core.view().running.is_some(), "waiting for the game");
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    t.processes.add(game, game + 1);

    t.clock.advance(Duration::from_millis(700)).await;
    assert!(t.core.view().running.is_some(), "the game runs, past the time a game has to show");
    until(&t, "the game's window came to the front", || t.windows.shown().contains(&Shown::Game(game))).await;

    t.processes.exit(game + 1);
    t.processes.exit(game);
    let exit = closed(&mut events).await;
    assert!(exit.success, "{exit:?}");
    assert!(launcher.signals().is_empty(), "Steam was left alone: {:?}", launcher.signals());
}

#[tokio::test]
async fn stop_ends_the_game_another_program_started_and_leaves_that_program_running() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    // Steam wasn't running: the launch started Steam's client, which stays.
    let (session, steam) = play(&t).await;
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();
    closed(&mut events).await;
    assert_eq!(t.processes.ended().first(), Some(&(vec![game], false)), "the game was asked to close");
    assert!(steam.signals().is_empty(), "Steam's client was left running: {:?}", steam.signals());
}

#[tokio::test]
async fn a_game_another_program_never_starts_ends_with_a_notice() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"]).handing_over("AppId=7 ")).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let mut events = t.core.events();
    let (_, launcher) = play(&t).await;
    launcher.exit(0);

    let exit = closed(&mut events).await;
    assert!(!exit.success, "{exit:?}");
    let notices = t.core.view().notices;
    assert!(notices.iter().any(|n| n.contains("A didn't start")), "{notices:?}");
    assert_eq!(t.core.view().result_prompt, None, "nothing was played");
    assert_eq!(emu.exits().len(), 1, "the plugin hears of it");
}

#[tokio::test]
async fn stop_before_the_game_another_program_starts_shows_ends_with_nothing_played() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"]).handing_over("AppId=7 ")).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let mut events = t.core.events();
    let (session, launcher) = play(&t).await;
    // Steam is still starting, and the player gives up.
    t.clock.advance(Duration::from_millis(100)).await;
    t.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();

    closed(&mut events).await;
    let view = t.core.view();
    assert!(view.notices.is_empty(), "the player stopped it: {:?}", view.notices);
    assert_eq!(view.result_prompt, None, "nothing was played");
    let stats = t.core.kernel().config().get(&["app", "playtime", "emu-a:G0"]);
    assert!(stats.is_null(), "no play time: {stats}");
    assert!(launcher.signals().is_empty(), "Steam was left alone");
}

#[tokio::test]
async fn the_same_game_already_running_before_play_isn_t_taken_for_the_one_play_starts() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"]).handing_over("AppId=7 ")).await;
    let mut events = t.core.events();
    // Started from Steam itself, before Nexus.
    t.processes.start(8000, STEAM_GAME);
    let (_, launcher) = play(&t).await;
    launcher.exit(0);
    t.clock.advance(Duration::from_millis(100)).await;
    let game = 9000;
    t.processes.start(game, STEAM_GAME);
    until(&t, "the new game came to the front", || t.windows.shown().contains(&Shown::Game(game))).await;

    t.processes.exit(8000);
    t.clock.advance(Duration::from_millis(200)).await;
    assert!(t.core.view().running.is_some(), "the other one ending doesn't end this one");
    t.processes.exit(game);
    closed(&mut events).await;
}

#[tokio::test]
async fn only_the_newest_three_notices_stay_and_each_closes_on_its_own() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    for n in 1..=4 {
        t.core.report(format!("Problem {n}"));
    }
    let view = t.core.view();
    assert_eq!(view.notices, ["Problem 2", "Problem 3", "Problem 4"], "the oldest goes");
    assert!(view.notice_cards.iter().all(|n| n.kind == nexus_ui_api::NoticeKind::Error), "a problem is an error");
    let ids: Vec<u64> = view.notice_cards.iter().map(|n| n.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "newest last, each its own id: {ids:?}");

    t.core.dispatch(Command::Notice(nexus_core::NoticeCmd::Dismiss(ids[1]))).await.unwrap();
    assert_eq!(t.core.view().notices, ["Problem 2", "Problem 4"], "only that one closes");
    t.core.dispatch(Command::Notice(nexus_core::NoticeCmd::Open(ids[2]))).await.unwrap();
    assert_eq!(t.core.view().notices, ["Problem 2"], "opening one closes only it");
    assert_eq!(t.core.view().screen, ScreenId::Home, "it had nothing to open");
}

#[tokio::test]
async fn what_the_desktop_is_missing_is_said_once_a_run() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.windows.set_problem("Install xdotool so games come to the front and the launcher comes back");
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the notice shows", || !t.core.view().notices.is_empty()).await;
    assert_eq!(t.core.view().notices, ["Install xdotool so games come to the front and the launcher comes back"]);
    assert_eq!(t.core.view().notice_cards[0].kind, nexus_ui_api::NoticeKind::Info, "advice, not a failure");

    process.exit(0);
    closed(&mut events).await;
    t.core.dispatch(Command::Notice(nexus_core::NoticeCmd::DismissAll)).await.unwrap();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    t.spawner.wait_for(2).await;
    t.clock.advance(Duration::from_millis(100)).await;

    assert!(t.core.view().notices.is_empty(), "not again in the same run: {:?}", t.core.view().notices);
}

#[tokio::test]
async fn two_presses_of_play_start_one_game() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A", "B"])).await;
    install(&t.core).await;

    let (a, b) = tokio::join!(
        t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))),
        t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G1")))),
    );

    assert!(a.is_ok() != b.is_ok(), "one starts, one is refused: {a:?} {b:?}");
    t.spawner.wait_for(1).await;
    assert_eq!(t.spawner.spawned().len(), 1);
}

#[tokio::test]
async fn a_desktop_that_can_t_say_leaves_the_pad_to_the_game() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    // As on Windows: no window can be shown, and the front is unknown.
    t.windows.set_front(Front::Unknown);
    t.windows.no_windows();
    let (_, _process) = play(&t).await;

    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;
    assert!(t.core.game_in_front(), "the game's window took the screen as it opened");
    t.core.dispatch(Command::Session(SessionCmd::ToggleFocus)).await.unwrap();
    assert!(!t.core.game_in_front(), "the PS button gives the pad back");
}

#[tokio::test]
async fn a_game_whose_window_the_desktop_never_saw_is_not_taken_for_closed() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    // As on Wayland: xdotool sees no window of a game that draws to a Wayland surface.
    t.windows.set_front(Front::Unknown);
    t.windows.no_windows();
    let (_, process) = play(&t).await;
    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;

    // Longer than the window may be gone before the game counts as closed.
    t.clock.advance(Duration::from_millis(300)).await;
    assert!(process.signals().is_empty(), "the game keeps running: {:?}", process.signals());
    assert!(t.core.view().running.is_some(), "the game keeps running");
}

#[tokio::test]
async fn the_log_of_the_run_before_is_kept() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let log = t.data_dir().join("logs/emu-a/G0.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, "the crash").unwrap();

    let (_, _process) = play(&t).await;

    let previous = t.data_dir().join("logs/emu-a/G0.previous.log");
    assert_eq!(std::fs::read_to_string(previous).unwrap(), "the crash");
}

#[tokio::test]
async fn the_running_screen_says_starting_until_the_game_shows_a_window() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.windows.no_windows();
    let (_, _process) = play(&t).await;

    assert_eq!(t.core.view().running.unwrap().state, "starting");
    // The host's show_timeout is short in tests: the launcher stops waiting.
    until(&t, "the launcher stops waiting", || t.core.view().running.is_some_and(|r| r.state == "running")).await;
    assert!(t.windows.shown().is_empty());
    assert!(!t.core.game_in_front(), "no window came to the front");
}

#[tokio::test]
async fn the_launcher_stays_behind_a_closed_game_when_the_player_says_so() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Pref(Pref::ReturnOnExit, false.into())).await.unwrap();
    assert!(!t.core.view().settings.return_on_exit);
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    process.exit(0);
    closed(&mut events).await;

    assert!(!t.windows.shown().contains(&Shown::Launcher), "{:?}", t.windows.shown());
}

#[tokio::test]
async fn the_ps_button_goes_from_the_game_to_the_launcher_and_back() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (_, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    t.core.dispatch(Command::Session(SessionCmd::ToggleFocus)).await.unwrap();
    assert!(!t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Launcher));

    t.core.dispatch(Command::Session(SessionCmd::ToggleFocus)).await.unwrap();
    assert!(t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Game(process.id)));
}

#[tokio::test]
async fn the_desktop_says_which_window_is_in_front() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    let (session, process) = play(&t).await;
    until(&t, "the game is in front", || t.core.game_in_front()).await;

    // The player clicked the launcher's window.
    t.windows.set_front(Front::Launcher);
    until(&t, "the launcher is in front", || !t.core.game_in_front()).await;

    // Resume, from the Running screen.
    t.core.dispatch(Command::Session(SessionCmd::ShowGame(session))).await.unwrap();
    assert!(t.core.game_in_front());
    assert_eq!(t.windows.shown().last(), Some(&Shown::Game(process.id)));
}

#[tokio::test]
async fn without_the_desktop_s_answer_the_last_window_shown_counts() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.windows.set_front(Front::Unknown);
    let (_, _process) = play(&t).await;

    until(&t, "the game is in front", || t.core.game_in_front()).await;
    t.core.dispatch(Command::Session(SessionCmd::ToggleFocus)).await.unwrap();
    assert!(!t.core.game_in_front());
}

#[tokio::test]
async fn stop_asks_the_game_to_close_before_it_kills_it() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.spawner.ignore_terminate();
    let mut events = t.core.events();
    let (session, process) = play(&t).await;

    t.core.dispatch(Command::Session(SessionCmd::Stop(session))).await.unwrap();
    closed(&mut events).await;

    // Asked, killed when it ignored that, then what it left running swept up.
    assert_eq!(process.signals(), [Signal::Terminate, Signal::Kill, Signal::Kill]);
    assert!(t.core.view().notices.is_empty(), "the player stopped it: no crash notice");
}

#[tokio::test]
async fn a_game_that_closes_right_after_it_starts_leaves_a_notice_with_its_log() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let mut events = t.core.events();
    let (_, process) = play(&t).await;
    // Joined a part at a time, as the core does: the notice shows its text, and on Windows a
    // joined "logs/emu-a/G0.log" keeps its forward slashes.
    let log = t.data_dir().join("logs").join("emu-a").join("G0.log");
    assert_eq!(process.command.log.as_deref(), Some(log.as_path()), "its output goes to its log");

    process.exit(3);
    closed(&mut events).await;

    let notices = t.core.view().notices;
    assert_eq!(notices, [format!("A closed right after it started (code 3). Its log: {}", log.display())]);
}

#[tokio::test]
async fn one_game_runs_at_a_time() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A", "B"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    let (_, _process) = play(&t).await;

    let err = t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G1")))).await.unwrap_err();

    assert_eq!(err.to_string(), "A is running: stop it first");
    assert_eq!(t.spawner.spawned().len(), 1);
}

#[tokio::test]
async fn play_time_adds_up_and_the_game_page_shows_it() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    t.core.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await.unwrap();
    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!((detail.played_secs, detail.last_played_days), (0, None), "never played");
    let mut events = t.core.events();
    let (_, process) = play(&t).await;

    // A session shorter than three seconds doesn't count.
    t.clock.advance(Duration::from_millis(3100)).await;
    process.exit(0);
    closed(&mut events).await;

    t.core.dispatch(Command::OpenGame(key("emu-a:G0"))).await.unwrap();
    let detail = t.core.view().detail.unwrap();
    assert_eq!((detail.played_secs, detail.last_played_days), (3, Some(0)));
}

#[tokio::test]
async fn a_program_outside_the_process_allowlist_is_not_spawned() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 10]);
    // Same plugin, but its manifest lets it spawn nothing.
    let t = host.plugin(emu.plugin_with("emu-a", "network = [\"builds.example\"]")).boot_core().await;
    install(&t.core).await;

    let err = t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap_err();

    assert!(matches!(err, CoreError::PermissionDenied { .. }), "{err}");
    assert!(t.spawner.spawned().is_empty());
}

#[tokio::test]
async fn a_launcher_s_precondition_stops_the_launch_and_says_why() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A", "B"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 10]);
    emu.refuses = Some(("G1", "firmware missing"));
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;
    install(&t.core).await;

    let mut events = t.core.events();
    t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G0")))).await.unwrap();
    let process = t.spawner.wait_for(1).await;
    // One game runs at a time: close this one first.
    process.exit(0);
    while !matches!(events.next().await, Some(Event::ProcessExited { .. })) {}

    let err = t.core.dispatch(Command::Session(SessionCmd::Launch(key("emu-a:G1")))).await.unwrap_err();
    assert!(err.to_string().contains("firmware missing"), "{err}");
    assert_eq!(t.spawner.spawned().len(), 1);
    // The player sees why Play did nothing.
    assert_eq!(t.core.view().notices, ["Can't start G1: firmware missing"]);
}

#[tokio::test]
async fn the_game_watcher_reads_the_desktop_every_round_on_the_clock_and_not_between() {
    let (t, _) = core_with(FakeEmulator::new(vec!["A"])).await;
    play(&t).await;
    t.until(Area::Sessions, |core| core.game_in_front()).await;

    // A round just saw the game in front: the next is `watch_every` away.
    t.windows.set_front(Front::Launcher);
    t.clock.advance(WATCH_EVERY / 2).await;
    assert!(t.core.game_in_front(), "not read between two rounds");
    t.clock.advance(WATCH_EVERY / 2 + Duration::from_millis(3)).await;
    assert!(!t.core.game_in_front(), "the next round read the launcher in front");
}
