//! The backup flow: Settings → Backup's commands. It asks each plugin with the backup capability
//! for its items, finds where each one is on this computer through the plugin's grants, and
//! hands the paths to the backup subsystem ([`crate::backup`]), which writes and reads the file.
//!
//! The backup folder is `app.backup_folder` ([`Pref::BackupFolder`]), or `~/Nexus Backups`.
//! Neither a backup nor a restore starts while a game runs or a game installs; a restore makes
//! a backup of what is there first.

use std::path::PathBuf;

use nexus_api::{BackupItem, BackupPlace, Capability, PluginId};
use serde::{Deserialize, Serialize};

use crate::backup::{ItemKey, RestorePlan, Source};
use crate::{Core, CoreError, CoreResult, DirStorage, Pref};

/// The backup folder when the player chose none, in the home folder.
pub const DEFAULT_FOLDER: &str = "Nexus Backups";

/// Settings → Backup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupCmd {
    /// Reads the backup folder's list again.
    Refresh,
    /// Backs up now; `large` takes in the items a plugin marks large.
    BackUp { large: bool },
    /// Puts the backup with this file name, in the backup folder, back.
    Restore(String),
}

pub(crate) async fn handle(core: &Core, command: BackupCmd) -> CoreResult<()> {
    match command {
        // With no folder (no home folder, and none chosen), the list is empty.
        BackupCmd::Refresh => {
            if let Ok(folder) = folder(core) {
                core.backup().list(folder).await;
            }
            Ok(())
        }
        BackupCmd::BackUp { large } => {
            ready(core)?;
            let sources = items(core, large).await.into_iter().map(|i| i.source).collect();
            core.backup().back_up(&folder(core)?, crate::sessions::now_secs(), sources).await.map(|_| ())
        }
        BackupCmd::Restore(file) => {
            ready(core)?;
            if file.contains(['/', '\\']) || crate::backup::archive::kind_of(&file).is_none() {
                return Err(CoreError::Invalid(format!("{file} isn't a backup in the backup folder")));
            }
            let found = items(core, true).await;
            let targets =
                found.iter().filter(|i| i.writable).map(|i| (i.source.key.clone(), i.source.path.clone())).collect();
            let hardware = core.ports().hardware.clone();
            let plan = RestorePlan {
                targets,
                current: found.into_iter().map(|i| i.source).collect(),
                now: crate::sessions::now_secs(),
                free: std::sync::Arc::new(move |path: &std::path::Path| {
                    crate::release_downloads::free_at(hardware.as_ref(), path)
                }),
            };
            core.backup().restore(&folder(core)?, &file, plan).await
        }
    }
}

/// Sets the backup folder, then reads the list there.
pub(crate) async fn set_folder(core: &Core, value: serde_json::Value) -> CoreResult<()> {
    core.preferences().set(Pref::BackupFolder, value).await?;
    handle(core, BackupCmd::Refresh).await
}

/// The backup folder: the player's (`~/` for the home folder), or `~/Nexus Backups`.
pub(crate) fn folder(core: &Core) -> CoreResult<PathBuf> {
    let home = core.kernel().host().home.clone();
    let chosen = Pref::BackupFolder.load(core.kernel().config());
    if let Some(path) = chosen.as_str().and_then(|p| crate::folders::expand(p.trim(), home.as_deref())) {
        return Ok(path);
    }
    let home = home.ok_or_else(|| CoreError::Invalid("no home folder for backups".into()))?;
    Ok(home.join(DEFAULT_FOLDER))
}

/// Why a backup, a restore, a game or a game install can't start now: one of them runs.
pub(crate) fn busy(core: &Core) -> Option<&'static str> {
    core.backup().busy().then_some("Wait for the backup or the restore to finish")
}

/// Refuses while a game runs or installs, or another backup job runs.
fn ready(core: &Core) -> CoreResult<()> {
    if core.sessions().running() {
        return Err(CoreError::Invalid("Close the game first: its files change while it runs".into()));
    }
    let installs = core.installs().snapshot();
    if installs.jobs.values().any(|job| job.phase == crate::game_installs::GamePhase::Installing) {
        return Err(CoreError::Invalid("Wait for the game install to finish".into()));
    }
    if core.backup().busy() {
        return Err(CoreError::Invalid(crate::backup::BUSY.into()));
    }
    Ok(())
}

/// An item where it is on this computer.
struct Found {
    source: Source,
    /// A restore may write there: the plugin's data folder, or a grant with `write = true`.
    writable: bool,
}

/// Every active plugin's items on this computer, the large ones only with `large`. An item the
/// grants refuse, or with a name that can't be a path, is left out with a line in the log.
async fn items(core: &Core, large: bool) -> Vec<Found> {
    let kernel = core.kernel();
    let mut out = Vec::new();
    for plugin in kernel.providers(Capability::Backup) {
        let provided = match kernel.provided(&plugin).await {
            Ok(provided) => provided,
            Err(e) => {
                tracing::warn!(%plugin, error = %e, "its items stay out of the backup");
                continue;
            }
        };
        let Some(backup) = provided.backup else { continue };
        let items = match kernel.guard(&plugin, "backup.items", backup.items()).await {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!(%plugin, error = %e, "its items stay out of the backup");
                continue;
            }
        };
        for item in items.into_iter().filter(|i| large || !i.large) {
            match place(core, &plugin, &item) {
                Ok((path, writable)) => out.push(Found {
                    source: Source {
                        key: ItemKey { plugin: plugin.as_str().to_owned(), name: item.name },
                        title: item.title,
                        path,
                        skip: item.skip,
                    },
                    writable,
                }),
                Err(e) => tracing::warn!(%plugin, item = item.name, error = %e, "the item stays out of the backup"),
            }
        }
    }
    out
}

/// Where `item` is on this computer, and whether a restore may write there.
fn place(core: &Core, plugin: &PluginId, item: &BackupItem) -> CoreResult<(PathBuf, bool)> {
    if !BackupItem::valid_name(&item.name) {
        return Err(CoreError::Invalid(format!("{:?} can't be an item's name", item.name)));
    }
    let kernel = core.kernel();
    let refused = |e: nexus_api::PluginError| CoreError::Plugin { plugin: plugin.clone(), error: e };
    match &item.place {
        BackupPlace::Data(path) => {
            let root = kernel.host().plugin_dir(plugin);
            Ok((DirStorage::new(root).resolve(path).map_err(refused)?, true))
        }
        BackupPlace::Folder(path) => {
            let folders = kernel.host_folders(plugin).ok_or_else(|| CoreError::UnknownPlugin(plugin.clone()))?;
            let real = folders.resolve(path).map_err(refused)?;
            let manifest = kernel.manifest(plugin)?;
            let writable = manifest.permissions.folder(&path.folder).is_some_and(|g| g.write);
            Ok((real, writable))
        }
    }
}
