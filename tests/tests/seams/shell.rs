//! The app shell the app ships, on a fake platform and the core's fake clock: render
//! coalescing, the tick, and the requests only the app carries out.

use std::time::Duration;

use nexus_core::{Area, Command, LibraryCmd, Pref, SessionCmd};
use nexus_testkit::PlatformCall;
use nexus_ui_host::{RENDER_GAP, TICK, TrailerPoll, TrailerRequest, TrailerStatus, UiRequest};

use crate::common::*;

/// A shell showing the default theme's Home, booted, with one render behind it.
fn shown() -> Composed {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().games(["Alpha"]).build()));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(LibraryCmd::Refresh));
    host.render().unwrap();
    assert!(!host.ui.waiting_for_art(), "Home has no art to wait for");
    host.platform.clear_wakes();
    host
}

#[test]
fn a_burst_of_core_events_renders_once_when_its_time_comes() {
    let mut host = shown();
    let before = host.renders();
    for _ in 0..20 {
        host.on_core_changed();
    }
    assert_eq!(host.platform.wakes(), [RENDER_GAP], "one wake, a frame after the last render");
    host.poll();
    assert_eq!(host.renders(), before, "the last render was just now");

    host.advance(RENDER_GAP);
    host.poll();
    assert_eq!(host.renders(), before + 1);
    host.poll();
    assert_eq!(host.renders(), before + 1, "once for the whole burst");

    host.on_core_changed();
    assert_eq!(host.platform.wakes().last(), Some(&RENDER_GAP), "the next change waits for its frame");
}

#[test]
fn a_change_long_after_the_last_render_renders_on_the_next_poll() {
    let mut host = shown();
    host.advance(Duration::from_secs(1));
    let before = host.renders();
    host.on_core_changed();
    assert_eq!(host.platform.wakes(), [Duration::ZERO]);
    host.poll();
    assert_eq!(host.renders(), before + 1);
}

#[test]
fn the_tick_re_renders_every_15_seconds_while_a_game_runs() {
    let mut host = shown();
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    host.on_tick();
    let idle = host.renders();
    host.on_tick();
    assert_eq!(host.renders(), idle, "nothing changed, no game: no render");

    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.until(Area::Sessions, |core| core.game_running());
    host.on_tick();
    let playing = host.renders();
    for _ in 0..59 {
        host.advance(TICK);
        host.on_tick();
    }
    assert_eq!(host.renders(), playing, "not before 15 s");
    host.advance(TICK);
    host.on_tick();
    assert_eq!(host.renders(), playing + 1, "the time played moves on");
}

#[test]
fn the_clock_s_minute_renders_once() {
    let mut host = shown();
    host.platform.set_local_time("9:38 PM");
    host.on_tick();
    let before = host.renders();
    host.on_tick();
    assert_eq!(host.renders(), before);
    host.platform.set_local_time("9:39 PM");
    host.on_tick();
    assert_eq!(host.renders(), before + 1);
    assert_eq!(host.ui.clock(), "9:39 PM");
}

#[test]
fn the_window_shown_line_goes_out_once() {
    let mut host = shown();
    host.on_tick();
    host.on_tick();
    let lines: Vec<_> = host.platform.calls().into_iter().filter(|c| matches!(c, PlatformCall::Announce(_))).collect();
    assert_eq!(lines, [PlatformCall::Announce("window shown".into())]);
}

#[test]
fn the_window_is_claimed_after_the_announcement_and_on_each_tick_until_the_platform_is_done() {
    let mut host = shown();
    host.platform.refuse_claims(2);
    for _ in 0..4 {
        host.on_tick();
    }
    let session_calls: Vec<_> = host
        .platform
        .calls()
        .into_iter()
        .filter(|c| matches!(c, PlatformCall::Announce(_) | PlatformCall::ClaimWindow))
        .collect();
    assert_eq!(
        session_calls,
        [
            PlatformCall::Announce("window shown".into()),
            PlatformCall::ClaimWindow,
            PlatformCall::ClaimWindow,
            PlatformCall::ClaimWindow,
        ],
        "the line first; two refusals ask again on the next tick; the third claim is the last"
    );
}

#[test]
fn log_out_tells_the_session_then_quits() {
    let mut host = shown();
    host.on_request(UiRequest::LogOut).unwrap();
    assert_eq!(host.platform.calls(), [PlatformCall::Announce("log out".into()), PlatformCall::Quit]);
}

#[test]
fn the_theme_s_requests_wait_for_the_next_poll() {
    let mut host = shown();
    host.requests.borrow_mut().push(UiRequest::OpenUrl("https://art.example/".into()));
    host.requests.borrow_mut().push(UiRequest::Restart);
    assert!(host.platform.calls().is_empty());
    host.poll();
    assert_eq!(host.platform.calls(), [PlatformCall::OpenUrl("https://art.example/".into()), PlatformCall::Restart]);
    assert!(host.requests.borrow().is_empty());
}

#[test]
fn a_theme_command_runs_on_the_core_off_the_ui_thread() {
    let mut host = shown();
    host.on_request(UiRequest::Core(Command::Navigate(nexus_core::Route::Settings))).unwrap();
    assert_eq!(host.platform.waiting_work(), 1, "started, not run");
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Home);
    host.run_work();
    assert_eq!(host.core().view().screen, nexus_ui_api::ScreenId::Settings);
}

#[test]
fn a_trailer_that_can_t_play_in_the_window_opens_in_the_browser_and_says_why() {
    let mut host = shown();
    host.platform.refuse_trailers("Trailers need libmpv");
    host.on_request(UiRequest::Trailer(TrailerRequest::Play("https://art.example/a.mp4".into()))).unwrap();
    assert_eq!(
        host.platform.calls(),
        [
            PlatformCall::PlayTrailer("https://art.example/a.mp4".into()),
            PlatformCall::OpenUrl("https://art.example/a.mp4".into())
        ]
    );
    let notices = host.core().view().notices;
    assert!(notices.iter().any(|n| n == "The trailer opened in your browser: Trailers need libmpv"), "{notices:?}");
}

#[test]
fn the_tick_follows_the_trailer_and_closes_it_at_its_end() {
    let mut host = shown();
    host.on_request(UiRequest::Trailer(TrailerRequest::Play("https://art.example/a.mp4".into()))).unwrap();
    assert_eq!(host.ui.trailer().map(|t| t.loading), Some(true), "opening");
    let playing = TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 };
    host.platform.set_trailer(Some(TrailerPoll::Playing(playing)));
    host.on_tick();
    assert_eq!(host.ui.trailer().map(|t| t.position), Some(42.0));

    host.platform.set_trailer(Some(TrailerPoll::Ended));
    host.on_tick();
    assert_eq!(host.platform.calls().last(), Some(&PlatformCall::CloseTrailer));
    assert_eq!(host.ui.trailer(), None);
}

#[test]
fn the_window_moves_once_to_the_monitor_picked_in_settings() {
    let mut host = shown();
    host.platform.set_monitors(&["DP-1", "DP-2"]);
    host.on_tick();
    assert!(!host.platform.calls().iter().any(|c| matches!(c, PlatformCall::MoveToMonitor { .. })), "none picked");
    host.dispatch(Command::Pref(Pref::Monitor, "DP-2".into()));
    host.on_tick();
    host.on_tick();
    let moves: Vec<_> =
        host.platform.calls().into_iter().filter(|c| matches!(c, PlatformCall::MoveToMonitor { .. })).collect();
    assert_eq!(moves, [PlatformCall::MoveToMonitor { monitor: "DP-2".into(), fullscreen: true }]);
}
