//! Renders the default theme's screens on a real library, to compare them with the old
//! launcher's. Opt in: `NEXUS_COMPARE_OUT=<folder for the PNGs>`, with the real KytyPS5 plugin
//! on `NEXUS_COMPARE_GAMES` (default `~/Games`) and, to play, the builds in
//! `NEXUS_COMPARE_BUILDS` (an `emu-kyty` plugin folder, cloned). Its own test binary: it
//! installs its own Slint platform.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::common::*;
use nexus_core::{Area, Command, Route};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint_interpreter::ComponentHandle;

struct Offscreen(Rc<MinimalSoftwareWindow>);

thread_local! {
    /// Slint's time on this thread: it moves only when `shoot` moves it.
    static ELAPSED: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        ELAPSED.with(|e| e.get())
    }
    fn debug_log(&self, arguments: std::fmt::Arguments) {
        if keeps_log_line(&arguments) {
            eprintln!("{arguments}");
        }
    }
}

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 900;

fn shoot(host: &mut Composed, window: &MinimalSoftwareWindow, path: &Path) {
    host.load_images();
    let mut pixels = vec![PremultipliedRgbaColor::default(); (WIDTH * HEIGHT) as usize];
    for _ in 0..4 {
        slint::platform::update_timers_and_animations();
        host.pump();
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, WIDTH as usize);
        });
        // Slint's time, and the core's: the store's art and the game watcher move on too.
        ELAPSED.with(|e| e.set(e.get() + Duration::from_millis(400)));
        host.advance(Duration::from_millis(400));
    }
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    image::save_buffer(path, &bytes, WIDTH, HEIGHT, image::ColorType::Rgba8).unwrap();
    println!("{}", path.display());
}

fn key(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
}

/// Copies a folder, symlinks as they are, with the system's `cp` (a clone where it can).
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    let flags = if cfg!(target_os = "macos") { "-cR" } else { "-R" };
    let status = std::process::Command::new("cp").arg(flags).arg(from).arg(to).status().unwrap();
    assert!(status.success(), "cp {from:?}");
}

#[test]
fn the_default_theme_on_a_real_library() {
    let Some(out) = std::env::var_os("NEXUS_COMPARE_OUT").map(PathBuf::from) else {
        eprintln!("SKIPPED: set NEXUS_COMPARE_OUT to render the screens on a real library");
        return;
    };
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let games = std::env::var_os("NEXUS_COMPARE_GAMES").map(PathBuf::from).unwrap_or(home.join("Games"));
    let builds = std::env::var_os("NEXUS_COMPARE_BUILDS").map(PathBuf::from);
    std::fs::create_dir_all(&out).unwrap();

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let mut host = composed_on_current_platform(|h| {
        if let Some(builds) = &builds {
            copy_tree(builds, &h.data_dir().join("plugins/emu-kyty"));
        }
        // Sony's answers and art, saved beforehand: the test's network is a fake one.
        if let Some(web) = std::env::var_os("NEXUS_COMPARE_WEB") {
            let manifest: std::collections::BTreeMap<String, String> =
                serde_json::from_slice(&std::fs::read(Path::new(&web).join("manifest.json")).unwrap()).unwrap();
            for (url, file) in manifest {
                h.http().serve(&url, std::fs::read(file).unwrap());
            }
        }
        h.first_run()
            .home(&home)
            .built_in(nexus_api::PluginByKind::emulator(emu_kyty::KytyPlugin))
            .built_in(nexus_api::PluginByKind::service(playstation_store::PlayStationStore))
    });
    host.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games] });
    host.ui.set_clock("9:38 PM".into());
    host.ui.start().unwrap();
    use slint::platform::Key;
    shoot(&mut host, &window, &out.join("new-boot.png"));
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    // The store's descriptions come in the background.
    host.until(Area::Library, |core| core.view().task.is_none() && core.view().boot.ready);
    shoot(&mut host, &window, &out.join("new-welcome.png"));
    key(&mut host, Key::Return);
    let first = host.core().view().home.games.first().map(|g| g.key.clone()).expect("a game in the folder");
    // The old screens show this play time.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let stats = serde_json::json!({ "secs": 4 * 3600 + 6 * 60, "count": 2, "last": now });
    host.core().kernel().config().set(&["app", "playtime", &first], stats).unwrap();
    host.dispatch(Command::Select(first.parse().unwrap()));
    host.dispatch(Command::DismissNotices);

    shoot(&mut host, &window, &out.join("new-home.png"));
    key(&mut host, Key::DownArrow);
    shoot(&mut host, &window, &out.join("new-home-actions.png"));
    key(&mut host, Key::UpArrow);
    // The Library, from the top bar.
    key(&mut host, Key::UpArrow);
    key(&mut host, Key::RightArrow);
    key(&mut host, Key::Return);
    shoot(&mut host, &window, &out.join("new-library.png"));
    key(&mut host, Key::Escape);
    host.dispatch(Command::OpenGame(first.parse().unwrap()));
    shoot(&mut host, &window, &out.join("new-hub.png"));
    host.dispatch(Command::Navigate(Route::Settings));
    shoot(&mut host, &window, &out.join("new-settings.png"));
    host.dispatch(Command::Navigate(Route::Home));
    host.dispatch(Command::Navigate(Route::Downloads));
    shoot(&mut host, &window, &out.join("new-downloads.png"));
    host.dispatch(Command::Navigate(Route::Home));
    // A game that starts: the fake spawner starts nothing.
    host.test.desktop.no_windows();
    host.dispatch(Command::Launch(first.parse().unwrap()));
    shoot(&mut host, &window, &out.join("new-launch.png"));
    host.test.desktop.windows_open();
    if host.core().view().install_prompt.is_some() {
        eprintln!("SKIPPED the running game: no KytyPS5 build in NEXUS_COMPARE_BUILDS, so Play asks to install it");
        return;
    }
    while host.core().view().running.is_some_and(|r| r.state == "starting") {
        host.advance(Duration::from_millis(20));
    }
    shoot(&mut host, &window, &out.join("new-home-playing.png"));
    let session = nexus_api::SessionId(host.core().view().running.unwrap().session);
    host.test.spawner.spawned()[0].exit(1);
    while host.core().game_running() {
        host.advance(Duration::from_millis(20));
    }
    let _ = session;
    shoot(&mut host, &window, &out.join("new-toast.png"));
}
