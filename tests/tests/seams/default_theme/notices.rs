//! Notices: how long each stays, and how they stack and close.

use super::*;

#[test]
fn a_notice_opens_what_it_is_about() {
    // A game whose plugin can't start games: Play leaves a notice about it.
    let mut host = default_theme();
    let core = host.core().clone();
    let failed = host.rt.block_on(core.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap()))));
    assert!(failed.is_err());
    host.ui.render().unwrap();
    settle(&mut host);
    assert_eq!(values(&host, "notice-open"), ["Open the Game Hub"]);

    press(&mut host, "notice-open", "Open the Game Hub");
    let view = host.core().view();
    assert_eq!(view.screen, nexus_ui_api::ScreenId::GameDetail);
    assert_eq!(view.game.as_deref(), Some("emu-a:G0"));
    assert!(view.notices.is_empty(), "and the notice goes");
}

#[test]
fn a_notice_goes_after_a_while_and_one_with_a_link_stays_longer() {
    let mut host = default_theme();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["Couldn't read the folder"]);
    wait(&mut host, 3000);
    assert_eq!(values(&host, "notice").len(), 1, "it stays long enough to read");
    // After 4 s it slides out, then goes.
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(values(&host, "notice").is_empty());

    // A new notice joins the others, and each goes 4 s after it came.
    host.core().report("First".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    wait(&mut host, 3000);
    host.core().report("Second".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["First", "Second"], "both show");
    for _ in 0..4 {
        wait(&mut host, 500);
    }
    assert_eq!(values(&host, "notice"), ["Second"], "First went 4 s after it came");
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty());

    // A notice with something to open stays 10 s.
    let core = host.core().clone();
    assert!(
        host.rt.block_on(core.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())))).is_err()
    );
    host.ui.render().unwrap();
    wait(&mut host, 100);
    for _ in 0..8 {
        wait(&mut host, 1000);
    }
    assert_eq!(values(&host, "notice-open"), ["Open the Game Hub"], "still there after 8 s");
    for _ in 0..3 {
        wait(&mut host, 1000);
    }
    assert!(host.core().view().notices.is_empty(), "gone after 10 s");
}

#[test]
fn delete_closes_the_notice() {
    let mut host = default_theme();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice").len(), 1);
    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    wait(&mut host, 1000);
    wait(&mut host, 1000);
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(values(&host, "notice").is_empty());
}

#[test]
fn notices_stack_up_to_three_and_delete_closes_only_the_newest() {
    let mut host = default_theme();
    for text in ["First", "Second", "Third", "Fourth"] {
        host.core().report(text.into());
    }
    host.ui.render().unwrap();
    wait(&mut host, 100);
    assert_eq!(values(&host, "notice"), ["Second", "Third", "Fourth"], "the oldest goes");
    assert_eq!(values(&host, "notice-kind"), ["error", "error", "error"], "a problem is an error");

    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    wait(&mut host, 1000);
    wait(&mut host, 1000);
    assert_eq!(host.core().view().notices, ["Second", "Third"], "only the newest closes");
    assert_eq!(values(&host, "notice"), ["Second", "Third"]);
}

#[test]
fn each_notice_keeps_its_own_time_when_an_older_one_goes() {
    let mut host = default_theme();
    host.core().report("First".into());
    host.ui.render().unwrap();
    wait(&mut host, 100);
    for _ in 0..4 {
        wait(&mut host, 500);
    }
    host.core().report("Second".into());
    host.ui.render().unwrap();
    // First goes 4 s after it came; Second stays its own 4 s.
    for _ in 0..7 {
        wait(&mut host, 500);
    }
    assert_eq!(host.core().view().notices, ["Second"], "First went");
    assert_eq!(values(&host, "notice"), ["Second"]);
    for _ in 0..3 {
        wait(&mut host, 500);
    }
    assert!(host.core().view().notices.is_empty(), "Second went 4 s after it came");
}
