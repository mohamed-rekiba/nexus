//! The game installs subsystem: a plugin's [`GameInstaller`] decodes a package in the
//! background, and this owns the job. It lists the job in Downloads, passes the player's cancel
//! on, says when the game is in, and removes the work folder a crash left behind.
//!
//! It also runs the hand-off from a finished download ([`unpacking`]): it unpacks the
//! download's archives through the [`Archives`] port, and installs from the unpacked copy,
//! which may move into the game folder rather than be copied.
//!
//! It follows the subsystem pattern of [`crate::torrents`]:
//!
//! - Its jobs sit behind its own lock, and it saves the work folder of each running install
//!   ([`records`], `app.game_installs`).
//! - It takes only the ports it needs ([`InstallPorts`]). The downloads come in through
//!   [`TorrentDownloads`]: it reads them, and writes their hand-off, the fields of a download
//!   that are the installs' to change ([`HandOff`]). They stay in the download's record, so
//!   the saved format is the same.
//! - The core resolves what needs the plugins first (the installer, the plugin's folders, the
//!   game folder) and hands it plain data ([`NewInstall`]).
//! - It publishes changes of its jobs as [`Area::Installs`]; the hand-off changes publish as
//!   the downloads' own.
//! - What the core must do goes out as events ([`InstallEvent`]), which the subsystem awaits:
//!   the library lists a new game before its job shows as done, and the player is told of a
//!   failure before the job shows it.
//!
//! Locks: it never calls out while it holds its lock, except to save a record.

mod records;
#[cfg(test)]
mod tests;
mod unpacking;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use nexus_api::{GameInstaller, HostPath, InstallJob, InstalledGame, JobId, PluginError, PluginId, PluginResult};
use serde::{Deserialize, Serialize};

pub(crate) use self::records::Records;
use crate::archives::UnpackPhase;
use crate::changes::{Area, Notifier};
use crate::folders::HostFolders;
use crate::torrents::{HandOff, TorrentJob, TorrentState};
use crate::{Archives, Clock, CoreError, CoreResult, Hardware};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GamePhase {
    Installing,
    Done,
    Failed,
    Cancelled,
}

impl GamePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GameJob {
    pub plugin: PluginId,
    pub source: HostPath,
    /// The plugin's work folder, once `begin` named it.
    pub work: Option<HostPath>,
    pub label: String,
    pub done: u64,
    pub total: u64,
    pub phase: GamePhase,
    pub error: Option<String>,
    pub cancel: Arc<AtomicBool>,
    /// The game the install made, once it is done.
    pub installed: Option<InstalledGame>,
}

/// What the player asks of game installs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GameInstallCmd {
    /// Installs a finished download into the game folder the player picked on the install
    /// sheet (`~/` for the home folder). `None`: the last one picked for the console, or else
    /// the console's folder in the first game folder. The folder must be inside a game folder.
    /// The download's archives unpack first.
    Download { id: String, folder: Option<PathBuf> },
    /// Installs a package the plugin listed, with its game installer: the slot action
    /// [`nexus_api::HOST_INSTALL_GAME`].
    Package { plugin: PluginId, source: HostPath },
    /// Deletes the download's unpacked archives; the torrent's files stay.
    RemoveUnpacked { id: String },
    /// Stops the install that runs from download `id`'s files; what it wrote goes.
    Cancel { id: String },
}

/// An install to start, with everything the core resolved for it.
pub(crate) struct NewInstall {
    /// From the Downloads list's numbers, which the emulator downloads share.
    pub id: JobId,
    pub plugin: PluginId,
    pub source: HostPath,
    /// The game folder the player picked; `None` leaves the choice to the plugin.
    pub into: Option<HostPath>,
    pub installer: Arc<dyn GameInstaller>,
    /// The player's folders as the plugin may use them.
    pub folders: HostFolders,
}

/// The installs' state, read whole. The jobs are shared, not copied.
#[derive(Clone, Debug, Default)]
pub(crate) struct InstallsSnapshot {
    pub jobs: Arc<BTreeMap<JobId, GameJob>>,
}

impl InstallsSnapshot {
    /// The newest install from `source` with `plugin`'s installer.
    pub fn latest(&self, plugin: &PluginId, source: &HostPath) -> Option<&GameJob> {
        self.jobs.values().rev().find(|g| &g.plugin == plugin && &g.source == source)
    }

    /// One of `plugin`'s installs runs.
    pub fn installing(&self, plugin: &PluginId) -> bool {
        self.jobs.values().any(|g| &g.plugin == plugin && g.phase == GamePhase::Installing)
    }
}

/// What the core must do about the installs. The subsystem waits for it.
pub(crate) enum InstallEvent {
    /// A game is in: the library lists it.
    Installed,
    /// An install failed: the player is told.
    Failed(String),
    /// A download's archives unpacked after the player pressed Install: the install starts.
    Unpacked {
        plugin: PluginId,
        source: HostPath,
        into: Option<HostPath>,
        /// The download's, for the notice when it can't start.
        title: String,
    },
}

/// Where the subsystem's events go: the future ends once the core is done with one.
pub(crate) type EventSink = Arc<dyn Fn(InstallEvent) -> BoxFuture<'static, ()> + Send + Sync>;

/// The catalogs' downloads, as the installs use them: read whole, and their hand-off
/// written.
pub(crate) trait TorrentDownloads: Send + Sync {
    fn job(&self, id: &str) -> Option<TorrentJob>;
    fn jobs(&self) -> Vec<TorrentJob>;
    /// Changes download `id`'s hand-off, saves it and publishes the change. `false` when there
    /// is no such download.
    fn hand_off(&self, id: &str, change: &mut dyn FnMut(&TorrentState, &mut HandOff)) -> bool;
}

impl TorrentDownloads for crate::torrents::TorrentsSubsystem {
    fn job(&self, id: &str) -> Option<TorrentJob> {
        crate::torrents::TorrentsSubsystem::job(self, id)
    }

    fn jobs(&self) -> Vec<TorrentJob> {
        self.snapshot().jobs.into_values().collect()
    }

    fn hand_off(&self, id: &str, change: &mut dyn FnMut(&TorrentState, &mut HandOff)) -> bool {
        crate::torrents::TorrentsSubsystem::hand_off(self, id, change).is_some()
    }
}

/// The ports the subsystem uses, and nothing else.
pub(crate) struct InstallPorts {
    pub archives: Arc<dyn Archives>,
    /// For the free space an unpack has.
    pub hardware: Arc<dyn Hardware>,
    pub clock: Arc<dyn Clock>,
    pub downloads: Arc<dyn TorrentDownloads>,
    /// The longest a plugin's `begin` may take.
    pub call_timeout: Duration,
}

#[derive(Default)]
struct State {
    jobs: Arc<BTreeMap<JobId, GameJob>>,
}

impl State {
    fn jobs(&mut self) -> &mut BTreeMap<JobId, GameJob> {
        Arc::make_mut(&mut self.jobs)
    }
}

struct Shared {
    ports: InstallPorts,
    records: Records,
    notifier: Notifier,
    events: EventSink,
    state: Mutex<State>,
}

/// The game installs. Cheap to clone; every clone is the same subsystem.
#[derive(Clone)]
pub(crate) struct GameInstallsSubsystem {
    inner: Arc<Shared>,
}

/// The handle the plugin reports through. The subsystem keeps no reference to it, so if the
/// plugin's work drops it without finishing (a panic), the job fails here.
struct HostJob {
    installs: Weak<Shared>,
    id: JobId,
    cancel: Arc<AtomicBool>,
    ended: AtomicBool,
    runtime: tokio::runtime::Handle,
    /// The game folder the player picked.
    into: Option<HostPath>,
    /// The latest progress, bytes done and in all: the plugin's thread never waits for the
    /// subsystem's state, which a screen update holds.
    latest: Arc<(AtomicU64, AtomicU64)>,
    /// A task is on its way to put `latest` in the state.
    reporting: Arc<AtomicBool>,
    /// When the last report went to the state, by `clock`.
    reported: Mutex<Option<Instant>>,
    clock: Arc<dyn Clock>,
    folders: Arc<HostFolders>,
    /// The source it installs from.
    source: HostPath,
    /// The folder in the source the host made itself and may give away: a download's unpacked
    /// archives. `take` moves only from inside it.
    disposable: Option<HostPath>,
}

/// Whether `path` is `folder` or inside it, in the same root.
fn inside(path: &HostPath, folder: &HostPath) -> bool {
    let (p, f) = (path.path.as_str().trim_end_matches('/'), folder.path.as_str().trim_end_matches('/'));
    path.folder == folder.folder && path.root == folder.root && (p == f || p.starts_with(&format!("{f}/")))
}

/// How often an install's progress reaches the state, and so the screen.
const REPORT_EVERY: Duration = Duration::from_millis(200);

impl HostJob {
    fn end(&self, result: PluginResult<InstalledGame>) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(inner) = self.installs.upgrade() else { return };
        let (id, folders) = (self.id, self.folders.clone());
        self.runtime.spawn(async move { GameInstallsSubsystem { inner }.ended(id, result, &folders).await });
    }
}

impl InstallJob for HostJob {
    /// Keeps the numbers, and at most every [`REPORT_EVERY`], or at the end, has a task put
    /// them in the state: a plugin reports every few MiB, and each report redraws the screen.
    fn progress(&self, done: u64, total: u64) {
        self.latest.0.store(done, Ordering::Release);
        self.latest.1.store(total, Ordering::Release);
        {
            let mut reported = self.reported.lock().unwrap_or_else(|p| p.into_inner());
            let due = done >= total
                || reported.is_none_or(|at| self.clock.now().saturating_duration_since(at) >= REPORT_EVERY);
            if !due || self.reporting.swap(true, Ordering::AcqRel) {
                return;
            }
            *reported = Some(self.clock.now());
        }
        let Some(inner) = self.installs.upgrade() else { return };
        let (id, latest, reporting) = (self.id, self.latest.clone(), self.reporting.clone());
        self.runtime.spawn(async move {
            let installs = GameInstallsSubsystem { inner };
            if let Some(job) = installs.state().jobs().get_mut(&id) {
                (job.done, job.total) = (latest.0.load(Ordering::Acquire), latest.1.load(Ordering::Acquire));
            }
            reporting.store(false, Ordering::Release);
            installs.changed();
        });
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn finished(&self, result: PluginResult<InstalledGame>) {
        self.end(result);
    }

    fn games_folder(&self) -> Option<HostPath> {
        self.into.clone()
    }

    /// Moves `from` when it is in the host's own copy (`disposable`) and the disk allows. Then
    /// the empty folders it leaves go, and a download whose unpacked copy is gone needs
    /// unpacking again.
    fn take(&self, from: &HostPath, to: &HostPath) -> bool {
        let Some(disposable) = self.disposable.as_ref().filter(|d| inside(from, d)) else { return false };
        let Some(inner) = self.installs.upgrade() else { return false };
        let folders = &self.folders;
        match folders.take(from, to) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                tracing::info!(error = %e, "the unpacked game couldn't move: copying it");
                return false;
            }
        }
        // The folders the move emptied, up to the unpacked copy itself.
        let mut left = folders.resolve(from).ok().and_then(|p| p.parent().map(std::path::Path::to_path_buf));
        let top = folders.resolve(disposable).ok();
        while let Some(dir) = left {
            let last = top.as_deref() == Some(dir.as_path());
            if std::fs::remove_dir(&dir).is_err() || last {
                break;
            }
            left = dir.parent().map(std::path::Path::to_path_buf);
        }
        if top.is_some_and(|t| !t.exists()) {
            let downloads = &inner.ports.downloads;
            for id in downloads.jobs().iter().filter(|j| j.source == self.source).map(|j| j.id.clone()) {
                downloads.hand_off(&id, &mut |_, h| {
                    h.unpack.phase = UnpackPhase::Waiting;
                    h.unpack.progress = 0.0;
                });
            }
        }
        true
    }
}

impl Drop for HostJob {
    fn drop(&mut self) {
        self.end(Err(PluginError::Other("the install stopped before it finished".into())));
    }
}

impl GameInstallsSubsystem {
    pub fn new(ports: InstallPorts, records: Records, notifier: Notifier, events: EventSink) -> Self {
        let state = Mutex::new(State::default());
        Self { inner: Arc::new(Shared { ports, records, notifier, events, state }) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn changed(&self) {
        self.inner.notifier.changed(Area::Installs);
    }

    async fn emit(&self, event: InstallEvent) {
        (self.inner.events)(event).await;
    }

    fn downloads(&self) -> &Arc<dyn TorrentDownloads> {
        &self.inner.ports.downloads
    }

    /// Changes download `id`'s hand-off. `false` when there is no such download.
    fn hand_off(&self, id: &str, mut change: impl FnMut(&TorrentState, &mut HandOff)) -> bool {
        self.downloads().hand_off(id, &mut change)
    }

    /// The installs, as they are now.
    pub fn snapshot(&self) -> InstallsSnapshot {
        InstallsSnapshot { jobs: self.state().jobs.clone() }
    }

    /// Runs a command that is the subsystem's alone. The core resolves
    /// [`GameInstallCmd::Download`] and [`GameInstallCmd::Package`] first: they need the
    /// plugins and the game folders.
    pub async fn handle(&self, command: GameInstallCmd) -> CoreResult<()> {
        match command {
            GameInstallCmd::RemoveUnpacked { id } => {
                self.remove_unpacked(&id).await;
                Ok(())
            }
            GameInstallCmd::Cancel { id } => {
                if let Some(install) = self.running_from(&id) {
                    self.cancel(install);
                }
                Ok(())
            }
            GameInstallCmd::Download { .. } | GameInstallCmd::Package { .. } => {
                Err(CoreError::Invalid("the core resolves installs first".into()))
            }
        }
    }

    /// The install that runs from download `id`'s files, if one does.
    pub fn running_from(&self, id: &str) -> Option<JobId> {
        let job = self.downloads().job(id)?;
        self.state()
            .jobs
            .iter()
            .find(|(_, g)| g.plugin == job.emulator && g.source == job.source && g.phase == GamePhase::Installing)
            .map(|(id, _)| *id)
    }

    /// Keeps why the last Install of download `id` didn't start, for the install sheet; empty
    /// clears it.
    pub fn set_install_error(&self, id: &str, error: String) {
        self.hand_off(id, |_, h| h.install_error.clone_from(&error));
    }

    /// The player pressed Install on finished download `id`, into the game folder `into`.
    /// Archives that wait unpack first, and the install starts after
    /// ([`InstallEvent::Unpacked`]). `Some` when there is nothing to unpack: the core starts
    /// the install from that source with that plugin now.
    pub fn install_download(&self, id: &str, into: HostPath) -> Option<(PluginId, HostPath)> {
        let job = self.downloads().job(id)?;
        self.hand_off(id, |_, h| h.install_into = Some(into.clone()));
        match first_step(job.hand_off.unpack.phase) {
            Step::Install => Some((job.emulator, job.source)),
            Step::Unpack => {
                self.unpack(id, true);
                None
            }
            Step::Nothing => None,
        }
    }

    /// Starts an install the core resolved. It fails at once when the plugin refuses the
    /// source; the rest runs in the background.
    pub async fn begin(&self, new: NewInstall) -> CoreResult<()> {
        let NewInstall { id, plugin, source, into, installer, folders } = new;
        let running = self
            .state()
            .jobs
            .values()
            .any(|j| j.plugin == plugin && j.source == source && j.phase == GamePhase::Installing);
        if running {
            return Err(CoreError::Invalid(format!("{} is already installing", source.path)));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let file = source.path.as_str().rsplit('/').next().unwrap_or_default().to_owned();
        self.state().jobs().insert(
            id,
            GameJob {
                plugin: plugin.clone(),
                source: source.clone(),
                work: None,
                label: format!("Install {file}"),
                done: 0,
                total: 0,
                phase: GamePhase::Installing,
                error: None,
                cancel: cancel.clone(),
                installed: None,
            },
        );
        // A download's own unpacked copy may move into the games folder rather than be copied.
        let disposable = self
            .downloads()
            .jobs()
            .iter()
            .any(|t| t.source == source && t.hand_off.unpack.phase == UnpackPhase::Unpacked)
            .then(|| source.join(crate::archives::UNPACKED).ok())
            .flatten();
        let folders = Arc::new(folders);
        let clock = self.inner.ports.clock.clone();
        let job = Arc::new(HostJob {
            installs: Arc::downgrade(&self.inner),
            id,
            cancel,
            ended: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
            into,
            latest: Arc::default(),
            reporting: Arc::default(),
            reported: Mutex::new(None),
            clock: clock.clone(),
            folders: folders.clone(),
            source: source.clone(),
            disposable,
        });
        let timeout = self.inner.ports.call_timeout;
        let call = installer.begin(&source, job);
        let begun = crate::guard::guarded(clock.as_ref(), &plugin, "game_installer.begin", timeout, call)
            .await
            .map_err(|error| CoreError::Plugin { plugin: plugin.clone(), error });
        let work = match begun {
            Ok(work) => work,
            Err(e) => {
                self.state().jobs().remove(&id);
                self.changed();
                return Err(e);
            }
        };
        // Under the lock, so the job's end can't run between the check and the save.
        let ended_already = {
            let mut state = self.state();
            match state.jobs().get_mut(&id) {
                Some(job) if job.phase == GamePhase::Installing => {
                    job.work = Some(work.clone());
                    self.inner.records.save(id, &plugin, &work);
                    None
                }
                Some(job) if job.phase != GamePhase::Done => Some(true),
                _ => Some(false),
            }
        };
        if ended_already == Some(true) {
            remove_work(&folders, &work).await;
        }
        self.changed();
        Ok(())
    }

    /// The player cancelled a game install. `false` when `id` isn't one.
    pub fn cancel(&self, id: JobId) -> bool {
        match self.state().jobs.get(&id) {
            Some(job) => {
                job.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    async fn ended(&self, id: JobId, result: PluginResult<InstalledGame>, folders: &HostFolders) {
        // The library lists the game before the job shows as done.
        if let Ok(game) = &result {
            tracing::info!(title = %game.title, "game installed");
            self.emit(InstallEvent::Installed).await;
        }
        let (phase, error) = match &result {
            Ok(_) => (GamePhase::Done, None),
            Err(_) if self.state().jobs.get(&id).is_some_and(|j| j.cancel.load(Ordering::SeqCst)) => {
                (GamePhase::Cancelled, None)
            }
            Err(e) => (GamePhase::Failed, Some(e.to_string())),
        };
        // The player hears of a failure before the job shows it.
        if let Some(error) = &error {
            self.emit(InstallEvent::Failed(error.clone())).await;
        }
        // The job shows its end only once its work folder is gone. `work` is unset while
        // `begin` runs; if `begin` returns before the job shows its end, the next round cleans
        // up its folder, else `begin` does.
        let mut cleaned = None;
        loop {
            let work = {
                let mut state = self.state();
                let Some(job) = state.jobs().get_mut(&id) else { return };
                if job.work == cleaned {
                    job.phase = phase;
                    if phase == GamePhase::Done {
                        job.done = job.total.max(job.done);
                    }
                    job.error.clone_from(&error);
                    job.installed = result.as_ref().ok().cloned();
                    break;
                }
                job.work.clone()
            };
            if let Some(work) = &work {
                self.inner.records.forget(id);
                if result.is_err() {
                    // The plugin removes its work folder on an error, unless it broke on the way.
                    remove_work(folders, work).await;
                }
            }
            cleaned = work;
        }
        self.changed();
    }

    /// Removes the work folders of installs that were running when the app last stopped.
    /// `folders` gives a plugin's folders, to find them on disk.
    pub fn clean_up(&self, folders: impl Fn(&PluginId) -> Option<HostFolders>) {
        for (plugin, work) in self.inner.records.take_all() {
            let Some(folders) = folders(&plugin) else { continue };
            match folders.resolve(&work) {
                Ok(path) if path.exists() => {
                    tracing::info!(path = %path.display(), "removing an install the app stopped in the middle of");
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        tracing::warn!(error = %e, "could not remove it");
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "an interrupted install's folder is out of reach"),
            }
        }
    }
}

/// Removes an install's work folder, checked against the plugin's grants.
async fn remove_work(folders: &HostFolders, work: &HostPath) {
    match folders.resolve(work) {
        Ok(path) if path.exists() => {
            if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), error = %e, "could not remove an install's work folder");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "an install's work folder is out of reach"),
    }
}

/// What Install does first with a finished download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Nothing to unpack, or it is unpacked: the install starts.
    Install,
    /// The archives unpack, then the install starts.
    Unpack,
    /// The archives are unpacking already: nothing starts.
    Nothing,
}

/// The first step of Install for a download whose archives are in `phase`.
fn first_step(phase: UnpackPhase) -> Step {
    match phase {
        UnpackPhase::None | UnpackPhase::Unpacked => Step::Install,
        UnpackPhase::Waiting | UnpackPhase::Failed | UnpackPhase::Cancelled => Step::Unpack,
        UnpackPhase::Unpacking => Step::Nothing,
    }
}
