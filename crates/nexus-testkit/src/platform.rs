//! The app shell's platform in tests.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use nexus_ui_host::{Platform, TrailerPoll};

/// What the shell asked the platform to do, in order, besides running work and waking.
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformCall {
    OpenUrl(String),
    OpenPath(PathBuf),
    Quit,
    Restart,
    Announce(String),
    PlaySound(String),
    MoveToMonitor { monitor: String, fullscreen: bool },
    PlayTrailer(String),
    PauseTrailer,
    SeekTrailer(f64),
    CloseTrailer,
    ClaimWindow,
}

#[derive(Default)]
struct State {
    calls: Vec<PlatformCall>,
    work: VecDeque<BoxFuture<'static, Result<(), String>>>,
    wakes: Vec<Duration>,
    monitors: Vec<String>,
    unfocused: bool,
    local_time: String,
    trailer_refusal: Option<String>,
    trailer: Option<TrailerPoll>,
    claim_refusals: usize,
}

/// A platform that records each call, and runs the shell's work only when the test says
/// ([`FakePlatform::run_work`]), in the order it came. Cheap to clone; one per UI thread.
#[derive(Clone, Default)]
pub struct FakePlatform {
    state: Rc<RefCell<State>>,
}

impl FakePlatform {
    /// Each call, in order.
    pub fn calls(&self) -> Vec<PlatformCall> {
        self.state.borrow().calls.clone()
    }

    /// Each time the shell asked to be polled, after how long, in order.
    pub fn wakes(&self) -> Vec<Duration> {
        self.state.borrow().wakes.clone()
    }

    /// Forgets the wakes so far.
    pub fn clear_wakes(&self) {
        self.state.borrow_mut().wakes.clear();
    }

    /// How many pieces of work wait to run.
    pub fn waiting_work(&self) -> usize {
        self.state.borrow().work.len()
    }

    /// Runs the work waiting, one piece at a time in the order it came, with the work that
    /// adds, until none waits. The errors, in order: a command that failed says why.
    pub async fn run_work(&self) -> Vec<String> {
        let mut errors = Vec::new();
        loop {
            let next = self.state.borrow_mut().work.pop_front();
            let Some(work) = next else { return errors };
            if let Err(error) = work.await {
                errors.push(error);
            }
        }
    }

    /// The monitors the window system names.
    pub fn set_monitors(&self, monitors: &[&str]) {
        self.state.borrow_mut().monitors = monitors.iter().map(|m| (*m).to_owned()).collect();
    }

    /// Whether the launcher's window has the keyboard focus (it has, by default).
    pub fn set_focused(&self, focused: bool) {
        self.state.borrow_mut().unfocused = !focused;
    }

    /// The local time the clock shows.
    pub fn set_local_time(&self, time: &str) {
        self.state.borrow_mut().local_time = time.to_owned();
    }

    /// The window system isn't done with the launcher's window: the next `refusals` claims
    /// say so, and the one after says it shows.
    pub fn refuse_claims(&self, refusals: usize) {
        self.state.borrow_mut().claim_refusals = refusals;
    }

    /// Trailers don't play in the window, with `error`.
    pub fn refuse_trailers(&self, error: &str) {
        self.state.borrow_mut().trailer_refusal = Some(error.to_owned());
    }

    /// What the trailer player says on the next tick.
    pub fn set_trailer(&self, trailer: Option<TrailerPoll>) {
        self.state.borrow_mut().trailer = trailer;
    }

    fn record(&self, call: PlatformCall) {
        self.state.borrow_mut().calls.push(call);
    }
}

impl Platform for FakePlatform {
    fn spawn(&self, work: BoxFuture<'static, Result<(), String>>) {
        self.state.borrow_mut().work.push_back(work);
    }

    fn wake_in(&self, after: Duration) {
        self.state.borrow_mut().wakes.push(after);
    }

    fn open_url(&self, url: &str) {
        self.record(PlatformCall::OpenUrl(url.to_owned()));
    }

    fn open_path(&self, path: &Path) {
        self.record(PlatformCall::OpenPath(path.to_path_buf()));
    }

    fn quit(&self) {
        self.record(PlatformCall::Quit);
    }

    fn restart(&self) {
        self.record(PlatformCall::Restart);
    }

    fn announce(&self, line: &str) {
        self.record(PlatformCall::Announce(line.to_owned()));
    }

    fn play_sound(&self, name: &str, _bytes: Arc<[u8]>) {
        self.record(PlatformCall::PlaySound(name.to_owned()));
    }

    fn local_time(&self) -> String {
        self.state.borrow().local_time.clone()
    }

    fn monitors(&self, _window: &slint_interpreter::Window) -> Vec<String> {
        self.state.borrow().monitors.clone()
    }

    fn move_to_monitor(&self, _window: &slint_interpreter::Window, monitor: &str, fullscreen: bool) {
        if self.state.borrow().monitors.iter().any(|m| m == monitor) {
            self.record(PlatformCall::MoveToMonitor { monitor: monitor.to_owned(), fullscreen });
        }
    }

    fn window_focused(&self, _window: &slint_interpreter::Window) -> bool {
        !self.state.borrow().unfocused
    }

    fn claim_window(&self) -> bool {
        self.record(PlatformCall::ClaimWindow);
        let mut state = self.state.borrow_mut();
        let refused = state.claim_refusals > 0;
        state.claim_refusals = state.claim_refusals.saturating_sub(1);
        !refused
    }

    fn play_trailer(&self, url: &str) -> Result<(), String> {
        self.record(PlatformCall::PlayTrailer(url.to_owned()));
        match self.state.borrow().trailer_refusal.clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn pause_trailer(&self) {
        self.record(PlatformCall::PauseTrailer);
    }

    fn seek_trailer(&self, seconds: f64) {
        self.record(PlatformCall::SeekTrailer(seconds));
    }

    fn close_trailer(&self) {
        self.record(PlatformCall::CloseTrailer);
        self.state.borrow_mut().trailer = None;
    }

    fn poll_trailer(&self) -> Option<TrailerPoll> {
        self.state.borrow().trailer.clone()
    }
}
