//! The core's side of launching games: what the sessions subsystem can't do alone.
//!
//! [`Command::Session`](crate::Command::Session) comes here first. A launch needs the plugin:
//! it prepares a spec, launch middleware may change or stop it, and the core checks the
//! plugin's permissions and resolves the spec's paths. The subsystem then spawns the game and
//! watches it ([`crate::sessions`]). When the game ends the core does what the other areas
//! need: notices, the "How far did it get?" question, the plugin's `on_exit`, and an update
//! that waited for the game. The other session commands go to the subsystem as they are.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nexus_api::{
    Capability, ConfigScope, Endpoint, Event, ExitInfo, GameKey, LaunchArg, LaunchMiddleware, LaunchRequest,
    LaunchSpec, Location, Middleware, Next, Order, Permissions, PluginError, PluginId, PluginResult, SessionId,
};
use nexus_ui_api::NoticeKind;
use serde_json::Value;

use crate::app::Route;
use crate::error::require;
use crate::order::sort_by_order;
use crate::sessions::{NewSession, PLAYED_AT_LEAST, SessionCmd, log_path};
use crate::{Core, CoreError, CoreResult, DirStorage, Kernel, ProcessExit, SpawnCommand, folders};

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
    Ok(SpawnCommand { program, args, env, cwd, log: None, keep_running: false })
}

/// A game that closes with an error sooner than this closed "right after it started".
const EARLY_EXIT: Duration = Duration::from_secs(10);

/// Runs a session command: a launch is resolved here, the rest goes to the subsystem.
pub(crate) async fn session(core: &Core, command: SessionCmd) -> CoreResult<()> {
    let SessionCmd::Launch(game) = command else { return core.sessions().handle(command).await };
    // A Catalog offer's Play starts the installed game it is.
    let torrents = core.torrents().snapshot();
    let game = crate::view::played_game(&core.state(), &torrents, &game);
    let result = launch(core, game.clone()).await;
    // The player pressed Play: say why nothing started.
    if let Err(error) = &result {
        let why = cant_start(core, &game, error);
        core.notice_opening(NoticeKind::Error, Route::GameDetail(game.clone()), why);
    }
    result
}

pub(crate) async fn launch(core: &Core, game: GameKey) -> CoreResult<()> {
    let kernel = core.kernel();
    let plugin = game.plugin.clone();
    let claim = core.sessions().claim()?;
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
    let command = spawn_command(kernel, &plugin, &manifest.permissions, &spec)?;
    let title = core
        .state()
        .library
        .iter()
        .find(|e| e.key == game)
        .map(|e| e.summary.title.clone())
        .unwrap_or_else(|| game.local.clone());
    let new = NewSession {
        game: game.clone(),
        title,
        build: core.build_of(&plugin),
        command,
        log: log_path(&kernel.host().data_dir, &game),
        game_process: spec.game_process,
    };
    let started = core.sessions().start(claim, new).await?;
    let session = started.session;
    core.push(Route::Running(session));

    let core = core.clone();
    tokio::spawn(async move {
        let ended = started.ended.await;
        let played = core.kernel().clock().now().saturating_duration_since(started.at);
        finish(&core, session, &game, ended, played, &launcher).await;
    });
    Ok(())
}

async fn finish(
    core: &Core,
    session: SessionId,
    game: &GameKey,
    // `None`: nothing was played, as the game another program starts never showed.
    ended: Option<ProcessExit>,
    played: Duration,
    launcher: &Arc<dyn nexus_api::Launcher>,
) {
    let played_any = ended.is_some();
    let ended = ended.unwrap_or(ProcessExit { code: None, success: false });
    let duration_ms = played.as_millis().try_into().unwrap_or(u64::MAX);
    let exit = ExitInfo { code: ended.code, success: ended.success, duration_ms };
    tracing::info!(plugin_id = %game.plugin, %session, ?exit, played_any, "game exited");
    core.state().nav.retain(|r| r != &Route::Running(session));
    let ended = core.sessions().end(session, played_any.then_some(played));
    if let Some(ended) = &ended
        && !played_any
        && !ended.stopping
    {
        let waited = core.kernel().host().hand_over_timeout.as_secs();
        core.game_problem(
            game,
            format!("{} didn't start: its game didn't show up within {waited} seconds", ended.title),
        );
    }
    let ended = ended.filter(|_| played_any);
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
        core.sessions().show_launcher().await;
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
