//! Downloads of the releases a catalog offers, through the host's [`Torrents`] port.
//!
//! A download starts only from the player's explicit choice. It first reads the torrent's
//! files (Resolving), then waits for the player to look at them (Ready), then checks the pieces
//! already on disk and downloads the rest. A finished download seeds while "Keep sharing
//! finished downloads" is on (app.seed_downloads, on by default), at most
//! [`SEED_UPLOAD_LIMIT`]; after a restart it checks its files first and seeds only when they are
//! whole. Stop seeding lasts until the next start; only the setting turns seeding off for good.
//!
//! Each download is saved under `app.torrents.<info hash>`: a restart brings unfinished ones
//! back paused. Files go to `<folder>/torrent-<info hash>`: the folder the player picked, inside
//! one of the emulator's packages folders, or else `<console>` in its first one. Its
//! [`nexus_api::GameInstaller`] takes them there; Install hands the folder over. A start checks
//! the free space first.

use std::path::{Path, PathBuf};

use nexus_api::{GAMES_FOLDER, GameKey, HostPath, PACKAGES_FOLDER, PluginId, RelPath};
use serde::{Deserialize, Serialize};

use crate::app::Core;
use crate::{CoreError, CoreResult, Kernel, TorrentFile, TorrentPhase, TorrentStart};

/// Where downloads are saved in the config.
const RECORDS: [&str; 2] = ["app", "torrents"];
/// How many files a download shows; the count says how many there are.
const SHOWN_FILES: usize = 100;
/// The most downloads kept at once.
const MAX_DOWNLOADS: usize = 32;

/// `1.5 GB`, for the free space check.
pub(crate) fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Why `needed` bytes don't fit in `free` in the `place` ("download folder"); `None` when they
/// fit. The Game Hub says the same before the download starts.
pub(crate) fn no_room(needed: u64, free: u64, place: &str) -> Option<String> {
    (free < needed).then(|| {
        format!("Not enough free space in the {place}: {} needed, {} free", gigabytes(needed), gigabytes(free))
    })
}

/// The free bytes on the disk that holds `folder`, which may not be there yet: the disk of the
/// nearest folder above it.
pub(crate) fn free_at(kernel: &Kernel, folder: &Path) -> Option<u64> {
    let there = folder.ancestors().find(|p| p.is_dir())?;
    kernel.hardware().free_bytes(there)
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

/// The console an emulator plays, which names its folders: PS5 when its manifest doesn't say.
pub(crate) fn console_of(kernel: &Kernel, emulator: &PluginId) -> String {
    kernel
        .manifest(emulator)
        .ok()
        .and_then(|m| m.console.as_ref().map(|c| c.as_str().to_owned()))
        .unwrap_or_else(|| "PS5".into())
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

const FILES_CHANGED: &str = "The downloaded files were moved, changed or deleted, so this download can't seed. Remove it, or download it again.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TorrentState {
    Resolving,
    Ready,
    Checking,
    Downloading,
    Paused,
    Complete,
    Failed,
    Cancelled,
}

impl TorrentState {
    pub fn active(self) -> bool {
        matches!(self, Self::Resolving | Self::Checking | Self::Downloading)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolving => "resolving",
            Self::Ready => "ready",
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One download, as it is saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TorrentJob {
    /// The torrent's info hash.
    pub id: String,
    /// The game the catalog offers, and the emulator that installs it.
    pub offer: GameKey,
    pub emulator: PluginId,
    pub title: String,
    pub version: String,
    pub region: String,
    pub magnet: String,
    /// The folder in the emulator's packages folder, and where that is on disk.
    pub source: HostPath,
    pub folder: PathBuf,
    pub state: TorrentState,
    pub done: u64,
    pub total: u64,
    /// The first [`SHOWN_FILES`] files.
    pub files: Vec<TorrentFile>,
    pub file_count: usize,
    pub error: String,
    #[serde(skip)]
    pub download_rate: u64,
    #[serde(skip)]
    pub upload_rate: u64,
    #[serde(skip)]
    pub peers: u32,
    #[serde(skip)]
    pub seeding: bool,
    /// Checking a finished download's files before it seeds again.
    #[serde(skip)]
    pub verifying: bool,
    /// Paused by "Download only through a VPN": it starts again once the VPN is connected.
    /// A restart forgets it.
    #[serde(skip)]
    pub waiting_vpn: bool,
    /// Its archives, found when it finished.
    #[serde(default)]
    pub unpack: crate::archives::Unpack,
    /// The game folder the player picked on the install sheet, once they pressed Install.
    #[serde(default)]
    pub install_into: Option<HostPath>,
    /// Why the last Install didn't start, for the install sheet.
    #[serde(skip)]
    pub install_error: String,
}

/// What the player can do with a download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorrentAction {
    /// Ready, paused, failed or cancelled: download it.
    Start,
    Pause,
    /// Stop it; the files stay.
    Cancel,
    /// Forget a download that isn't running; the files stay.
    Remove,
    /// Forget a download that isn't running, and delete its folder: the downloaded files and
    /// their unpacked copies. An installed game is elsewhere and stays.
    DeleteFiles,
    StopSeeding,
    /// Hand a finished download to the emulator's game installer, after its archives are
    /// unpacked.
    Install,
    /// Delete the unpacked archives; the torrent's files stay.
    RemoveUnpacked,
    /// Stop the install of the download; what it wrote goes.
    CancelInstall,
}

/// The console a release is for, which names its folder: PS5 when the catalog doesn't say.
fn console(platform: &str) -> &str {
    if platform.is_empty() { "PS5" } else { platform }
}

impl Core {
    pub(crate) fn seed_on(&self) -> bool {
        self.kernel().config().get(&["app", "seed_downloads"]).as_bool().unwrap_or(true)
    }

    fn save_torrent(&self, job: &TorrentJob) {
        let value = serde_json::to_value(job).unwrap_or_default();
        if let Err(e) = self.kernel().config().set(&[RECORDS[0], RECORDS[1], &job.id], value) {
            tracing::warn!(error = %e, "could not save a download");
        }
    }

    fn forget_torrent(&self, id: &str) {
        let config = self.kernel().config();
        let mut records = config.get(&RECORDS);
        if let Some(map) = records.as_object_mut() {
            map.remove(id);
            let _ = config.set(&RECORDS, records);
        }
    }

    /// Changes a download, saves it and shows it.
    pub(crate) fn update_torrent(&self, id: &str, change: impl FnOnce(&mut TorrentJob)) {
        let job = {
            let mut state = self.state();
            let Some(job) = state.torrents.get_mut(id) else { return };
            change(job);
            job.clone()
        };
        self.save_torrent(&job);
        self.changed();
    }

    /// The downloads the last run left: unfinished ones come back paused, and finished ones
    /// seed again after a check of their files, while seeding is on.
    pub(crate) fn restore_torrents(&self) {
        let records = self.kernel().config().get(&RECORDS);
        let Some(map) = records.as_object() else { return };
        let mut jobs: Vec<TorrentJob> =
            map.values().filter_map(|v| serde_json::from_value::<TorrentJob>(v.clone()).ok()).collect();
        for job in &mut jobs {
            job.unpack.restored(&job.folder);
            if job.state.active() {
                job.state =
                    if job.state == TorrentState::Resolving { TorrentState::Failed } else { TorrentState::Paused };
                if job.state == TorrentState::Failed {
                    job.error = "The app stopped before the torrent's files were read: try again.".into();
                }
            }
        }
        let seed = self.seed_on();
        {
            let mut state = self.state();
            for job in &jobs {
                state.torrents.insert(job.id.clone(), job.clone());
            }
        }
        for job in jobs {
            self.save_torrent(&job);
            if seed && job.state == TorrentState::Complete && job.error.is_empty() {
                let core = self.clone();
                tokio::spawn(async move { core.start_torrent(&job.id, TorrentStart::SeedOnly).await });
            }
        }
    }

    /// The player asked to download `release` of the offered game `offer`: reads the torrent's
    /// files, and waits for the player to start it.
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
        crate::running::blocking(self, move |desktop, _| desktop.open_link(&link)).await.map_err(CoreError::Invalid)?;
        let store = crate::view::system_of(self.kernel(), &release.platform).name;
        tracing::info!(%offer, link = %release.store, "asked the store to install");
        self.notice(format!("{store} is installing {}: it shows in your Library when it's done", release.title));
        self.changed();
        Ok(())
    }

    pub(crate) async fn download_release(
        &self,
        offer: &GameKey,
        release: &str,
        folder: Option<&std::path::Path>,
    ) -> CoreResult<()> {
        let found = self.offer(offer).ok_or_else(|| CoreError::Invalid(format!("no game {offer} in a catalog")))?;
        let release = found
            .releases
            .iter()
            .find(|r| r.id == release)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("no release {release} of {offer}")))?;
        if !release.store.is_empty() {
            return self.install_in_store(offer, &release).await;
        }
        if self.state().torrents.contains_key(&release.id) {
            return Ok(());
        }
        let chosen = folder.is_some();
        if self.state().torrents.len() >= MAX_DOWNLOADS {
            return Err(CoreError::Invalid(format!("Remove an old download first (at most {MAX_DOWNLOADS})")));
        }
        let (source, folder) = self.torrent_folder(&found.emulator, &release.platform, &release.id, folder)?;
        // A picked folder may be on another disk than the one the Game Hub checked.
        if chosen
            && let (Some(size), Some(free)) = (release.size_bytes, free_at(self.kernel(), &folder))
            && let Some(error) = no_room(size, free, "download folder")
        {
            return Err(CoreError::Invalid(error));
        }
        if let Some(parent) = folder.parent().filter(|_| chosen) {
            save_folder(self.kernel(), "download_folder", console(&release.platform), parent);
        }
        let job = TorrentJob {
            id: release.id.clone(),
            offer: offer.clone(),
            emulator: found.emulator.clone(),
            title: release.title.chars().filter(|c| !c.is_control()).take(200).collect(),
            version: release.version.clone(),
            region: release.region.clone(),
            magnet: release.torrent.clone(),
            source,
            folder,
            state: TorrentState::Resolving,
            done: 0,
            total: release.size_bytes.unwrap_or(0),
            files: Vec::new(),
            file_count: 0,
            error: String::new(),
            download_rate: 0,
            upload_rate: 0,
            peers: 0,
            seeding: false,
            verifying: false,
            waiting_vpn: false,
            unpack: Default::default(),
            install_into: None,
            install_error: String::new(),
        };
        self.state().torrents.insert(job.id.clone(), job.clone());
        self.save_torrent(&job);
        self.changed();
        // Even its files come from the swarm: without the VPN it waits.
        if self.vpn_gate(&job.id).await {
            let core = self.clone();
            tokio::spawn(async move { core.resolve_torrent(job.id, job.magnet).await });
        }
        Ok(())
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
            console(platform),
            chosen,
            "download_folder",
            "packages folder",
        )?;
        let name = format!("torrent-{id}");
        Ok((at.join(&name).map_err(|e| CoreError::Invalid(e.to_string()))?, folder.join(name)))
    }

    /// Reads the torrent's files again, for a download the VPN stopped before it had them.
    pub(crate) async fn read_torrent(&self, id: &str) {
        let Some(magnet) = self.state().torrents.get(id).map(|j| j.magnet.clone()) else { return };
        self.update_torrent(id, |job| {
            job.state = TorrentState::Resolving;
            job.error.clear();
        });
        if !self.vpn_gate(id).await {
            return;
        }
        let core = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { core.resolve_torrent(id, magnet).await });
    }

    async fn resolve_torrent(&self, id: String, magnet: String) {
        match self.kernel().torrents().resolve(&magnet).await {
            Ok(info) => self.update_torrent(&id, |job| {
                if job.state != TorrentState::Resolving {
                    return;
                }
                job.total = info.files.iter().map(|f| f.size).sum();
                job.file_count = info.files.len();
                job.files = info.files.into_iter().take(SHOWN_FILES).collect();
                job.state = TorrentState::Ready;
            }),
            Err(e) => self.update_torrent(&id, |job| {
                if job.state == TorrentState::Resolving {
                    job.state = TorrentState::Failed;
                    job.error = format!("The torrent's files couldn't be read: {e}");
                }
            }),
        }
    }

    pub(crate) async fn start_torrent(&self, id: &str, how: TorrentStart) {
        if !self.vpn_gate(id).await {
            return;
        }
        let Some(job) = self.state().torrents.get(id).cloned() else { return };
        let seeding = how == TorrentStart::SeedOnly;
        self.update_torrent(id, |job| {
            job.error.clear();
            if seeding {
                job.verifying = true;
            } else {
                job.state = TorrentState::Checking;
            }
        });
        if let Err(e) = tokio::fs::create_dir_all(&job.folder).await {
            self.update_torrent(id, |job| {
                job.state = TorrentState::Failed;
                job.verifying = false;
                job.error = format!("The download folder couldn't be made: {e}");
            });
            return;
        }
        // After the folder is made: a folder that isn't there has no disk to ask.
        let needed = job.total.saturating_sub(job.done);
        if !seeding
            && let Some(free) = self.kernel().hardware().free_bytes(&job.folder)
            && let Some(error) = no_room(needed, free, "download folder")
        {
            self.update_torrent(id, |job| {
                job.state = TorrentState::Failed;
                job.error = error;
            });
            return;
        }
        if let Err(e) = self.kernel().torrents().start(id, &job.magnet, &job.folder, how).await {
            self.update_torrent(id, |job| {
                job.verifying = false;
                if !seeding {
                    job.state = TorrentState::Failed;
                }
                job.error = e.to_string();
            });
            return;
        }
        self.watch_torrents();
    }

    /// Reads the running downloads' state every `torrent_poll` while any runs, and checks the
    /// VPN every `vpn_poll` while one runs or waits for it, or a plugin waits for it, with the
    /// setting on.
    pub(crate) fn watch_torrents(&self) {
        if self.state().torrent_watch {
            return;
        }
        self.state().torrent_watch = true;
        let core = self.clone();
        tokio::spawn(async move {
            let mut checked = std::time::Instant::now();
            loop {
                let mut running = core.poll_torrents();
                let waiting = core.any_waiting_vpn() || core.plugin_waiting_vpn();
                if (waiting || (running && core.vpn_only())) && checked.elapsed() >= core.kernel().host().vpn_poll {
                    core.check_vpn().await;
                    checked = std::time::Instant::now();
                    // What it started is read on the next round.
                    running = true;
                }
                if !running && !waiting {
                    break;
                }
                tokio::time::sleep(core.kernel().host().torrent_poll).await;
            }
            core.state().torrent_watch = false;
        });
    }

    /// Reads each running download once. `false` when none runs.
    pub(crate) fn poll_torrents(&self) -> bool {
        let ids: Vec<String> = {
            let state = self.state();
            state
                .torrents
                .values()
                .filter(|j| j.state.active() || j.seeding || j.verifying)
                .map(|j| j.id.clone())
                .collect()
        };
        let port = self.kernel().torrents().clone();
        let mut running = false;
        let seed = self.seed_on();
        for id in ids {
            let Some(status) = port.status(&id) else { continue };
            running = true;
            let mut finished = None;
            let mut stopped = None;
            self.update_torrent(&id, |job| {
                job.download_rate = status.download_rate;
                job.upload_rate = status.upload_rate;
                job.peers = status.peers;
                if status.total > 0 {
                    job.total = status.total;
                }
                match status.phase {
                    TorrentPhase::Checking if job.verifying => {}
                    TorrentPhase::Checking => {
                        job.state = TorrentState::Checking;
                        job.done = status.done;
                    }
                    TorrentPhase::Downloading => {
                        job.state = TorrentState::Downloading;
                        job.done = status.done;
                    }
                    TorrentPhase::Seeding | TorrentPhase::Finished => {
                        let was = job.state;
                        job.state = TorrentState::Complete;
                        job.done = job.total;
                        job.verifying = false;
                        job.seeding = status.phase == TorrentPhase::Seeding;
                        if was != TorrentState::Complete {
                            finished = Some(job.title.clone());
                            job.unpack = crate::archives::Unpack::of(&job.folder);
                        }
                    }
                    TorrentPhase::Failed if job.verifying => {
                        job.verifying = false;
                        job.seeding = false;
                        job.error = FILES_CHANGED.into();
                    }
                    TorrentPhase::Failed => {
                        // Once for each stop: the next polls see it failed already.
                        if job.state != TorrentState::Failed {
                            stopped = Some(job.title.clone());
                        }
                        job.state = TorrentState::Failed;
                        job.error = status.error.clone();
                    }
                }
            });
            let (stop, failed) = {
                let state = self.state();
                let job = state.torrents.get(&id);
                (
                    job.is_some_and(|j| j.state == TorrentState::Complete && (!j.seeding || !seed) && !j.verifying),
                    job.is_some_and(|j| j.state == TorrentState::Failed || !j.error.is_empty()),
                )
            };
            if let Some(title) = finished {
                if self
                    .state()
                    .torrents
                    .get(&id)
                    .is_some_and(|j| j.unpack.phase == crate::archives::UnpackPhase::Waiting)
                {
                    self.unpack_download(&id, false);
                }
                self.notice_opening(
                    nexus_ui_api::NoticeKind::Success,
                    crate::Route::Downloads,
                    format!("Download complete: {title}. Install it from Downloads."),
                );
            }
            if let Some(title) = stopped {
                self.notice_opening(
                    nexus_ui_api::NoticeKind::Error,
                    crate::Route::Downloads,
                    format!("Download stopped: {title}. Open Downloads to see why"),
                );
            }
            if stop || failed {
                let port = port.clone();
                let core = self.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    let _ = port.stop(&id).await;
                    core.update_torrent(&id, |job| job.seeding = false);
                });
            }
        }
        running
    }

    /// Runs the player's action on a download.
    pub(crate) async fn torrent_action(&self, id: &str, action: TorrentAction) -> CoreResult<()> {
        let job =
            self.state().torrents.get(id).cloned().ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
        let port = self.kernel().torrents().clone();
        match action {
            TorrentAction::Start
                if matches!(
                    job.state,
                    TorrentState::Ready | TorrentState::Paused | TorrentState::Failed | TorrentState::Cancelled
                ) =>
            {
                // The VPN stopped it before it had its files: it reads them first.
                if job.state == TorrentState::Paused && job.file_count == 0 {
                    self.read_torrent(id).await;
                } else {
                    self.start_torrent(id, TorrentStart::Download { seed: self.seed_on() }).await;
                }
            }
            TorrentAction::Pause if matches!(job.state, TorrentState::Checking | TorrentState::Downloading) => {
                port.stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update_torrent(id, |job| {
                    job.state = TorrentState::Paused;
                    job.download_rate = 0;
                    job.peers = 0;
                });
            }
            TorrentAction::Cancel if job.unpack.phase == crate::archives::UnpackPhase::Unpacking => {
                if let Some(cancel) = &job.unpack.cancel {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            TorrentAction::Cancel if !matches!(job.state, TorrentState::Complete | TorrentState::Cancelled) => {
                port.stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update_torrent(id, |job| {
                    job.state = TorrentState::Cancelled;
                    job.waiting_vpn = false;
                    job.download_rate = 0;
                    job.peers = 0;
                });
            }
            TorrentAction::Remove if !job.state.active() && !job.seeding => {
                port.stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.state().torrents.remove(id);
                self.forget_torrent(id);
                self.changed();
            }
            TorrentAction::DeleteFiles
                if !job.state.active()
                    && !job.seeding
                    && job.unpack.phase != crate::archives::UnpackPhase::Unpacking
                    && !self.state().game_installs.values().any(|g| {
                        g.plugin == job.emulator
                            && g.source == job.source
                            && g.phase == crate::game_installs::GamePhase::Installing
                    }) =>
            {
                port.stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.state().torrents.remove(id);
                self.forget_torrent(id);
                // Only the folder Nexus made for this download (`torrent-<id>`).
                let own = job.folder.file_name().is_some_and(|name| name == format!("torrent-{id}").as_str());
                if own
                    && let Err(e) = tokio::fs::remove_dir_all(&job.folder).await
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    self.problem(format!("Couldn't delete {}: {e}", job.folder.display()));
                }
                self.changed();
            }
            TorrentAction::StopSeeding if job.seeding || job.verifying => {
                port.stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update_torrent(id, |job| {
                    job.seeding = false;
                    job.verifying = false;
                    job.upload_rate = 0;
                });
            }
            TorrentAction::Install => self.install_download(id, None).await?,
            TorrentAction::RemoveUnpacked => self.remove_unpacked(id).await,
            TorrentAction::CancelInstall => {
                let running = self
                    .state()
                    .game_installs
                    .iter()
                    .find(|(_, g)| {
                        g.plugin == job.emulator
                            && g.source == job.source
                            && g.phase == crate::game_installs::GamePhase::Installing
                    })
                    .map(|(id, _)| *id);
                if let Some(install) = running {
                    self.cancel_game_install(install);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The player pressed Install on the install sheet, with the game folder they picked
    /// (`None` for the one the sheet shows). The archives unpack first. Why it doesn't start
    /// stays on the download, for the sheet.
    pub(crate) async fn install_download(&self, id: &str, folder: Option<&Path>) -> CoreResult<()> {
        self.update_torrent(id, |job| job.install_error.clear());
        let started = self.start_install(id, folder).await;
        if let Err(e) = &started {
            self.update_torrent(id, |job| job.install_error = e.to_string());
        }
        started
    }

    async fn start_install(&self, id: &str, folder: Option<&Path>) -> CoreResult<()> {
        let job =
            self.state().torrents.get(id).cloned().ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
        if job.state != TorrentState::Complete {
            return Ok(());
        }
        let kernel = self.kernel();
        let (into, path) = install_folder(kernel, &job.emulator, folder)?;
        if let Some(free) = free_at(kernel, &path)
            && let Some(error) = no_room(job.total, free, "game folder")
        {
            return Err(CoreError::Invalid(error));
        }
        if folder.is_some() {
            save_folder(kernel, "install_folder", &console_of(kernel, &job.emulator), &path);
        }
        self.update_torrent(id, |job| job.install_into = Some(into.clone()));
        use crate::archives::UnpackPhase as U;
        match job.unpack.phase {
            U::None | U::Unpacked => self.install_game(&job.emulator, job.source.clone(), Some(into)).await?,
            U::Waiting | U::Failed | U::Cancelled => self.unpack_download(id, true),
            U::Unpacking => {}
        }
        Ok(())
    }

    /// "Keep sharing finished downloads" turned off: every download stops seeding.
    pub(crate) async fn stop_all_seeding(&self) {
        let seeding: Vec<String> =
            self.state().torrents.values().filter(|j| j.seeding || j.verifying).map(|j| j.id.clone()).collect();
        for id in seeding {
            let _ = self.torrent_action(&id, TorrentAction::StopSeeding).await;
        }
    }
}
