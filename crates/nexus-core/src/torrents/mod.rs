//! The torrents subsystem: downloads of the releases a catalog offers, through the host's
//! [`Torrents`] port.
//!
//! A download starts only from the player's explicit choice. It first reads the torrent's
//! files (Resolving), then waits for the player to look at them (Ready), then checks the pieces
//! already on disk and downloads the rest. A finished download seeds while "Keep sharing
//! finished downloads" is on (app.seed_downloads, on by default), at most
//! [`crate::SEED_UPLOAD_LIMIT`]; after a restart it checks its files first and seeds only when
//! they are whole. Stop seeding lasts until the next start; only the setting turns seeding off
//! for good. The rules for what the engine's numbers mean live in [`phase`], once.
//!
//! Each download is saved under `app.torrents.<info hash>` ([`records`]): a restart brings
//! unfinished ones back paused. Files go to `<folder>/torrent-<info hash>`, a folder the core
//! picks before it adds the download.
//!
//! # The subsystem pattern
//!
//! The core's areas follow one shape, and this module is the first:
//!
//! - It owns its state behind its own lock ([`TorrentsSubsystem`]), and its saved records.
//!   Nothing else holds that lock or writes those keys.
//! - It takes only the ports it needs ([`TorrentPorts`]), never the `Kernel` or the `Core`.
//! - The core sends it commands ([`TorrentsSubsystem::handle`]) and reads it whole
//!   ([`TorrentsSubsystem::snapshot`]). A read changes nothing.
//! - It publishes each change of its state as [`Area::Torrents`] on the core's change signal.
//! - What other areas must do about it goes out as events ([`TorrentEvent`]): the core turns a
//!   finished download into an unpack and a notice. It never calls into the core.
//! - A command that needs another area first (a catalog's release, a game folder, an install)
//!   is resolved by the core, which then hands the subsystem plain data
//!   ([`TorrentsSubsystem::add`]). The fields other areas still write on a download
//!   ([`HandOff`]) change through one call ([`TorrentsSubsystem::hand_off`]).
//!
//! Locks: the subsystem never calls out while it holds its lock, and the core never takes it
//! while it holds its own state.

pub(crate) mod phase;
mod records;
mod vpn;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use nexus_api::{GameKey, HostPath, PluginId};
use serde::{Deserialize, Serialize};

pub(crate) use self::phase::TorrentState;
use self::phase::{Action, Phase, TorrentStart};
pub(crate) use self::records::Records;
use crate::changes::{Area, Notifier};
use crate::vpn::VpnRule;
use crate::{Clock, CoreError, CoreResult, Hardware, TorrentFile, Torrents};

/// How many files a download shows; the count says how many there are.
const SHOWN_FILES: usize = 100;
/// The most downloads kept at once.
pub(crate) const MAX_DOWNLOADS: usize = 32;

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

/// The fields of a download that other areas write: the unpacking of its archives and the
/// install from it. They are saved with the download.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct HandOff {
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
    /// How the torrent was started this run.
    #[serde(skip)]
    pub start: Option<TorrentStart>,
    #[serde(flatten)]
    pub hand_off: HandOff,
}

impl TorrentJob {
    fn phase(&self) -> Phase {
        Phase {
            state: self.state,
            done: self.done,
            total: self.total,
            seeding: self.seeding,
            verifying: self.verifying,
            error: self.error.clone(),
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.state = phase.state;
        self.done = phase.done;
        self.total = phase.total;
        self.seeding = phase.seeding;
        self.verifying = phase.verifying;
        self.error = phase.error;
    }

    /// Running in the engine, or about to: polled.
    fn running(&self) -> bool {
        self.state.active() || self.seeding || self.verifying
    }
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
    /// A store's install: open the store's own Downloads page.
    OpenStore,
}

/// What the player asks of the catalogs' downloads.
#[derive(Clone, Debug, PartialEq)]
pub enum TorrentCmd {
    /// Downloads a release of a game the catalogs offer: the player agreed. It reads the
    /// torrent's files first and waits for the player to start it.
    Download {
        offer: GameKey,
        /// The release's id, its torrent's info hash.
        release: String,
        /// The folder the player picked, inside a packages folder (`~/` for the home folder).
        /// `None`: the last one picked for the console, or else the console's folder in the
        /// first packages folder.
        folder: Option<PathBuf>,
    },
    /// Installs a finished download into the game folder the player picked on the install
    /// sheet (`~/` for the home folder). `None`: the last one picked for the console, or else
    /// the console's folder in the first game folder. The folder must be inside a game folder.
    Install { id: String, folder: Option<PathBuf> },
    /// The player's action on a download, by its id.
    Act { id: String, action: TorrentAction },
    /// "Keep sharing finished downloads": on seeds them; off stops every one.
    SeedDownloads(bool),
}

/// A download to add, with everything the core resolved for it: the release, the emulator
/// and the folder.
#[derive(Clone, Debug)]
pub(crate) struct NewDownload {
    pub id: String,
    pub offer: GameKey,
    pub emulator: PluginId,
    pub title: String,
    pub version: String,
    pub region: String,
    pub magnet: String,
    pub source: HostPath,
    pub folder: PathBuf,
    /// The catalog's size, until the torrent's files say.
    pub total: u64,
}

/// The subsystem's state, read whole.
#[derive(Clone, Debug, Default)]
pub(crate) struct TorrentsSnapshot {
    /// By info hash.
    pub jobs: BTreeMap<String, TorrentJob>,
    /// Why the last download the player asked for on a game's Game Hub didn't start, for its
    /// download sheet; gone at the next try or when a Game Hub opens.
    pub download_error: Option<(GameKey, String)>,
    /// "Keep sharing finished downloads".
    pub sharing: bool,
}

/// What other areas must do about a change in the downloads.
#[derive(Clone, Debug)]
pub(crate) enum TorrentEvent {
    /// The download finished: its archives, if it has any, can unpack.
    Finished { id: String, title: String },
    /// The download stopped with an error.
    Stopped { title: String },
    /// Something the player should hear of, such as a folder that didn't delete.
    Problem(String),
    /// The VPNs, as the VPN rule just read them.
    VpnsRead(Vec<crate::Vpn>),
    /// A plugin that waited for the VPN can go on: the VPN is back.
    VpnBack,
}

/// Where the subsystem's events go.
pub(crate) type EventSink = Arc<dyn Fn(TorrentEvent) + Send + Sync>;

/// The ports the subsystem uses, and nothing else.
pub(crate) struct TorrentPorts {
    pub torrents: Arc<dyn Torrents>,
    /// For the free space before a download starts.
    pub hardware: Arc<dyn Hardware>,
    pub clock: Arc<dyn Clock>,
}

/// How often the watch reads the running torrents, and checks the VPN.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Polls {
    pub torrents: Duration,
    pub vpn: Duration,
}

#[derive(Default)]
struct State {
    jobs: BTreeMap<String, TorrentJob>,
    /// The watch task runs.
    watching: bool,
    /// The interface the torrent port is bound to; `None` for any.
    vpn_bound: Option<String>,
    /// The kill switch is looking at the VPN.
    vpn_checking: bool,
    download_error: Option<(GameKey, String)>,
}

struct Shared {
    ports: TorrentPorts,
    polls: Polls,
    records: Records,
    /// "Download only through a VPN", with the VPNs.
    vpn: VpnRule,
    notifier: Notifier,
    events: EventSink,
    state: Mutex<State>,
}

/// The catalogs' downloads. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct TorrentsSubsystem {
    inner: Arc<Shared>,
}

impl TorrentsSubsystem {
    pub fn new(
        ports: TorrentPorts,
        polls: Polls,
        records: Records,
        vpn: VpnRule,
        notifier: Notifier,
        events: EventSink,
    ) -> Self {
        let state = Mutex::new(State::default());
        Self { inner: Arc::new(Shared { ports, polls, records, vpn, notifier, events, state }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn port(&self) -> &Arc<dyn Torrents> {
        &self.inner.ports.torrents
    }

    fn emit(&self, event: TorrentEvent) {
        (self.inner.events)(event);
    }

    fn changed(&self) {
        self.inner.notifier.changed(Area::Torrents);
    }

    /// The downloads, the last download error and the sharing setting, as they are now.
    pub fn snapshot(&self) -> TorrentsSnapshot {
        let state = self.state();
        TorrentsSnapshot {
            jobs: state.jobs.clone(),
            download_error: state.download_error.clone(),
            sharing: self.inner.records.sharing(),
        }
    }

    /// One download, as it is now.
    pub fn job(&self, id: &str) -> Option<TorrentJob> {
        self.state().jobs.get(id).cloned()
    }

    /// "Keep sharing finished downloads".
    pub fn sharing(&self) -> bool {
        self.inner.records.sharing()
    }

    /// Keeps why the last download the player asked for didn't start, or clears it.
    pub fn set_download_error(&self, error: Option<(GameKey, String)>) {
        let changed = {
            let mut state = self.state();
            let changed = state.download_error != error;
            state.download_error = error;
            changed
        };
        if changed {
            self.changed();
        }
    }

    /// Changes a download, saves it and publishes the change. `None` when there is no such
    /// download.
    fn update<R>(&self, id: &str, change: impl FnOnce(&mut TorrentJob) -> R) -> Option<R> {
        let (job, out) = {
            let mut state = self.state();
            let job = state.jobs.get_mut(id)?;
            let out = change(job);
            (job.clone(), out)
        };
        self.inner.records.save(&job);
        self.changed();
        Some(out)
    }

    /// Changes the fields other areas write on download `id` ([`HandOff`]): the unpacking and
    /// the install. The one way in for them.
    pub fn hand_off<R>(&self, id: &str, change: impl FnOnce(&TorrentState, &mut HandOff) -> R) -> Option<R> {
        self.update(id, |job| change(&job.state, &mut job.hand_off))
    }

    /// Runs a command that is the subsystem's alone. The core resolves
    /// [`TorrentCmd::Download`] and [`TorrentCmd::Install`] first: they need the catalogs and
    /// the game folders.
    pub async fn handle(&self, command: TorrentCmd) -> CoreResult<()> {
        match command {
            TorrentCmd::Act { id, action } => self.act(&id, action).await,
            TorrentCmd::SeedDownloads(on) => {
                self.inner.records.set_sharing(on).map_err(|e| CoreError::Io(e.to_string()))?;
                if !on {
                    self.stop_all_seeding().await;
                }
                self.changed();
                Ok(())
            }
            TorrentCmd::Download { .. } | TorrentCmd::Install { .. } => {
                Err(CoreError::Invalid("the core resolves downloads and installs first".into()))
            }
        }
    }

    /// The download is there already.
    pub fn has(&self, id: &str) -> bool {
        self.state().jobs.contains_key(id)
    }

    /// No room for another download.
    pub fn full(&self) -> bool {
        self.state().jobs.len() >= MAX_DOWNLOADS
    }

    /// Adds a download the player asked for, and reads its torrent's files. Nothing downloads
    /// until the player starts it.
    pub async fn add(&self, new: NewDownload) -> CoreResult<()> {
        if self.has(&new.id) {
            return Ok(());
        }
        if self.full() {
            return Err(CoreError::Invalid(format!("Remove an old download first (at most {MAX_DOWNLOADS})")));
        }
        let job = TorrentJob {
            id: new.id,
            offer: new.offer,
            emulator: new.emulator,
            title: new.title.chars().filter(|c| !c.is_control()).take(200).collect(),
            version: new.version,
            region: new.region,
            magnet: new.magnet,
            source: new.source,
            folder: new.folder,
            state: TorrentState::Resolving,
            done: 0,
            total: new.total,
            files: Vec::new(),
            file_count: 0,
            error: String::new(),
            download_rate: 0,
            upload_rate: 0,
            peers: 0,
            seeding: false,
            verifying: false,
            waiting_vpn: false,
            start: None,
            hand_off: HandOff::default(),
        };
        self.state().jobs.insert(job.id.clone(), job.clone());
        self.inner.records.save(&job);
        self.changed();
        // Even its files come from the swarm: without the VPN it waits.
        if self.vpn_gate(&job.id).await {
            let this = self.clone();
            tokio::spawn(async move { this.resolve(job.id, job.magnet).await });
        }
        Ok(())
    }

    /// The downloads the last run left: unfinished ones come back paused, and finished ones
    /// seed again after a check of their files, while seeding is on.
    pub fn restore(&self) {
        let mut jobs = self.inner.records.load();
        for job in &mut jobs {
            job.hand_off.unpack.restored(&job.folder);
            if job.state.active() {
                job.state =
                    if job.state == TorrentState::Resolving { TorrentState::Failed } else { TorrentState::Paused };
                if job.state == TorrentState::Failed {
                    job.error = "The app stopped before the torrent's files were read: try again.".into();
                }
            }
        }
        let seed = self.sharing();
        {
            let mut state = self.state();
            for job in &jobs {
                state.jobs.insert(job.id.clone(), job.clone());
            }
        }
        for job in jobs {
            self.inner.records.save(&job);
            if seed && job.state == TorrentState::Complete && job.error.is_empty() {
                let this = self.clone();
                tokio::spawn(async move { this.start(&job.id, TorrentStart::SeedOnly).await });
            }
        }
    }

    /// Stops every torrent, so each writes down the pieces it has. The downloads stay as they
    /// are, so the next launch starts them again.
    pub async fn shutdown(&self) {
        if let Err(error) = self.port().stop_all().await {
            tracing::warn!(%error, "the torrents didn't stop");
        }
    }

    /// Reads the torrent's files again, for a download the VPN stopped before it had them.
    async fn read_again(&self, id: &str) {
        let Some(magnet) = self.job(id).map(|j| j.magnet) else { return };
        self.update(id, |job| {
            job.state = TorrentState::Resolving;
            job.error.clear();
        });
        if !self.vpn_gate(id).await {
            return;
        }
        let this = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { this.resolve(id, magnet).await });
    }

    async fn resolve(&self, id: String, magnet: String) {
        match self.port().resolve(&magnet).await {
            Ok(info) => self.update(&id, |job| {
                if job.state != TorrentState::Resolving {
                    return;
                }
                job.total = info.files.iter().map(|f| f.size).sum();
                job.file_count = info.files.len();
                job.files = info.files.into_iter().take(SHOWN_FILES).collect();
                job.state = TorrentState::Ready;
            }),
            Err(e) => self.update(&id, |job| {
                if job.state == TorrentState::Resolving {
                    job.state = TorrentState::Failed;
                    job.error = format!("The torrent's files couldn't be read: {e}");
                }
            }),
        };
    }

    async fn start(&self, id: &str, how: TorrentStart) {
        if !self.vpn_gate(id).await {
            return;
        }
        let Some(job) = self.job(id) else { return };
        let seeding = how == TorrentStart::SeedOnly;
        self.update(id, |job| {
            job.error.clear();
            job.start = Some(how);
            if seeding {
                job.verifying = true;
            } else {
                job.state = TorrentState::Checking;
            }
        });
        if let Err(e) = tokio::fs::create_dir_all(&job.folder).await {
            self.update(id, |job| {
                job.state = TorrentState::Failed;
                job.verifying = false;
                job.error = format!("The download folder couldn't be made: {e}");
            });
            return;
        }
        // After the folder is made: a folder that isn't there has no disk to ask.
        let needed = job.total.saturating_sub(job.done);
        if !seeding
            && let Some(free) = self.inner.ports.hardware.free_bytes(&job.folder)
            && let Some(error) = no_room(needed, free, "download folder")
        {
            self.update(id, |job| {
                job.state = TorrentState::Failed;
                job.error = error;
            });
            return;
        }
        if let Err(e) = self.port().start(id, &job.magnet, &job.folder, seeding).await {
            self.update(id, |job| {
                job.verifying = false;
                if !seeding {
                    job.state = TorrentState::Failed;
                }
                job.error = e.to_string();
            });
            return;
        }
        self.watch();
    }

    /// Reads the running downloads' state every torrent poll while any runs, and checks the
    /// VPN every VPN poll while one runs or waits for it, or a plugin waits for it, with the
    /// setting on.
    pub fn watch(&self) {
        if std::mem::replace(&mut self.state().watching, true) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            let clock = this.inner.ports.clock.clone();
            let polls = this.inner.polls;
            let mut checked = clock.now();
            loop {
                let mut running = this.poll().await;
                let waiting = this.any_waiting_vpn() || this.plugin_waits_for_vpn();
                if (waiting || (running && this.vpn_only()))
                    && clock.now().saturating_duration_since(checked) >= polls.vpn
                {
                    this.check_vpn().await;
                    checked = clock.now();
                    // What it started is read on the next round.
                    running = true;
                }
                if !running && !waiting {
                    break;
                }
                clock.sleep(polls.torrents).await;
            }
            this.state().watching = false;
        });
    }

    /// Reads each running download once, runs the phase rules on what the engine says, and
    /// carries out their actions in order. `false` when none runs.
    async fn poll(&self) -> bool {
        let ids: Vec<String> = self.state().jobs.values().filter(|j| j.running()).map(|j| j.id.clone()).collect();
        let sharing = self.sharing();
        let mut running = false;
        for id in ids {
            let Some(stats) = self.port().stats(&id) else { continue };
            running = true;
            let Some((actions, title)) = self.update(&id, |job| {
                job.download_rate = stats.download_rate;
                job.upload_rate = stats.upload_rate;
                job.peers = stats.peers;
                let start = if job.verifying {
                    TorrentStart::SeedOnly
                } else {
                    job.start.unwrap_or(TorrentStart::Download { seed: sharing })
                };
                let (phase, actions) = phase::next(&job.phase(), &stats, start, sharing);
                job.set_phase(phase);
                if actions.contains(&Action::Finished) {
                    job.hand_off.unpack = crate::archives::Unpack::of(&job.folder);
                }
                (actions, job.title.clone())
            }) else {
                continue;
            };
            for action in actions {
                match action {
                    Action::Unpause => {
                        if let Err(e) = self.port().unpause(&id).await {
                            tracing::warn!(error = %e, "a torrent didn't connect to seed");
                        }
                    }
                    Action::Stop => {
                        let _ = self.port().stop(&id).await;
                        self.update(&id, |job| job.seeding = false);
                    }
                    Action::Finished => self.emit(TorrentEvent::Finished { id: id.clone(), title: title.clone() }),
                    Action::Stopped => self.emit(TorrentEvent::Stopped { title: title.clone() }),
                }
            }
        }
        running
    }

    /// Runs the player's action on a download.
    async fn act(&self, id: &str, action: TorrentAction) -> CoreResult<()> {
        let job = self.job(id).ok_or_else(|| CoreError::Invalid(format!("no download {id}")))?;
        let unpacking = job.hand_off.unpack.phase == crate::archives::UnpackPhase::Unpacking;
        match action {
            TorrentAction::Start
                if matches!(
                    job.state,
                    TorrentState::Ready | TorrentState::Paused | TorrentState::Failed | TorrentState::Cancelled
                ) =>
            {
                // The VPN stopped it before it had its files: it reads them first.
                if job.state == TorrentState::Paused && job.file_count == 0 {
                    self.read_again(id).await;
                } else {
                    self.start(id, TorrentStart::Download { seed: self.sharing() }).await;
                }
            }
            TorrentAction::Pause if matches!(job.state, TorrentState::Checking | TorrentState::Downloading) => {
                self.port().stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update(id, |job| {
                    job.state = TorrentState::Paused;
                    job.download_rate = 0;
                    job.peers = 0;
                });
            }
            // While its archives unpack, Cancel stops the unpacking.
            TorrentAction::Cancel if unpacking => {
                if let Some(cancel) = &job.hand_off.unpack.cancel {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            TorrentAction::Cancel if !matches!(job.state, TorrentState::Complete | TorrentState::Cancelled) => {
                self.port().stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update(id, |job| {
                    job.state = TorrentState::Cancelled;
                    job.waiting_vpn = false;
                    job.download_rate = 0;
                    job.peers = 0;
                });
            }
            TorrentAction::Remove if !job.state.active() && !job.seeding => {
                self.port().stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.forget(id);
            }
            // The core refuses it first while the download's install runs.
            TorrentAction::DeleteFiles if !job.state.active() && !job.seeding && !unpacking => {
                self.port().stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.state().jobs.remove(id);
                self.inner.records.forget(id);
                // Only the folder Nexus made for this download (`torrent-<id>`).
                let own = job.folder.file_name().is_some_and(|name| name == format!("torrent-{id}").as_str());
                if own
                    && let Err(e) = tokio::fs::remove_dir_all(&job.folder).await
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    self.emit(TorrentEvent::Problem(format!("Couldn't delete {}: {e}", job.folder.display())));
                }
                self.changed();
            }
            TorrentAction::StopSeeding if job.seeding || job.verifying => {
                self.port().stop(id).await.map_err(|e| CoreError::Io(e.to_string()))?;
                self.update(id, |job| {
                    job.seeding = false;
                    job.verifying = false;
                    job.upload_rate = 0;
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Forgets a download: it leaves the list and the config.
    fn forget(&self, id: &str) {
        self.state().jobs.remove(id);
        self.inner.records.forget(id);
        self.changed();
    }

    /// "Keep sharing finished downloads" turned off: every download stops seeding.
    async fn stop_all_seeding(&self) {
        let seeding: Vec<String> =
            self.state().jobs.values().filter(|j| j.seeding || j.verifying).map(|j| j.id.clone()).collect();
        for id in seeding {
            let _ = self.act(&id, TorrentAction::StopSeeding).await;
        }
    }
}
