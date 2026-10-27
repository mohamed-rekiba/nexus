//! The shell's platform: the system calls the app shell (`nexus_ui_host::AppShell`) makes, on
//! the UI thread. Work runs on the tokio runtime; what comes back reaches the shell through
//! Slint's event loop.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use nexus_core::Opener;
use nexus_ui_host::{Platform, TrailerPoll, TrailerStatus};
use slint::winit_030::WinitWindowAccessor;
use slint_interpreter::ComponentHandle;

use crate::desktop::SystemOpener;
use crate::{audio, clock, desktop, trailer, with_app};

pub struct SystemPlatform {
    pub runtime: tokio::runtime::Handle,
    pub player: audio::Player,
    /// Set by Restart: the launcher starts again once it has shut down.
    pub restart: Arc<AtomicBool>,
}

/// Calls the shell's poll on the UI thread.
fn poll_shell() {
    with_app(|app| app.shell.poll());
}

impl Platform for SystemPlatform {
    fn spawn(&self, work: BoxFuture<'static, Result<(), String>>) {
        self.runtime.spawn(async move {
            if let Err(error) = work.await {
                tracing::warn!(%error, "command failed");
            }
            let _ = slint_interpreter::invoke_from_event_loop(poll_shell);
        });
    }

    fn wake_in(&self, after: Duration) {
        slint::Timer::single_shot(after, poll_shell);
    }

    fn open_url(&self, url: &str) {
        if let Err(error) = SystemOpener.open_link(url) {
            tracing::warn!(%error, "could not open the page");
        }
    }

    fn open_path(&self, path: &Path) {
        if let Err(error) = SystemOpener.open_path(path) {
            tracing::warn!(%error, ?path, "could not open it");
        }
    }

    fn quit(&self) {
        let _ = slint::quit_event_loop();
    }

    fn restart(&self) {
        self.restart.store(true, Ordering::SeqCst);
        self.quit();
    }

    fn announce(&self, line: &str) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{line}").and_then(|()| out.flush());
    }

    fn play_sound(&self, name: &str, bytes: Arc<[u8]>) {
        self.player.play(name, bytes);
    }

    fn local_time(&self) -> String {
        clock::now()
    }

    fn monitors(&self, window: &slint::Window) -> Vec<String> {
        window.with_winit_window(|w| w.available_monitors().filter_map(|m| m.name()).collect()).unwrap_or_default()
    }

    /// Full screen goes off for the move and on again, so it fills the new monitor.
    fn move_to_monitor(&self, window: &slint::Window, monitor: &str, fullscreen: bool) {
        window.with_winit_window(|w| {
            let Some(target) = w.available_monitors().find(|m| m.name().as_deref() == Some(monitor)) else {
                return;
            };
            if w.current_monitor().and_then(|m| m.name()).as_deref() == Some(monitor) {
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
    }

    /// The pad reads controllers whatever window is in front. Without a window to ask, the
    /// launcher has the focus.
    fn window_focused(&self, window: &slint::Window) -> bool {
        window.with_winit_window(|w| w.has_focus()).unwrap_or(true)
    }

    /// Under Gamescope the window shows only once it carries Gamescope's id (desktop.rs).
    fn claim_window(&self) -> bool {
        std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_none() || desktop::claim_gamescope()
    }

    /// Frames come from the player's thread to the window.
    fn play_trailer(&self, url: &str) -> Result<(), String> {
        let frame = |pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>| {
            let _ = slint_interpreter::invoke_from_event_loop(move || {
                with_app(|app| app.shell.ui.show_trailer_frame(pixels))
            });
        };
        trailer::open(url, frame)
    }

    fn pause_trailer(&self) {
        trailer::toggle_pause();
    }

    fn seek_trailer(&self, seconds: f64) {
        trailer::seek(seconds, false);
    }

    fn close_trailer(&self) {
        trailer::close();
    }

    fn poll_trailer(&self) -> Option<TrailerPoll> {
        let status = trailer::poll()?;
        Some(match status.ended {
            Some(_) => TrailerPoll::Ended,
            None => TrailerPoll::Playing(TrailerStatus {
                playing: !status.paused,
                loading: status.loading,
                position: status.position,
                duration: status.duration,
            }),
        })
    }
}

/// macOS full screen without a Space of its own: the window fills the screen over the menu bar
/// and Dock in the current Space. In its own Space, bringing the app to the front after a game
/// closed didn't switch back to that Space, so the launcher stayed out of sight.
#[cfg(target_os = "macos")]
pub fn simple_fullscreen(window: &slint::Window, on: bool) {
    use slint::winit_030::winit::platform::macos::WindowExtMacOS;
    window.with_winit_window(|w| {
        if w.simple_fullscreen() != on {
            w.set_simple_fullscreen(on);
        }
    });
}

/// Brings the launcher's window to the front, from any thread.
pub fn raise_launcher() {
    let _ = slint_interpreter::invoke_from_event_loop(|| {
        with_app(|app| {
            let Some(instance) = app.shell.ui.instance() else { return };
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

/// Ctrl-C, or a request to close the app (SIGTERM, SIGHUP): quits as the Quit button does, so
/// the app closes down. A second one quits at once.
pub async fn quit_on_signal() {
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
