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

use crate::backup::{BrowsePlace, ItemKey, PlaceKind, RestorePlan, Source};
use crate::{Core, CoreError, CoreResult, DirStorage, Pref};

/// The backup folder when the player chose none, in the home folder.
pub const DEFAULT_FOLDER: &str = "Nexus Backups";
/// How many folders a restore came from the browser keeps as Recent places.
pub const RECENT: usize = 5;

/// Settings → Backup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupCmd {
    /// Reads the backup folder's list again.
    Refresh,
    /// Backs up now; `large` takes in the items a plugin marks large.
    BackUp {
        large: bool,
    },
    /// Puts the backup with this file name, in the backup folder, back.
    Restore(String),
    /// Puts the backup at this path back: one the browser found in another folder.
    RestoreFile(PathBuf),
    /// Deletes the backup with this file name from the backup folder.
    Delete(String),
    /// Opens the browser at this folder, or moves it there. With none, it opens at the last
    /// folder a restore came from, else at the backup folder.
    Browse(Option<PathBuf>),
    CloseBrowser,
    /// Lists the browser's hidden folders, or not.
    ShowHidden(bool),
    /// Shows only the browser's entries whose name holds this text, in any case; all for empty
    /// text.
    Filter(String),
}

pub(crate) async fn handle(core: &Core, command: BackupCmd) -> CoreResult<()> {
    match command {
        // What a backup holds here, and the list. With no folder (no home folder, and none
        // chosen), the list is empty.
        BackupCmd::Refresh => {
            let found = items(core, true).await;
            let titles = |large: bool| {
                let mut titles: Vec<String> = Vec::new();
                for item in found.iter().filter(|i| i.large == large) {
                    if !titles.contains(&item.source.title) {
                        titles.push(item.source.title.clone());
                    }
                }
                titles
            };
            let holds = [vec![crate::backup::SETTINGS.to_owned()], titles(false)].concat();
            core.backup().set_items(holds, titles(true));
            list(core).await;
            Ok(())
        }
        BackupCmd::BackUp { large } => {
            ready(core)?;
            let sources = items(core, large).await.into_iter().map(|i| i.source).collect();
            let computer = core.ports().hardware.computer_name();
            core.backup().back_up(&folder(core)?, crate::sessions::now_secs(), &computer, sources).await.map(|_| ())
        }
        BackupCmd::Restore(file) => {
            ready(core)?;
            restore(core, folder(core)?.join(in_folder(&file)?)).await
        }
        BackupCmd::RestoreFile(path) => {
            let named = path.file_name().and_then(|n| n.to_str()).and_then(crate::backup::archive::kind_of);
            if !path.is_absolute() || named.is_none() || !path.is_file() {
                return Err(CoreError::Invalid(format!("{} isn't a backup", path.display())));
            }
            // Refused before the job starts, and the browser stays open.
            let read = path.clone();
            tokio::task::spawn_blocking(move || crate::backup::archive::read_manifest(&read))
                .await
                .map_err(|e| CoreError::Invalid(e.to_string()))?
                .map_err(|e| CoreError::Invalid(format!("{}: {e}", path.display())))?;
            restore(core, path.clone()).await?;
            if let Some(from) = path.parent() {
                remember(core, from);
            }
            Ok(())
        }
        BackupCmd::Delete(file) => {
            let file = in_folder(&file)?;
            core.backup().delete(&folder(core)?, &file).await
        }
        BackupCmd::Browse(at) => {
            let places = places(core);
            let at = match at {
                Some(at) => at,
                None => recent(core)
                    .into_iter()
                    .next()
                    .or_else(|| folder(core).ok().filter(|f| f.is_dir()))
                    .or_else(|| places.first().map(|p| p.path.clone()))
                    .ok_or_else(|| CoreError::Invalid("no folder to start from".into()))?,
            };
            if !at.is_absolute() || !at.is_dir() {
                return Err(CoreError::Invalid(format!("{} isn't a folder", at.display())));
            }
            core.backup().browse(at, places).await
        }
        BackupCmd::CloseBrowser => {
            core.backup().close_browser();
            Ok(())
        }
        BackupCmd::ShowHidden(hidden) => core.backup().show_hidden(hidden).await,
        BackupCmd::Filter(text) => core.backup().filter(text),
    }
}

/// The folders restores came from that are still there, newest first.
fn recent(core: &Core) -> Vec<PathBuf> {
    let saved = core.preferences().get(Pref::BackupRecent);
    let paths = saved.as_array().into_iter().flatten().filter_map(serde_json::Value::as_str);
    paths.map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()).collect()
}

/// Keeps `from` first among the folders restores came from, at most [`RECENT`] of them.
fn remember(core: &Core, from: &std::path::Path) {
    let saved = core.preferences().get(Pref::BackupRecent);
    let from = from.display().to_string();
    let mut paths = vec![from.clone()];
    paths.extend(
        saved
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter(|p| *p != from)
            .map(str::to_owned),
    );
    paths.truncate(RECENT);
    if let Err(e) = core.preferences().save(Pref::BackupRecent, serde_json::json!(paths)) {
        tracing::warn!(error = %e, "the folder the restore came from isn't kept as a recent place");
    }
}

/// The backups in the backup folder, read again.
pub(crate) async fn list(core: &Core) {
    if let Ok(folder) = folder(core) {
        core.backup().list(folder).await;
    }
}

/// `file`, when it names a backup in the backup folder: no folder in it.
fn in_folder(file: &str) -> CoreResult<String> {
    if file.contains(['/', '\\']) || crate::backup::archive::kind_of(file).is_none() {
        return Err(CoreError::Invalid(format!("{file} isn't a backup in the backup folder")));
    }
    Ok(file.to_owned())
}

/// Puts the backup at `path` back into this computer's places.
async fn restore(core: &Core, path: PathBuf) -> CoreResult<()> {
    ready(core)?;
    let found = items(core, true).await;
    let targets = found.iter().map(|i| (i.source.key.clone(), i.source.path.clone())).collect();
    let hardware = core.ports().hardware.clone();
    let plan = RestorePlan {
        targets,
        current: found.into_iter().map(|i| i.source).collect(),
        now: crate::sessions::now_secs(),
        computer: hardware.computer_name(),
        free: std::sync::Arc::new(move |at: &std::path::Path| crate::release_downloads::free_at(hardware.as_ref(), at)),
    };
    core.backup().restore(&path, &folder(core)?, plan).await
}

/// Where the browser can go: Home, the Downloads, Desktop and Documents folders that are there,
/// the backup folder, each drive that is there, then the folders restores came from that no
/// other place is.
fn places(core: &Core) -> Vec<BrowsePlace> {
    let mut out = Vec::new();
    let mut add = |name: String, path: PathBuf, kind: PlaceKind, detail: String| {
        out.push(BrowsePlace { name, path, kind, detail });
    };
    let home = core.kernel().host().home.clone();
    if let Some(home) = &home {
        add("Home".into(), home.clone(), PlaceKind::Home, String::new());
        for (name, kind) in
            [("Downloads", PlaceKind::Downloads), ("Desktop", PlaceKind::Desktop), ("Documents", PlaceKind::Documents)]
        {
            let path = home.join(name);
            if path.is_dir() {
                add(name.into(), path, kind, String::new());
            }
        }
    }
    if let Ok(folder) = folder(core)
        && folder.is_dir()
    {
        add("Backup folder".into(), folder, PlaceKind::Backups, String::new());
    }
    let hardware = core.ports().hardware.clone();
    for drive in drives(home.as_deref()) {
        let name =
            drive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| drive.display().to_string());
        let size = hardware.disk_bytes(&drive).map(disk_text).unwrap_or_default();
        add(name, drive, PlaceKind::Drive, size);
    }
    for path in recent(core) {
        if out.iter().any(|p| p.path == path) {
            continue;
        }
        let name =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let shown = crate::display::short_home(&path.display().to_string(), home.as_deref().and_then(|h| h.to_str()));
        out.push(BrowsePlace { name, path, kind: PlaceKind::Recent, detail: shown });
    }
    out
}

/// A drive's size: `1.6 TB` from 1000 GB up, else as other sizes (`32.0 GB`).
fn disk_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 { format!("{:.1} TB", bytes as f64 / 1e12) } else { crate::display::size_text(bytes) }
}

/// The drives that are there: USB sticks and other disks the system mounted.
fn drives(home: Option<&std::path::Path>) -> Vec<PathBuf> {
    let children = |dir: PathBuf| -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut found: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        found.sort();
        found
    };
    if cfg!(target_os = "windows") {
        return ('C'..='Z').map(|d| PathBuf::from(format!("{d}:\\"))).filter(|p| p.is_dir()).collect();
    }
    if cfg!(target_os = "macos") {
        return children(PathBuf::from("/Volumes"));
    }
    let user = home.and_then(|h| h.file_name()).map(|n| n.to_owned());
    let mut out = Vec::new();
    if let Some(user) = user {
        out.extend(children(PathBuf::from("/run/media").join(&user)));
        out.extend(children(PathBuf::from("/media").join(&user)));
    }
    out.extend(children(PathBuf::from("/mnt")));
    out
}

/// Sets the backup folder, then reads the list there.
pub(crate) async fn set_folder(core: &Core, value: serde_json::Value) -> CoreResult<()> {
    core.preferences().set(Pref::BackupFolder, value).await?;
    list(core).await;
    Ok(())
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
    large: bool,
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
                Ok(path) => out.push(Found {
                    source: Source {
                        key: ItemKey { plugin: plugin.as_str().to_owned(), name: item.name },
                        title: item.title,
                        path,
                        skip: item.skip,
                    },
                    large: item.large,
                }),
                Err(e) => tracing::warn!(%plugin, item = item.name, error = %e, "the item stays out of the backup"),
            }
        }
    }
    out
}

/// Where `item` is on this computer. A restore writes there whether or not the plugin may: the
/// host does the writing, and the plugin named the place as its own data. So a plugin needs no
/// write access to a folder it only reads, such as one with keys and firmware.
fn place(core: &Core, plugin: &PluginId, item: &BackupItem) -> CoreResult<PathBuf> {
    if !BackupItem::valid_name(&item.name) {
        return Err(CoreError::Invalid(format!("{:?} can't be an item's name", item.name)));
    }
    let kernel = core.kernel();
    let refused = |e: nexus_api::PluginError| CoreError::Plugin { plugin: plugin.clone(), error: e };
    match &item.place {
        BackupPlace::Data(path) => {
            let root = kernel.host().plugin_dir(plugin);
            DirStorage::new(root).resolve(path).map_err(refused)
        }
        BackupPlace::Folder(path) => {
            let folders = kernel.host_folders(plugin).ok_or_else(|| CoreError::UnknownPlugin(plugin.clone()))?;
            folders.resolve(path).map_err(refused)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drive_of_a_thousand_gb_or_more_says_tb() {
        assert_eq!(super::disk_text(1_645_500_000_000), "1.6 TB");
        assert_eq!(super::disk_text(32_000_000_000), "32.0 GB");
    }
}
