//! The core's side of the catalogs' downloads and of game installs: what the torrents and the
//! game installs subsystems can't do alone.
//!
//! [`Command::Torrent`](crate::Command::Torrent) comes here first. A download needs the
//! catalog's release and a folder inside one of the emulator's packages folders: the core
//! resolves them and adds the download to the subsystem. A release its store installs never
//! reaches the subsystem. The rest goes to the subsystem as it is. The subsystem's events come
//! back here: a finished download unpacks and says so.
//!
//! [`Command::GameInstall`](crate::Command::GameInstall) comes here too. An install needs the
//! plugin's [`nexus_api::GameInstaller`], its folders and a game folder: the core resolves them
//! and hands them to the game installs subsystem, which takes the files from the download's
//! folder. Install checks the free space first. The subsystem's events come back here: a new
//! game refreshes the library, and a failure is a notice.

use std::path::Path;

use nexus_api::{GameKey, HostPath, PluginId};

use crate::app::Core;
use crate::changes::Area;
use crate::folders::{HostFolders, Purpose};
use crate::game_installs::{GameInstallCmd, InstallEvent, NewInstall};
use crate::torrents::{NewDownload, TorrentAction, TorrentCmd, TorrentEvent, TorrentState, no_room};
use crate::{CoreError, CoreResult, Kernel};

/// The free bytes on the disk that holds `folder`, which may not be there yet: the disk of the
/// nearest folder above it.
pub(crate) fn free_at(hardware: &dyn crate::Hardware, folder: &Path) -> Option<u64> {
    let there = folder.ancestors().find(|p| p.is_dir())?;
    hardware.free_bytes(there)
}

/// The console an emulator plays, which names its folders: the plugin's id when its manifest
/// doesn't say.
pub(crate) fn console_of(kernel: &Kernel, emulator: &PluginId) -> String {
    kernel
        .manifest(emulator)
        .ok()
        .and_then(|m| m.console.as_ref().map(|c| c.as_str().to_owned()))
        .unwrap_or_else(|| emulator.to_string())
}

/// The player's folders as `emulator` may use them.
pub(crate) fn folders_of(kernel: &Kernel, emulator: &PluginId) -> CoreResult<HostFolders> {
    kernel.host_folders(emulator).ok_or_else(|| CoreError::UnknownPlugin(emulator.clone()))
}

/// The console a release is for, which names its folder: the emulator's when the catalog
/// doesn't say.
pub(crate) fn console(kernel: &Kernel, emulator: &PluginId, platform: &str) -> String {
    if platform.is_empty() { console_of(kernel, emulator) } else { platform.to_owned() }
}

/// Runs the player's command on the catalogs' downloads.
pub(crate) async fn torrent(core: &Core, command: TorrentCmd) -> CoreResult<()> {
    match command {
        TorrentCmd::Download { offer, release, folder } => {
            core.torrents().set_download_error(None);
            let started = download_release(core, &offer, &release, folder.as_deref()).await;
            crate::disks::refresh(core);
            // The sheet that asked says why nothing downloads, and so does a notice, for
            // when the player closed the sheet.
            if let Err(e) = &started {
                core.torrents().set_download_error(Some((offer, e.to_string())));
                core.notices().problem(e.to_string());
            }
            started
        }
        TorrentCmd::Act { id, action } => {
            if let Some(store) = id.strip_prefix("store:") {
                return store_install_action(core, store, action).await;
            }
            if !core.torrents().has(&id) {
                return Err(CoreError::Invalid(format!("no download {id}")));
            }
            match action {
                // Not while the download's install reads its files.
                TorrentAction::DeleteFiles if core.installs().running_from(&id).is_some() => Ok(()),
                action => core.torrents().handle(TorrentCmd::Act { id, action }).await,
            }
        }
        command @ TorrentCmd::SeedDownloads(_) => core.torrents().handle(command).await,
    }
}

/// What the torrents subsystem tells the rest of the core.
pub(crate) fn torrent_event(core: &Core, event: TorrentEvent) {
    match event {
        TorrentEvent::Finished { id, title } => {
            let waiting = core
                .torrents()
                .job(&id)
                .is_some_and(|j| j.hand_off.unpack.phase == crate::archives::UnpackPhase::Waiting);
            if waiting {
                core.installs().unpack(&id, false);
            }
            crate::disks::refresh(core);
            core.notices().opening(
                nexus_ui_api::NoticeKind::Success,
                crate::Route::Downloads,
                format!("Download complete: {title}. Install it from Downloads."),
            );
        }
        TorrentEvent::Stopped { title } => {
            crate::disks::refresh(core);
            core.notices().opening(
                nexus_ui_api::NoticeKind::Error,
                crate::Route::Downloads,
                format!("Download stopped: {title}. Open Downloads to see why"),
            )
        }
        TorrentEvent::Problem(message) => core.notices().problem(message),
        TorrentEvent::VpnsRead(vpns) => core.vpn().keep(vpns),
        // Not in the VPN check: the catalogs' fetches would hold up the kill switch.
        TorrentEvent::VpnBack => {
            let core = core.clone();
            tokio::spawn(async move { crate::catalog_reads::refresh_releases(&core).await });
        }
    }
}

/// An action on the store's install `store` (`<plugin>:<release>`): only opening the store's
/// page; the store pauses and cancels its own.
async fn store_install_action(core: &Core, store: &str, action: TorrentAction) -> CoreResult<()> {
    let install = core
        .state()
        .store_installs
        .iter()
        .find(|(plugin, i)| format!("{plugin}:{}", i.release) == store)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no install {store}")))?;
    let (plugin, install) = install;
    if action != TorrentAction::OpenStore {
        return Err(CoreError::Invalid(format!("{} does that in its own Downloads", install.title)));
    }
    if !core.kernel().manifest(&plugin)?.permissions.allows_link(&install.open) {
        return Err(CoreError::PermissionDenied { plugin, action: format!("open {}", install.open) });
    }
    let link = install.open.clone();
    crate::running::open_link(core, link).await.map_err(CoreError::Invalid)
}

/// A release its store installs: the store's link opens (`steam://install/440`), if the
/// catalog plugin's manifest allows it, and the store takes it from there. Nothing downloads
/// here; the game shows in the Library once the store installed it.
async fn install_in_store(core: &Core, offer: &GameKey, release: &nexus_api::Release) -> CoreResult<()> {
    let plugin = &offer.plugin;
    let allowed = core.kernel().manifest(plugin)?.permissions.allows_link(&release.store);
    if !allowed {
        return Err(CoreError::PermissionDenied { plugin: plugin.clone(), action: format!("open {}", release.store) });
    }
    let link = release.store.clone();
    crate::running::open_link(core, link).await.map_err(CoreError::Invalid)?;
    let store = crate::snapshot::Plugins::new(core.kernel()).system(&release.platform).name;
    tracing::info!(%offer, link = %release.store, "asked the store to install");
    // The store writes its install down a moment later: its progress is read often for now.
    core.state().store_watch_until = Some(core.ports().clock.now() + std::time::Duration::from_secs(60));
    core.notices().info(format!("{store} is installing {}: it shows in your Library when it's done", release.title));
    core.changed(Area::Downloads);
    Ok(())
}

/// The player asked to download `release` of the offered game `offer`: the subsystem reads
/// the torrent's files, and waits for the player to start it.
async fn download_release(core: &Core, offer: &GameKey, release: &str, folder: Option<&Path>) -> CoreResult<()> {
    let found = core
        .library()
        .snapshot()
        .offer(offer)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no game {offer} in a catalog")))?;
    let release = found
        .releases
        .iter()
        .find(|r| r.id == release)
        .cloned()
        .ok_or_else(|| CoreError::Invalid(format!("no release {release} of {offer}")))?;
    if !release.store.is_empty() {
        return install_in_store(core, offer, &release).await;
    }
    let torrents = core.torrents();
    if torrents.has(&release.id) {
        return Ok(());
    }
    let chosen = folder.is_some();
    if torrents.full() {
        return Err(CoreError::Invalid(format!(
            "Remove an old download first (at most {})",
            crate::torrents::MAX_DOWNLOADS
        )));
    }
    let folders = folders_of(core.kernel(), &found.emulator)?;
    let for_console = console(core.kernel(), &found.emulator, &release.platform);
    let (at, console_folder) = folders.place(Purpose::Download, &for_console, folder)?;
    let name = format!("torrent-{}", release.id);
    let source = at.join(&name).map_err(|e| CoreError::Invalid(e.to_string()))?;
    let folder = console_folder.join(name);
    // A picked folder may be on another disk than the one the Game Hub checked.
    if chosen
        && let (Some(size), Some(free)) = (release.size_bytes, free_at(core.ports().hardware.as_ref(), &folder))
        && let Some(error) = no_room(size, free, "download folder")
    {
        return Err(CoreError::Invalid(error));
    }
    if chosen {
        folders.remember(Purpose::Download, &for_console, &console_folder);
    }
    torrents
        .add(NewDownload {
            id: release.id.clone(),
            offer: offer.clone(),
            emulator: found.emulator.clone(),
            title: release.title.clone(),
            version: release.version.clone(),
            region: release.region.clone(),
            magnet: release.torrent.clone(),
            source,
            folder,
            total: release.size_bytes.unwrap_or(0),
        })
        .await
}

/// Runs the player's command on game installs.
pub(crate) async fn game_install(core: &Core, command: GameInstallCmd) -> CoreResult<()> {
    match command {
        GameInstallCmd::Download { id, folder } => {
            if !core.torrents().has(&id) {
                return Err(CoreError::Invalid(format!("no download {id}")));
            }
            install_download(core, &id, folder.as_deref()).await
        }
        GameInstallCmd::Package { plugin, source } => install_game(core, &plugin, source, None).await,
        command => core.installs().handle(command).await,
    }
}

/// Starts installing `source` with the plugin's game installer, into the game folder `into`
/// (`None`: the plugin's choice).
pub(crate) async fn install_game(
    core: &Core,
    plugin: &PluginId,
    source: HostPath,
    into: Option<HostPath>,
) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let installer = crate::error::require(plugin, nexus_api::Capability::GameInstaller, provided.game_installer)?;
    let folders = kernel.host_folders(plugin).ok_or_else(|| CoreError::Invalid(format!("{plugin} has no manifest")))?;
    let id = core.inner.downloads.next_id();
    core.installs().begin(NewInstall { id, plugin: plugin.clone(), source, into, installer, folders }).await
}

/// What the game installs subsystem tells the rest of the core.
pub(crate) async fn install_event(core: &Core, event: InstallEvent) {
    match event {
        InstallEvent::Installed => {
            if let Err(e) = Box::pin(core.dispatch(crate::Command::Library(crate::LibraryCmd::Refresh))).await {
                tracing::warn!(error = %e, "library refresh after an install failed");
            }
        }
        InstallEvent::Failed(error) => core.notices().opening(
            nexus_ui_api::NoticeKind::Error,
            crate::Route::Downloads,
            format!("The install failed: {error}"),
        ),
        InstallEvent::Unpacked { plugin, source, into, title } => {
            if let Err(e) = install_game(core, &plugin, source, into).await {
                core.notices().opening(
                    nexus_ui_api::NoticeKind::Error,
                    crate::Route::Downloads,
                    format!("{title} couldn't install: {e}"),
                );
            }
        }
    }
}

/// The player pressed Install on the install sheet, with the game folder they picked
/// (`None` for the one the sheet shows). The archives unpack first. Why it doesn't start
/// stays on the download, for the sheet.
async fn install_download(core: &Core, id: &str, folder: Option<&Path>) -> CoreResult<()> {
    core.installs().set_install_error(id, String::new());
    let started = start_install(core, id, folder).await;
    if let Err(e) = &started {
        core.installs().set_install_error(id, e.to_string());
    }
    started
}

async fn start_install(core: &Core, id: &str, folder: Option<&Path>) -> CoreResult<()> {
    let job = core.torrents().job(id).ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
    if job.state != TorrentState::Complete {
        return Ok(());
    }
    let kernel = core.kernel();
    let folders = folders_of(kernel, &job.emulator)?;
    let console = console_of(kernel, &job.emulator);
    let (into, path) = folders.place(Purpose::Install, &console, folder)?;
    if let Some(free) = free_at(core.ports().hardware.as_ref(), &path)
        && let Some(error) = no_room(job.total, free, "game folder")
    {
        return Err(CoreError::Invalid(error));
    }
    if folder.is_some() {
        folders.remember(Purpose::Install, &console, &path);
    }
    if let Some((plugin, source)) = core.installs().install_download(id, into.clone()) {
        install_game(core, &plugin, source, Some(into)).await?;
    }
    Ok(())
}
