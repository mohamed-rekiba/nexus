//! A running game, from its spawn to its end. A game is its processes, as the
//! [`ProcessTree`](crate::ProcessTree) port lists them: the one the launcher started, its process
//! group, and what a sandbox moved out of the group below them. The game ends when none of them
//! is left, so a start script that starts the game in the background and exits keeps its
//! session. On the way the module shows the game's window, keeps `game_in_front` up to date,
//! and carries out Stop.
//!
//! A launch can hand the game to another program (`LaunchSpec::game_process`), as
//! `steam steam://rungameid/…` asks Steam's client to start it. The game is then the process
//! with those words in its command line, once it shows, and the processes below it; the program
//! started here is left to itself.

use std::time::Duration;

use nexus_api::SessionId;
use tokio::sync::oneshot;

use crate::changes::Area;
use crate::{Core, Front, ProcessExit, ProcessHandle, ProcessTree, Windows};

/// How often the module looks for a started game's window, until it shows.
const SHOW_EVERY: Duration = Duration::from_millis(250);
/// On a desktop where a game ends with its window: the window was gone this many looks in a
/// row. More than one, so a window that is remade (full screen on or off) doesn't count.
const WINDOW_GONE_FOR: u32 = 3;
/// How often Stop looks whether the game closed, within the host's `stop_grace`.
const STOP_EVERY: Duration = Duration::from_millis(50);

/// The exit of a game that a kill ended.
const KILLED: ProcessExit = ProcessExit { code: None, success: false };
/// The exit of a game another program started, which ended by itself: its code is that
/// program's to know.
const ENDED: ProcessExit = ProcessExit { code: None, success: true };

/// The process a session follows as the game's first.
enum Followed {
    /// The one the launcher started.
    Started(Box<dyn ProcessHandle>),
    /// One another program started, found by its command line.
    Found(u32),
}

/// Runs a window or process-list call, which may run a helper program, off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    core: &Core,
    call: impl FnOnce(&dyn Windows, &dyn ProcessTree) -> T + Send + 'static,
) -> T {
    let (desktop, processes) = (core.kernel().windows().clone(), core.kernel().processes().clone());
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

/// A launch that hands the game to another program: the text of the game's command line, and
/// the processes that had it before the launch (the same game started from Steam itself),
/// which aren't the one this launch starts.
pub(crate) struct HandOver {
    game_process: String,
    before: Vec<u32>,
}

/// The hand-over of a launch whose game has `game_process` in its command line: looked up
/// before the launch starts anything.
pub(crate) async fn hand_over(core: &Core, game_process: String) -> HandOver {
    let looking = game_process.clone();
    let before = blocking(core, move |_, processes| processes.find(&looking)).await;
    HandOver { game_process, before }
}

/// The session `session` of the game `process` started, until it ends: its first process's
/// exit (or a kill's). `stop` is Stop. With `hand_over`, `process` hands the game over to the
/// process with that text in its command line. `None` when nothing was played: that process
/// never showed within the host's `hand_over_timeout`, or Stop came first.
pub(crate) async fn run(
    core: Core,
    session: SessionId,
    process: Box<dyn ProcessHandle>,
    mut stop: oneshot::Receiver<()>,
    hand_over: Option<HandOver>,
) -> Option<ProcessExit> {
    let Some(HandOver { game_process, before }) = hand_over else {
        return Some(follow(core, session, Followed::Started(process), stop).await);
    };
    // Steam's client, when the launch started it, keeps running after the game: it is reaped
    // here, never stopped.
    tokio::spawn(async move {
        let mut process = process;
        if let Err(e) = process.wait().await {
            tracing::debug!(error = %e, "the program that handed the game over wasn't waited for");
        }
    });
    let (host, clock) = (core.kernel().host().clone(), core.kernel().clock().clone());
    let deadline = clock.now() + host.hand_over_timeout;
    loop {
        tokio::select! {
            // Nothing of the game runs yet: nothing to close, and nothing was played.
            _ = &mut stop => return None,
            () = clock.sleep(SHOW_EVERY.min(host.watch_every)) => {}
        }
        let looking = game_process.clone();
        let found = blocking(&core, move |_, processes| processes.find(&looking)).await;
        if let Some(id) = found.into_iter().find(|id| !before.contains(id)) {
            tracing::info!(%session, id, "the game another program started is running");
            if let Some(s) = core.state().sessions.get_mut(&session) {
                s.pid = Some(id);
            }
            return Some(follow(core, session, Followed::Found(id), stop).await);
        }
        if clock.now() >= deadline {
            return None;
        }
    }
}

/// The game whose first process is `followed`, until none of its processes is left.
async fn follow(
    core: Core,
    session: SessionId,
    mut followed: Followed,
    mut stop: oneshot::Receiver<()>,
) -> ProcessExit {
    let (host, clock) = (core.kernel().host().clone(), core.kernel().clock().clone());
    let root = match &followed {
        Followed::Started(process) => process.id(),
        Followed::Found(id) => Some(*id),
    };
    let deadline = clock.now() + host.show_timeout;
    let mut first: Option<ProcessExit> = None;
    let mut shown = false;
    // Only a game whose window was seen can lose it: a desktop that can't see the game's
    // window (a Wayland window, which xdotool can't find) must not take it for closed.
    let mut had_window = false;
    let mut gone = 0;
    let exit = loop {
        let every = if shown { host.watch_every } else { SHOW_EVERY.min(host.watch_every) };
        tokio::select! {
            exit = wait(&mut followed), if first.is_none() => {
                first = Some(exit);
            }
            _ = &mut stop => break close(&core, &mut followed, root, first).await,
            () = clock.sleep(every) => {}
        }
        let root_exited = first.is_some();
        if !shown {
            // Until the game shows, only its processes and one try to show them: each call
            // may run a helper program.
            let waited = clock.now() >= deadline;
            let (ids, showing, front) = blocking(&core, move |desktop, processes| {
                let ids = members(processes, root, root_exited);
                let showing = !ids.is_empty() && desktop.show_game(&ids);
                let front = if !showing && waited { desktop.front() } else { Front::Unknown };
                (ids, showing, front)
            })
            .await;
            ended_by_itself(&followed, &ids, &mut first);
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
                core.changed(Area::Sessions);
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
        ended_by_itself(&followed, &look.ids, &mut first);
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
                core.changed(Area::Sessions);
            }
        }
    };
    // What the first process left in its group goes with it (on Windows, what it started).
    if let Followed::Started(process) = &mut followed
        && let Err(e) = process.kill().await
    {
        tracing::warn!(error = %e, "could not stop what the game left running");
    }
    exit
}

/// The first process's exit: the launcher's own, waited for. A found one has no handle to wait
/// on: the process list says when it ended ([`ended_by_itself`]).
async fn wait(followed: &mut Followed) -> ProcessExit {
    match followed {
        Followed::Started(process) => process.wait().await.unwrap_or(KILLED),
        Followed::Found(_) => std::future::pending().await,
    }
}

/// A found first process ended when the process list no longer has it.
fn ended_by_itself(followed: &Followed, ids: &[u32], first: &mut Option<ProcessExit>) {
    if let Followed::Found(id) = followed
        && first.is_none()
        && !ids.contains(id)
    {
        *first = Some(ENDED);
    }
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
async fn close(core: &Core, followed: &mut Followed, root: Option<u32>, mut first: Option<ProcessExit>) -> ProcessExit {
    // A launcher's process is waited for, so its id leaves the list once it exited; a found
    // one stays in it while it runs.
    let drop_root = matches!(followed, Followed::Started(_));
    let root_exited = first.is_some();
    let ids = blocking(core, move |_, processes| members(processes, root, root_exited)).await;
    if let Followed::Started(process) = followed
        && let Err(e) = process.terminate().await
    {
        tracing::warn!(error = %e, "could not ask the game to close");
    }
    let asked = ids.clone();
    blocking(core, move |_, processes| processes.end(&asked, false)).await;
    let clock = core.kernel().clock();
    let deadline = clock.now() + core.kernel().host().stop_grace;
    while clock.now() < deadline {
        let waited = match followed {
            Followed::Started(process) if first.is_none() => clock.timeout(STOP_EVERY, process.wait()).await,
            _ => None,
        };
        match waited {
            Some(Ok(exit)) => first = Some(exit),
            // Timed out: the wait was the pause. Otherwise pause here, or this loop spins.
            None if first.is_none() && drop_root => {}
            _ => clock.sleep(STOP_EVERY).await,
        }
        let left = blocking(core, move |_, processes| members(processes, root, drop_root)).await;
        ended_by_itself(followed, &left, &mut first);
        if let Some(exit) = first
            && left.is_empty()
        {
            return exit;
        }
    }
    tracing::info!("the game didn't close: killing it");
    if let Followed::Started(process) = followed
        && let Err(e) = process.kill().await
    {
        tracing::warn!(error = %e, "could not stop the game");
    }
    let left = blocking(core, move |_, processes| members(processes, root, drop_root)).await;
    blocking(core, move |_, processes| processes.end(&left, true)).await;
    match (first, followed) {
        (Some(exit), _) => exit,
        (None, Followed::Started(process)) => process.wait().await.unwrap_or(KILLED),
        (None, Followed::Found(_)) => KILLED,
    }
}

/// The desktop's call for the launcher's window, off the async threads.
pub(crate) async fn show_launcher(core: &Core) {
    blocking(core, |desktop, _| desktop.show_launcher()).await;
}

/// Opens `link` with the system's opener, off the async threads. The caller checked it against
/// the plugin's `permissions.links`.
pub(crate) async fn open_link(core: &Core, link: String) -> Result<(), String> {
    let opener = core.kernel().opener().clone();
    tokio::task::spawn_blocking(move || opener.open_link(&link)).await.expect("the opener doesn't panic")
}

/// Why the desktop can't do its part, once a run: a missing tool or permission.
pub(crate) async fn desktop_problem(core: &Core) -> Option<String> {
    blocking(core, |desktop, _| desktop.problem()).await
}
