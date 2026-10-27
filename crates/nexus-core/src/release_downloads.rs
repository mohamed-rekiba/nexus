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

use std::path::{Path, PathBuf};

use nexus_api::{GAMES_FOLDER, GameKey, HostPath, PACKAGES_FOLDER, PluginId, RelPath};

use crate::app::Core;
use crate::changes::Area;
use crate::game_installs::{GameInstallCmd, InstallEvent, NewInstall};
use crate::torrents::{NewDownload, TorrentAction, TorrentCmd, TorrentEvent, TorrentState, no_room};
use crate::{CoreError, CoreResult, Kernel};

/// The free bytes on the disk that holds `folder`, which may not be there yet: the disk of the
/// nearest folder above it.
pub(crate) fn free_at(hardware: &dyn crate::Hardware, folder: &Path) -> Option<u64> {
    let there = folder.ancestors().find(|p| p.is_dir())?;
    hardware.free_bytes(there)
}

/// A folder inside one of `emulator`'s `grant` folders (`packages` or `games`): `chosen`, or
/// else the one last picked for the console (saved under `app.<saved>.<console>`), or else
/// `<console>` in the first one. `what` names the grant in the errors ("packages folder").
pub(crate) fn folder_in(
    kernel: &Kernel,
    emulator: &PluginId,
    grant: &str,
    console: &str,
    chosen: Option<&Path>,
    saved: &str,
    what: &str,
) -> CoreResult<(HostPath, PathBuf)> {
    let manifest = kernel.manifest(emulator)?;
    let host = kernel.host();
    let home = host.home.as_deref();
    let roots = manifest
        .permissions
        .folder(grant)
        .map(|g| crate::folders::roots(kernel.config(), home, host.platform, g))
        .unwrap_or_default();
    // The root that holds `folder`, and the path from that root.
    let inside = |folder: &Path| {
        roots.iter().enumerate().find_map(|(i, root)| {
            let rest = folder.strip_prefix(root).ok()?;
            let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            Some((i as u32, parts))
        })
    };
    let (root, parts) = match chosen {
        Some(path) => {
            let path = crate::folders::expand(&path.to_string_lossy(), home)
                .ok_or_else(|| CoreError::Invalid(format!("{} isn't a full path", path.display())))?;
            inside(&path).ok_or_else(|| {
                CoreError::Invalid(format!(
                    "{} isn't in a {what}. Pick a folder inside a {what}, or add it in Settings",
                    path.display()
                ))
            })?
        }
        None => kernel
            .config()
            .get(&["app", saved, console])
            .as_str()
            .and_then(|saved| inside(Path::new(saved)))
            .unwrap_or_else(|| (0, vec![console.to_owned()])),
    };
    let rel = RelPath::new(parts.join("/")).map_err(|e| CoreError::Invalid(e.to_string()))?;
    let at = HostPath::new(grant, root, rel);
    let folder = crate::folders::resolve(kernel.config(), home, host.platform, &manifest.permissions, &at)
        .map_err(|e| CoreError::Invalid(format!("{} has no {what}: {e}", manifest.name)))?;
    Ok((at, folder))
}

/// Saves `folder` as the last one picked for `console`, under `app.<saved>`.
fn save_folder(kernel: &Kernel, saved: &str, console: &str, folder: &Path) {
    let value = serde_json::Value::String(folder.display().to_string());
    if let Err(e) = kernel.config().set(&["app", saved, console], value) {
        tracing::warn!(error = %e, "could not save the folder");
    }
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

/// Where an install of `emulator`'s goes: `chosen`, or else the folder last picked for its
/// console, or else the console's folder in the first game folder.
pub(crate) fn install_folder(
    kernel: &Kernel,
    emulator: &PluginId,
    chosen: Option<&Path>,
) -> CoreResult<(HostPath, PathBuf)> {
    let console = console_of(kernel, emulator);
    folder_in(kernel, emulator, GAMES_FOLDER, &console, chosen, "install_folder", "game folder")
}

/// The console a release is for, which names its folder: the emulator's when the catalog
/// doesn't say.
fn console(kernel: &Kernel, emulator: &PluginId, platform: &str) -> String {
    if platform.is_empty() { console_of(kernel, emulator) } else { platform.to_owned() }
}

impl Core {
    /// Runs the player's command on the catalogs' downloads.
    pub(crate) async fn torrent(&self, command: TorrentCmd) -> CoreResult<()> {
        match command {
            TorrentCmd::Download { offer, release, folder } => {
                self.torrents().set_download_error(None);
                let started = self.download_release(&offer, &release, folder.as_deref()).await;
                // The sheet that asked says why nothing downloads, and so does a notice, for
                // when the player closed the sheet.
                if let Err(e) = &started {
                    self.torrents().set_download_error(Some((offer, e.to_string())));
                    self.notices().problem(e.to_string());
                }
                started
            }
            TorrentCmd::Act { id, action } => {
                if let Some(store) = id.strip_prefix("store:") {
                    return self.store_install_action(store, action).await;
                }
                if !self.torrents().has(&id) {
                    return Err(CoreError::Invalid(format!("no download {id}")));
                }
                match action {
                    // Not while the download's install reads its files.
                    TorrentAction::DeleteFiles if self.installs().running_from(&id).is_some() => Ok(()),
                    action => self.torrents().handle(TorrentCmd::Act { id, action }).await,
                }
            }
            command @ TorrentCmd::SeedDownloads(_) => self.torrents().handle(command).await,
        }
    }

    /// What the torrents subsystem tells the rest of the core.
    pub(crate) fn torrent_event(&self, event: TorrentEvent) {
        match event {
            TorrentEvent::Finished { id, title } => {
                let waiting = self
                    .torrents()
                    .job(&id)
                    .is_some_and(|j| j.hand_off.unpack.phase == crate::archives::UnpackPhase::Waiting);
                if waiting {
                    self.installs().unpack(&id, false);
                }
                self.notices().opening(
                    nexus_ui_api::NoticeKind::Success,
                    crate::Route::Downloads,
                    format!("Download complete: {title}. Install it from Downloads."),
                );
            }
            TorrentEvent::Stopped { title } => self.notices().opening(
                nexus_ui_api::NoticeKind::Error,
                crate::Route::Downloads,
                format!("Download stopped: {title}. Open Downloads to see why"),
            ),
            TorrentEvent::Problem(message) => self.notices().problem(message),
            TorrentEvent::VpnsRead(vpns) => self.vpn().keep(vpns),
            // Not in the VPN check: the catalogs' fetches would hold up the kill switch.
            TorrentEvent::VpnBack => {
                let core = self.clone();
                tokio::spawn(async move { core.refresh_releases().await });
            }
        }
    }

    /// An action on the store's install `store` (`<plugin>:<release>`): only opening the store's
    /// page; the store pauses and cancels its own.
    async fn store_install_action(&self, store: &str, action: TorrentAction) -> CoreResult<()> {
        let install = self
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
        if !self.kernel().manifest(&plugin)?.permissions.allows_link(&install.open) {
            return Err(CoreError::PermissionDenied { plugin, action: format!("open {}", install.open) });
        }
        let link = install.open.clone();
        crate::running::open_link(self, link).await.map_err(CoreError::Invalid)
    }

    /// A release its store installs: the store's link opens (`steam://install/440`), if the
    /// catalog plugin's manifest allows it, and the store takes it from there. Nothing downloads
    /// here; the game shows in the Library once the store installed it.
    async fn install_in_store(&self, offer: &GameKey, release: &nexus_api::Release) -> CoreResult<()> {
        let plugin = &offer.plugin;
        let allowed = self.kernel().manifest(plugin)?.permissions.allows_link(&release.store);
        if !allowed {
            return Err(CoreError::PermissionDenied {
                plugin: plugin.clone(),
                action: format!("open {}", release.store),
            });
        }
        let link = release.store.clone();
        crate::running::open_link(self, link).await.map_err(CoreError::Invalid)?;
        let store = crate::view::system_of(self.kernel(), &release.platform).name;
        tracing::info!(%offer, link = %release.store, "asked the store to install");
        // The store writes its install down a moment later: its progress is read often for now.
        self.state().store_watch_until = Some(self.ports().clock.now() + std::time::Duration::from_secs(60));
        self.notices()
            .info(format!("{store} is installing {}: it shows in your Library when it's done", release.title));
        self.changed(Area::Downloads);
        Ok(())
    }

    /// The player asked to download `release` of the offered game `offer`: the subsystem reads
    /// the torrent's files, and waits for the player to start it.
    async fn download_release(&self, offer: &GameKey, release: &str, folder: Option<&Path>) -> CoreResult<()> {
        let found = self
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
            return self.install_in_store(offer, &release).await;
        }
        let torrents = self.torrents();
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
        let (source, folder) = self.torrent_folder(&found.emulator, &release.platform, &release.id, folder)?;
        // A picked folder may be on another disk than the one the Game Hub checked.
        if chosen
            && let (Some(size), Some(free)) = (release.size_bytes, free_at(self.ports().hardware.as_ref(), &folder))
            && let Some(error) = no_room(size, free, "download folder")
        {
            return Err(CoreError::Invalid(error));
        }
        if let Some(parent) = folder.parent().filter(|_| chosen) {
            save_folder(
                self.kernel(),
                "download_folder",
                &console(self.kernel(), &found.emulator, &release.platform),
                parent,
            );
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

    /// Where a download of `emulator`'s goes: in `chosen`, or else in the folder last picked
    /// for the console, or else in the console's folder in the first packages folder. The
    /// folder must be inside one of the emulator's packages folders, so its installer reads it.
    pub(crate) fn torrent_folder(
        &self,
        emulator: &PluginId,
        platform: &str,
        id: &str,
        chosen: Option<&Path>,
    ) -> CoreResult<(HostPath, PathBuf)> {
        let kernel = self.kernel();
        let (at, folder) = folder_in(
            kernel,
            emulator,
            PACKAGES_FOLDER,
            &console(kernel, emulator, platform),
            chosen,
            "download_folder",
            "packages folder",
        )?;
        let name = format!("torrent-{id}");
        Ok((at.join(&name).map_err(|e| CoreError::Invalid(e.to_string()))?, folder.join(name)))
    }

    /// Runs the player's command on game installs.
    pub(crate) async fn game_install(&self, command: GameInstallCmd) -> CoreResult<()> {
        match command {
            GameInstallCmd::Download { id, folder } => {
                if !self.torrents().has(&id) {
                    return Err(CoreError::Invalid(format!("no download {id}")));
                }
                self.install_download(&id, folder.as_deref()).await
            }
            GameInstallCmd::Package { plugin, source } => self.install_game(&plugin, source, None).await,
            command => self.installs().handle(command).await,
        }
    }

    /// Starts installing `source` with the plugin's game installer, into the game folder `into`
    /// (`None`: the plugin's choice).
    pub(crate) async fn install_game(
        &self,
        plugin: &PluginId,
        source: HostPath,
        into: Option<HostPath>,
    ) -> CoreResult<()> {
        let kernel = self.kernel();
        let provided = kernel.provided(plugin).await?;
        let installer = crate::error::require(plugin, nexus_api::Capability::GameInstaller, provided.game_installer)?;
        let folders = crate::app::plugin_folders(kernel, plugin)
            .ok_or_else(|| CoreError::Invalid(format!("{plugin} has no manifest")))?;
        let id = self.inner.downloads.next_id();
        self.installs().begin(NewInstall { id, plugin: plugin.clone(), source, into, installer, folders }).await
    }

    /// What the game installs subsystem tells the rest of the core.
    pub(crate) async fn install_event(&self, event: InstallEvent) {
        match event {
            InstallEvent::Installed => {
                if let Err(e) = Box::pin(self.dispatch(crate::Command::Library(crate::LibraryCmd::Refresh))).await {
                    tracing::warn!(error = %e, "library refresh after an install failed");
                }
            }
            InstallEvent::Failed(error) => self.notices().opening(
                nexus_ui_api::NoticeKind::Error,
                crate::Route::Downloads,
                format!("The install failed: {error}"),
            ),
            InstallEvent::Unpacked { plugin, source, into, title } => {
                if let Err(e) = self.install_game(&plugin, source, into).await {
                    self.notices().opening(
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
    async fn install_download(&self, id: &str, folder: Option<&Path>) -> CoreResult<()> {
        self.installs().set_install_error(id, String::new());
        let started = self.start_install(id, folder).await;
        if let Err(e) = &started {
            self.installs().set_install_error(id, e.to_string());
        }
        started
    }

    async fn start_install(&self, id: &str, folder: Option<&Path>) -> CoreResult<()> {
        let job = self.torrents().job(id).ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
        if job.state != TorrentState::Complete {
            return Ok(());
        }
        let kernel = self.kernel();
        let (into, path) = install_folder(kernel, &job.emulator, folder)?;
        if let Some(free) = free_at(self.ports().hardware.as_ref(), &path)
            && let Some(error) = no_room(job.total, free, "game folder")
        {
            return Err(CoreError::Invalid(error));
        }
        if folder.is_some() {
            save_folder(kernel, "install_folder", &console_of(kernel, &job.emulator), &path);
        }
        if let Some((plugin, source)) = self.installs().install_download(id, into.clone()) {
            self.install_game(&plugin, source, Some(into)).await?;
        }
        Ok(())
    }
}
