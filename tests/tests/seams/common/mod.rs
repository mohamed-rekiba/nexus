//! A composed host for seam 3: the core and the app shell the app ships, on Slint's testing
//! backend, with a fake platform and the core's fake clock.

#![allow(dead_code)] // Each test binary uses part of this module.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use nexus_api::*;
use nexus_core::{Area, Command, Core};
pub use nexus_testkit::{FakeCatalog, FakeService, FakeStore, ThemeDriver};
use nexus_testkit::{FakeHost, FakePlatform, TestCore};
use nexus_ui_host::{AppShell, ShellOptions, UiRequest};
use slint_interpreter::ComponentHandle;

/// `path` with this system's separator: `~/Games` is `~\Games` on Windows, as the app shows it.
pub fn native(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// A full path on this system for a Unix one: `/x` stays `/x`, and is `C:\x` on Windows.
pub fn full_path(unix: &str) -> String {
    if cfg!(windows) { format!("C:{}", native(unix)) } else { unix.to_owned() }
}

pub fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

/// The core and the app shell on it. It derefs to the shell: `host.ui` is the theme host.
pub struct Composed {
    pub rt: tokio::runtime::Runtime,
    pub test: TestCore,
    pub shell: AppShell,
    pub platform: FakePlatform,
    /// The theme's requests the shell hasn't carried out: [`Composed::pump`] does.
    pub requests: Rc<RefCell<Vec<UiRequest>>>,
    events: futures::stream::BoxStream<'static, Event>,
}

impl std::ops::Deref for Composed {
    type Target = AppShell;
    fn deref(&self) -> &AppShell {
        &self.shell
    }
}

impl std::ops::DerefMut for Composed {
    fn deref_mut(&mut self) -> &mut AppShell {
        &mut self.shell
    }
}

/// Boots a core with the default theme plus whatever `build` adds, and an app shell on it.
/// Starts Slint's testing backend for this thread.
pub fn composed(build: impl FnOnce(FakeHost) -> FakeHost) -> Composed {
    thread_local!(static SLINT: () = quiet_testing_backend());
    SLINT.with(|_| {});
    composed_on_current_platform(build)
}

/// False for the log line Slint's element search writes for the items its interpreter keeps no
/// element info for (see [`quiet_testing_backend`]); every test platform drops it.
pub fn keeps_log_line(arguments: &std::fmt::Arguments) -> bool {
    !arguments.to_string().starts_with("The use of the ElementHandle API requires")
}

/// Slint's testing backend, as `i_slint_backend_testing::init_no_event_loop` sets it up, minus one
/// log line. Slint 1.18's interpreter keeps no element info for the items it adds itself (layout
/// and clip wrappers), so every accessible-label search logs "The use of the ElementHandle API
/// requires the presence of debug info…" for each of them, tens of thousands of times a run;
/// `SLINT_EMIT_DEBUG_INFO` doesn't change that. The search works all the same. Every other log
/// line goes through. It doesn't forward `bind_context`: the backend only needs the context for
/// its event loop, which a backend without threading doesn't have.
fn quiet_testing_backend() {
    use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
    use slint::platform::{Clipboard, EventLoopProxy, Platform, PlatformError, WindowAdapter};

    struct Quiet(TestingBackend);

    impl Platform for Quiet {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            self.0.create_window_adapter()
        }
        fn duration_since_start(&self) -> std::time::Duration {
            self.0.duration_since_start()
        }
        fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
            self.0.set_clipboard_text(text, clipboard)
        }
        fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
            self.0.clipboard_text(clipboard)
        }
        fn run_event_loop(&self) -> Result<(), PlatformError> {
            self.0.run_event_loop()
        }
        fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
            self.0.new_event_loop_proxy()
        }
        fn open_url(&self, url: &str) -> Result<(), PlatformError> {
            self.0.open_url(url)
        }
        fn debug_log(&self, arguments: std::fmt::Arguments) {
            if keeps_log_line(&arguments) {
                self.0.debug_log(arguments)
            }
        }
    }

    // renderer_name exists only when the testing backend is built with a renderer.
    #[allow(clippy::needless_update)] // the update fills renderer_name when it exists
    let options = TestingBackendOptions { mock_time: true, threading: false, ..Default::default() };
    slint::platform::set_platform(Box::new(Quiet(TestingBackend::new(options)))).expect("platform already initialized");
}

/// Like [`composed`], for a test that set its own Slint platform.
pub fn composed_on_current_platform(build: impl FnOnce(FakeHost) -> FakeHost) -> Composed {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let host = build(FakeHost::new().built_in(nexus_api::PluginBox::theme(theme_default::DefaultTheme)));
    let test = rt.block_on(host.boot_core());
    let platform = FakePlatform::default();
    let options = ShellOptions { dev: false, monitor: None, thumbs: test.data_dir().join("cache/thumbs") };
    let clock = Arc::new(test.clock.clone());
    let shell = rt.block_on(AppShell::new(test.core.clone(), Rc::new(platform.clone()), clock, options));
    let requests = shell.requests();
    let events = test.core.events();
    Composed { rt, test, shell, platform, requests, events }
}

impl Composed {
    pub fn core(&self) -> &Core {
        &self.test.core
    }

    /// Runs a command on the core through the shell, as a theme's request, then renders. The
    /// renders a test asks for load no images: [`Composed::load_images`] does.
    pub fn dispatch(&mut self, command: Command) {
        self.shell.on_request(UiRequest::Core(command)).unwrap();
        self.run_work();
        self.shell.ui.render().unwrap();
    }

    /// Runs the work the shell started (commands and image loads), in order, and hands the
    /// core's events since to the shell. A command that failed fails the test.
    pub fn run_work(&mut self) {
        let errors = self.rt.block_on(self.platform.run_work());
        assert!(errors.is_empty(), "a command failed: {errors:?}");
        while let Some(Some(event)) = self.events.next().now_or_never() {
            self.shell.on_core_event(&event);
        }
    }

    /// Waits until each catalog on the Catalog is the copy fetched from the catalog address,
    /// then renders: the Game Catalog fetches in the background, and the first read gets the
    /// bundled copy.
    pub fn catalogs_fetched(&mut self) {
        self.rt.block_on(catalogs_fetched(&self.test.core));
        self.shell.ui.render().unwrap();
    }

    /// Runs the core until `done` holds: see [`TestCore::until`]. It doesn't render.
    pub fn until(&self, area: Area, done: impl FnMut(&Core) -> bool) {
        self.rt.block_on(self.test.until(area, done));
    }

    /// Moves the core's clock on by `by`: see [`nexus_testkit::FakeClock::advance`]. It
    /// doesn't render.
    pub fn advance(&self, by: std::time::Duration) {
        self.rt.block_on(self.test.clock.advance(by));
    }

    /// Runs the core until every task waits: see [`nexus_testkit::FakeClock::run_until_idle`].
    /// It doesn't render.
    pub fn run_until_idle(&self) {
        self.rt.block_on(self.test.clock.run_until_idle());
    }

    /// Carries out what the theme asked for through the shell, as the app would, one request
    /// at a time: a core command runs to its end, and then, as for a render the theme asked
    /// for, the view renders now.
    pub fn pump(&mut self) {
        let requests: Vec<UiRequest> = self.requests.borrow_mut().drain(..).collect();
        for request in requests {
            let render = matches!(request, UiRequest::Core(_) | UiRequest::Render);
            self.shell.on_request(request).unwrap();
            self.run_work();
            if render {
                self.shell.ui.render().unwrap();
            }
        }
    }

    /// Loads every image the last render asked for through the shell's loader, then shows
    /// them, as the app does when they come in.
    pub fn load_images(&mut self) {
        self.shell.load_wanted();
        self.run_work();
        self.shell.flush().unwrap();
    }

    pub fn instance(&self) -> &slint_interpreter::ComponentInstance {
        self.ui.instance().expect("a theme is shown")
    }

    /// A controller button, as the app hands it to the theme, carried out.
    pub fn pad(&mut self, button: nexus_ui_host::PadButton) {
        self.shell.ui.pad(button).unwrap();
        self.pump();
        self.settle();
    }

    pub fn resize(&self, width: f32, height: f32) {
        self.instance().window().set_size(slint::LogicalSize::new(width, height));
    }
}

impl ThemeDriver for Composed {
    fn shown(&self) -> &slint_interpreter::ComponentInstance {
        self.instance()
    }

    fn redraw(&mut self) {
        self.shell.ui.render().unwrap();
    }

    fn carry_out(&mut self) {
        self.pump();
    }
}

/// Waits until each catalog the core has is the copy fetched from the catalog address: the Game
/// Catalog fetches in the background, and the first read gets the bundled copy. It fails when
/// the catalogs never change to that, rather than letting the test go on with the bundled copy
/// and fail somewhere that doesn't say why.
pub async fn catalogs_fetched(core: &Core) {
    nexus_testkit::until(core, Area::Catalogs, |core| {
        let sources = core.view().catalog.sources;
        !sources.is_empty() && sources.iter().all(|s| s.contains(": fetched from "))
    })
    .await;
}

/// Moves the core's clock one round of the game watcher (the fake host's `watch_every`) at a
/// time until `done` is true, for at most five seconds of it, then shows it: the watcher's
/// rounds are what change the desktop and the game's state.
pub fn watch_until(host: &mut Composed, what: &str, done: impl Fn(&Core) -> bool) {
    let mut rounds = 0;
    while !done(host.core()) {
        assert!(rounds < 250, "not {what} after five seconds of the clock");
        host.advance(std::time::Duration::from_millis(20));
        rounds += 1;
    }
    host.ui.render().unwrap();
    host.settle();
}

/// Lets the theme's moves reach the core, as the app would: a move in the Home row or a grid
/// goes to the core a moment after the last one. It moves the clock twice, as [`settle`], then
/// carries out what the theme asked for.
pub fn rest(host: &mut Composed) {
    for _ in 0..2 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(150));
        host.ui.render().unwrap();
    }
    host.pump();
}

/// Writes a data-only theme folder `<parent>/<theme_id>` with `theme.toml` built from
/// `theme_section` and the given files.
pub fn write_theme(parent: &Path, theme_id: &str, theme_section: &str, files: &[(&str, &str)]) {
    let dir = parent.join(theme_id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("theme.toml"),
        format!(
            "id = \"{theme_id}\"\nkind = \"theme\"\nname = \"{theme_id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
             capabilities = [\"theme\"]\n[runtime]\ntype = \"data\"\n[theme]\nui_api = \"^0.2\"\n{theme_section}\n"
        ),
    )
    .unwrap();
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

/// The test-only theme in `tests/fixtures/themes`: a grid that extends the default theme.
pub fn fixture_themes() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/themes")
}
