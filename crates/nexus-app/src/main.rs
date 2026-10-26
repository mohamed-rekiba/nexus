//! The launcher.
//!
//! The core runs on a tokio runtime. The UI runs on the main thread in Slint's event loop.
//! Core events schedule a render on the UI thread; theme callbacks become core commands on
//! the runtime.

mod archives;
mod audio;
mod clock;
mod desktop;
mod disk;
mod gamepad;
mod gpu_probe;
mod hardware;
mod http;
mod libarchive;
mod processes;
#[cfg(test)]
mod real_releases;
mod torrents;
mod trailer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use nexus_api::PluginId;
use nexus_core::{Command, Core, HostConfig, Kernel};
use nexus_ui_host::{ThemeHost, UiRequest};
use slint_interpreter::ComponentHandle;

// Linked for their compile-time plugin factories.
use emu_kyty as _;
use emu_pc as _;
use emu_shadps4 as _;
use playstation_store as _;
use release_catalog as _;
use system_host as _;
use theme_default as _;

const USAGE: &str = "\
nexus [options]

  --theme <id>        Show this theme now (not saved)
  --safe-mode         Load built-in plugins and the default theme only
  --dev               Reload theme files when they change
  --windowed          Open in a normal window for this run (full screen is the default)
  --monitor <name>    Open on this monitor for this run, such as DP-2 (not saved; a screen
                      picked in Settings wins)
  --data-dir <path>   Where settings and plugin data live
  --extensions <path> Folder of third-party plugins and themes (default: <data-dir>/extensions)
  --folder <name>=<path>
                      Map a named host folder, such as games=/srv/games (saved; repeat
                      for more folders under one name)
  --supports <feature>
                      Exit 0 when this launcher has the feature (gamescope,
                      system-settings, os-install), else 1
  --version           Show the version, on one line
  --help              Show this help

Environment:
  NEXUS_SCALE=<n>     Draw everything at this size for this run, from 0.8 to 1.4 (overrides
                      Size of everything in Settings)
";

struct Args {
    theme: Option<PluginId>,
    safe_mode: bool,
    dev: bool,
    windowed: bool,
    monitor: Option<String>,
    data_dir: PathBuf,
    extensions: Option<PathBuf>,
    folders: std::collections::BTreeMap<String, Vec<PathBuf>>,
}

fn home_dir() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name).map(PathBuf::from)
}

fn default_data_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming")).join("nexus")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/nexus")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("nexus")
    }
}

/// Themes and plugins that ship with the launcher: `extensions` next to the binary (and, in a
/// macOS app bundle, in its `Resources`). A packaged build puts the example themes there.
fn bundled_extensions() -> PathBuf {
    let Ok(exe) = std::env::current_exe() else { return PathBuf::from("extensions") };
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    let resources = dir.join("../Resources/extensions");
    if resources.is_dir() { resources } else { dir.join("extensions") }
}

/// What `--version` prints: one line, which Nexus OS keeps in its release marker.
fn version_line() -> String {
    format!("nexus {}", env!("CARGO_PKG_VERSION"))
}

/// Whether this launcher has `feature`, for `--supports` (docs/nexus-os.md):
/// - `gamescope`: it runs in a Gamescope session and sets the window ids Gamescope shows;
/// - `system-settings`: Settings → System for Nexus OS;
/// - `os-install`: the stick's welcome and install screens, which the stick's build checks.
///
/// A launcher that doesn't know the option exits with an error, so a session can tell the two
/// apart.
fn supports(feature: &str) -> bool {
    matches!(feature, "gamescope" | "system-settings" | "os-install")
}

/// The size of everything from `NEXUS_SCALE`, for this run: a number, which the theme host
/// keeps between 0.8 and 1.4. `None` when it isn't set or isn't a number.
fn size_override(value: Option<&str>) -> Option<f32> {
    value?.trim().parse::<f32>().ok().filter(|size| size.is_finite())
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        theme: None,
        safe_mode: false,
        dev: false,
        windowed: false,
        monitor: None,
        data_dir: default_data_dir(),
        extensions: None,
        folders: Default::default(),
    };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--theme" => args.theme = Some(PluginId::new(value("--theme")?).map_err(|e| e.to_string())?),
            "--safe-mode" => args.safe_mode = true,
            "--dev" => args.dev = true,
            "--windowed" => args.windowed = true,
            "--monitor" => args.monitor = Some(value("--monitor")?),
            "--data-dir" => args.data_dir = PathBuf::from(value("--data-dir")?),
            "--extensions" => args.extensions = Some(PathBuf::from(value("--extensions")?)),
            "--folder" => {
                let mapping = value("--folder")?;
                let (name, path) = mapping.split_once('=').ok_or("--folder needs <name>=<path>")?;
                args.folders.entry(name.to_owned()).or_default().push(PathBuf::from(path));
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("{}", version_line());
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    // The graphics probe the "This PC" check runs in a child process: print the cards, exit.
    let raw: Vec<String> = std::env::args().collect();
    if let Some(at) = raw.iter().position(|a| a == "--supports") {
        std::process::exit(if raw.get(at + 1).is_some_and(|f| supports(f)) { 0 } else { 1 });
    }
    if raw.iter().any(|a| a == "--probe-gpu") {
        let data_dir = raw.iter().position(|a| a == "--data-dir").and_then(|i| raw.get(i + 1)).map(PathBuf::from);
        let found = gpu_probe::print(&data_dir.unwrap_or_else(default_data_dir));
        std::process::exit(if found { 0 } else { 1 });
    }
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    apply_launcher_update(&args.data_dir);
    if let Err(message) = run(args) {
        tracing::error!("{message}");
        eprintln!("launcher: {message}");
        std::process::exit(1);
    }
}

/// What the UI thread owns.
struct App {
    ui: ThemeHost,
    core: Core,
    runtime: tokio::runtime::Handle,
    images: Arc<ImageLoader>,
}

/// Loads images as the old launcher's image pool: off the UI thread, a few at a time, from a
/// disk cache when it can. Big art is made smaller once, and the smaller copy kept in
/// `cache/thumbs`, so a warm start decodes small JPEGs instead of 4K PNGs.
struct ImageLoader {
    thumbs: PathBuf,
    fetching: tokio::sync::Semaphore,
    decoding: tokio::sync::Semaphore,
}

/// How many images are fetched at once, and how many decode at once.
const FETCH_AT_ONCE: usize = 8;
const DECODE_AT_ONCE: usize = 4;

type Pixels = slint_interpreter::SharedPixelBuffer<slint_interpreter::Rgba8Pixel>;

impl ImageLoader {
    /// The smaller copy of an image at a size: `<hash>-thumb.jpg` or `<hash>-full.jpg`, and
    /// `<hash>-cut-thumb.jpg` or `<hash>-cut-full.jpg` for one with a crop.
    fn thumb(&self, (image, size, crop): &nexus_ui_host::ImageKey) -> PathBuf {
        let mut hash = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&format!("{image:?}"), &mut hash);
        let cut = if *crop == nexus_ui_host::Crop::None { "" } else { "cut-" };
        self.thumbs.join(format!("{:016x}-{cut}{}.jpg", std::hash::Hasher::finish(&hash), size.as_str()))
    }

    /// `None` when the image left the screen before its turn: nothing is fetched or decoded.
    async fn load(
        &self,
        core: &Core,
        key: &nexus_ui_host::ImageKey,
        still: &nexus_ui_host::StillWanted,
    ) -> Option<Result<Pixels, String>> {
        if !still.contains(key) {
            return None;
        }
        let thumb = self.thumb(key);
        let (bytes, from_thumb) = match tokio::fs::read(&thumb).await {
            Ok(bytes) => (bytes, true),
            Err(_) => {
                let turn = match self.fetching.acquire().await {
                    Ok(turn) => turn,
                    Err(e) => return Some(Err(e.to_string())),
                };
                // It may have left the screen while it waited.
                if !still.contains(key) {
                    return None;
                }
                let fetched = core.image_bytes(&key.0).await;
                drop(turn);
                match fetched {
                    Ok(bytes) => (bytes, false),
                    Err(e) => return Some(Err(e.to_string())),
                }
            }
        };
        let turn = match self.decoding.acquire().await {
            Ok(turn) => turn,
            Err(e) => return Some(Err(e.to_string())),
        };
        if !still.contains(key) {
            return None;
        }
        let size = key.1;
        // The kept copy is cut already.
        let crop = if from_thumb { nexus_ui_host::Crop::None } else { key.2 };
        let prepared = tokio::task::spawn_blocking(move || nexus_ui_host::prepare(&bytes, size, crop)).await;
        drop(turn);
        Some(self.keep(prepared, &thumb, from_thumb).await)
    }

    async fn keep(
        &self,
        prepared: Result<Result<nexus_ui_host::Prepared, String>, tokio::task::JoinError>,
        thumb: &std::path::Path,
        from_thumb: bool,
    ) -> Result<Pixels, String> {
        let prepared = prepared.map_err(|e| e.to_string())??;
        if !from_thumb && let Some(smaller) = &prepared.smaller {
            let _ = tokio::fs::create_dir_all(&self.thumbs).await;
            if let Err(e) = tokio::fs::write(thumb, smaller).await {
                tracing::debug!(error = %e, "could not keep a smaller copy");
            }
        }
        Ok(prepared.pixels)
    }
}

/// Deletes the files in the cover cache `dir` whose names [`ImageLoader::thumb`] doesn't make.
/// Folders, and everything outside `dir`, stay.
fn sweep_thumbs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let kept = name.to_str().is_some_and(|name| {
            let hash = name.strip_suffix("-thumb.jpg").or_else(|| name.strip_suffix("-full.jpg"));
            let hash = hash.map(|h| h.strip_suffix("-cut").unwrap_or(h));
            hash.is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        });
        if !kept && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Opens an https:// page in the system's browser.
fn open_with_system(url: &str) {
    let url = url.to_owned();
    let opener = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C".into(), "start".into(), String::new(), url])
    } else {
        ("xdg-open", vec![url])
    };
    if let Err(error) = std::process::Command::new(opener.0).args(&opener.1).spawn() {
        tracing::warn!(%error, "could not open the page");
    }
}

/// The trailer player, from the theme. Frames come from the player's thread to the window.
fn play_trailer(request: nexus_ui_host::TrailerRequest) {
    use nexus_ui_host::{TrailerRequest, TrailerStatus};
    match request {
        TrailerRequest::Play(url) => {
            let frame = |pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>| {
                let _ = slint_interpreter::invoke_from_event_loop(move || {
                    with_app(|app| app.ui.show_trailer_frame(pixels))
                });
            };
            match trailer::open(&url, frame) {
                Ok(()) => with_app(|app| {
                    app.ui.set_trailer(Some(TrailerStatus { loading: true, ..TrailerStatus::default() }))
                }),
                // The browser plays it instead, and the player is told why.
                Err(error) => {
                    tracing::warn!(%error, "the trailer didn't open in the window");
                    open_with_system(&url);
                    with_app(|app| app.core.report(format!("The trailer opened in your browser: {error}")));
                }
            }
        }
        TrailerRequest::TogglePause => trailer::toggle_pause(),
        TrailerRequest::Seek(seconds) => trailer::seek(seconds, false),
        TrailerRequest::Close => {
            trailer::close();
            with_app(|app| app.ui.set_trailer(None));
        }
    }
    render();
}

/// The installed launcher this binary is part of: the AppImage file, `Nexus.app`, or the
/// package folder (with its LICENSE and extensions/). `None` for a build that isn't
/// installed, such as `cargo run`'s: an update never replaces that.
fn install_root() -> Option<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(PathBuf::from(appimage));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if dir.ends_with("Contents/MacOS") {
        return dir.parent()?.parent().filter(|app| app.extension().is_some_and(|e| e == "app")).map(PathBuf::from);
    }
    (dir.join("LICENSE").is_file() && dir.join("extensions").is_dir()).then(|| dir.to_path_buf())
}

/// The program the system starts at login: the AppImage file, or this binary in an installed
/// package or app bundle. `None` for a build that isn't installed.
fn login_program() -> Option<PathBuf> {
    let root = install_root()?;
    if root.is_file() { Some(root) } else { std::env::current_exe().ok() }
}

/// A launcher update kept by the last run goes in now, and the new version starts instead.
fn apply_launcher_update(data_dir: &std::path::Path) {
    use nexus_core::launcher_update::{Install, apply_staged, clean_old_files, staged};
    // A package manager or Nexus OS owns these files: nothing is ever swapped in here.
    if matches!(Install::detect().method(), "package" | "system") {
        return;
    }
    let Some(root) = install_root() else {
        if staged(data_dir).is_some() {
            tracing::warn!("a launcher update is waiting, but this build isn't installed from a package");
        }
        return;
    };
    clean_old_files(&root);
    if let Some(update) = staged(data_dir).filter(|s| s.file.to_string_lossy().ends_with("-setup.exe")) {
        run_installer(data_dir, &update.file);
        return;
    }
    // Where this binary sits in the install, for the new one at the same place.
    let exe = std::env::current_exe().ok();
    let inside = exe.as_deref().and_then(|e| e.strip_prefix(&root).ok()).map(PathBuf::from);
    match apply_staged(data_dir, &root) {
        Ok(Some(version)) => {
            tracing::info!(%version, "launcher updated; starting the new version");
            // The old binary was moved away: start the new one in its place.
            restart_as(inside.map(|p| root.join(p)));
        }
        Ok(None) => {}
        Err(error) => tracing::error!(%error, "the launcher update didn't go in; this version runs"),
    }
}

/// Windows' installer put this launcher in: the new installer replaces it, silently, and starts
/// the new version when it's done (its `[Run]` entry), so this one ends. Tried once.
fn run_installer(data_dir: &std::path::Path, installer: &std::path::Path) {
    let _ = std::fs::remove_file(data_dir.join("updates/ready.json"));
    match std::process::Command::new(installer).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).spawn() {
        Ok(_) => {
            tracing::info!(installer = %installer.display(), "updating with the new installer");
            std::process::exit(0);
        }
        Err(error) => tracing::error!(%error, "the new installer didn't start; this version runs"),
    }
}

/// Starts this launcher again with the same arguments, and ends this one.
fn restart() {
    restart_as(None);
}

/// Like [`restart`], with `exe` in place of this binary when it moved.
fn restart_as(exe: Option<PathBuf>) {
    let exe = exe.or_else(|| std::env::var_os("APPIMAGE").map(PathBuf::from)).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    match std::process::Command::new(&exe).args(std::env::args_os().skip(1)).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => tracing::error!(%error, "could not start the launcher again"),
    }
}

/// Set by Restart to update: the launcher starts again once it has shut down.
static RESTART: AtomicBool = AtomicBool::new(false);

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            f(app);
        }
    });
}

/// macOS full screen without a Space of its own: the window fills the screen over the menu bar
/// and Dock in the current Space. In its own Space, bringing the app to the front after a game
/// closed didn't switch back to that Space, so the launcher stayed out of sight.
#[cfg(target_os = "macos")]
fn simple_fullscreen(window: &slint::Window, on: bool) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::platform::macos::WindowExtMacOS;
    window.with_winit_window(|w| {
        if w.simple_fullscreen() != on {
            w.set_simple_fullscreen(on);
        }
    });
}

/// Whether the launcher's window has the keyboard focus, as the window system says: the pad
/// reads controllers whatever window is in front. Without a window to ask, it has.
fn window_has_focus(app: &App) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    app.ui.instance().and_then(|i| i.window().with_winit_window(|w| w.has_focus())).unwrap_or(true)
}

/// The monitors' names, and the launcher's window moved to the one the player chose. Full
/// screen goes off for the move and on again, so it fills the new monitor.
///
/// `flag` is `--monitor`, with the saved monitor at the start. It wins over the saved monitor
/// until the player picks a screen in Settings.
fn place_on_monitor(
    app: &mut App,
    last: &mut Vec<String>,
    placed: &mut Option<String>,
    flag: &mut Option<(String, Option<String>)>,
) {
    use slint::winit_030::WinitWindowAccessor;
    let Some(instance) = app.ui.instance() else { return };
    let saved = app.core.monitor();
    if flag.as_ref().is_some_and(|(_, at_start)| *at_start != saved) {
        *flag = None;
    }
    let wanted = flag.as_ref().map(|(name, _)| name.clone()).or(saved);
    let fullscreen = app.core.fullscreen_on();
    let mut names = Vec::new();
    // Moved once per choice: the player may still drag the window elsewhere.
    let changed = wanted != *placed;
    instance.window().with_winit_window(|w| {
        names = w.available_monitors().filter_map(|m| m.name()).collect();
        if !changed {
            return;
        }
        let Some(wanted) = &wanted else { return };
        let Some(target) = w.available_monitors().find(|m| m.name().as_deref() == Some(wanted.as_str())) else {
            return;
        };
        if w.current_monitor().and_then(|m| m.name()).as_deref() == Some(wanted.as_str()) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            use slint::winit_030::winit::platform::macos::WindowExtMacOS;
            w.set_simple_fullscreen(false);
            w.set_outer_position(target.position());
            if fullscreen {
                w.set_simple_fullscreen(true);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            use slint::winit_030::winit::window::Fullscreen;
            w.set_fullscreen(None);
            w.set_outer_position(target.position());
            if fullscreen {
                w.set_fullscreen(Some(Fullscreen::Borderless(Some(target))));
            }
        }
    });
    if changed && wanted.as_ref().is_none_or(|w| names.contains(w)) {
        *placed = wanted;
    }
    if names != *last {
        app.ui.set_monitors(names.clone());
        *last = names;
    }
}

/// The longest the app waits for the core to close down when it quits.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(10);

/// Ctrl-C, or a request to close the app (SIGTERM, SIGHUP): quits as the Quit button does, so
/// the app closes down. A second one quits at once.
async fn quit_on_signal() {
    #[cfg(unix)]
    let asked = || async {
        use tokio::signal::unix::{SignalKind, signal};
        let kinds = [SignalKind::interrupt(), SignalKind::terminate(), SignalKind::hangup()];
        let mut signals: Vec<_> = kinds.into_iter().filter_map(|k| signal(k).ok()).collect();
        if signals.is_empty() {
            return std::future::pending().await;
        }
        let waits = signals.iter_mut().map(|s| Box::pin(s.recv()));
        futures::future::select_all(waits).await;
    };
    #[cfg(not(unix))]
    let asked = || async {
        let _ = tokio::signal::ctrl_c().await;
    };
    asked().await;
    tracing::info!("asked to quit; closing down");
    let _ = slint_interpreter::invoke_from_event_loop(|| {
        let _ = slint::quit_event_loop();
    });
    asked().await;
    tracing::warn!("asked again; quitting at once");
    std::process::exit(130);
}

/// Brings the launcher's window to the front, from any thread.
fn raise_launcher() {
    let _ = slint_interpreter::invoke_from_event_loop(|| {
        with_app(|app| {
            use slint::winit_030::WinitWindowAccessor;
            let Some(instance) = app.ui.instance() else { return };
            instance.window().with_winit_window(|window| {
                // Wayland can't tell or undo a minimize, and warns on every try.
                if window.is_minimized() == Some(true) {
                    window.set_minimized(false);
                }
                window.focus_window();
            });
        });
    });
}

/// The least time between two renders: a frame at 120 Hz. A burst of images or core changes
/// renders once a frame, so the frames between keep their time for animations.
const RENDER_GAP: Duration = Duration::from_millis(8);

/// How long images that come in wait to show together. Covers arrive one by one while the
/// player scrolls; each showing on its own would take the UI thread's time from the scroll.
const IMAGES_GAP: Duration = Duration::from_millis(100);

thread_local! {
    static IMAGES_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RENDER_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_RENDER: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Renders once, however many images or core changes arrive before it: each render rebuilds
/// and rebinds the whole view. At once, unless the last render was less than [`RENDER_GAP`]
/// ago. On the UI thread.
fn schedule_render() {
    if RENDER_PENDING.with(|p| p.replace(true)) {
        return;
    }
    let since = LAST_RENDER.with(|l| l.get()).map(|at| at.elapsed()).unwrap_or(RENDER_GAP);
    slint::Timer::single_shot(RENDER_GAP.saturating_sub(since), || {
        RENDER_PENDING.with(|p| p.set(false));
        render();
    });
}

/// Shows the images that came in, at most once each [`IMAGES_GAP`], with the view already
/// bound: nothing in the core changed. A render waiting to run shows them anyway.
fn schedule_images() {
    if IMAGES_PENDING.with(|p| p.replace(true)) {
        return;
    }
    slint::Timer::single_shot(IMAGES_GAP, || {
        IMAGES_PENDING.with(|p| p.set(false));
        if RENDER_PENDING.with(|p| p.get()) {
            return;
        }
        LAST_RENDER.with(|l| l.set(Some(std::time::Instant::now())));
        with_app(|app| {
            if let Err(error) = app.ui.render_images() {
                tracing::error!(%error, "render failed");
            }
            load_wanted(app);
        });
    });
}

/// Shows the current state, and starts loading any images it needs.
fn render() {
    LAST_RENDER.with(|l| l.set(Some(std::time::Instant::now())));
    with_app(|app| {
        if let Err(error) = app.ui.render() {
            tracing::error!(%error, "render failed");
        }
        load_wanted(app);
    });
}

/// Starts loading the images the last render asked for and doesn't have.
fn load_wanted(app: &mut App) {
    {
        let still = app.ui.still_wanted();
        for key in app.ui.wanted_images() {
            let core = app.core.clone();
            let loader = app.images.clone();
            let still = still.clone();
            app.runtime.spawn(async move {
                let pixels = loader.load(&core, &key, &still).await;
                let _ = slint_interpreter::invoke_from_event_loop(move || {
                    with_app(|app| app.ui.add_pixels(key, pixels));
                    schedule_images();
                });
            });
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    let mut host = HostConfig::new(&args.data_dir);
    host.home = home_dir();
    host.safe_mode = args.safe_mode;
    let extensions = args.extensions.clone().unwrap_or_else(|| args.data_dir.join("extensions"));

    let core = runtime.block_on(async {
        let (kernel, report) = Kernel::builder(host)
            .http(Arc::new(http::ReqwestTransport::new()?))
            .processes(Arc::new(processes::SystemProcesses))
            .desktop(Arc::new(desktop::SystemDesktop { raise: Box::new(raise_launcher), login: login_program() }))
            .hardware(Arc::new(hardware::SystemHardware { data_dir: args.data_dir.clone() }))
            .torrents(Arc::new(torrents::RqbitTorrents::new(args.data_dir.join("torrents"), runtime.handle().clone())))
            .archives(Arc::new(archives::HostArchives))
            .built_in_plugins()
            .discover(&extensions)
            .discover(&bundled_extensions())
            .boot()
            .await;
        for rejected in &report.rejected {
            tracing::warn!(source = %rejected.source, error = %rejected.error, "plugin not loaded");
        }
        tracing::info!(plugins = ?report.order, "booted");
        let core = Core::start(kernel).await;
        for (name, paths) in &args.folders {
            core.dispatch(Command::SetFolders { name: name.clone(), paths: paths.clone() })
                .await
                .map_err(|e| format!("--folder {name}: {e}"))?;
        }
        Ok::<_, String>(core)
    })?;
    let handle = runtime.handle().clone();
    handle.spawn(quit_on_signal());

    // Theme callbacks: core commands go to the runtime. A theme switch waits until the
    // current callback has returned, since the switch drops the view tree running it.
    let player = audio::Player::start();
    let requests = {
        let core = core.clone();
        let handle = handle.clone();
        Rc::new(move |request: UiRequest| match request {
            UiRequest::Core(command) => {
                let core = core.clone();
                handle.spawn(async move {
                    if let Err(error) = core.dispatch(command).await {
                        tracing::warn!(%error, "command failed");
                    }
                });
            }
            UiRequest::SwitchTheme(theme) => {
                slint::Timer::single_shot(Duration::ZERO, move || {
                    with_app(|app| {
                        if let Err(error) = app.ui.switch(&theme) {
                            tracing::error!(%error, "theme switch failed; keeping the current theme");
                        }
                    });
                });
            }
            UiRequest::OpenUrl(url) => open_with_system(&url),
            UiRequest::OpenPath(path) => {
                // A folder opens in the file manager, a log in the system's text viewer.
                let opener = if cfg!(target_os = "macos") {
                    "open"
                } else if cfg!(windows) {
                    "explorer"
                } else {
                    "xdg-open"
                };
                if let Err(error) = std::process::Command::new(opener).arg(&path).spawn() {
                    tracing::warn!(%error, ?path, "could not open it");
                }
            }
            UiRequest::Quit => {
                let _ = slint::quit_event_loop();
            }
            UiRequest::Restart => {
                RESTART.store(true, Ordering::SeqCst);
                let _ = slint::quit_event_loop();
            }
            // The Nexus session reads this line when Nexus ends, and ends too instead of
            // starting it again.
            UiRequest::LogOut => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = writeln!(out, "log out").and_then(|()| out.flush());
                let _ = slint::quit_event_loop();
            }
            UiRequest::Trailer(request) => play_trailer(request),
            UiRequest::Render => schedule_render(),
            UiRequest::PlaySound(name) => {
                let player = player.clone();
                // Not inside the theme's callback: the UI host may be borrowed there.
                slint::Timer::single_shot(Duration::ZERO, move || {
                    with_app(|app| match app.ui.sound(&name) {
                        Some(bytes) => player.play(&name, bytes),
                        None => tracing::debug!(%name, "no such sound, or sounds are off"),
                    });
                });
            }
        })
    };

    let mut ui = runtime.block_on(ThemeHost::new(core.clone(), requests));
    ui.set_trailers_available(trailer::available());
    ui.set_windowed(args.windowed);
    let size = std::env::var("NEXUS_SCALE").ok();
    if let Some(size) = size_override(size.as_deref()) {
        ui.set_size(size);
    }
    #[cfg(target_os = "macos")]
    ui.set_fullscreen_with(Box::new(simple_fullscreen));
    ui.start_with(args.theme.clone()).map_err(|e| format!("the default theme failed to start: {e}"))?;
    let thumbs = args.data_dir.join("cache/thumbs");
    handle.spawn_blocking({
        let thumbs = thumbs.clone();
        move || sweep_thumbs(&thumbs)
    });
    let images = Arc::new(ImageLoader {
        thumbs,
        fetching: tokio::sync::Semaphore::new(FETCH_AT_ONCE),
        decoding: tokio::sync::Semaphore::new(DECODE_AT_ONCE),
    });
    APP.with(|app| *app.borrow_mut() = Some(App { ui, core: core.clone(), runtime: handle.clone(), images }));

    // Controllers: each press goes to the theme on the UI thread.
    gamepad::spawn(
        |button, repeated| {
            let _ = slint_interpreter::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.ui.set_window_focused(window_has_focus(app));
                    let pressed = if repeated { app.ui.pad_repeated(button) } else { app.ui.pad(button) };
                    if let Err(error) = pressed {
                        tracing::warn!(%error, "controller input failed");
                    }
                });
            });
        },
        // The controller's name, for the launch splash.
        |name| {
            let _ = slint_interpreter::invoke_from_event_loop(move || {
                with_app(|app| app.ui.set_pad_name(name));
                render();
            });
        },
    );

    // Core events: at most one render waiting on the UI thread.
    let pending = Arc::new(AtomicBool::new(false));
    let mut events = core.events();
    handle.spawn(async move {
        while events.next().await.is_some() {
            if !pending.swap(true, Ordering::SeqCst) {
                let pending = pending.clone();
                let _ = slint_interpreter::invoke_from_event_loop(move || {
                    pending.store(false, Ordering::SeqCst);
                    schedule_render();
                });
            }
        }
    });

    // Window size changes move breakpoints; dev mode reloads edited themes; while a game runs,
    // the time played on the Running screen moves on.
    let timer = slint::Timer::default();
    let dev = args.dev;
    let mut last_size = None;
    let mut ticks = 0u32;
    let mut last_clock = String::new();
    let mut monitors = Vec::new();
    let mut placed_on = None;
    let mut monitor_flag = args.monitor.clone().map(|name| (name, core.monitor()));
    let mut announced = false;
    // Under Gamescope the window shows only once it carries Gamescope's id (desktop.rs).
    let mut claimed = std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_none();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
        let mut resized = false;
        let mut playing = false;
        let mut changed_clock = false;
        let mut waiting = false;
        let mut trailer_open = false;
        ticks = ticks.wrapping_add(1);
        with_app(|app| {
            if dev {
                match app.ui.reload_if_changed() {
                    Ok(true) => tracing::info!("theme reloaded"),
                    Ok(false) => {}
                    Err(error) => tracing::error!(%error, "theme edit doesn't load; keeping the last good version"),
                }
            }
            place_on_monitor(app, &mut monitors, &mut placed_on, &mut monitor_flag);
            let size = app.ui.instance().map(|i| i.window().size());
            // The Nexus session's watchdog waits for this line: a start that never shows a window
            // is started again.
            if !announced && size.is_some_and(|s| s.width > 0 && s.height > 0) {
                announced = true;
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = writeln!(out, "window shown").and_then(|()| out.flush());
            }
            if announced && !claimed {
                claimed = desktop::claim_gamescope();
            }
            resized = size != last_size;
            last_size = size;
            playing = app.core.game_running();
            waiting = app.ui.waiting_for_art();
            // The trailer's time and state, and its end.
            if let Some(status) = trailer::poll() {
                if status.ended.is_some() {
                    trailer::close();
                    app.ui.set_trailer(None);
                } else {
                    app.ui.set_trailer(Some(nexus_ui_host::TrailerStatus {
                        playing: !status.paused,
                        loading: status.loading,
                        position: status.position,
                        duration: status.duration,
                    }));
                }
                trailer_open = true;
            }
            let now = clock::now();
            if now != last_clock {
                app.ui.set_clock(now.clone());
                last_clock = now;
                changed_clock = true;
            }
        });
        // Every 15 seconds: the time is shown in minutes.
        // While the boot screen waits for Home's art, each tick: it has a time limit.
        if resized || changed_clock || waiting || trailer_open || (playing && ticks.is_multiple_of(60)) {
            render();
        }
    });

    let starting = core.clone();
    handle.spawn(async move {
        let _ = starting.dispatch(Command::RefreshLibrary).await;
    });
    render();

    slint_interpreter::run_event_loop().map_err(|e| e.to_string())?;
    drop(timer);
    APP.with(|app| app.borrow_mut().take());
    // While the runtime runs: the torrents write down their pieces on its tasks. A shutdown
    // that hangs doesn't keep the app open.
    if runtime.block_on(async { tokio::time::timeout(SHUTDOWN_LIMIT, core.shutdown()).await }).is_err() {
        tracing::warn!("shutting down took more than {} s; quitting anyway", SHUTDOWN_LIMIT.as_secs());
    }
    // Restart to update: the next start puts the kept update in.
    if RESTART.load(Ordering::SeqCst) {
        restart();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn parse(args: &[&str]) -> Result<super::Args, String> {
        super::parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn monitor_takes_a_name_for_this_run() {
        assert_eq!(parse(&["--monitor", "DELL U2720Q"]).unwrap().monitor.as_deref(), Some("DELL U2720Q"));
        assert_eq!(parse(&["--windowed"]).unwrap().monitor, None);
        assert_eq!(parse(&["--monitor"]).err().as_deref(), Some("--monitor needs a value"));
        assert!(parse(&["--screen", "x"]).err().unwrap_or_default().starts_with("unknown option --screen"));
    }

    #[test]
    fn nexus_scale_sets_the_size_when_it_is_a_number() {
        assert_eq!(super::size_override(Some("1.25")), Some(1.25));
        assert_eq!(super::size_override(Some(" 0.9 ")), Some(0.9));
        assert_eq!(super::size_override(Some("big")), None);
        assert_eq!(super::size_override(Some("NaN")), None);
        assert_eq!(super::size_override(None), None);
    }

    #[test]
    fn version_is_one_line_with_the_release_version() {
        let line = super::version_line();
        assert!(!line.contains('\n'), "{line:?}");
        let workspace = include_str!("../../../Cargo.toml");
        let version = workspace.lines().find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"')).unwrap();
        assert_eq!(line, format!("nexus {version}"));
    }

    #[test]
    fn the_launcher_supports_gamescope_the_os_s_settings_and_its_install() {
        assert!(super::supports("gamescope"));
        assert!(super::supports("system-settings"));
        assert!(super::supports("os-install"), "the stick's build checks it");
        assert!(!super::supports("wayland"));
        assert!(!super::supports(""));
    }

    #[test]
    fn the_cover_cache_keeps_only_the_names_the_loader_makes() {
        let dir = std::env::temp_dir().join(format!("nexus-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let names = [
            "0123456789abcdef-thumb.jpg",
            "0123456789abcdef-full.jpg",
            "0123456789abcdef-cut-thumb.jpg",
            "0123456789abcdef.jpg",
            "0123456789abcdef-cut.jpg",
            "notes.txt",
        ];
        for name in names {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::write(dir.join("folder/0123456789abcdef.jpg"), b"x").unwrap();

        super::sweep_thumbs(&dir);
        let mut left: Vec<_> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(
            left,
            ["0123456789abcdef-cut-thumb.jpg", "0123456789abcdef-full.jpg", "0123456789abcdef-thumb.jpg", "folder"]
        );
        assert!(dir.join("folder/0123456789abcdef.jpg").exists(), "a folder is left as it is");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
