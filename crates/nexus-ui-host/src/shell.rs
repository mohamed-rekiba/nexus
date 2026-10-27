//! The app shell: what the launcher does on the UI thread around the theme host. It routes
//! the theme's requests, coalesces renders, loads images, and runs the tick: dev reload, the
//! monitor, the "window shown" line, the Gamescope claim, the trailer, the clock's minute, and
//! the renders while a game runs.
//!
//! The shell never touches the system itself: it asks its [`Platform`]. The app plugs in the
//! real one; the test kit's `FakePlatform` records each call. The shell reads the time from
//! the core's [`Clock`] port, so a test's `FakeClock` moves it. Timers stay on the platform's
//! side: the shell says when it wants [`AppShell::poll`] again ([`Platform::wake_in`]), and a
//! poll does what is due by the clock.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use nexus_api::Event;
use nexus_core::{Clock, Core};
use nexus_ui_api::ThemeError;
use slint_interpreter::ComponentHandle;

use crate::host::{ThemeHost, TrailerRequest, TrailerStatus, UiRequest};
use crate::images::ImageKey;
use crate::input::PadButton;
use crate::loader::{ImageLoader, Pixels};

/// The least time between two renders: a frame at 120 Hz. A burst of images or core changes
/// renders once a frame, so the frames between keep their time for animations.
pub const RENDER_GAP: Duration = Duration::from_millis(8);

/// How long images that come in wait to show together. Covers arrive one by one while the
/// player scrolls; each showing on its own would take the UI thread's time from the scroll.
pub const IMAGES_GAP: Duration = Duration::from_millis(100);

/// How often the app calls [`AppShell::on_tick`].
pub const TICK: Duration = Duration::from_millis(250);

/// How often the Running screen renders while a game runs: the time played shows in minutes.
const PLAYING_GAP: Duration = Duration::from_secs(15);

/// What the system does for the shell: the app's adapter, or the test kit's fake. Every call is
/// on the UI thread. The `window` the calls get is the launcher's.
pub trait Platform {
    /// Runs `work` off the UI thread. An error is a command that failed: the app logs it. Once
    /// `work` ends, the app calls [`AppShell::poll`] on the UI thread.
    fn spawn(&self, work: BoxFuture<'static, Result<(), String>>);
    /// Calls [`AppShell::poll`] on the UI thread `after` from now.
    fn wake_in(&self, after: Duration);
    /// Opens an `https://` page in the system's browser.
    fn open_url(&self, url: &str);
    /// Opens a game's folder or log with the system's opener.
    fn open_path(&self, path: &Path);
    /// Closes the launcher.
    fn quit(&self);
    /// Closes the launcher, and starts it again once it has shut down.
    fn restart(&self);
    /// Writes `line` on its own line to the standard output, where the Nexus session reads it.
    fn announce(&self, line: &str);
    /// Plays the theme's sound `name`, whose file is `bytes`.
    fn play_sound(&self, name: &str, bytes: Arc<[u8]>);
    /// The local time the clock shows, for example "9:38 PM".
    fn local_time(&self) -> String;
    /// The monitors' names.
    fn monitors(&self, window: &slint_interpreter::Window) -> Vec<String>;
    /// Moves `window` to the monitor named `monitor`, full screen there when `fullscreen`.
    /// Nothing when there is no such monitor, or the window is on it already.
    fn move_to_monitor(&self, window: &slint_interpreter::Window, monitor: &str, fullscreen: bool);
    /// Whether `window` has the keyboard focus, as the window system says.
    fn window_focused(&self, window: &slint_interpreter::Window) -> bool;
    /// Makes the launcher's window show where the session needs a step for it (Gamescope).
    /// `true` once done, or when nothing needs doing; `false` asks again on the next tick.
    fn claim_window(&self) -> bool;
    /// Plays the trailer at `url` in the window. The error says why it can't.
    fn play_trailer(&self, url: &str) -> Result<(), String>;
    fn pause_trailer(&self);
    /// Moves the trailer `seconds` on, or back.
    fn seek_trailer(&self, seconds: f64);
    fn close_trailer(&self);
    /// The trailer's state, or `None` when none is open.
    fn poll_trailer(&self) -> Option<TrailerPoll>;
}

/// The trailer open, as the player says.
#[derive(Clone, Debug, PartialEq)]
pub enum TrailerPoll {
    Playing(TrailerStatus),
    /// It played to the end or failed: it closes.
    Ended,
}

/// How this run of the shell behaves.
pub struct ShellOptions {
    /// Reloads theme files when they change (`--dev`).
    pub dev: bool,
    /// The monitor for this run (`--monitor`). A screen the player picks in Settings wins.
    pub monitor: Option<String>,
    /// Where the image loader keeps its smaller copies.
    pub thumbs: PathBuf,
}

/// The theme's requests, waiting for the next [`AppShell::poll`].
pub type RequestQueue = Rc<RefCell<Vec<UiRequest>>>;

/// An image load that ended: the image, and its pixels or `None` when it left the screen.
type Loaded = (ImageKey, Option<Result<Pixels, String>>);

pub struct AppShell {
    /// The theme host the shell renders through. The app and the tests set it up directly.
    pub ui: ThemeHost,
    core: Core,
    platform: Rc<dyn Platform>,
    clock: Arc<dyn Clock>,
    requests: RequestQueue,
    images: Arc<ImageLoader>,
    loaded: (Sender<Loaded>, Receiver<Loaded>),
    renders: Coalescer,
    tick: TickPolicy,
    dev: bool,
    monitors: Monitors,
    /// How many times the shell rendered.
    rendered: u64,
}

impl AppShell {
    /// A shell over a theme host on `core`. The host shows nothing until the app starts it
    /// (`shell.ui.start_with(..)`).
    pub async fn new(core: Core, platform: Rc<dyn Platform>, clock: Arc<dyn Clock>, options: ShellOptions) -> Self {
        let requests: RequestQueue = Rc::default();
        // A theme's callback runs while the host is busy: its request waits for the next poll.
        let sink = {
            let (requests, platform) = (requests.clone(), platform.clone());
            Rc::new(move |request: UiRequest| {
                let mut queue = requests.borrow_mut();
                if queue.is_empty() {
                    platform.wake_in(Duration::ZERO);
                }
                queue.push(request);
            })
        };
        let ui = ThemeHost::new(core.clone(), sink).await;
        let flag = options.monitor.map(|name| (name, core.monitor()));
        Self {
            ui,
            core,
            platform,
            clock,
            requests,
            images: Arc::new(ImageLoader::new(options.thumbs)),
            loaded: channel(),
            renders: Coalescer::default(),
            tick: TickPolicy::default(),
            dev: options.dev,
            monitors: Monitors { flag, ..Monitors::default() },
            rendered: 0,
        }
    }

    /// The theme's requests waiting for the next poll. Tests read and change it.
    pub fn requests(&self) -> RequestQueue {
        self.requests.clone()
    }

    /// How many times the shell rendered.
    pub fn renders(&self) -> u64 {
        self.rendered
    }

    /// Does what waits on the UI thread: the theme's requests, the images that came in, and a
    /// render whose time came.
    pub fn poll(&mut self) {
        let requests = std::mem::take(&mut *self.requests.borrow_mut());
        for request in requests {
            if let Err(error) = self.on_request(request) {
                tracing::error!(%error, "the theme's request failed");
            }
        }
        self.take_loaded();
        let now = self.clock.now();
        while let Some(due) = self.renders.due(now) {
            match due {
                Due::Render => self.render_logged(),
                Due::Images => self.show_images_logged(),
            }
        }
        if let Some(at) = self.renders.next() {
            self.platform.wake_in(at.saturating_duration_since(now));
        }
    }

    /// Does now what waits for its time: the images that came in, then a render or the images
    /// waiting. For a test, whose clock moves only when it says.
    pub fn flush(&mut self) -> Result<(), ThemeError> {
        self.take_loaded();
        match self.renders.take() {
            Some(Due::Render) => self.render(),
            Some(Due::Images) => self.show_images(),
            None => Ok(()),
        }
    }

    /// Carries out one of the theme's requests. A theme switch that fails keeps the current
    /// theme and says why; a render that fails says why.
    pub fn on_request(&mut self, request: UiRequest) -> Result<(), ThemeError> {
        match request {
            UiRequest::Core(command) => {
                let core = self.core.clone();
                self.platform.spawn(Box::pin(async move { core.dispatch(command).await.map_err(|e| e.to_string()) }));
            }
            UiRequest::SwitchTheme(theme) => self.ui.switch(&theme)?,
            UiRequest::OpenUrl(url) => self.platform.open_url(&url),
            UiRequest::OpenPath(path) => self.platform.open_path(&path),
            UiRequest::Quit => self.platform.quit(),
            UiRequest::Restart => self.platform.restart(),
            // The Nexus session reads this line when Nexus ends, and ends too instead of
            // starting it again.
            UiRequest::LogOut => {
                self.platform.announce("log out");
                self.platform.quit();
            }
            UiRequest::Trailer(request) => return self.trailer(request),
            UiRequest::Render => self.ask_render(),
            UiRequest::PlaySound(name) => match self.ui.sound(&name) {
                Some(bytes) => self.platform.play_sound(&name, bytes),
                None => tracing::debug!(%name, "no such sound, or sounds are off"),
            },
        }
        Ok(())
    }

    /// The core changed: a render, at most once each [`RENDER_GAP`].
    pub fn on_core_event(&mut self, _event: &Event) {
        self.ask_render();
    }

    /// An image load ended: it shows with the others that came in, at most once each
    /// [`IMAGES_GAP`].
    pub fn on_pixels(&mut self, key: ImageKey, pixels: Option<Result<Pixels, String>>) {
        self.ui.add_pixels(key, pixels);
        if let Some(wait) = self.renders.ask_images(self.clock.now()) {
            self.platform.wake_in(wait);
        }
    }

    /// A gamepad button was pressed, or held and repeated: it goes to the theme, as the window
    /// system's focus allows (see [`ThemeHost::pad`]).
    pub fn on_pad(&mut self, button: PadButton, repeated: bool) {
        let focused = self.ui.instance().is_none_or(|i| self.platform.window_focused(i.window()));
        self.ui.set_window_focused(focused);
        let pressed = if repeated { self.ui.pad_repeated(button) } else { self.ui.pad(button) };
        if let Err(error) = pressed {
            tracing::warn!(%error, "controller input failed");
        }
    }

    /// The connected controller's name, for the launch splash, or `None` when none is.
    pub fn on_pad_name(&mut self, name: Option<String>) {
        self.ui.set_pad_name(name);
        self.render_logged();
    }

    /// Every [`TICK`]: dev mode reloads edited theme files, the window goes to its monitor,
    /// the session hears that the window shows, the trailer moves on, and the view renders when
    /// the window's size, the clock's minute or the trailer changed, while the boot screen
    /// waits for Home's art, and every 15 s while a game runs.
    pub fn on_tick(&mut self) {
        if self.dev {
            match self.ui.reload_if_changed() {
                Ok(true) => tracing::info!("theme reloaded"),
                Ok(false) => {}
                Err(error) => tracing::error!(%error, "theme edit doesn't load; keeping the last good version"),
            }
        }
        self.place_on_monitor();
        let trailer_open = match self.platform.poll_trailer() {
            Some(TrailerPoll::Ended) => {
                self.platform.close_trailer();
                self.ui.set_trailer(None);
                true
            }
            Some(TrailerPoll::Playing(status)) => {
                self.ui.set_trailer(Some(status));
                true
            }
            None => false,
        };
        let tick = Tick {
            now: self.clock.now(),
            size: self.ui.instance().map(|i| i.window().size()).map(|s| (s.width, s.height)),
            playing: self.core.game_running(),
            waiting: self.ui.waiting_for_art(),
            trailer_open,
            clock: self.platform.local_time(),
        };
        let done = self.tick.tick(tick);
        if done.announce {
            // The Nexus session's watchdog waits for this line: a start that never shows a
            // window is started again.
            self.platform.announce("window shown");
        }
        if done.claim {
            self.tick.claimed(self.platform.claim_window());
        }
        if let Some(clock) = done.clock {
            self.ui.set_clock(clock);
        }
        if done.render {
            self.render_logged();
        }
    }

    /// Shows the current state now, and starts loading the images it needs.
    pub fn render(&mut self) -> Result<(), ThemeError> {
        self.renders.rendered(self.clock.now());
        self.rendered += 1;
        let shown = self.ui.render();
        self.load_wanted();
        shown
    }

    /// Starts loading the images the last render asked for and doesn't have. A render does it.
    pub fn load_wanted(&mut self) {
        let still = self.ui.still_wanted();
        for key in self.ui.wanted_images() {
            let (core, loader, still, loaded) =
                (self.core.clone(), self.images.clone(), still.clone(), self.loaded.0.clone());
            self.platform.spawn(Box::pin(async move {
                let image = key.0.clone();
                let fetch = || async move { core.image_bytes(&image).await.map_err(|e| e.to_string()) };
                let pixels = loader.load(fetch, &key, &still).await;
                // The shell is gone when the app closed meanwhile.
                let _ = loaded.send((key, pixels));
                Ok(())
            }));
        }
    }

    fn render_logged(&mut self) {
        if let Err(error) = self.render() {
            tracing::error!(%error, "render failed");
        }
    }

    /// Shows the images that came in, with the view already bound: nothing in the core changed.
    fn show_images(&mut self) -> Result<(), ThemeError> {
        self.renders.rendered(self.clock.now());
        self.rendered += 1;
        let shown = self.ui.render_images();
        self.load_wanted();
        shown
    }

    fn show_images_logged(&mut self) {
        if let Err(error) = self.show_images() {
            tracing::error!(%error, "render failed");
        }
    }

    fn ask_render(&mut self) {
        if let Some(wait) = self.renders.ask_render(self.clock.now()) {
            self.platform.wake_in(wait);
        }
    }

    fn take_loaded(&mut self) {
        while let Ok((key, pixels)) = self.loaded.1.try_recv() {
            self.on_pixels(key, pixels);
        }
    }

    /// The trailer player. A trailer that can't play in the window opens in the browser, and
    /// the player is told why.
    fn trailer(&mut self, request: TrailerRequest) -> Result<(), ThemeError> {
        match request {
            TrailerRequest::Play(url) => match self.platform.play_trailer(&url) {
                Ok(()) => self.ui.set_trailer(Some(TrailerStatus { loading: true, ..TrailerStatus::default() })),
                Err(error) => {
                    tracing::warn!(%error, "the trailer didn't open in the window");
                    self.platform.open_url(&url);
                    self.core.report(format!("The trailer opened in your browser: {error}"));
                }
            },
            TrailerRequest::TogglePause => self.platform.pause_trailer(),
            TrailerRequest::Seek(seconds) => self.platform.seek_trailer(seconds),
            TrailerRequest::Close => {
                self.platform.close_trailer();
                self.ui.set_trailer(None);
            }
        }
        self.render()
    }

    /// The monitors' names, and the launcher's window moved to the one the player chose. The
    /// window moves once per choice: the player may still drag it elsewhere.
    ///
    /// `--monitor` wins over the saved monitor until the player picks a screen in Settings.
    fn place_on_monitor(&mut self) {
        let saved = self.core.monitor();
        let state = &mut self.monitors;
        if state.flag.as_ref().is_some_and(|(_, at_start)| *at_start != saved) {
            state.flag = None;
        }
        let wanted = state.flag.as_ref().map(|(name, _)| name.clone()).or(saved);
        let Some(instance) = self.ui.instance() else { return };
        let window = instance.window();
        let names = self.platform.monitors(window);
        let changed = wanted != state.placed;
        if changed && let Some(wanted) = &wanted {
            self.platform.move_to_monitor(window, wanted, self.core.fullscreen_on());
        }
        if changed && wanted.as_ref().is_none_or(|w| names.contains(w)) {
            state.placed = wanted;
        }
        if names != state.shown {
            state.shown = names.clone();
            self.ui.set_monitors(names);
        }
    }
}

/// Where the window is, and what Settings lists.
#[derive(Default)]
struct Monitors {
    /// The names Settings lists.
    shown: Vec<String>,
    /// The monitor the window was moved to.
    placed: Option<String>,
    /// `--monitor`, with the saved monitor at the start.
    flag: Option<(String, Option<String>)>,
}

/// What is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// A render: the core changed.
    Render,
    /// The images that came in, shown with the view already bound.
    Images,
}

/// When renders run: once however many changes or images arrive before. A render runs at once,
/// unless the last one was less than [`RENDER_GAP`] ago. Images that came in show after
/// [`IMAGES_GAP`], unless a render waiting shows them first.
#[derive(Default)]
struct Coalescer {
    last: Option<Instant>,
    render_at: Option<Instant>,
    images_at: Option<Instant>,
}

impl Coalescer {
    /// Asks for a render. How long until it is due, or `None` when one was asked for already.
    fn ask_render(&mut self, now: Instant) -> Option<Duration> {
        if self.render_at.is_some() {
            return None;
        }
        let at = self.last.map_or(now, |last| (last + RENDER_GAP).max(now));
        self.render_at = Some(at);
        Some(at - now)
    }

    /// Asks to show the images that came in. How long until that is due, or `None` when it was
    /// asked for already.
    fn ask_images(&mut self, now: Instant) -> Option<Duration> {
        if self.images_at.is_some() {
            return None;
        }
        self.images_at = Some(now + IMAGES_GAP);
        Some(IMAGES_GAP)
    }

    /// What is due at `now`, once each: a render first. Images due while a render waits are
    /// left to it.
    fn due(&mut self, now: Instant) -> Option<Due> {
        if self.render_at.is_some_and(|at| at <= now) {
            self.render_at = None;
            return Some(Due::Render);
        }
        if self.images_at.is_some_and(|at| at <= now) {
            self.images_at = None;
            if self.render_at.is_none() {
                return Some(Due::Images);
            }
        }
        None
    }

    /// What waits, whatever its time: a render first.
    fn take(&mut self) -> Option<Due> {
        if self.render_at.take().is_some() {
            self.images_at = None;
            return Some(Due::Render);
        }
        self.images_at.take().map(|_| Due::Images)
    }

    /// When the next thing waiting is due.
    fn next(&self) -> Option<Instant> {
        self.render_at.into_iter().chain(self.images_at).min()
    }

    /// A render ran at `now`: it showed what a render or images waiting would have.
    fn rendered(&mut self, now: Instant) {
        self.last = Some(now);
        self.render_at = None;
        self.images_at = None;
    }
}

/// What a tick sees.
struct Tick {
    now: Instant,
    /// The window's size in pixels, when there is a window.
    size: Option<(u32, u32)>,
    /// A game runs.
    playing: bool,
    /// The boot screen waits for Home's art.
    waiting: bool,
    /// A trailer is open.
    trailer_open: bool,
    /// The local time, as the clock shows it.
    clock: String,
}

/// What a tick does.
#[derive(Debug, Default, PartialEq, Eq)]
struct TickDone {
    /// Tell the session the window shows.
    announce: bool,
    /// Ask the platform to make the window show ([`Platform::claim_window`]).
    claim: bool,
    /// The clock's new text.
    clock: Option<String>,
    render: bool,
}

/// The tick's rules, over what it saw last.
#[derive(Default)]
struct TickPolicy {
    size: Option<(u32, u32)>,
    clock: String,
    announced: bool,
    claimed: bool,
    /// While a game runs: when the Running screen renders next.
    playing_at: Option<Instant>,
}

impl TickPolicy {
    fn tick(&mut self, tick: Tick) -> TickDone {
        let mut done = TickDone::default();
        if !self.announced && tick.size.is_some_and(|(w, h)| w > 0 && h > 0) {
            self.announced = true;
            done.announce = true;
        }
        done.claim = self.announced && !self.claimed;
        let resized = tick.size != self.size;
        self.size = tick.size;
        if tick.clock != self.clock {
            self.clock = tick.clock.clone();
            done.clock = Some(tick.clock);
        }
        let playing = match (tick.playing, self.playing_at) {
            (false, _) => {
                self.playing_at = None;
                false
            }
            (true, Some(at)) if at <= tick.now => {
                self.playing_at = Some(tick.now + PLAYING_GAP);
                true
            }
            (true, Some(_)) => false,
            (true, None) => {
                self.playing_at = Some(tick.now + PLAYING_GAP);
                false
            }
        };
        done.render = resized || done.clock.is_some() || tick.waiting || tick.trailer_open || playing;
        done
    }

    /// What the platform said to the claim: `true` once the window shows.
    fn claimed(&mut self, done: bool) {
        self.claimed = done;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_burst_of_changes_renders_once_a_frame() {
        let start = Instant::now();
        let mut renders = Coalescer::default();
        renders.rendered(start);
        assert_eq!(renders.ask_render(start), Some(RENDER_GAP), "the last render was just now");
        for _ in 0..9 {
            assert_eq!(renders.ask_render(start + MS), None, "asked already");
        }
        assert_eq!(renders.due(start + 7 * MS), None);
        assert_eq!(renders.due(start + RENDER_GAP), Some(Due::Render));
        assert_eq!(renders.due(start + RENDER_GAP), None, "once");
    }

    #[test]
    fn a_change_long_after_the_last_render_renders_at_once() {
        let start = Instant::now();
        let mut renders = Coalescer::default();
        assert_eq!(renders.ask_render(start), Some(Duration::ZERO), "no render yet");
        assert_eq!(renders.due(start), Some(Due::Render));
        renders.rendered(start);
        let later = start + Duration::from_secs(1);
        assert_eq!(renders.ask_render(later), Some(Duration::ZERO));
        assert_eq!(renders.due(later), Some(Due::Render));
    }

    #[test]
    fn images_show_together_and_a_render_waiting_shows_them() {
        let start = Instant::now();
        let mut renders = Coalescer::default();
        assert_eq!(renders.ask_images(start), Some(IMAGES_GAP));
        assert_eq!(renders.ask_images(start + MS), None);
        assert_eq!(renders.due(start + 99 * MS), None);
        assert_eq!(renders.due(start + IMAGES_GAP), Some(Due::Images));

        renders.rendered(start + IMAGES_GAP);
        renders.ask_images(start + IMAGES_GAP);
        let late = start + 2 * IMAGES_GAP;
        renders.ask_render(late - MS);
        assert_eq!(renders.next(), Some(late - MS));
        assert_eq!(renders.due(late), Some(Due::Render));
        renders.rendered(late);
        assert_eq!(renders.due(late), None, "the render showed the images");
        assert_eq!(renders.next(), None);
    }

    #[test]
    fn a_render_now_takes_the_place_of_one_waiting() {
        let start = Instant::now();
        let mut renders = Coalescer::default();
        renders.rendered(start);
        renders.ask_render(start);
        renders.rendered(start + MS);
        assert_eq!(renders.due(start + RENDER_GAP), None);
        assert_eq!(renders.next(), None);
    }

    #[test]
    fn what_waits_can_be_taken_before_its_time() {
        let start = Instant::now();
        let mut renders = Coalescer::default();
        renders.rendered(start);
        renders.ask_images(start);
        assert_eq!(renders.take(), Some(Due::Images));
        renders.ask_images(start);
        renders.ask_render(start);
        assert_eq!(renders.take(), Some(Due::Render), "it shows the images too");
        assert_eq!(renders.take(), None);
    }

    fn tick(now: Instant) -> Tick {
        Tick {
            now,
            size: Some((1280, 800)),
            playing: false,
            waiting: false,
            trailer_open: false,
            clock: "9:38 PM".into(),
        }
    }

    #[test]
    fn the_tick_re_renders_every_15_seconds_while_a_game_runs() {
        let start = Instant::now();
        let mut policy = TickPolicy::default();
        assert!(policy.tick(tick(start)).render, "the first: the window's size and the clock are new");
        let playing = |at| Tick { playing: true, ..tick(at) };
        let mut renders = Vec::new();
        for n in 1..=130 {
            let at = start + TICK * n;
            if policy.tick(playing(at)).render {
                renders.push(at - start);
            }
        }
        assert_eq!(renders, [TICK + PLAYING_GAP, TICK + 2 * PLAYING_GAP]);
        assert!(!policy.tick(tick(start + TICK * 131)).render, "the game ended");
    }

    #[test]
    fn the_tick_renders_when_the_size_the_minute_the_art_or_the_trailer_change() {
        let start = Instant::now();
        let mut policy = TickPolicy::default();
        policy.tick(tick(start));
        assert!(!policy.tick(tick(start + TICK)).render, "nothing changed");
        assert!(policy.tick(Tick { size: Some((1920, 1080)), ..tick(start) }).render);
        let minute = policy.tick(Tick { clock: "9:39 PM".into(), ..tick(start) });
        assert_eq!((minute.render, minute.clock.as_deref()), (true, Some("9:39 PM")));
        assert!(policy.tick(Tick { clock: "9:39 PM".into(), waiting: true, ..tick(start) }).render);
        assert!(policy.tick(Tick { clock: "9:39 PM".into(), trailer_open: true, ..tick(start) }).render);
    }

    #[test]
    fn the_window_is_announced_once_it_has_a_size_and_claimed_until_that_works() {
        let start = Instant::now();
        let mut policy = TickPolicy::default();
        let none = policy.tick(Tick { size: Some((0, 0)), ..tick(start) });
        assert!(!none.announce && !none.claim, "no window to show yet");
        let shown = policy.tick(tick(start));
        assert!(shown.announce && shown.claim);
        policy.claimed(false);
        let again = policy.tick(tick(start));
        assert!(!again.announce && again.claim, "announced once, claimed until it works");
        policy.claimed(true);
        assert!(!policy.tick(tick(start)).claim);
    }
}
