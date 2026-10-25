//! Launching games: the plugin prepares a spec, launch middleware may change or stop it, the
//! host checks permissions, spawns the process and watches it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nexus_api::{
    Capability, ConfigScope, Endpoint, Event, ExitInfo, GameKey, LaunchArg, LaunchMiddleware, LaunchRequest,
    LaunchSpec, Location, Middleware, Next, Order, Permissions, PluginError, PluginId, PluginResult, SessionId,
};
use serde_json::Value;

use crate::app::{Route, Session};
use crate::error::require;
use crate::order::sort_by_order;
use crate::{Core, CoreError, CoreResult, Desktop, DirStorage, Front, Kernel, ProcessExit, SpawnCommand, folders};

/// Wraps one plugin's middleware so its calls get the plugin's guard and span.
struct Guarded {
    plugin: PluginId,
    kernel: Kernel,
    inner: Arc<LaunchMiddleware>,
}

#[async_trait]
impl Middleware<LaunchRequest, LaunchSpec> for Guarded {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn order(&self) -> Order {
        self.inner.order()
    }

    async fn call(
        &self,
        request: LaunchRequest,
        next: Next<'_, LaunchRequest, LaunchSpec>,
    ) -> PluginResult<LaunchSpec> {
        // `next` runs the rest of the chain inside this guard; each link adds its own.
        self.kernel.guard(&self.plugin, "launch_middleware", self.inner.call(request, next)).await.map_err(
            |e| match e {
                CoreError::Plugin { error, .. } => error,
                other => PluginError::Other(other.to_string()),
            },
        )
    }
}

struct Done;

#[async_trait]
impl Endpoint<LaunchRequest, LaunchSpec> for Done {
    async fn call(&self, request: LaunchRequest) -> PluginResult<LaunchSpec> {
        Ok(request.spec)
    }
}

/// Emulator settings with the game's own settings on top.
async fn merged_config(core: &Core, game: &GameKey) -> CoreResult<Value> {
    let mut config = core.config_value(&game.plugin, &ConfigScope::Emulator).await?;
    let game_config = core.config_value(&game.plugin, &ConfigScope::Game(game.local.clone())).await?;
    if let (Some(base), Some(over)) = (config.as_object_mut(), game_config.as_object()) {
        base.extend(over.clone());
    } else if !game_config.is_null() {
        config = game_config;
    }
    Ok(config)
}

async fn middleware_chain(core: &Core) -> Vec<Arc<LaunchMiddleware>> {
    let kernel = core.kernel();
    let mut links = Vec::new();
    for plugin in kernel.providers(Capability::LaunchMiddleware) {
        let Ok(provided) = kernel.provided(&plugin).await else { continue };
        for inner in provided.launch_middleware {
            let id = format!("{plugin}:{}", inner.name());
            let order = inner.order();
            let link: Arc<LaunchMiddleware> =
                Arc::new(Guarded { plugin: plugin.clone(), kernel: kernel.clone(), inner });
            links.push((id, order, link));
        }
    }
    sort_by_order(links)
}

/// The real paths of a checked spec.
fn spawn_command(
    kernel: &Kernel,
    plugin: &PluginId,
    permissions: &Permissions,
    spec: &LaunchSpec,
) -> CoreResult<SpawnCommand> {
    let storage = DirStorage::new(kernel.host().plugin_dir(plugin));
    let failed = |error| CoreError::Plugin { plugin: plugin.clone(), error };
    let resolve = |location: &Location| -> CoreResult<PathBuf> {
        match location {
            Location::Data(path) => storage.resolve(path).map_err(failed),
            Location::Folder(path) => folders::resolve(
                kernel.config(),
                kernel.host().home.as_deref(),
                kernel.host().platform,
                permissions,
                path,
            )
            .map_err(failed),
            // The spawner finds it on PATH.
            Location::System(name) => Ok(PathBuf::from(name)),
        }
    };
    let program = resolve(&spec.program)?;
    let cwd = match (&spec.cwd, &spec.program) {
        (Some(Location::System(_)), _) => {
            return Err(CoreError::Invalid(format!("{plugin}: a working folder can't be a system program")));
        }
        (Some(cwd), _) => resolve(cwd)?,
        (None, Location::System(name)) => {
            return Err(CoreError::Invalid(format!("{plugin}: {name} is a system program, so the launch needs a cwd")));
        }
        (None, _) => program.parent().map(ToOwned::to_owned).unwrap_or_else(|| storage.root().to_path_buf()),
    };
    let value = |arg: &LaunchArg| -> CoreResult<String> {
        match arg {
            LaunchArg::Text(text) => Ok(text.clone()),
            LaunchArg::Path { path: Location::System(_) } => {
                Err(CoreError::Invalid(format!("{plugin}: an argument path can't be a system program")))
            }
            LaunchArg::Path { path } => Ok(resolve(path)?.to_string_lossy().into_owned()),
        }
    };
    let args = spec.args.iter().map(value).collect::<CoreResult<Vec<_>>>()?;
    let env = spec.env.iter().map(|(k, v)| Ok((k.clone(), value(v)?))).collect::<CoreResult<_>>()?;
    Ok(SpawnCommand { program, args, env, cwd, log: None })
}

/// Where a game's output goes: `logs/<plugin>/<game>.log` in the data folder.
pub(crate) fn log_path(kernel: &Kernel, game: &GameKey) -> PathBuf {
    let name: String =
        game.local.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    kernel.host().data_dir.join("logs").join(game.plugin.as_str()).join(format!("{name}.log"))
}

/// Play time of a game, saved under `app.playtime.<game>`.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PlayStats {
    /// Seconds played, in all.
    pub secs: u64,
    pub count: u64,
    /// When it last closed, in seconds since 1970.
    pub last: u64,
}

/// A session shorter than this didn't really play.
const PLAYED_AT_LEAST: Duration = Duration::from_secs(3);
/// A game that closes with an error sooner than this closed "right after it started".
const EARLY_EXIT: Duration = Duration::from_secs(10);

pub(crate) fn play_stats(kernel: &Kernel, game: &GameKey) -> PlayStats {
    let value = kernel.config().get(&["app", "playtime", &game.to_string()]);
    serde_json::from_value(value).unwrap_or_default()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whole days, of 24 hours, from `secs` (since 1970) to now. The core knows no time zone,
/// so 0 means "in the last 24 hours", not "since midnight".
pub(crate) fn days_since(secs: u64) -> u64 {
    now_secs().saturating_sub(secs) / 86_400
}

fn add_play_time(core: &Core, game: &GameKey, played: Duration) {
    if played < PLAYED_AT_LEAST {
        return;
    }
    let mut stats = play_stats(core.kernel(), game);
    stats.secs += played.as_secs();
    stats.count += 1;
    stats.last = now_secs();
    let value = serde_json::to_value(stats).expect("play stats serialize");
    if let Err(e) = core.kernel().config().set(&["app", "playtime", &game.to_string()], value) {
        tracing::warn!(error = %e, %game, "could not save the play time");
    }
}

/// A launch in progress. While it lives, no other game starts; it lets go when dropped.
struct Launching(Core);

impl Launching {
    /// One game at a time: the gamepad and the screen belong to it. The check and the claim
    /// happen under one lock, so two presses of Play can't both start a game.
    fn claim(core: &Core) -> CoreResult<Self> {
        let mut state = core.state();
        if let Some(running) = state.sessions.values().next() {
            return Err(CoreError::Invalid(format!("{} is running: stop it first", running.title)));
        }
        if state.launching {
            return Err(CoreError::Invalid("a game is starting: wait for it".into()));
        }
        state.launching = true;
        Ok(Self(core.clone()))
    }
}

impl Drop for Launching {
    fn drop(&mut self) {
        self.0.state().launching = false;
    }
}

/// Keeps the log of the run before as `<game>.previous.log`, so a crash's log survives the
/// next start.
fn keep_previous_log(log: &std::path::Path) {
    if log.exists() {
        let previous = log.with_extension("previous.log");
        if let Err(e) = std::fs::rename(log, &previous) {
            tracing::warn!(error = %e, log = %log.display(), "could not keep the last log");
        }
    }
}

pub(crate) async fn launch(core: &Core, game: GameKey) -> CoreResult<()> {
    let kernel = core.kernel();
    let plugin = game.plugin.clone();
    let _launching = Launching::claim(core)?;
    // An install or update moves the emulator's files and the games' saves: no game starts
    // until it is done.
    if core.state().emulators.get(&plugin).is_some_and(|e| e.busy) {
        let name = kernel.manifest(&plugin).map(|m| m.name.clone()).unwrap_or_else(|_| plugin.to_string());
        return Err(CoreError::Invalid(format!("{name} is updating: play when it is done")));
    }
    let provided = kernel.provided(&plugin).await?;
    let launcher = require(&plugin, Capability::Launcher, provided.launcher.clone())?;
    // A first play: the player agrees to the emulator's download before anything happens.
    if core.ask_to_install(&game, &provided).await? {
        return Ok(());
    }
    let config = merged_config(core, &game).await?;
    let spec = kernel.guard(&plugin, "launcher.prepare", launcher.prepare(&game.local, &config)).await?;

    let chain = middleware_chain(core).await;
    let spec = Next::new(&chain, &Done)
        .run(LaunchRequest { game: game.clone(), spec })
        .await
        .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })?;

    let manifest = kernel.manifest(&plugin)?;
    if !manifest.permissions.allows_program(&spec.program) {
        return Err(CoreError::PermissionDenied { plugin, action: format!("run {}", spec.program) });
    }
    let mut command = spawn_command(kernel, &plugin, &manifest.permissions, &spec)?;
    let log = log_path(kernel, &game);
    keep_previous_log(&log);
    command.log = Some(log.clone());
    tracing::info!(plugin_id = %plugin, %game, log = %log.display(), "running: {}", command.shell_line());
    let mut process =
        kernel.spawner().spawn(command).await.map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error })?;

    let session = core.next_session();
    let title = core
        .state()
        .library
        .iter()
        .find(|e| e.key == game)
        .map(|e| e.summary.title.clone())
        .unwrap_or_else(|| game.local.clone());
    let pid = process.id();
    let started = Instant::now();
    let build = core.build_of(&plugin);
    core.state().sessions.insert(
        session,
        Session { game: game.clone(), title, stopping: false, shown: false, pid, started, log, build },
    );
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    core.inner.stops.lock().unwrap_or_else(|p| p.into_inner()).insert(session, stop_tx);
    tracing::info!(plugin_id = %plugin, %session, %game, ?pid, "game started");
    kernel.events().publish(Event::ProcessStarted { session, game: game.clone() });
    core.push(Route::Running(session));
    tokio::spawn(watch(core.clone(), session));
    // Once a run, at the first game: what the desktop needs to do its part.
    if !core.inner.desktop_checked.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let core = core.clone();
        tokio::spawn(async move {
            if let Some(problem) = on_desktop(&core, |d| d.problem()).await {
                core.notice(problem);
            }
        });
    }

    let core = core.clone();
    tokio::spawn(async move {
        let exit = tokio::select! {
            // A launcher that starts the game in the background ends first: the session lasts
            // until the game ends too.
            exit = process.wait_for_all() => exit.unwrap_or(ProcessExit { code: None, success: false }),
            _ = stop_rx => close(&core, process.as_mut()).await,
        };
        // What the game started and left running goes with it.
        if let Err(e) = process.kill().await {
            tracing::warn!(error = %e, "could not stop what the game left running");
        }
        let exit = ExitInfo {
            code: exit.code,
            success: exit.success,
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        };
        finish(&core, session, &game, exit, &launcher).await;
    });
    Ok(())
}

/// Asks the game to close, so the emulator saves its caches, and kills it if it is still
/// there after the host's `stop_grace`.
async fn close(core: &Core, process: &mut dyn crate::ProcessHandle) -> ProcessExit {
    let killed = ProcessExit { code: None, success: false };
    if let Err(e) = process.terminate().await {
        tracing::warn!(error = %e, "could not ask the game to close");
    }
    if let Ok(Ok(exit)) = tokio::time::timeout(core.kernel().host().stop_grace, process.wait_for_all()).await {
        return exit;
    }
    tracing::info!("the game didn't close: killing it");
    if let Err(e) = process.kill().await {
        tracing::warn!(error = %e, "could not stop the game");
    }
    // Only the program's own exit: what it left running is gone with the kill.
    process.wait().await.unwrap_or(killed)
}

/// Runs a desktop call, which may run a helper program, off the async threads.
async fn on_desktop<T: Send + 'static>(core: &Core, call: impl FnOnce(&dyn Desktop) -> T + Send + 'static) -> T {
    let desktop = core.kernel().desktop().clone();
    tokio::task::spawn_blocking(move || call(desktop.as_ref())).await.expect("desktop calls don't panic")
}

/// How often the launcher looks for a started game's window.
const WATCH_EVERY: Duration = Duration::from_millis(250);
/// How often, after that, it asks which window is in front. Each ask runs a helper program,
/// so not too often.
const FRONT_EVERY: Duration = Duration::from_secs(1);
/// A game whose window was gone this many asks in a row was closed by the player: it is
/// stopped. More than one, so a window that is remade (full screen on or off) doesn't count.
const WINDOW_GONE_FOR: u32 = 3;

/// Brings process `pid`'s window to the front; `false` without a process or a window.
async fn show(core: &Core, pid: Option<u32>) -> bool {
    match pid {
        Some(pid) => on_desktop(core, move |d| d.show_game(pid)).await,
        None => false,
    }
}

/// Brings the game's window to the front as soon as it has one (the Running screen says
/// "starting" until then), then keeps `game_in_front` up to date while the game runs.
async fn watch(core: Core, session: SessionId) {
    let deadline = Instant::now() + core.kernel().host().show_timeout;
    let alive = |core: &Core| core.state().sessions.get(&session).filter(|s| !s.stopping).map(|s| s.pid);
    // A window the desktop found: one it brought to the front, or one it saw later.
    let mut had_window;
    loop {
        let Some(pid) = alive(&core) else { return };
        let shown = show(&core, pid).await;
        had_window = shown;
        if shown || Instant::now() >= deadline {
            // A desktop that can't say (Windows, or macOS without the Accessibility
            // permission) can't show the game either; the game's own window took the screen
            // as it opened, so the pad is the game's until the PS button.
            let unknown = !shown && on_desktop(&core, |d| d.front()).await == Front::Unknown;
            let mut state = core.state();
            if let Some(s) = state.sessions.get_mut(&session) {
                s.shown = true;
                state.game_in_front |= shown || unknown;
            }
            drop(state);
            core.changed();
            break;
        }
        tokio::time::sleep(WATCH_EVERY).await;
    }
    // Only a game whose window was seen can lose it: a desktop that can't see the game's
    // window (a Wayland window, which xdotool can't find) must not take it for closed.
    let mut gone = 0;
    while let Some(pid) = alive(&core) {
        let (in_front, window) = on_desktop(&core, move |d| {
            let in_front = match d.front() {
                Front::Launcher => Some(false),
                Front::Other(front) => Some(pid.is_some_and(|pid| d.is_game(pid, front))),
                // The last window the launcher showed counts.
                Front::Unknown => None,
            };
            (in_front, pid.and_then(|pid| d.has_window(pid)))
        })
        .await;
        had_window |= window == Some(true);
        gone = if had_window && window == Some(false) { gone + 1 } else { 0 };
        if gone >= WINDOW_GONE_FOR {
            tracing::info!(%session, "the game's window closed: stopping it");
            if let Err(e) = stop(&core, session) {
                tracing::warn!(error = %e, "could not stop the game");
            }
            return;
        }
        if let Some(in_front) = in_front {
            let mut state = core.state();
            let changed = state.game_in_front != in_front;
            state.game_in_front = in_front;
            drop(state);
            if changed {
                core.changed();
            }
        }
        tokio::time::sleep(FRONT_EVERY).await;
    }
}

async fn finish(
    core: &Core,
    session: SessionId,
    game: &GameKey,
    exit: ExitInfo,
    launcher: &Arc<dyn nexus_api::Launcher>,
) {
    tracing::info!(plugin_id = %game.plugin, %session, ?exit, "game exited");
    core.inner.stops.lock().unwrap_or_else(|p| p.into_inner()).remove(&session);
    let ended = {
        let mut state = core.state();
        state.nav.retain(|r| r != &Route::Running(session));
        state.game_in_front = false;
        state.sessions.remove(&session)
    };
    let played = Duration::from_millis(exit.duration_ms);
    add_play_time(core, game, played);
    if let Some(ended) = &ended
        && !exit.success
        && !ended.stopping
    {
        core.game_problem(game, crash_notice(&ended.title, &exit, played, &ended.log));
    }
    // The player says how far it got, once for each build, unless they stopped it at once.
    if let Some(ended) = ended
        && (!ended.stopping || played >= PLAYED_AT_LEAST)
        && !core.result_asked(game, ended.build.as_deref())
    {
        core.state().result_prompt = Some((game.clone(), ended.build));
    }
    if core.return_on_exit_on() {
        on_desktop(core, |d| d.show_launcher()).await;
    }
    if let Err(e) = core.kernel().guard(&game.plugin, "launcher.on_exit", launcher.on_exit(&game.local, &exit)).await {
        tracing::warn!(error = %e, "on_exit failed");
    }
    core.kernel().events().publish(Event::ProcessExited { session, game: game.clone(), exit });
    core.kernel().events().publish(Event::StateChanged);
    core.game_closed(&game.plugin);
}

/// "Can't start Astro Bot: firmware missing". A plugin's own words, without the error's kind.
pub(crate) fn cant_start(core: &Core, game: &GameKey, error: &CoreError) -> String {
    let title = core
        .state()
        .library
        .iter()
        .find(|e| &e.key == game)
        .map(|e| e.summary.title.clone())
        .unwrap_or_else(|| game.local.clone());
    let reason = match error {
        CoreError::Plugin { error: PluginError::Precondition(why) | PluginError::NotFound(why), .. } => why.clone(),
        other => other.to_string(),
    };
    format!("Can't start {title}: {reason}")
}

/// "Astro Bot closed right after it started (code 1). Its log: …".
fn crash_notice(title: &str, exit: &ExitInfo, played: Duration, log: &std::path::Path) -> String {
    let how = match exit.code {
        Some(code) => format!("code {code}"),
        None => "stopped by the system".into(),
    };
    let when = if played < EARLY_EXIT { "right after it started" } else { "with an error" };
    format!("{title} closed {when} ({how}). Its log: {}", log.display())
}

pub(crate) fn stop(core: &Core, session: SessionId) -> CoreResult<()> {
    let stop = core.inner.stops.lock().unwrap_or_else(|p| p.into_inner()).remove(&session);
    let stop = stop.ok_or(CoreError::UnknownSession(session))?;
    if let Some(s) = core.state().sessions.get_mut(&session) {
        s.stopping = true;
    }
    let _ = stop.send(());
    core.kernel().events().publish(Event::StateChanged);
    Ok(())
}

pub(crate) async fn show_game(core: &Core, session: SessionId) -> CoreResult<()> {
    let pid = core.state().sessions.get(&session).ok_or(CoreError::UnknownSession(session))?.pid;
    if !show(core, pid).await {
        return Err(CoreError::Invalid("the game has no window to show yet".into()));
    }
    core.state().game_in_front = true;
    core.changed();
    Ok(())
}

/// The PS button: the launcher when the game is in front, else the game.
pub(crate) async fn toggle_focus(core: &Core) {
    let Some(session) = core.state().sessions.keys().next().copied() else { return };
    if game_in_front(core) {
        on_desktop(core, |d| d.show_launcher()).await;
        core.state().game_in_front = false;
        core.changed();
    } else if let Err(e) = show_game(core, session).await {
        tracing::info!(error = %e, "the game can't come to the front");
    }
}

/// Reads app state only: the watcher asks the desktop.
pub(crate) fn game_in_front(core: &Core) -> bool {
    let state = core.state();
    !state.sessions.is_empty() && state.game_in_front
}
