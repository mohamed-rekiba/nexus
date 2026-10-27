//! The launcher.
//!
//! The core runs on a tokio runtime. The UI runs on the main thread in Slint's event loop,
//! where the app shell (`nexus_ui_host::AppShell`) owns the theme host. This file wires the
//! two: core events, controller presses and the tick go to the shell on the UI thread, and the
//! shell reaches the system through `platform.rs`.

mod archives;
mod args;
mod audio;
mod clock;
mod desktop;
mod disk;
mod gamepad;
mod gpu_probe;
mod hardware;
mod http;
mod libarchive;
mod platform;
mod processes;
#[cfg(test)]
mod real_releases;
mod torrents;
mod trailer;
mod update;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use nexus_core::{Command, Core, HostConfig, Kernel, TokioClock};
use nexus_ui_host::{AppShell, ShellOptions};

// Linked for their compile-time plugin factories.
use emu_kyty as _;
use emu_pc as _;
use emu_shadps4 as _;
use emu_steam as _;
use playstation_store as _;
use release_catalog as _;
use system_host as _;
use theme_default as _;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    // The graphics probe the "This PC" check runs in a child process: print the cards, exit.
    let raw: Vec<String> = std::env::args().collect();
    if let Some(at) = raw.iter().position(|a| a == "--supports") {
        std::process::exit(if raw.get(at + 1).is_some_and(|f| args::supports(f)) { 0 } else { 1 });
    }
    if raw.iter().any(|a| a == "--probe-gpu") {
        let data_dir = raw.iter().position(|a| a == "--data-dir").and_then(|i| raw.get(i + 1)).map(PathBuf::from);
        let found = gpu_probe::print(&data_dir.unwrap_or_else(args::default_data_dir));
        std::process::exit(if found { 0 } else { 1 });
    }
    let args = match args::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    update::apply_launcher_update(&args.data_dir);
    if let Err(message) = run(args) {
        tracing::error!("{message}");
        eprintln!("launcher: {message}");
        std::process::exit(1);
    }
}

/// The longest the app waits for the core to close down when it quits.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(10);

/// What the UI thread owns.
struct App {
    shell: AppShell,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the UI thread's app, once it is there and until it closes.
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            f(app);
        }
    });
}

fn run(args: args::Args) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    let mut host = HostConfig::new(&args.data_dir);
    host.home = args::home_dir();
    host.safe_mode = args.safe_mode;
    let extensions = args.extensions.clone().unwrap_or_else(|| args.data_dir.join("extensions"));

    let core = runtime.block_on(async {
        let (kernel, report) = Kernel::builder(host)
            .http(Arc::new(http::ReqwestTransport::new()?))
            .processes(Arc::new(processes::SystemProcesses))
            .windows(Arc::new(desktop::SystemWindows { raise: Box::new(platform::raise_launcher) }))
            .login_session(Arc::new(desktop::SystemLoginSession { login: update::login_program() }))
            .opener(Arc::new(desktop::SystemOpener))
            .hardware(Arc::new(hardware::SystemHardware { data_dir: args.data_dir.clone() }))
            .torrents(Arc::new(torrents::RqbitTorrents::new(args.data_dir.join("torrents"))))
            .archives(Arc::new(archives::HostArchives))
            .clock(Arc::new(TokioClock))
            .built_in_plugins()
            .discover(&extensions)
            .discover(&args::bundled_extensions())
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
    handle.spawn(platform::quit_on_signal());

    let restart = Arc::new(AtomicBool::new(false));
    let system =
        platform::SystemPlatform { runtime: handle.clone(), player: audio::Player::start(), restart: restart.clone() };
    let thumbs = args.data_dir.join("cache/thumbs");
    let options = ShellOptions { dev: args.dev, monitor: args.monitor.clone(), thumbs: thumbs.clone() };
    let mut shell = runtime.block_on(AppShell::new(core.clone(), Rc::new(system), Arc::new(TokioClock), options));
    shell.ui.set_trailers_available(trailer::available());
    shell.ui.set_windowed(args.windowed);
    if let Some(size) = args::size_override(std::env::var("NEXUS_SCALE").ok().as_deref()) {
        shell.ui.set_size(size);
    }
    #[cfg(target_os = "macos")]
    shell.ui.set_fullscreen_with(Box::new(platform::simple_fullscreen));
    shell.ui.start_with(args.theme.clone()).map_err(|e| format!("the default theme failed to start: {e}"))?;
    handle.spawn_blocking(move || nexus_ui_host::sweep_thumbs(&thumbs));
    APP.with(|app| *app.borrow_mut() = Some(App { shell }));

    // Controllers: each press, and the controller's name for the launch splash. The event loop
    // refuses work only once the app closes, so a refusal drops the press.
    gamepad::spawn(
        |button, repeated| {
            let _ =
                slint_interpreter::invoke_from_event_loop(move || with_app(|app| app.shell.on_pad(button, repeated)));
        },
        |name| {
            let _ = slint_interpreter::invoke_from_event_loop(move || with_app(|app| app.shell.on_pad_name(name)));
        },
    );

    // Core events: at most one waiting on the UI thread; the shell coalesces the renders.
    let pending = Arc::new(AtomicBool::new(false));
    let mut events = core.events();
    handle.spawn(async move {
        while let Some(event) = events.next().await {
            if !pending.swap(true, Ordering::SeqCst) {
                let pending = pending.clone();
                // Refused only once the app closes: no render is due then.
                let _ = slint_interpreter::invoke_from_event_loop(move || {
                    pending.store(false, Ordering::SeqCst);
                    with_app(|app| app.shell.on_core_event(&event));
                });
            }
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, nexus_ui_host::TICK, || with_app(|app| app.shell.on_tick()));

    let starting = core.clone();
    handle.spawn(async move {
        if let Err(error) = starting.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh)).await {
            tracing::warn!(%error, "the first Library refresh failed");
        }
    });
    with_app(|app| {
        if let Err(error) = app.shell.render() {
            tracing::error!(%error, "render failed");
        }
    });

    slint_interpreter::run_event_loop().map_err(|e| e.to_string())?;
    drop(timer);
    APP.with(|app| app.borrow_mut().take());
    // While the runtime runs: the torrents write down their pieces on its tasks. A shutdown
    // that hangs doesn't keep the app open.
    if runtime.block_on(async { tokio::time::timeout(SHUTDOWN_LIMIT, core.shutdown()).await }).is_err() {
        tracing::warn!("shutting down took more than {} s; quitting anyway", SHUTDOWN_LIMIT.as_secs());
    }
    // Restart to update: the next start puts the kept update in.
    if restart.load(Ordering::SeqCst) {
        update::restart();
    }
    Ok(())
}
