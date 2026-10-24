//! Game installs: a plugin's [`GameInstaller`] decodes a package in the background; the core
//! owns the job. It lists the job in Downloads, passes the player's cancel on, refreshes the
//! library at the end, and removes the work folder a crash left behind.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use nexus_api::{Capability, HostPath, InstallJob, InstalledGame, JobId, PluginError, PluginId, PluginResult};
use serde_json::Value;

use crate::app::{Core, Inner};
use crate::error::require;
use crate::{CoreError, CoreResult};

/// Where a running install's work folder is saved, by job id, so the next start can remove it.
const RECORDS: [&str; 2] = ["app", "game_installs"];

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

/// The handle the plugin reports through. The core keeps no reference to it, so if the
/// plugin's work drops it without finishing (a panic), the job fails here.
struct HostJob {
    core: Weak<Inner>,
    id: JobId,
    cancel: Arc<AtomicBool>,
    ended: AtomicBool,
    runtime: tokio::runtime::Handle,
    /// The game folder the player picked.
    into: Option<HostPath>,
    /// The latest progress, bytes done and in all: the plugin's thread never waits for the
    /// core's state, which a screen update holds.
    latest: Arc<(AtomicU64, AtomicU64)>,
    /// A task is on its way to put `latest` in the state.
    reporting: Arc<AtomicBool>,
    /// When the last report went to the state.
    reported: Mutex<Option<Instant>>,
    /// The plugin, and the source it installs from.
    plugin: PluginId,
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
        let Some(inner) = self.core.upgrade() else { return };
        let id = self.id;
        self.runtime.spawn(async move { Core { inner }.game_install_ended(id, result).await });
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
            let due = done >= total || reported.is_none_or(|at| at.elapsed() >= REPORT_EVERY);
            if !due || self.reporting.swap(true, Ordering::AcqRel) {
                return;
            }
            *reported = Some(Instant::now());
        }
        let Some(inner) = self.core.upgrade() else { return };
        let (id, latest, reporting) = (self.id, self.latest.clone(), self.reporting.clone());
        self.runtime.spawn(async move {
            let core = Core { inner };
            if let Some(job) = core.state().game_installs.get_mut(&id) {
                (job.done, job.total) = (latest.0.load(Ordering::Acquire), latest.1.load(Ordering::Acquire));
            }
            reporting.store(false, Ordering::Release);
            core.changed();
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
        let Some(inner) = self.core.upgrade() else { return false };
        let core = Core { inner };
        let Some(folders) = crate::app::plugin_folders(core.kernel(), &self.plugin) else { return false };
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
            let mut state = core.state();
            for job in state.torrents.values_mut().filter(|j| j.source == self.source) {
                job.unpack.phase = crate::archives::UnpackPhase::Waiting;
                job.unpack.progress = 0.0;
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

impl Core {
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
        let installer = require(plugin, Capability::GameInstaller, provided.game_installer)?;
        let running = self
            .state()
            .game_installs
            .values()
            .any(|j| &j.plugin == plugin && j.source == source && j.phase == GamePhase::Installing);
        if running {
            return Err(CoreError::Invalid(format!("{} is already installing", source.path)));
        }
        let id = self.inner.downloads.next_id();
        let cancel = Arc::new(AtomicBool::new(false));
        let file = source.path.as_str().rsplit('/').next().unwrap_or_default().to_owned();
        self.state().game_installs.insert(
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
            .state()
            .torrents
            .values()
            .any(|t| t.source == source && t.unpack.phase == crate::archives::UnpackPhase::Unpacked)
            .then(|| source.join(crate::archives::UNPACKED).ok())
            .flatten();
        let job = Arc::new(HostJob {
            core: Arc::downgrade(&self.inner),
            id,
            cancel,
            ended: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
            into,
            latest: Arc::default(),
            reporting: Arc::default(),
            reported: Mutex::new(None),
            plugin: plugin.clone(),
            source: source.clone(),
            disposable,
        });
        let begun = kernel.guard(plugin, "game_installer.begin", installer.begin(&source, job)).await;
        let work = match begun {
            Ok(work) => work,
            Err(e) => {
                self.state().game_installs.remove(&id);
                self.changed();
                return Err(e);
            }
        };
        // Under the state lock, so the job's end can't run between the check and the save.
        let ended_already = {
            let mut state = self.state();
            match state.game_installs.get_mut(&id) {
                Some(job) if job.phase == GamePhase::Installing => {
                    job.work = Some(work.clone());
                    let record = serde_json::json!({ "plugin": plugin, "work": work });
                    if let Err(e) = kernel.config().set(&[RECORDS[0], RECORDS[1], &id.0.to_string()], record) {
                        tracing::warn!(error = %e, "could not save the install's record");
                    }
                    None
                }
                Some(job) if job.phase != GamePhase::Done => Some(true),
                _ => Some(false),
            }
        };
        if ended_already == Some(true) {
            self.remove_work(plugin, &work).await;
        }
        self.changed();
        Ok(())
    }

    /// The player cancelled a game install. `false` when `id` isn't one.
    pub(crate) fn cancel_game_install(&self, id: JobId) -> bool {
        match self.state().game_installs.get(&id) {
            Some(job) => {
                job.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    async fn game_install_ended(&self, id: JobId, result: PluginResult<InstalledGame>) {
        // The library lists the game before the job shows as done.
        if let Ok(game) = &result {
            tracing::info!(title = %game.title, "game installed");
            if let Err(e) = Box::pin(self.dispatch(crate::Command::RefreshLibrary)).await {
                tracing::warn!(error = %e, "library refresh after an install failed");
            }
        }
        let (phase, error) = match &result {
            Ok(_) => (GamePhase::Done, None),
            Err(_) if self.state().game_installs.get(&id).is_some_and(|j| j.cancel.load(Ordering::SeqCst)) => {
                (GamePhase::Cancelled, None)
            }
            Err(e) => (GamePhase::Failed, Some(e.to_string())),
        };
        // The job shows its end only once its work folder is gone, and a failed job shows it
        // with its notice. `work` is unset while `begin` runs; if `begin` returns before the
        // job shows its end, the next round cleans up its folder, else `install_game` does.
        let mut cleaned = None;
        loop {
            let (work, plugin) = {
                let mut state = self.state();
                let Some(job) = state.game_installs.get_mut(&id) else { return };
                if job.work == cleaned {
                    job.phase = phase;
                    if phase == GamePhase::Done {
                        job.done = job.total.max(job.done);
                    }
                    job.error.clone_from(&error);
                    job.installed = result.as_ref().ok().cloned();
                    if let Some(error) = error {
                        state.post_notice(
                            nexus_ui_api::NoticeKind::Error,
                            format!("The install failed: {error}"),
                            Some(crate::Route::Downloads),
                            None,
                        );
                    }
                    break;
                }
                (job.work.clone(), job.plugin.clone())
            };
            if let Some(work) = &work {
                self.forget_record(id);
                if result.is_err() {
                    // The plugin removes its work folder on an error, unless it broke on the way.
                    self.remove_work(&plugin, work).await;
                }
            }
            cleaned = work;
        }
        self.changed();
    }

    /// Removes an install's work folder, checked against the plugin's grants.
    async fn remove_work(&self, plugin: &PluginId, work: &HostPath) {
        let kernel = self.kernel();
        let Ok(manifest) = kernel.manifest(plugin) else { return };
        match crate::folders::resolve(
            kernel.config(),
            kernel.host().home.as_deref(),
            kernel.host().platform,
            &manifest.permissions,
            work,
        ) {
            Ok(path) if path.exists() => {
                if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                    tracing::warn!(path = %path.display(), error = %e, "could not remove an install's work folder");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "an install's work folder is out of reach"),
        }
    }

    fn forget_record(&self, id: JobId) {
        let config = self.kernel().config();
        let mut records = config.get(&RECORDS);
        if let Some(map) = records.as_object_mut() {
            map.remove(&id.0.to_string());
            let _ = config.set(&RECORDS, records);
        }
    }

    /// Removes the work folders of installs that were running when the app last stopped.
    pub(crate) fn clean_up_game_installs(&self) {
        let kernel = self.kernel();
        let records = kernel.config().get(&RECORDS);
        let Some(map) = records.as_object() else { return };
        for record in map.values() {
            let plugin = record.get("plugin").and_then(Value::as_str).and_then(|p| PluginId::new(p).ok());
            let work = record.get("work").and_then(|w| serde_json::from_value::<HostPath>(w.clone()).ok());
            let (Some(plugin), Some(work)) = (plugin, work) else { continue };
            let Ok(manifest) = kernel.manifest(&plugin) else { continue };
            let host = kernel.host();
            match crate::folders::resolve(
                kernel.config(),
                host.home.as_deref(),
                host.platform,
                &manifest.permissions,
                &work,
            ) {
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
        let _ = kernel.config().set(&RECORDS, serde_json::json!({}));
    }
}
