//! Settings → Backup: one file that holds the app's settings and what each plugin names of its
//! data ([`nexus_api::Backup`]): emulator settings, keys, firmware and saves. The file is
//! described in [`archive`].
//!
//! The subsystem writes and reads the files, and keeps the list of the backups in the backup
//! folder and the job in progress. It knows no plugin: the core asks the plugins for their
//! items and where they are on this computer ([`crate::backups`]), and hands it the paths.
//! Its changes are [`Area::Backup`].

pub(crate) mod archive;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::{CoreError, CoreResult};

pub(crate) use archive::Source;

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
    pub backups: Vec<BackupFile>,
    /// Why the last job failed. Empty when it didn't.
    pub error: String,
    /// What the last job did, such as "Backed up to nexus-backup-….tar.gz". Empty before one.
    pub done: String,
    /// A restore finished: plugins read their settings when Nexus starts, so it starts again.
    pub restart: bool,
}

#[derive(Clone)]
pub struct BackupSubsystem {
    inner: Arc<Shared>,
}

struct Shared {
    state: Mutex<BackupSnapshot>,
    config: Arc<ConfigService>,
    notifier: Notifier,
}

impl BackupSubsystem {
    pub(crate) fn new(config: Arc<ConfigService>, notifier: Notifier) -> Self {
        Self { inner: Arc::new(Shared { state: Mutex::default(), config, notifier }) }
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
        let backups = tokio::task::spawn_blocking(move || list(&folder)).await.unwrap_or_default();
        self.state().backups = backups;
        self.changed();
    }

    /// Starts a job, or refuses when one runs.
    fn begin(&self, restoring: bool) -> CoreResult<()> {
        let mut state = self.state();
        if state.job.is_some() {
            return Err(CoreError::Invalid("a backup or a restore is already running".into()));
        }
        state.job = Some(BackupJob { restoring, done: 0, total: 0 });
        state.error.clear();
        state.done.clear();
        drop(state);
        self.changed();
        Ok(())
    }

    /// Ends the job: `done` says what it did, or the error why it failed.
    fn end(&self, outcome: Result<String, String>) {
        let mut state = self.state();
        state.job = None;
        match outcome {
            Ok(done) => state.done = done,
            Err(error) => state.error = error,
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

    /// The backup of `sources` and the app's settings, in `folder`, made at `made` (seconds
    /// since 1970). `before_restore` names the one a restore makes first. The file's path.
    pub(crate) async fn back_up(
        &self,
        folder: &Path,
        made: u64,
        sources: Vec<Source>,
        before_restore: bool,
    ) -> CoreResult<PathBuf> {
        self.begin(false)?;
        let written = self.write(folder, made, sources, before_restore).await;
        let outcome = match &written {
            Ok(path) => Ok(format!("Backed up to {}", file_name(path))),
            Err(e) => Err(format!("The backup failed: {e}")),
        };
        self.end(outcome);
        self.list(folder.to_path_buf()).await;
        written
    }

    async fn write(&self, folder: &Path, made: u64, sources: Vec<Source>, before_restore: bool) -> CoreResult<PathBuf> {
        let config = archive::config_subset(&self.inner.config.get(&["app"]), &self.inner.config.get(&["plugins"]));
        let dest = folder.join(archive::file_name(made, before_restore));
        let progress = self.progress();
        let path = dest.clone();
        tokio::task::spawn_blocking(move || {
            archive::write(&path, env!("CARGO_PKG_VERSION"), made, &config, &sources, &progress)
        })
        .await
        .map_err(|e| CoreError::Invalid(e.to_string()))?
        .map_err(|e| CoreError::Invalid(e.to_string()))?;
        Ok(dest)
    }

    /// Puts the backup `file` of `folder` back: each item into `targets`, and the app's
    /// settings, keeping this computer's folders. `first` is the backup of what is there now,
    /// made before anything is written.
    pub(crate) async fn restore(
        &self,
        folder: &Path,
        file: &str,
        targets: BTreeMap<(String, String), PathBuf>,
        first: (u64, Vec<Source>),
    ) -> CoreResult<()> {
        let (made, sources) = first;
        if let Err(e) = self.back_up(folder, made, sources, true).await {
            self.state().error = format!("Nothing was restored: the backup of what is here now failed: {e}");
            self.changed();
            return Err(e);
        }
        self.begin(true)?;
        let progress = self.progress();
        let path = folder.join(file);
        let restored = tokio::task::spawn_blocking(move || archive::restore(&path, &targets, &progress))
            .await
            .map_err(|e| CoreError::Invalid(e.to_string()))
            .and_then(|r| r.map_err(|e| CoreError::Invalid(e.to_string())));
        let outcome = match &restored {
            Ok(restored) => {
                self.put_back_settings(&restored.config);
                let mut done = format!("Restored {file}. Restart Nexus to finish");
                if !restored.left_out.is_empty() {
                    done =
                        format!("{done}. Left out, with no place on this computer: {}", restored.left_out.join(", "));
                }
                self.state().restart = true;
                Ok(done)
            }
            Err(e) => Err(format!("The restore failed: {e}")),
        };
        self.end(outcome);
        restored.map(|_| ())
    }

    /// The backup's settings: each of [`archive::APP_KEYS`] it has, and each plugin's. The
    /// folders and the rest of this computer's settings stay.
    fn put_back_settings(&self, config: &Value) {
        let config_service = &self.inner.config;
        let save = |keys: &[&str], value: &Value| {
            if let Err(e) = config_service.set(keys, value.clone()) {
                tracing::warn!(error = %e, ?keys, "a setting from the backup wasn't saved");
            }
        };
        if let Some(app) = config.get("app").and_then(Value::as_object) {
            for key in archive::APP_KEYS {
                if let Some(value) = app.get(key) {
                    save(&["app", key], value);
                }
            }
        }
        if let Some(plugins) = config.get("plugins").and_then(Value::as_object) {
            for (plugin, value) in plugins {
                save(&["plugins", plugin], value);
            }
        }
    }
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
            if !file.starts_with(archive::PREFIX) || !file.ends_with(archive::EXTENSION) {
                return None;
            }
            let manifest = archive::read_manifest(&entry.path()).ok()?;
            let mut items: Vec<String> = Vec::new();
            for item in &manifest.items {
                if !items.contains(&item.title) {
                    items.push(item.title.clone());
                }
            }
            Some(BackupFile {
                before_restore: file.contains("-before-restore"),
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
