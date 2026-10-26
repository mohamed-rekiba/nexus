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
use crate::{Core, CoreError, CoreResult, DirStorage, Kernel, SpawnCommand, folders, running};

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
    let process =
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
    // Once a run, at the first game: what the desktop needs to do its part.
    if !core.inner.desktop_checked.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let core = core.clone();
        tokio::spawn(async move {
            if let Some(problem) = running::desktop_problem(&core).await {
                core.notice(problem);
            }
        });
    }

    let core = core.clone();
    tokio::spawn(async move {
        let exit = running::run(core.clone(), session, process, stop_rx).await;
        let exit = ExitInfo {
            code: exit.code,
            success: exit.success,
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        };
        finish(&core, session, &game, exit, &launcher).await;
    });
    Ok(())
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
        running::show_launcher(core).await;
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
    if !running::show(core, pid).await {
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
        running::show_launcher(core).await;
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
