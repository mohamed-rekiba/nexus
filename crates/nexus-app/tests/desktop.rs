//! The desktop adapter against the system's real windows (the fourth seam in
//! docs/architecture.md). Opt in with `NEXUS_DESKTOP_TESTS=1`: the tests open windows and
//! change which one is in front. With `NEXUS_DESKTOP_REQUIRE=1` too, a skip fails the run:
//! CI sets it where the runner can do everything (Linux under Xvfb).
//!
//! The helper windows are this program, started again with `NEXUS_DESKTOP_HELPER_HIDE_MS`:
//! a window must run on a process's main thread, so this test has its own `main`. A system
//! that doesn't say which window is in front (macOS without the Accessibility permission)
//! skips, and says why.
//!
//! Start at login changes the real entry for this user (the autostart file, the launch agent
//! or the registry's `Run` value), then puts back what was there.

#[path = "../src/desktop.rs"]
#[allow(dead_code, unused_imports)] // desktop.rs's unit tests run in the app's own tests; this harness skips them.
mod desktop;

use std::process::{Child, Command};
use std::time::{Duration, Instant};

use desktop::{SystemLoginSession, SystemWindows};
use nexus_core::{Front, LoginSession, Windows};
use slint::winit_030::winit;

/// Set in a helper: it opens a window, and hides it after this many milliseconds (0: never).
const HELPER: &str = "NEXUS_DESKTOP_HELPER_HIDE_MS";

/// The system's windows and login session, as the app plugs them in.
struct SystemDesktop {
    windows: SystemWindows,
    session: SystemLoginSession,
}

/// One test: it gets the system's desktop.
type Test = fn(&SystemDesktop) -> Outcome;

enum Outcome {
    Pass,
    Skip(String),
    Fail(String),
}

/// A helper window's process, killed when the test is done with it.
struct Helper(Child);

impl Helper {
    fn start(hide_after_ms: u64) -> Self {
        let exe = std::env::current_exe().expect("this test's path");
        Self(Command::new(exe).env(HELPER, hide_after_ms.to_string()).spawn().expect("a helper"))
    }

    fn id(&self) -> u32 {
        self.0.id()
    }

    fn running(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Polls `done` for at most `limit`.
fn until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

/// Waits for the helper's window. `Err` is the outcome when it doesn't come.
fn window_of(desktop: &SystemDesktop, helper: &Helper) -> Result<(), Outcome> {
    let id = helper.id();
    let mut answer = None;
    until(Duration::from_secs(20), || {
        answer = desktop.windows.has_window(&[id]);
        answer == Some(true)
    });
    match answer {
        Some(true) => Ok(()),
        None => Err(Outcome::Skip("the desktop doesn't say which windows a process has".into())),
        Some(false) => Err(Outcome::Fail(format!("helper {id} has no window after 20 s"))),
    }
}

fn in_front(desktop: &SystemDesktop, id: u32) -> bool {
    desktop.windows.front() == Front::Other(id)
}

fn a_window_shown_comes_to_the_front(desktop: &SystemDesktop) -> Outcome {
    let (a, b) = (Helper::start(0), Helper::start(0));
    if let Err(outcome) = window_of(desktop, &a).and_then(|()| window_of(desktop, &b)) {
        return outcome;
    }
    if desktop.windows.front() == Front::Unknown {
        return Outcome::Skip("the desktop doesn't say which window is in front".into());
    }
    for helper in [&a, &b, &a] {
        let id = helper.id();
        if !desktop.windows.show_game(&[id]) {
            return Outcome::Fail(format!("show_game({id}) said no"));
        }
        if !until(Duration::from_secs(5), || in_front(desktop, id)) {
            let why = format!("helper {id} isn't in front: {:?} is", desktop.windows.front());
            // Windows decides who may take the front; a runner's session can refuse it.
            if cfg!(windows) {
                return Outcome::Skip(format!("Windows didn't let the window come to the front here: {why}"));
            }
            return Outcome::Fail(why);
        }
    }
    Outcome::Pass
}

fn a_window_that_closed_is_gone_while_its_program_runs(desktop: &SystemDesktop) -> Outcome {
    let mut helper = Helper::start(2000);
    if let Err(outcome) = window_of(desktop, &helper) {
        return outcome;
    }
    let id = helper.id();
    if !until(Duration::from_secs(10), || desktop.windows.has_window(&[id]) == Some(false)) {
        return Outcome::Fail(format!("helper {id} still has a window after it hid it"));
    }
    if !helper.running() {
        return Outcome::Fail("the helper ended; it should run on without its window".into());
    }
    Outcome::Pass
}

/// Whether the system's start-at-login entry for the launcher is there.
#[cfg(target_os = "linux")]
fn login_entry() -> bool {
    desktop::login::autostart_dir().is_ok_and(|dir| dir.join("nexus.desktop").is_file())
}

#[cfg(target_os = "macos")]
fn login_entry() -> bool {
    desktop::login::agents_dir().is_ok_and(|dir| dir.join(format!("{}.plist", desktop::login::AGENT)).is_file())
}

#[cfg(windows)]
fn login_entry() -> bool {
    Command::new("reg.exe")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "Nexus"])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn start_at_login_writes_the_system_s_entry_and_takes_it_out(desktop: &SystemDesktop) -> Outcome {
    let before = login_entry();
    let outcome = (|| {
        desktop.session.start_at_login(true).map_err(|e| format!("on: {e}"))?;
        if !login_entry() {
            return Err("on, but the entry isn't there".to_owned());
        }
        desktop.session.start_at_login(false).map_err(|e| format!("off: {e}"))?;
        if login_entry() {
            return Err("off, but the entry is still there".to_owned());
        }
        // Off again: an entry that is gone already is fine.
        desktop.session.start_at_login(false).map_err(|e| format!("off again: {e}"))
    })();
    if before {
        let _ = desktop.session.start_at_login(true);
    }
    match outcome {
        Ok(()) => Outcome::Pass,
        Err(why) => Outcome::Fail(why),
    }
}

fn main() {
    if let Ok(ms) = std::env::var(HELPER) {
        helper_window(ms.parse().unwrap_or(0));
        return;
    }
    if std::env::var_os("NEXUS_DESKTOP_TESTS").is_none() {
        println!("SKIPPED: set NEXUS_DESKTOP_TESTS=1 to test the desktop adapter (it opens windows)");
        return;
    }
    let desktop = SystemDesktop {
        windows: SystemWindows { raise: Box::new(|| {}) },
        session: SystemLoginSession { login: std::env::current_exe().ok() },
    };
    if let Some(problem) = desktop.windows.problem() {
        println!("the desktop says: {problem}");
    }
    let tests: [(&str, Test); 3] = [
        ("a_window_shown_comes_to_the_front", a_window_shown_comes_to_the_front),
        ("a_window_that_closed_is_gone_while_its_program_runs", a_window_that_closed_is_gone_while_its_program_runs),
        (
            "start_at_login_writes_the_system_s_entry_and_takes_it_out",
            start_at_login_writes_the_system_s_entry_and_takes_it_out,
        ),
    ];
    let require = std::env::var_os("NEXUS_DESKTOP_REQUIRE").is_some();
    let mut failed = 0;
    for (name, test) in tests {
        match test(&desktop) {
            Outcome::Pass => println!("test {name} ... ok"),
            Outcome::Skip(why) if require => {
                println!("test {name} ... FAILED: skipped, and NEXUS_DESKTOP_REQUIRE is set: {why}");
                failed += 1;
            }
            Outcome::Skip(why) => println!("test {name} ... SKIPPED: {why}"),
            Outcome::Fail(why) => {
                println!("test {name} ... FAILED: {why}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

/// A plain window, hidden after `hide_after_ms` if that is not 0. Runs until killed.
fn helper_window(hide_after_ms: u64) {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window, WindowId};

    struct App {
        window: Option<Window>,
        hide_at: Option<Instant>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                let attributes = Window::default_attributes()
                    .with_title("nexus desktop test")
                    .with_inner_size(winit::dpi::LogicalSize::new(320.0, 200.0));
                self.window = Some(event_loop.create_window(attributes).expect("a window"));
            }
        }

        // A close request is ignored: only the test ends a helper.
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            match (self.hide_at, &self.window) {
                (Some(at), Some(window)) if Instant::now() >= at => {
                    window.set_visible(false);
                    self.hide_at = None;
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
                (Some(at), _) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
                _ => event_loop.set_control_flow(ControlFlow::Wait),
            }
        }
    }

    let hide_at = (hide_after_ms > 0).then(|| Instant::now() + Duration::from_millis(hide_after_ms));
    let event_loop = EventLoop::new().expect("an event loop");
    event_loop.run_app(&mut App { window: None, hide_at }).expect("the helper's event loop");
}
