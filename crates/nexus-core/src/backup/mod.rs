//! Settings → Backup: one file that holds the app's settings and what each plugin names of its
//! data ([`nexus_api::Backup`]): emulator settings, keys, firmware and saves. The file is
//! described in [`archive`].
//!
//! The subsystem writes and reads the files, and keeps the list of the backups in the backup
//! folder and the job in progress. It knows no plugin: the core asks the plugins for their
//! items and where they are on this computer ([`crate::backups`]), and hands it the paths.
//! Its changes are [`Area::Backup`].
//!
//! A restore's settings belong to other subsystems, which hold them while Nexus runs. So a
//! restore leaves them in [`PENDING`] in the data folder, and the next start puts them back
//! ([`apply_pending`]) before any subsystem starts: "Restart Nexus to finish".

pub(crate) mod archive;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

pub(crate) use archive::{ItemKey, Kind, Source};

use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::{CoreError, CoreResult};

/// In the data folder: the settings of the last restore, until the next start puts them back.
pub(crate) const PENDING: &str = "restored-settings.json";
/// Why a second job can't start.
pub(crate) const BUSY: &str = "A backup or a restore is already running";

/// A backup or a restore in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct BackupJob {
    pub restoring: bool,
    pub done: u64,
    pub total: u64,
}

/// One backup in the backup folder.
#[derive(Clone, Debug, PartialEq)]
pub struct BackupFile {
    /// Its file name in the backup folder.
    pub file: String,
    /// When it was made, in seconds since 1970.
    pub made: u64,
    pub bytes: u64,
    /// What it holds: its items' titles, each once.
    pub items: Vec<String>,
    /// The backup a restore made of what was there before.
    pub before_restore: bool,
}

/// The subsystem's state, as the view reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackupSnapshot {
    pub job: Option<BackupJob>,
    /// Newest first.
    pub backups: Arc<Vec<BackupFile>>,
    /// Why the last job failed. Empty when it didn't.
    pub error: String,
    /// What the last job did, such as "Backed up to nexus-backup-….tar.gz". Empty before one.
    pub done: String,
    /// A restore finished: its settings go back at the next start.
    pub restart: bool,
}

/// The free space on the disk of a path: the `Hardware` port's answer.
pub(crate) type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// What a restore needs besides the file.
pub(crate) struct RestorePlan {
    /// Where each item goes on this computer.
    pub targets: BTreeMap<ItemKey, PathBuf>,
    /// Every item here, for the backup of what is here first.
    pub current: Vec<Source>,
    /// Now, in seconds since 1970: that backup's time.
    pub now: u64,
    pub free: FreeSpace,
}

#[derive(Clone)]
pub struct BackupSubsystem {
    inner: Arc<Shared>,
}

struct Shared {
    state: Mutex<BackupSnapshot>,
    config: Arc<ConfigService>,
    data_dir: PathBuf,
    notifier: Notifier,
}

impl BackupSubsystem {
    pub(crate) fn new(config: Arc<ConfigService>, data_dir: PathBuf, notifier: Notifier) -> Self {
        Self { inner: Arc::new(Shared { state: Mutex::default(), config, data_dir, notifier }) }
    }

    fn state(&self) -> MutexGuard<'_, BackupSnapshot> {
        self.inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn changed(&self) {
        self.inner.notifier.changed(Area::Backup);
    }

    pub fn snapshot(&self) -> BackupSnapshot {
        self.state().clone()
    }

    /// A backup or a restore runs.
    pub fn busy(&self) -> bool {
        self.state().job.is_some()
    }

    /// Reads the backups in `folder` again. A file that isn't a backup this Nexus reads is
    /// left out.
    pub async fn list(&self, folder: PathBuf) {
        let backups = match tokio::task::spawn_blocking(move || list(&folder)).await {
            Ok(backups) => backups,
            Err(e) => {
                tracing::warn!(error = %e, "the backup folder wasn't read");
                Vec::new()
            }
        };
        self.state().backups = Arc::new(backups);
        self.changed();
    }

    /// Starts a job, or refuses when one runs.
    fn begin(&self, restoring: bool) -> CoreResult<()> {
        let mut state = self.state();
        if state.job.is_some() {
            return Err(CoreError::Invalid(BUSY.into()));
        }
        state.job = Some(BackupJob { restoring, done: 0, total: 0 });
        state.error.clear();
        state.done.clear();
        drop(state);
        self.changed();
        Ok(())
    }

    /// Ends the job: `done` says what it did, or the error why it failed.
    fn end(&self, outcome: &Result<String, String>) {
        let mut state = self.state();
        state.job = None;
        match outcome {
            Ok(done) => state.done.clone_from(done),
            Err(error) => state.error.clone_from(error),
        }
        drop(state);
        self.changed();
    }

    /// The job's bytes so far. The view hears of it each tenth of the way.
    fn progress(&self) -> impl Fn(u64, u64) + Send + Sync + 'static {
        let this = self.clone();
        move |done, total| {
            let step = (total / 10).max(1);
            let mut state = this.state();
            let Some(job) = state.job.as_mut() else { return };
            let crossed = done / step != job.done / step;
            (job.done, job.total) = (done, total);
            drop(state);
            if crossed {
                this.changed();
            }
        }
    }

    /// The backup of `sources` and the app's settings in `folder`, made at `made` (seconds
    /// since 1970). The file's path.
    pub(crate) async fn back_up(&self, folder: &Path, made: u64, sources: Vec<Source>) -> CoreResult<PathBuf> {
        self.begin(false)?;
        let written = self.write(folder, made, sources, Kind::Backup).await;
        let outcome = match &written {
            Ok(path) => Ok(format!("Backed up to {}", file_name(path))),
            Err(e) => Err(format!("The backup failed: {e}")),
        };
        self.end(&outcome);
        self.list(folder.to_path_buf()).await;
        written
    }

    async fn write(&self, folder: &Path, made: u64, sources: Vec<Source>, kind: Kind) -> CoreResult<PathBuf> {
        let config = archive::config_subset(&self.inner.config.get(&["app"]), &self.inner.config.get(&["plugins"]));
        let dest = folder.join(archive::file_name(made, kind));
        let progress = self.progress();
        let path = dest.clone();
        blocking(move || archive::write(&path, env!("CARGO_PKG_VERSION"), made, &config, &sources, &progress)).await?;
        Ok(dest)
    }

    /// Puts the backup `file` of `folder` back. First it backs up what is here of the items
    /// the backup has; nothing is restored when that fails. The settings wait in [`PENDING`]
    /// for the next start.
    pub(crate) async fn restore(&self, folder: &Path, file: &str, plan: RestorePlan) -> CoreResult<()> {
        self.begin(true)?;
        let outcome = self.restore_in_job(folder, file, plan).await;
        self.end(&outcome);
        if outcome.is_ok() {
            self.state().restart = true;
            self.changed();
        }
        self.list(folder.to_path_buf()).await;
        outcome.map(|_| ()).map_err(CoreError::Invalid)
    }

    async fn restore_in_job(&self, folder: &Path, file: &str, plan: RestorePlan) -> Result<String, String> {
        let path = folder.join(file);
        let read = path.clone();
        let manifest = blocking(move || archive::read_manifest(&read)).await.map_err(|e| format!("{file}: {e}"))?;
        let has: Vec<ItemKey> = manifest.items.iter().map(archive::ItemRecord::key).collect();
        let current: Vec<Source> =
            plan.current.into_iter().filter(|s| has.contains(&s.key) && plan.targets.contains_key(&s.key)).collect();
        let before = self
            .write(folder, plan.now, current, Kind::BeforeRestore)
            .await
            .map_err(|e| format!("Nothing was restored: the backup of what is here failed: {e}"))?;
        let (targets, free, progress) = (plan.targets, plan.free, self.progress());
        let restored = blocking(move || archive::restore(&path, &targets, &*free, &progress))
            .await
            .map_err(|e| format!("The restore stopped: {e}. What was here before is in {}", file_name(&before)))?;
        let pending = self.inner.data_dir.join(PENDING);
        let settings = serde_json::to_vec(&restored.config).map_err(|e| e.to_string())?;
        blocking(move || std::fs::write(&pending, settings))
            .await
            .map_err(|e| format!("The files are back, but the settings weren't kept: {e}"))?;
        let mut done = format!("Restored {file}. Restart Nexus to finish");
        if !restored.left_out.is_empty() {
            let names: Vec<String> = restored.left_out.iter().map(ToString::to_string).collect();
            done = format!("{done}. Left out, with no place to write here: {}", names.join(", "));
        }
        Ok(done)
    }
}

/// Runs `work` off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::Invalid(e.to_string()))?
        .map_err(|e| CoreError::Invalid(e.to_string()))
}

/// Puts back the settings a restore left in `<data_dir>/`[`PENDING`], then removes the file.
/// Called at start, before any subsystem reads the config. Each of [`archive::APP_KEYS`] the
/// backup has goes back, and each plugin's settings; the folders and the rest stay.
pub(crate) fn apply_pending(config: &ConfigService, data_dir: &Path) {
    let path = data_dir.join(PENDING);
    let Ok(bytes) = std::fs::read(&path) else { return };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(settings) => put_back(config, &settings),
        Err(e) => tracing::warn!(error = %e, "a restore's settings weren't read: they stay as they were"),
    }
    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!(error = %e, "a restore's settings file stayed: they go back again at the next start");
    }
}

fn put_back(config: &ConfigService, settings: &Value) {
    let save = |keys: &[&str], value: &Value| {
        if let Err(e) = config.set(keys, value.clone()) {
            tracing::warn!(error = %e, ?keys, "a setting from the backup wasn't saved");
        }
    };
    if let Some(app) = settings.get("app").and_then(Value::as_object) {
        for key in archive::APP_KEYS {
            if let Some(value) = app.get(key) {
                save(&["app", key], value);
            }
        }
    }
    if let Some(plugins) = settings.get("plugins").and_then(Value::as_object) {
        for (plugin, value) in plugins {
            save(&["plugins", plugin], value);
        }
    }
    tracing::info!("the settings of the last restore are back");
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned()
}

/// The backups in `folder`, newest first.
fn list(folder: &Path) -> Vec<BackupFile> {
    let Ok(dir) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<BackupFile> = dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file = entry.file_name().to_str()?.to_owned();
            let kind = archive::kind_of(&file)?;
            let manifest = archive::read_manifest(&entry.path()).ok()?;
            let mut items: Vec<String> = Vec::new();
            for item in &manifest.items {
                if !items.contains(&item.title) {
                    items.push(item.title.clone());
                }
            }
            Some(BackupFile {
                before_restore: kind == Kind::BeforeRestore,
                bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
                file,
                made: manifest.made,
                items,
            })
        })
        .collect();
    out.sort_by(|a, b| b.made.cmp(&a.made).then_with(|| b.file.cmp(&a.file)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_s_settings_go_back_at_the_next_start_and_this_computer_s_folders_stay() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigService::in_memory();
        config.set(&["app", "folders"], serde_json::json!({ "games": ["/here"] })).unwrap();
        config.set(&["app", "theme"], serde_json::json!("classic")).unwrap();
        let settings = serde_json::json!({
            "app": { "theme": "default", "folders": { "games": ["/there"] } },
            "plugins": { "emu": { "settings": { "fullscreen": false } } },
        });
        std::fs::write(dir.path().join(PENDING), settings.to_string()).unwrap();

        apply_pending(&config, dir.path());
        assert_eq!(config.get(&["app", "theme"]), "default");
        assert_eq!(
            config.get(&["app", "folders"]),
            serde_json::json!({ "games": ["/here"] }),
            "not a key a backup holds"
        );
        assert_eq!(config.get(&["plugins", "emu", "settings", "fullscreen"]), false);
        assert!(!dir.path().join(PENDING).exists(), "once");
    }
}
