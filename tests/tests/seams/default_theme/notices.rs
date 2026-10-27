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
    host.settle();
    assert_eq!(host.values("notice-open"), ["Open the Game Hub"]);

    host.press("notice-open", "Open the Game Hub");
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
    host.wait(std::time::Duration::from_millis(100));
    assert_eq!(host.values("notice"), ["Couldn't read the folder"]);
    host.wait(std::time::Duration::from_millis(3000));
    assert_eq!(host.values("notice").len(), 1, "it stays long enough to read");
    // After 4 s it slides out, then goes.
    for _ in 0..3 {
        host.wait(std::time::Duration::from_millis(1000));
    }
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(host.values("notice").is_empty());

    // A new notice joins the others, and each goes 4 s after it came.
    host.core().report("First".into());
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    host.wait(std::time::Duration::from_millis(3000));
    host.core().report("Second".into());
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    assert_eq!(host.values("notice"), ["First", "Second"], "both show");
    for _ in 0..4 {
        host.wait(std::time::Duration::from_millis(500));
    }
    assert_eq!(host.values("notice"), ["Second"], "First went 4 s after it came");
    for _ in 0..3 {
        host.wait(std::time::Duration::from_millis(1000));
    }
    assert!(host.core().view().notices.is_empty());

    // A notice with something to open stays 10 s.
    let core = host.core().clone();
    assert!(
        host.rt.block_on(core.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())))).is_err()
    );
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    for _ in 0..8 {
        host.wait(std::time::Duration::from_millis(1000));
    }
    assert_eq!(host.values("notice-open"), ["Open the Game Hub"], "still there after 8 s");
    for _ in 0..3 {
        host.wait(std::time::Duration::from_millis(1000));
    }
    assert!(host.core().view().notices.is_empty(), "gone after 10 s");
}

#[test]
fn delete_closes_the_notice() {
    let mut host = default_theme();
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    assert_eq!(host.values("notice").len(), 1);
    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    host.wait(std::time::Duration::from_millis(1000));
    host.wait(std::time::Duration::from_millis(1000));
    assert!(host.core().view().notices.is_empty(), "dismissed");
    assert!(host.values("notice").is_empty());
}

#[test]
fn notices_stack_up_to_three_and_delete_closes_only_the_newest() {
    let mut host = default_theme();
    for text in ["First", "Second", "Third", "Fourth"] {
        host.core().report(text.into());
    }
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    assert_eq!(host.values("notice"), ["Second", "Third", "Fourth"], "the oldest goes");
    assert_eq!(host.values("notice-kind"), ["error", "error", "error"], "a problem is an error");

    let text: slint::SharedString = slint::platform::Key::Delete.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
    host.wait(std::time::Duration::from_millis(1000));
    host.wait(std::time::Duration::from_millis(1000));
    assert_eq!(host.core().view().notices, ["Second", "Third"], "only the newest closes");
    assert_eq!(host.values("notice"), ["Second", "Third"]);
}

#[test]
fn each_notice_keeps_its_own_time_when_an_older_one_goes() {
    let mut host = default_theme();
    host.core().report("First".into());
    host.ui.render().unwrap();
    host.wait(std::time::Duration::from_millis(100));
    for _ in 0..4 {
        host.wait(std::time::Duration::from_millis(500));
    }
    host.core().report("Second".into());
    host.ui.render().unwrap();
    // First goes 4 s after it came; Second stays its own 4 s.
    for _ in 0..7 {
        host.wait(std::time::Duration::from_millis(500));
    }
    assert_eq!(host.core().view().notices, ["Second"], "First went");
    assert_eq!(host.values("notice"), ["Second"]);
    for _ in 0..3 {
        host.wait(std::time::Duration::from_millis(500));
    }
    assert!(host.core().view().notices.is_empty(), "Second went 4 s after it came");
}
