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
/// What every backup holds besides the plugins' items, as the player reads it.
pub(crate) const SETTINGS: &str = "Nexus settings";
/// Why a second job can't start.
pub(crate) const BUSY: &str = "A backup or a restore is already running";
/// Why the browser's commands do nothing.
const CLOSED: &str = "the backup browser isn't open";

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
    /// What it holds: [`SETTINGS`], then its items' titles, each once.
    pub items: Vec<String>,
    /// The backup a restore made of what was there before.
    pub before_restore: bool,
    /// The name of the computer that made it; empty when the backup doesn't say.
    pub computer: String,
    /// A newer Nexus made it, in a format this one doesn't restore.
    pub newer: bool,
}

/// What a place the browser goes to is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Downloads,
    Desktop,
    Documents,
    /// The backup folder.
    Backups,
    /// A disk the system mounted, such as a USB stick.
    Drive,
    /// A folder a restore came from.
    Recent,
}

impl PlaceKind {
    /// Its name for the theme: `home`, `downloads`, … `recent`.
    pub fn as_str(self) -> &'static str {
        match self {
            PlaceKind::Home => "home",
            PlaceKind::Downloads => "downloads",
            PlaceKind::Desktop => "desktop",
            PlaceKind::Documents => "documents",
            PlaceKind::Backups => "backups",
            PlaceKind::Drive => "drive",
            PlaceKind::Recent => "recent",
        }
    }
}

/// One place the browser goes to: Home, the backup folder, a drive, a recent folder.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowsePlace {
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaceKind,
    /// A drive's size, such as "32.0 GB"; a recent folder's path. Empty for the others.
    pub detail: String,
}

/// A folder or a backup in the browser's folder.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseEntry {
    pub path: PathBuf,
    /// A backup's details; `None` for a folder.
    pub backup: Option<BackupFile>,
    /// A folder's: what is in it ("2 backups", "empty", "can't be read"). Empty for a backup,
    /// and for a folder with other things but no backups.
    pub detail: String,
}

/// "Restore from another folder": the folder the browser shows, and where it can go.
#[derive(Clone, Debug, PartialEq)]
pub struct Browser {
    pub at: PathBuf,
    pub places: Vec<BrowsePlace>,
    /// Its folders with backups, its other folders, then its backups newest first. Hidden
    /// folders only with `hidden`.
    pub entries: Vec<BrowseEntry>,
    /// Hidden folders are listed.
    pub hidden: bool,
    /// The text the player typed: the view shows only the entries whose name holds it, in any
    /// case. Empty for all.
    pub filter: String,
}

impl Browser {
    /// The entries the filter lets through.
    pub fn shown(&self) -> impl Iterator<Item = &BrowseEntry> {
        let filter = self.filter.to_lowercase();
        self.entries.iter().filter(move |e| {
            filter.is_empty()
                || e.path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&filter))
        })
    }
}

/// The subsystem's state, as the view reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackupSnapshot {
    pub job: Option<BackupJob>,
    /// Newest first.
    pub backups: Arc<Vec<BackupFile>>,
    /// What a backup holds on this computer now: the titles of the plugins' items, and of the
    /// large ones, which a backup takes only when asked.
    pub holds: Arc<Vec<String>>,
    pub large: Arc<Vec<String>>,
    /// The browser, while it is open.
    pub browser: Option<Arc<Browser>>,
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
    /// This computer's name, for that backup.
    pub computer: String,
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

    /// What a backup holds here now: the titles of the plugins' items, and of the large ones.
    pub(crate) fn set_items(&self, holds: Vec<String>, large: Vec<String>) {
        let mut state = self.state();
        state.holds = Arc::new(holds);
        state.large = Arc::new(large);
        drop(state);
        self.changed();
    }

    /// Opens the browser at `at`, or moves it there, with `places` to go to. Hidden folders
    /// stay listed or not as they were; the filter is cleared.
    pub(crate) async fn browse(&self, at: PathBuf, places: Vec<BrowsePlace>) -> CoreResult<()> {
        let hidden = self.state().browser.as_ref().is_some_and(|b| b.hidden);
        let dir = at.clone();
        let entries = blocking(move || browse(&dir, hidden)).await?;
        self.state().browser = Some(Arc::new(Browser { at, places, entries, hidden, filter: String::new() }));
        self.changed();
        Ok(())
    }

    /// Lists the browser's hidden folders, or not.
    pub(crate) async fn show_hidden(&self, hidden: bool) -> CoreResult<()> {
        let open = self.state().browser.clone().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
        let dir = open.at.clone();
        let entries = blocking(move || browse(&dir, hidden)).await?;
        let mut state = self.state();
        // Moved or closed in the meantime: that newer state stays.
        if state.browser.as_ref().is_some_and(|b| b.at == open.at) {
            let mut browser = (*open).clone();
            (browser.entries, browser.hidden) = (entries, hidden);
            state.browser = Some(Arc::new(browser));
            drop(state);
            self.changed();
        }
        Ok(())
    }

    /// Shows only the browser's entries whose name holds `text`; all for empty text.
    pub(crate) fn filter(&self, text: String) -> CoreResult<()> {
        let mut state = self.state();
        let open = state.browser.as_ref().ok_or_else(|| CoreError::Invalid(CLOSED.into()))?;
        let mut browser = (**open).clone();
        browser.filter = text;
        state.browser = Some(Arc::new(browser));
        drop(state);
        self.changed();
        Ok(())
    }

    pub(crate) fn close_browser(&self) {
        self.state().browser = None;
        self.changed();
    }

    /// Removes the backup `file` of `folder`, then reads the list again.
    pub(crate) async fn delete(&self, folder: &Path, file: &str) -> CoreResult<()> {
        if self.busy() {
            return Err(CoreError::Invalid(BUSY.into()));
        }
        let path = folder.join(file);
        let removed = blocking(move || std::fs::remove_file(&path)).await;
        {
            let mut state = self.state();
            state.error.clear();
            match &removed {
                Ok(()) => state.done = format!("Deleted {file}"),
                Err(e) => state.error = format!("{file} wasn't deleted: {e}"),
            }
        }
        self.list(folder.to_path_buf()).await;
        removed
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
    /// since 1970) on the computer named `computer`. The file's path.
    pub(crate) async fn back_up(
        &self,
        folder: &Path,
        made: u64,
        computer: &str,
        sources: Vec<Source>,
    ) -> CoreResult<PathBuf> {
        self.begin(false)?;
        let written = self.write(folder, made, computer, sources, Kind::Backup).await;
        let outcome = match &written {
            Ok(path) => Ok(format!("Backed up to {}", file_name(path))),
            Err(e) => Err(format!("The backup failed: {e}")),
        };
        self.end(&outcome);
        self.list(folder.to_path_buf()).await;
        written
    }

    async fn write(
        &self,
        folder: &Path,
        made: u64,
        computer: &str,
        sources: Vec<Source>,
        kind: Kind,
    ) -> CoreResult<PathBuf> {
        let config = archive::config_subset(&self.inner.config.get(&["app"]), &self.inner.config.get(&["plugins"]));
        let dest = folder.join(archive::file_name(made, kind));
        let progress = self.progress();
        let (path, computer) = (dest.clone(), computer.to_owned());
        blocking(move || {
            archive::write(&path, env!("CARGO_PKG_VERSION"), &computer, made, &config, &sources, &progress)
        })
        .await?;
        Ok(dest)
    }

    /// Puts the backup at `path` back. First it backs up what is here of the items the backup
    /// has, into `folder`; nothing is restored when that fails. The settings wait in
    /// [`PENDING`] for the next start.
    pub(crate) async fn restore(&self, path: &Path, folder: &Path, plan: RestorePlan) -> CoreResult<()> {
        self.begin(true)?;
        self.state().browser = None;
        let outcome = self.restore_in_job(path, folder, plan).await;
        self.end(&outcome);
        if outcome.is_ok() {
            self.state().restart = true;
            self.changed();
        }
        self.list(folder.to_path_buf()).await;
        outcome.map(|_| ()).map_err(CoreError::Invalid)
    }

    async fn restore_in_job(&self, path: &Path, folder: &Path, plan: RestorePlan) -> Result<String, String> {
        let file = file_name(path);
        let path = path.to_path_buf();
        let read = path.clone();
        let manifest = blocking(move || archive::read_manifest(&read)).await.map_err(|e| format!("{file}: {e}"))?;
        let has: Vec<ItemKey> = manifest.items.iter().map(archive::ItemRecord::key).collect();
        let current: Vec<Source> =
            plan.current.into_iter().filter(|s| has.contains(&s.key) && plan.targets.contains_key(&s.key)).collect();
        let before = self
            .write(folder, plan.now, &plan.computer, current, Kind::BeforeRestore)
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

/// The backup at `path`, of any format; `None` for a file that isn't a backup.
fn backup_file(path: &Path) -> Option<BackupFile> {
    let file = path.file_name()?.to_str()?.to_owned();
    let kind = archive::kind_of(&file)?;
    let manifest = archive::peek_manifest(path).ok()?;
    let mut items: Vec<String> = vec![SETTINGS.to_owned()];
    for item in &manifest.items {
        if !items.contains(&item.title) {
            items.push(item.title.clone());
        }
    }
    Some(BackupFile {
        before_restore: kind == Kind::BeforeRestore,
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        file,
        made: manifest.made,
        items,
        computer: manifest.computer,
        newer: manifest.format > archive::FORMAT,
    })
}

fn newest_first(backups: &mut [BackupFile]) {
    backups.sort_by(|a, b| b.made.cmp(&a.made).then_with(|| b.file.cmp(&a.file)));
}

/// The backups in `folder` this Nexus restores, newest first.
fn list(folder: &Path) -> Vec<BackupFile> {
    let Ok(dir) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<BackupFile> =
        dir.filter_map(Result::ok).filter_map(|e| backup_file(&e.path())).filter(|b| !b.newer).collect();
    newest_first(&mut out);
    out
}

/// The folders of `dir` that hold backups, its other folders, each by name, then its backups
/// newest first. Hidden folders only with `hidden`; other files and links are left out.
fn browse(dir: &Path, hidden: bool) -> std::io::Result<Vec<BrowseEntry>> {
    let mut folders = Vec::new();
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else { continue };
        let name = entry.file_name();
        if kind.is_dir() && (hidden || !name.to_string_lossy().starts_with('.')) {
            let inside = inside(&entry.path());
            folders.push((entry.path(), inside));
        } else if kind.is_file()
            && let Some(backup) = backup_file(&entry.path())
        {
            backups.push((entry.path(), backup));
        }
    }
    folders.sort_by_key(|(p, inside)| {
        (!matches!(inside, Inside::Backups(_)), p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
    });
    backups.sort_by_key(|b| std::cmp::Reverse(b.1.made));
    let mut out: Vec<BrowseEntry> =
        folders.into_iter().map(|(path, inside)| BrowseEntry { path, backup: None, detail: inside.text() }).collect();
    out.extend(backups.into_iter().map(|(path, backup)| BrowseEntry {
        path,
        backup: Some(backup),
        detail: String::new(),
    }));
    Ok(out)
}

/// What is in a folder, as its row says.
enum Inside {
    Backups(usize),
    Empty,
    Other,
    Unreadable,
}

impl Inside {
    fn text(&self) -> String {
        match self {
            Inside::Backups(1) => "1 backup".into(),
            Inside::Backups(n) => format!("{n} backups"),
            Inside::Empty => "empty".into(),
            Inside::Other => String::new(),
            Inside::Unreadable => "can't be read".into(),
        }
    }
}

/// What is in `dir`, by the names of its files: no backup is opened.
fn inside(dir: &Path) -> Inside {
    let Ok(read) = std::fs::read_dir(dir) else { return Inside::Unreadable };
    let (mut any, mut backups) = (false, 0);
    for entry in read.filter_map(Result::ok) {
        any = true;
        if entry.file_type().is_ok_and(|t| t.is_file())
            && entry.file_name().to_str().and_then(archive::kind_of).is_some()
        {
            backups += 1;
        }
    }
    match (backups, any) {
        (0, false) => Inside::Empty,
        (0, true) => Inside::Other,
        (n, _) => Inside::Backups(n),
    }
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
