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

use nexus_api::{
    Capability, CompatLevel, ConfigScope, Event, ExitInfo, GameKey, LaunchArg, LaunchSpec, Location, Permissions,
    PluginError, PluginId, SessionId,
};
use nexus_ui_api::NoticeKind;
use serde_json::Value;

use crate::app::Route;
use crate::changes::Area;
use crate::error::require;
use crate::sessions::{NewSession, PLAYED_AT_LEAST, SessionCmd, log_path};
use crate::{Core, CoreError, CoreResult, DirStorage, Kernel, ProcessExit, SpawnCommand, folders};

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
    let (library, torrents, installs) =
        (core.library().snapshot(), core.torrents().snapshot(), core.installs().snapshot());
    let game = crate::ownership::Ownership::new(&library, &torrents.jobs, &installs).played_game(&game);
    let result = launch(core, game.clone()).await;
    // The player pressed Play: say why nothing started.
    if let Err(error) = &result {
        let why = cant_start(core, &game, error);
        core.notices().opening(NoticeKind::Error, Route::GameDetail(game.clone()), why);
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
    if crate::emulator_builds::ask_to_install(core, &game, &provided).await? {
        return Ok(());
    }
    let config = merged_config(core, &game).await?;
    let spec = kernel.guard(&plugin, "launcher.prepare", launcher.prepare(&game.local, &config)).await?;

    let manifest = kernel.manifest(&plugin)?;
    if !manifest.permissions.allows_program(&spec.program) {
        return Err(CoreError::PermissionDenied { plugin, action: format!("run {}", spec.program) });
    }
    let command = spawn_command(kernel, &plugin, &manifest.permissions, &spec)?;
    let title = core.library().snapshot().title(&game);
    let new = NewSession {
        game: game.clone(),
        title,
        build: crate::emulator_builds::build_of(core, &plugin),
        command,
        log: log_path(&kernel.host().data_dir, &game),
        game_process: spec.game_process,
    };
    let started = core.sessions().start(claim, new).await?;
    let session = started.session;
    crate::navigation::push(core, Route::Running(session));

    let core = core.clone();
    tokio::spawn(async move {
        let ended = started.ended.await;
        let played = core.ports().clock.now().saturating_duration_since(started.at);
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
        core.notices().game_problem(
            game,
            format!("{} didn't start: its game didn't show up within {waited} seconds", ended.title),
        );
    }
    let ended = ended.filter(|_| played_any);
    if let Some(ended) = &ended
        && !exit.success
        && !ended.stopping
    {
        core.notices().game_problem(game, crash_notice(&ended.title, &exit, played, &ended.log));
    }
    // The player says how far it got, once for each build, unless they stopped it at once.
    if let Some(ended) = ended
        && (!ended.stopping || played >= PLAYED_AT_LEAST)
        && !core.library().answered(game, ended.build.as_deref())
    {
        core.state().result_prompt = Some((game.clone(), ended.build));
    }
    if core.prefs().return_on_exit {
        core.sessions().show_launcher().await;
    }
    if let Err(e) = core.kernel().guard(&game.plugin, "launcher.on_exit", launcher.on_exit(&game.local, &exit)).await {
        tracing::warn!(error = %e, "on_exit failed");
    }
    core.kernel().events().publish(Event::ProcessExited { session, game: game.clone(), exit });
    core.kernel().events().publish(Event::StateChanged);
    crate::emulator_builds::game_closed(core, &game.plugin);
}

/// "Can't start Astro Bot: firmware missing". A plugin's own words, without the error's kind.
pub(crate) fn cant_start(core: &Core, game: &GameKey, error: &CoreError) -> String {
    let title = core.library().snapshot().title(game);
    let reason = match error {
        CoreError::Plugin { error: PluginError::Precondition(why) | PluginError::NotFound(why), .. } => why.clone(),
        CoreError::Plugin { plugin, error: PluginError::OwnBuildMissing { has_downloaded, folder, program } } => {
            let page = core.kernel().manifest(plugin).map(|m| m.name.clone()).unwrap_or_else(|_| plugin.to_string());
            own_build_missing(&page, folder, program, *has_downloaded)
        }
        other => other.to_string(),
    };
    format!("Can't start {title}: {reason}")
}

/// Why a game with own build on doesn't start: `page` is the emulator's page, `folder` the
/// host folder for the build, `program` its name on the PATH. A downloaded build is the
/// likely way out, so with one the words say so first.
fn own_build_missing(page: &str, folder: &str, program: &str, has_downloaded: bool) -> String {
    let fix = if has_downloaded {
        format!(
            "turn off Own build on the {page} page to play with the downloaded build, or map the {folder} folder, or put {program} on your PATH"
        )
    } else {
        format!("map the {folder} folder on the {page} page, or put {program} on your PATH")
    };
    format!("Own build is on, but your build isn't there: {fix}")
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

/// "Rate how it runs": asks "How far did it get?" for `game`, on the build it runs on.
pub(crate) fn ask_result(core: &Core, game: GameKey) -> CoreResult<()> {
    let build = crate::emulator_builds::build_of(core, &game.plugin);
    core.state().result_prompt = Some((game, build));
    core.changed(Area::Sessions);
    Ok(())
}

/// The player's answer to "How far did it get?", or `None` for "Not now".
pub(crate) fn report_result(core: &Core, level: Option<CompatLevel>) -> CoreResult<()> {
    let Some((game, build)) = core.state().result_prompt.take() else { return Ok(()) };
    core.library().answer(&game, level, build.as_deref())?;
    core.changed(Area::Sessions);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_build_missing_says_how_out() {
        assert_eq!(
            super::own_build_missing("PS9", "emu-build", "emu", false),
            "Own build is on, but your build isn't there: map the emu-build folder on the PS9 page, or put emu on your PATH"
        );
        assert_eq!(
            super::own_build_missing("PS9", "emu-build", "emu", true),
            "Own build is on, but your build isn't there: turn off Own build on the PS9 page to play with the downloaded build, or map the emu-build folder, or put emu on your PATH",
            "with a downloaded build, turning own build off comes first"
        );
    }
}
