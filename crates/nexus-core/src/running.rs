//! A running game, from its spawn to its end. A game is its processes, as the
//! [`ProcessTree`](crate::ProcessTree) port lists them: the one the launcher started, its process
//! group, and what a sandbox moved out of the group below them. The game ends when none of them
//! is left, so a start script that starts the game in the background and exits keeps its
//! session. On the way the module shows the game's window, keeps `game_in_front` up to date,
//! and carries out Stop.

use std::time::{Duration, Instant};

use nexus_api::SessionId;
use tokio::sync::oneshot;

use crate::{Core, Desktop, Front, ProcessExit, ProcessHandle, ProcessTree};

/// How often the module looks for a started game's window, until it shows.
const SHOW_EVERY: Duration = Duration::from_millis(250);
/// On a desktop where a game ends with its window: the window was gone this many looks in a
/// row. More than one, so a window that is remade (full screen on or off) doesn't count.
const WINDOW_GONE_FOR: u32 = 3;
/// How often Stop looks whether the game closed, within the host's `stop_grace`.
const STOP_EVERY: Duration = Duration::from_millis(50);

/// The exit of a game that a kill ended.
const KILLED: ProcessExit = ProcessExit { code: None, success: false };

/// Runs a desktop or process-list call, which may run a helper program, off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    core: &Core,
    call: impl FnOnce(&dyn Desktop, &dyn ProcessTree) -> T + Send + 'static,
) -> T {
    let (desktop, processes) = (core.kernel().desktop().clone(), core.kernel().processes().clone());
    tokio::task::spawn_blocking(move || call(desktop.as_ref(), processes.as_ref()))
        .await
        .expect("desktop and process calls don't panic")
}

/// The game's live processes. Once the process the launcher started has exited, its id is no
/// longer the game's: the system may give it to another program.
fn members(processes: &dyn ProcessTree, root: Option<u32>, root_exited: bool) -> Vec<u32> {
    let Some(root) = root else { return Vec::new() };
    let mut ids = processes.members(root);
    if root_exited {
        ids.retain(|id| *id != root);
    }
    ids
}

/// Brings the game started as `root` to the front; `false` when it has no window yet.
pub(crate) async fn show(core: &Core, root: Option<u32>) -> bool {
    blocking(core, move |desktop, processes| {
        let ids = members(processes, root, false);
        !ids.is_empty() && desktop.show_game(&ids)
    })
    .await
}

/// The session `session` of the game `process`, until it ends: its first process's exit (or a
/// kill's). `stop` is Stop.
pub(crate) async fn run(
    core: Core,
    session: SessionId,
    mut process: Box<dyn ProcessHandle>,
    mut stop: oneshot::Receiver<()>,
) -> ProcessExit {
    let host = core.kernel().host().clone();
    let root = process.id();
    let deadline = Instant::now() + host.show_timeout;
    let mut first: Option<ProcessExit> = None;
    let mut shown = false;
    // Only a game whose window was seen can lose it: a desktop that can't see the game's
    // window (a Wayland window, which xdotool can't find) must not take it for closed.
    let mut had_window = false;
    let mut gone = 0;
    let exit = loop {
        let every = if shown { host.watch_every } else { SHOW_EVERY.min(host.watch_every) };
        tokio::select! {
            exit = process.wait(), if first.is_none() => {
                first = Some(exit.unwrap_or(KILLED));
            }
            _ = &mut stop => break close(&core, process.as_mut(), root, first).await,
            _ = tokio::time::sleep(every) => {}
        }
        let root_exited = first.is_some();
        if !shown {
            // Until the game shows, only its processes and one try to show them: each call
            // may run a helper program.
            let waited = Instant::now() >= deadline;
            let (ids, showing, front) = blocking(&core, move |desktop, processes| {
                let ids = members(processes, root, root_exited);
                let showing = !ids.is_empty() && desktop.show_game(&ids);
                let front = if !showing && waited { desktop.front() } else { Front::Unknown };
                (ids, showing, front)
            })
            .await;
            if let Some(exit) = first
                && ids.is_empty()
            {
                break exit;
            }
            if showing || waited {
                shown = true;
                had_window = showing;
                // A desktop that can't say (Windows, or macOS without the Accessibility
                // permission) can't show the game either; the game's own window took the
                // screen as it opened, so the pad is the game's until the PS button.
                let in_front = showing || front == Front::Unknown;
                let mut state = core.state();
                if let Some(s) = state.sessions.get_mut(&session) {
                    s.shown = true;
                    state.game_in_front |= in_front;
                }
                drop(state);
                core.changed();
            }
            continue;
        }
        let look = blocking(&core, move |desktop, processes| {
            let ids = members(processes, root, root_exited);
            let front = desktop.front();
            let window = if ids.is_empty() { Some(false) } else { desktop.has_window(&ids) };
            Look { ends_with_window: desktop.window_close_ends_game(), front, window, ids }
        })
        .await;
        if let Some(exit) = first
            && look.ids.is_empty()
        {
            break exit;
        }
        had_window |= look.window == Some(true);
        gone = if had_window && look.window == Some(false) { gone + 1 } else { 0 };
        if look.ends_with_window && gone >= WINDOW_GONE_FOR {
            tracing::info!(%session, "the game's window closed: stopping it");
            if let Err(e) = crate::launch::stop(&core, session) {
                tracing::warn!(error = %e, "could not stop the game");
            }
            continue;
        }
        let in_front = match look.front {
            Front::Launcher => Some(false),
            Front::Other(front) => Some(look.ids.contains(&front)),
            // The last window the launcher showed counts.
            Front::Unknown => None,
        };
        if let Some(in_front) = in_front {
            let mut state = core.state();
            let changed = state.game_in_front != in_front;
            state.game_in_front = in_front;
            drop(state);
            if changed {
                core.changed();
            }
        }
    };
    // What the first process left in its group goes with it (on Windows, what it started).
    if let Err(e) = process.kill().await {
        tracing::warn!(error = %e, "could not stop what the game left running");
    }
    exit
}

/// One look at the game: its processes, which window is in front, whether it has a window.
struct Look {
    ids: Vec<u32>,
    front: Front,
    window: Option<bool>,
    ends_with_window: bool,
}

/// Stop: asks the game to close, so the emulator saves its caches, and kills what is left
/// after the host's `stop_grace`.
async fn close(
    core: &Core,
    process: &mut dyn ProcessHandle,
    root: Option<u32>,
    mut first: Option<ProcessExit>,
) -> ProcessExit {
    let root_exited = first.is_some();
    let ids = blocking(core, move |_, processes| members(processes, root, root_exited)).await;
    if let Err(e) = process.terminate().await {
        tracing::warn!(error = %e, "could not ask the game to close");
    }
    let asked = ids.clone();
    blocking(core, move |_, processes| processes.end(&asked, false)).await;
    let deadline = Instant::now() + core.kernel().host().stop_grace;
    while Instant::now() < deadline {
        let waited = if first.is_none() { tokio::time::timeout(STOP_EVERY, process.wait()).await.ok() } else { None };
        match waited {
            Some(Ok(exit)) => first = Some(exit),
            // Timed out: the wait was the pause. Otherwise pause here, or this loop spins.
            None if first.is_none() => {}
            _ => tokio::time::sleep(STOP_EVERY).await,
        }
        let left = blocking(core, move |_, processes| members(processes, root, true)).await;
        if let Some(exit) = first
            && left.is_empty()
        {
            return exit;
        }
    }
    tracing::info!("the game didn't close: killing it");
    if let Err(e) = process.kill().await {
        tracing::warn!(error = %e, "could not stop the game");
    }
    let left = blocking(core, move |_, processes| members(processes, root, true)).await;
    blocking(core, move |_, processes| processes.end(&left, true)).await;
    match first {
        Some(exit) => exit,
        None => process.wait().await.unwrap_or(KILLED),
    }
}

/// The desktop's call for the launcher's window, off the async threads.
pub(crate) async fn show_launcher(core: &Core) {
    blocking(core, |desktop, _| desktop.show_launcher()).await;
}

/// Why the desktop can't do its part, once a run: a missing tool or permission.
pub(crate) async fn desktop_problem(core: &Core) -> Option<String> {
    blocking(core, |desktop, _| desktop.problem()).await
}
