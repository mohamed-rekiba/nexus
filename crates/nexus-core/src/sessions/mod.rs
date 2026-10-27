//! The sessions subsystem: the games that run, from their spawn to their end, and the
//! launcher's and the game's windows while they do.
//!
//! One game runs at a time: the gamepad and the screen belong to it. A launch claims the
//! subsystem ([`SessionsSubsystem::claim`]) before its checks, so two presses of Play can't
//! both start a game. The core resolves the launch (the plugin's spec, launch middleware,
//! permissions and paths) and hands the subsystem a command to spawn ([`NewSession`]). The
//! subsystem spawns it and watches it ([`watch`]) until none of the game's processes is left,
//! or Stop closes it. The core then ends the session ([`SessionsSubsystem::end`]): the
//! subsystem saves the play time (`app.playtime`, [`playtime`]), and the core does what the
//! other areas need, such as notices, the "How far did it get?" question and the plugin's
//! `on_exit`.
//!
//! It follows the subsystem pattern of [`crate::torrents`]: its own lock, only the ports it
//! needs ([`SessionPorts`]), [`SessionsSubsystem::handle`] and [`SessionsSubsystem::snapshot`],
//! and [`Area::Sessions`] on the core's change signal. It never calls out while it holds its
//! lock, and never calls into the core.

mod playtime;
mod watch;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use nexus_api::{Event, GameKey, PluginId, SessionId};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub(crate) use self::playtime::{PLAYED_AT_LEAST, days_since, play_stats};
use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::{Clock, CoreError, CoreResult, Desktop, EventBus, ProcessExit, ProcessSpawner, ProcessTree, SpawnCommand};

/// What the player asks of the running games.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SessionCmd {
    /// Starts a game. One game runs at a time.
    Launch(GameKey),
    /// Asks the game to close, and kills it if it doesn't.
    Stop(SessionId),
    /// Brings the running game's window back to the front.
    ShowGame(SessionId),
    /// The PS button while a game runs: from the game to the launcher, or back to the game.
    ToggleFocus,
}

/// A running game.
#[derive(Clone, Debug)]
pub(crate) struct Session {
    pub game: GameKey,
    pub title: String,
    pub stopping: bool,
    /// The game's window came to the front, or the launcher stopped waiting for one.
    pub shown: bool,
    /// The system's id for the game's process.
    pub pid: Option<u32>,
    /// By the subsystem's clock.
    pub started: Instant,
    /// Where the game's output goes.
    pub log: PathBuf,
    /// The emulator build the game runs on, when the emulator has builds.
    pub build: Option<String>,
}

/// The subsystem's state, read whole.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionsSnapshot {
    pub sessions: BTreeMap<SessionId, Session>,
    /// The last window the launcher brought to the front was the game's, or the desktop says
    /// the game's window is in front.
    pub game_in_front: bool,
}

impl SessionsSnapshot {
    /// One of `game`'s sessions runs.
    pub fn runs(&self, game: &GameKey) -> bool {
        self.sessions.values().any(|s| &s.game == game)
    }
}

/// A game to start, with everything the core resolved for it.
pub(crate) struct NewSession {
    pub game: GameKey,
    pub title: String,
    pub build: Option<String>,
    /// The checked program, with its real paths. The subsystem sets its log.
    pub command: SpawnCommand,
    /// Where the game's output goes.
    pub log: PathBuf,
    /// The text of the game's command line when the program hands the game to another one
    /// (`LaunchSpec::game_process`).
    pub game_process: Option<String>,
}

/// A started session.
pub(crate) struct Started {
    pub session: SessionId,
    /// When it started, by the subsystem's clock.
    pub at: Instant,
    /// Resolves when the game ended: its first process's exit (or a kill's). `None` when
    /// nothing was played: the game another program starts never showed, or Stop came first.
    pub ended: BoxFuture<'static, Option<ProcessExit>>,
}

/// What the core must do about the sessions.
#[derive(Clone, Debug)]
pub(crate) enum SessionEvent {
    /// Why the desktop can't do its part, once a run, at the first game.
    Problem(String),
}

/// Where the subsystem's events go.
pub(crate) type EventSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

/// The ports the subsystem uses, and nothing else.
pub(crate) struct SessionPorts {
    pub spawner: Arc<dyn ProcessSpawner>,
    pub processes: Arc<dyn ProcessTree>,
    pub desktop: Arc<dyn Desktop>,
    pub clock: Arc<dyn Clock>,
}

/// How long the watch waits for each step of a game's life, from the host's settings.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timings {
    /// How often a shown game is looked at.
    pub watch_every: Duration,
    /// How long a game may take to show its window.
    pub show_timeout: Duration,
    /// How long the game another program starts may take to show up.
    pub hand_over_timeout: Duration,
    /// How long Stop waits for the game to close before it kills it.
    pub stop_grace: Duration,
}

#[derive(Default)]
struct State {
    sessions: BTreeMap<SessionId, Session>,
    /// The last window the launcher brought to the front was the game's. It stands in for
    /// the desktop when the desktop doesn't say which window is in front.
    game_in_front: bool,
    /// A launch is between its checks and its session: no other game may start.
    launching: bool,
    stops: HashMap<SessionId, oneshot::Sender<()>>,
    /// The last session's number.
    last_session: u64,
    /// The desktop was asked what it is missing, at the first game of this run.
    desktop_checked: bool,
}

struct Shared {
    ports: SessionPorts,
    timings: Timings,
    config: Arc<ConfigService>,
    /// For [`Event::ProcessStarted`], which plugins hear.
    bus: EventBus,
    notifier: Notifier,
    events: EventSink,
    state: Mutex<State>,
}

/// The running games. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct SessionsSubsystem {
    inner: Arc<Shared>,
}

/// A launch in progress. While it lives, no other game starts; it lets go when dropped.
pub(crate) struct Claim(SessionsSubsystem);

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.state().launching = false;
    }
}

/// Where a game's output goes: `logs/<plugin>/<game>.log` in the data folder `data_dir`.
pub(crate) fn log_path(data_dir: &Path, game: &GameKey) -> PathBuf {
    let name: String =
        game.local.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    data_dir.join("logs").join(game.plugin.as_str()).join(format!("{name}.log"))
}

/// Keeps the log of the run before as `<game>.previous.log`, so a crash's log survives the
/// next start.
fn keep_previous_log(log: &Path) {
    if log.exists() {
        let previous = log.with_extension("previous.log");
        if let Err(e) = std::fs::rename(log, &previous) {
            tracing::warn!(error = %e, log = %log.display(), "could not keep the last log");
        }
    }
}

impl SessionsSubsystem {
    pub fn new(
        ports: SessionPorts,
        timings: Timings,
        config: Arc<ConfigService>,
        bus: EventBus,
        notifier: Notifier,
        events: EventSink,
    ) -> Self {
        let state = Mutex::new(State::default());
        Self { inner: Arc::new(Shared { ports, timings, config, bus, notifier, events, state }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn changed(&self) {
        self.inner.notifier.changed(Area::Sessions);
    }

    /// The running games and the window in front, as they are now.
    pub fn snapshot(&self) -> SessionsSnapshot {
        let state = self.state();
        SessionsSnapshot { sessions: state.sessions.clone(), game_in_front: state.game_in_front }
    }

    /// A game runs.
    pub fn running(&self) -> bool {
        !self.state().sessions.is_empty()
    }

    /// The running games' sessions.
    pub fn session_ids(&self) -> Vec<SessionId> {
        self.state().sessions.keys().copied().collect()
    }

    /// One of `plugin`'s games runs.
    pub fn plays(&self, plugin: &PluginId) -> bool {
        self.state().sessions.values().any(|s| &s.game.plugin == plugin)
    }

    /// One of `game`'s sessions runs.
    pub fn runs(&self, game: &GameKey) -> bool {
        self.state().sessions.values().any(|s| &s.game == game)
    }

    /// A game has the screen. Reads the subsystem's state only: the watch asks the desktop.
    pub fn game_in_front(&self) -> bool {
        let state = self.state();
        !state.sessions.is_empty() && state.game_in_front
    }

    /// Runs a command that is the subsystem's alone. The core resolves [`SessionCmd::Launch`]
    /// first: it needs the plugin and the library.
    pub async fn handle(&self, command: SessionCmd) -> CoreResult<()> {
        match command {
            SessionCmd::Stop(session) => self.stop(session),
            SessionCmd::ShowGame(session) => self.show_game(session).await,
            SessionCmd::ToggleFocus => {
                self.toggle_focus().await;
                Ok(())
            }
            SessionCmd::Launch(_) => Err(CoreError::Invalid("the core resolves launches first".into())),
        }
    }

    /// One game at a time. The check and the claim happen under one lock, so two presses of
    /// Play can't both start a game.
    pub fn claim(&self) -> CoreResult<Claim> {
        let mut state = self.state();
        if let Some(running) = state.sessions.values().next() {
            return Err(CoreError::Invalid(format!("{} is running: stop it first", running.title)));
        }
        if state.launching {
            return Err(CoreError::Invalid("a game is starting: wait for it".into()));
        }
        state.launching = true;
        Ok(Claim(self.clone()))
    }

    /// Spawns the game the core resolved, adds its session and tells the plugins
    /// ([`Event::ProcessStarted`]). `claim` lets go once the session is there. The caller awaits [`Started::ended`], then calls [`Self::end`].
    pub async fn start(&self, claim: Claim, new: NewSession) -> CoreResult<Started> {
        let NewSession { game, title, build, mut command, log, game_process } = new;
        let plugin = game.plugin.clone();
        command.keep_running = game_process.is_some();
        let hand_over = match game_process {
            Some(game_process) => Some(watch::hand_over(self, game_process).await),
            None => None,
        };
        keep_previous_log(&log);
        command.log = Some(log.clone());
        tracing::info!(plugin_id = %plugin, %game, log = %log.display(), "running: {}", command.shell_line());
        let process = self
            .inner
            .ports
            .spawner
            .spawn(command)
            .await
            .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })?;

        let pid = process.id();
        let started = self.inner.ports.clock.now();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (session, check_desktop) = {
            let mut state = self.state();
            state.last_session += 1;
            let session = SessionId(state.last_session);
            state.sessions.insert(
                session,
                Session { game: game.clone(), title, stopping: false, shown: false, pid, started, log, build },
            );
            state.stops.insert(session, stop_tx);
            (session, !std::mem::replace(&mut state.desktop_checked, true))
        };
        drop(claim);
        tracing::info!(plugin_id = %plugin, %session, %game, ?pid, "game started");
        self.inner.bus.publish(Event::ProcessStarted { session, game });
        self.changed();
        // Once a run, at the first game: what the desktop needs to do its part.
        if check_desktop {
            let this = self.clone();
            tokio::spawn(async move {
                if let Some(problem) = this.blocking(|desktop, _| desktop.problem()).await {
                    (this.inner.events)(SessionEvent::Problem(problem));
                }
            });
        }
        let this = self.clone();
        let ended = Box::pin(async move { watch::run(this, session, process, stop_rx, hand_over).await });
        Ok(Started { session, at: started, ended })
    }

    /// Ends the session once its game ended: it leaves the running games, and the game's
    /// play time grows by `played` (`None` when nothing was played). The session as it ended;
    /// `None` when there was no such session.
    pub fn end(&self, session: SessionId, played: Option<Duration>) -> Option<Session> {
        let ended = {
            let mut state = self.state();
            state.stops.remove(&session);
            state.game_in_front = false;
            state.sessions.remove(&session)
        };
        if let (Some(ended), Some(played)) = (&ended, played) {
            playtime::add_play_time(&self.inner.config, &ended.game, played);
        }
        self.changed();
        ended
    }

    /// Stop: the watch asks the game to close, and kills it if it doesn't.
    pub fn stop(&self, session: SessionId) -> CoreResult<()> {
        {
            let mut state = self.state();
            let stop = state.stops.remove(&session).ok_or(CoreError::UnknownSession(session))?;
            if let Some(s) = state.sessions.get_mut(&session) {
                s.stopping = true;
            }
            let _ = stop.send(());
        }
        self.changed();
        Ok(())
    }

    /// Brings the game's window to the front.
    pub async fn show_game(&self, session: SessionId) -> CoreResult<()> {
        let pid = self.state().sessions.get(&session).ok_or(CoreError::UnknownSession(session))?.pid;
        if !watch::show(self, pid).await {
            return Err(CoreError::Invalid("the game has no window to show yet".into()));
        }
        self.state().game_in_front = true;
        self.changed();
        Ok(())
    }

    /// The PS button: the launcher when the game is in front, else the game.
    pub async fn toggle_focus(&self) {
        let Some(session) = self.state().sessions.keys().next().copied() else { return };
        if self.game_in_front() {
            self.show_launcher().await;
            self.state().game_in_front = false;
            self.changed();
        } else if let Err(e) = self.show_game(session).await {
            tracing::info!(error = %e, "the game can't come to the front");
        }
    }

    /// Brings the launcher's window to the front.
    pub async fn show_launcher(&self) {
        self.blocking(|desktop, _| desktop.show_launcher()).await;
    }

    /// Runs a desktop or process-list call, which may run a helper program, off the async
    /// threads.
    async fn blocking<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn Desktop, &dyn ProcessTree) -> T + Send + 'static,
    ) -> T {
        let (desktop, processes) = (self.inner.ports.desktop.clone(), self.inner.ports.processes.clone());
        tokio::task::spawn_blocking(move || call(desktop.as_ref(), processes.as_ref()))
            .await
            .expect("desktop and process calls don't panic")
    }

    fn clock(&self) -> &Arc<dyn Clock> {
        &self.inner.ports.clock
    }

    fn timings(&self) -> Timings {
        self.inner.timings
    }

    /// The game another program started showed up as process `id`.
    fn found(&self, session: SessionId, id: u32) {
        if let Some(s) = self.state().sessions.get_mut(&session) {
            s.pid = Some(id);
        }
    }

    /// The game showed, or the watch stopped waiting for it; `in_front` when its window took
    /// the screen.
    fn shown(&self, session: SessionId, in_front: bool) {
        {
            let mut state = self.state();
            if let Some(s) = state.sessions.get_mut(&session) {
                s.shown = true;
                state.game_in_front |= in_front;
            }
        }
        self.changed();
    }

    /// The desktop says whether the game's window is in front.
    fn set_in_front(&self, in_front: bool) {
        let changed = {
            let mut state = self.state();
            let changed = state.game_in_front != in_front;
            state.game_in_front = in_front;
            changed
        };
        if changed {
            self.changed();
        }
    }
}
