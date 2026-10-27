//! Each emulator's builds: read what is installed and available, install, update and uninstall
//! a build, and look for updates every few hours. An install is saved under
//! `app.installs.<plugin>` while it runs, so a crash resumes it. An update for an emulator whose
//! game is running waits until the game closes. A first play of a game whose emulator isn't
//! installed asks the player first.

use std::sync::Arc;

use nexus_api::{
    Capability, DownloadRequest, Event, GameKey, InstallPhase, PluginId, PluginKind, Provided, SlotId, SlotRequest,
};
use nexus_ui_api::NoticeKind;

use crate::app::{Core, Route};
use crate::app_state::InstallPrompt;
use crate::changes::Area;
use crate::error::require;
use crate::{CoreError, CoreResult};

/// How often the core looks for emulator updates by itself, and how long after start it
/// looks first.
pub const UPDATE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);
const FIRST_UPDATE_CHECK: std::time::Duration = std::time::Duration::from_secs(60);

/// An install in progress, saved under `app.installs.<plugin>` so a crash resumes it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct InstallRecord {
    version: String,
    from: Option<String>,
    phase: InstallPhase,
}

/// Installed and available versions, and panels, for one emulator.
pub(crate) async fn refresh_emulator(core: &Core, plugin: &PluginId) {
    let kernel = core.kernel();
    let Ok(provided) = kernel.provided(plugin).await else { return };
    let mut emu = core.state().emulators.get(plugin).cloned().unwrap_or_default();
    if let Some(installer) = &provided.installer {
        match kernel.guard(plugin, "installer.installed", installer.installed()).await {
            Ok(list) => {
                emu.own = list.iter().any(|v| v.own);
                emu.installed = list.into_iter().map(|v| v.version).collect();
            }
            Err(e) => core.notices().problem(e.to_string()),
        }
        match kernel.guard(plugin, "installer.available", installer.available()).await {
            Ok(list) => emu.available = list.into_iter().map(|v| v.version).collect(),
            Err(e) => core.notices().problem(e.to_string()),
        }
    }
    if let Some(slots) = &provided.slots {
        let request = SlotRequest { slot: SlotId::EmulatorPanel, game: None };
        if let Ok(items) = kernel.guard(plugin, "slots.items", slots.items(&request)).await {
            emu.panels = items;
        }
    }
    core.state().emulators.insert(plugin.clone(), emu);
}

pub(crate) async fn refresh_emulators(core: &Core) {
    for plugin in core.kernel().of_kind(PluginKind::Emulator) {
        refresh_emulator(core, &plugin).await;
    }
}

fn set_install(core: &Core, plugin: &PluginId, record: Option<InstallRecord>, phase: InstallPhase, version: &str) {
    let key = ["app", "installs", plugin.as_str()];
    let saved = match record {
        Some(record) => core.kernel().config().set(&key, serde_json::to_value(record).expect("records serialize")),
        None => core.kernel().config().remove(&key),
    };
    if let Err(e) = saved {
        tracing::warn!(error = %e, "could not save install state");
    }
    if let Some(emu) = core.state().emulators.get_mut(plugin) {
        emu.busy = matches!(phase, InstallPhase::Downloading | InstallPhase::Installing);
    }
    core.kernel().events().publish(Event::InstallChanged {
        plugin: plugin.clone(),
        version: version.into(),
        state: phase,
    });
    core.changed(Area::Downloads);
}

/// The install workflow: plan, download, check, install. Each phase is saved, so a crash
/// resumes it at the next start.
pub(crate) async fn install(
    core: &Core,
    plugin: &PluginId,
    version: Option<String>,
    from: Option<String>,
) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = require(plugin, Capability::Installer, provided.installer.clone())?;
    let downloader = require(plugin, Capability::Downloader, provided.downloader.clone())?;
    let version = match version {
        Some(v) => v,
        None => kernel
            .guard(plugin, "installer.available", installer.available())
            .await?
            .into_iter()
            .next()
            .map(|v| v.version)
            .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?,
    };
    core.state().emulators.entry(plugin.clone()).or_default();
    let record = |phase| InstallRecord { version: version.clone(), from: from.clone(), phase };
    set_install(core, plugin, Some(record(InstallPhase::Downloading)), InstallPhase::Downloading, &version);

    let result: CoreResult<()> = async {
        let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
        let plan = kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await?;
        let files = core.inner.downloads.run(plugin, plan).await?;
        set_install(core, plugin, Some(record(InstallPhase::Installing)), InstallPhase::Installing, &version);
        match &from {
            Some(from) => kernel.guard(plugin, "installer.update", installer.update(from, &version, &files)).await?,
            None => kernel.guard(plugin, "installer.install", installer.install(&version, &files)).await?,
        };
        Ok(())
    }
    .await;

    match &result {
        Ok(()) => set_install(core, plugin, None, InstallPhase::Installed, &version),
        Err(e) => {
            set_install(core, plugin, None, InstallPhase::Failed, &version);
            core.notices().opening(
                NoticeKind::Error,
                Route::Plugin(plugin.clone()),
                format!("Could not install {plugin} {version}: {e}"),
            );
        }
    }
    refresh_emulator(core, plugin).await;
    core.changed(Area::Downloads);
    result
}

pub(crate) async fn update(core: &Core, plugin: &PluginId) -> CoreResult<()> {
    refresh_emulator(core, plugin).await;
    let emu = core.state().emulators.get(plugin).cloned().unwrap_or_default();
    let from = emu.installed.last().cloned().ok_or_else(|| CoreError::Invalid(format!("{plugin} is not installed")))?;
    let to = emu.update().ok_or_else(|| CoreError::Invalid(format!("{plugin} is up to date")))?;
    install(core, plugin, Some(to), Some(from)).await
}

pub(crate) async fn uninstall(core: &Core, plugin: &PluginId, version: &str) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = require(plugin, Capability::Installer, provided.installer)?;
    kernel.guard(plugin, "installer.uninstall", installer.uninstall(version)).await?;
    set_install(core, plugin, None, InstallPhase::Uninstalled, version);
    refresh_emulator(core, plugin).await;
    Ok(())
}

/// Re-runs installs that were in progress when the app last stopped.
pub(crate) async fn resume_installs(core: &Core) {
    let pending = core.kernel().config().get(&["app", "installs"]);
    let Some(pending) = pending.as_object() else { return };
    for (plugin, saved) in pending.clone() {
        let (Ok(plugin), Ok(record)) = (PluginId::new(plugin), serde_json::from_value::<InstallRecord>(saved)) else {
            continue;
        };
        tracing::info!(plugin_id = %plugin, version = %record.version, "resuming an interrupted install");
        let core = core.clone();
        tokio::spawn(async move {
            let _ = install(&core, &plugin, Some(record.version), record.from).await;
        });
    }
}

/// Runs [`Command::CheckUpdates`] every few hours while the app runs.
pub(crate) fn schedule_update_checks(core: &Core) {
    let weak = Arc::downgrade(&core.inner);
    let clock = core.ports().clock.clone();
    tokio::spawn(async move {
        clock.sleep(FIRST_UPDATE_CHECK).await;
        loop {
            let Some(inner) = weak.upgrade() else { return };
            let core = Core { inner };
            if core.prefs().auto_update {
                check_updates(&core).await;
            }
            drop(core);
            clock.sleep(UPDATE_CHECK_EVERY).await;
        }
    });
}

/// Looks for updates of every installed emulator, and installs them when automatic
/// updates are on. One whose game is running waits for it to close.
pub(crate) async fn check_updates(core: &Core) {
    for plugin in core.kernel().of_kind(PluginKind::Emulator) {
        let installs = core.kernel().manifest(&plugin).is_ok_and(|m| m.capabilities.contains(&Capability::Installer));
        if !installs {
            continue;
        }
        refresh_emulator(core, &plugin).await;
        let emu = core.state().emulators.get(&plugin).cloned().unwrap_or_default();
        if emu.installed.is_empty() || emu.own || emu.update().is_none() || !core.prefs().auto_update {
            continue;
        }
        if plays(core, &plugin) {
            tracing::info!(plugin_id = %plugin, "an update waits for the game to close");
            core.state().pending_updates.insert(plugin);
            continue;
        }
        if let Err(e) = update(core, &plugin).await {
            core.notices().problem(e.to_string());
        }
    }
    core.changed(Area::Downloads);
}

/// One of the emulator's games is running.
fn plays(core: &Core, plugin: &PluginId) -> bool {
    core.sessions().plays(plugin)
}

/// A game closed: an update that waited for it installs now.
pub(crate) fn game_closed(core: &Core, plugin: &PluginId) {
    if plays(core, plugin) || !core.state().pending_updates.remove(plugin) {
        return;
    }
    let (core, plugin) = (core.clone(), plugin.clone());
    tokio::spawn(async move {
        if let Err(e) = update(&core, &plugin).await {
            core.notices().problem(e.to_string());
        }
        core.changed(Area::Downloads);
    });
}

/// The game's emulator isn't installed: asks the player first. Returns whether it asked.
pub(crate) async fn ask_to_install(core: &Core, game: &GameKey, provided: &Provided) -> CoreResult<bool> {
    let kernel = core.kernel();
    let (Some(installer), Some(downloader)) = (&provided.installer, &provided.downloader) else { return Ok(false) };
    let plugin = &game.plugin;
    if !kernel.guard(plugin, "installer.installed", installer.installed()).await?.is_empty() {
        return Ok(false);
    }
    let version = kernel
        .guard(plugin, "installer.available", installer.available())
        .await?
        .into_iter()
        .next()
        .map(|v| v.version)
        .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no version to install")))?;
    let request = DownloadRequest::EmulatorBuild { version: Some(version.clone()) };
    let size_bytes = match kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await {
        Ok(plan) => plan.artifacts.iter().map(|a| a.size).sum::<Option<u64>>(),
        Err(_) => None,
    };
    let emulator = kernel.manifest(plugin).map(|m| m.emulator_name().to_owned()).unwrap_or_else(|_| plugin.to_string());
    let title = core.library().snapshot().title(game);
    core.state().install_prompt = Some(InstallPrompt { game: game.clone(), emulator, version, title, size_bytes });
    core.changed(Area::Sessions);
    Ok(true)
}

/// The emulator build `plugin` runs games on: its newest installed one.
pub(crate) fn build_of(core: &Core, plugin: &PluginId) -> Option<String> {
    core.state().emulators.get(plugin).and_then(|e| e.installed.last().cloned())
}

/// The player agreed to install the emulator a game needs: install it, then play.
pub(crate) async fn confirm_install(core: &Core) -> CoreResult<()> {
    let Some(prompt) = core.state().install_prompt.take() else { return Ok(()) };
    core.changed(Area::Sessions);
    install(core, &prompt.game.plugin, Some(prompt.version), None).await?;
    crate::launch::launch(core, prompt.game).await
}

/// The player said "not now" to installing the emulator.
pub(crate) fn cancel_install(core: &Core) -> CoreResult<()> {
    core.state().install_prompt = None;
    core.changed(Area::Sessions);
    Ok(())
}
