//! A fake unpacker: the test says what each archive holds, and whether it fails or waits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nexus_api::{PluginError, PluginResult};
use nexus_core::{ArchiveKind, Archives, UnpackProgress};

/// An archive's files: each path and its bytes.
type Files = Vec<(String, Vec<u8>)>;

#[derive(Default)]
struct State {
    /// The first part's file name -> what the archive holds.
    contents: BTreeMap<String, Files>,
    /// The first part's file name -> its links (path, target) and empty folders, made on Unix.
    links: BTreeMap<String, Vec<(String, String)>>,
    folders: BTreeMap<String, Vec<String>>,
    /// The first part's file name -> why it fails.
    failures: BTreeMap<String, String>,
    /// Unpacks wait while this is set.
    held: bool,
    /// Counts the cuts: an unpack that began before the latest cut never ends.
    cuts: u64,
    /// Every unpack: its kind and its parts' file names.
    unpacked: Vec<(ArchiveKind, Vec<String>)>,
}

/// A fake unpacker. Archives are known by their first part's file name.
#[derive(Clone, Default)]
pub struct FakeArchives {
    state: Arc<Mutex<State>>,
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl FakeArchives {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// The archive whose first part is named `first_part` holds these files.
    pub fn holds(&self, first_part: &str, files: &[(&str, &[u8])]) {
        let files = files.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        self.lock().contents.insert(first_part.to_owned(), files);
    }

    /// The archive whose first part is named `first_part` also holds these links, each to its
    /// target, and these empty folders: a Wine prefix's `dosdevices`, a library's `.so` links.
    pub fn also_holds(&self, first_part: &str, links: &[(&str, &str)], folders: &[&str]) {
        let mut state = self.lock();
        state.links.insert(first_part.to_owned(), links.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect());
        state.folders.insert(first_part.to_owned(), folders.iter().map(|f| f.to_string()).collect());
    }

    /// Unpacking the archive whose first part is named `first_part` fails with `why`.
    pub fn fails(&self, first_part: &str, why: &str) {
        self.lock().failures.insert(first_part.to_owned(), why.to_owned());
    }

    /// While held, unpacks wait (and stop when cancelled).
    pub fn hold(&self, held: bool) {
        self.lock().held = held;
    }

    /// The unpacks that run now never end, as in an app that closed during them, and the
    /// hold comes off for the unpacks that begin after.
    pub fn cut(&self) {
        let mut state = self.lock();
        state.cuts += 1;
        state.held = false;
    }

    /// Every unpack so far: its kind and its parts' file names.
    pub fn unpacked(&self) -> Vec<(ArchiveKind, Vec<String>)> {
        self.lock().unpacked.clone()
    }
}

#[async_trait]
impl Archives for FakeArchives {
    async fn unpack(
        &self,
        kind: ArchiveKind,
        parts: &[PathBuf],
        into: &Path,
        progress: UnpackProgress,
        cancel: Arc<AtomicBool>,
    ) -> PluginResult<()> {
        let first = parts.first().map(|p| file_name(p)).unwrap_or_default();
        let cuts = {
            let mut state = self.lock();
            state.unpacked.push((kind, parts.iter().map(|p| file_name(p)).collect()));
            state.cuts
        };
        while self.lock().held {
            if cancel.load(Ordering::Acquire) {
                return Err(PluginError::Other("The unpacking was cancelled".into()));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if self.lock().cuts != cuts {
            std::future::pending::<()>().await;
        }
        if let Some(why) = self.lock().failures.get(&first).cloned() {
            return Err(PluginError::Other(why));
        }
        let files = self.lock().contents.get(&first).cloned().unwrap_or_default();
        let total = files.iter().map(|(_, b)| b.len() as u64).sum();
        let mut done = 0;
        for (path, bytes) in files {
            let path = into.join(path);
            std::fs::create_dir_all(path.parent().unwrap_or(into)).map_err(|e| PluginError::Other(e.to_string()))?;
            std::fs::write(&path, &bytes).map_err(|e| PluginError::Other(e.to_string()))?;
            done += bytes.len() as u64;
            progress(done, total);
        }
        let io = |e: std::io::Error| PluginError::Other(e.to_string());
        for folder in self.lock().folders.get(&first).cloned().unwrap_or_default() {
            std::fs::create_dir_all(into.join(folder)).map_err(io)?;
        }
        #[cfg(unix)]
        for (path, target) in self.lock().links.get(&first).cloned().unwrap_or_default() {
            let path = into.join(path);
            std::fs::create_dir_all(path.parent().unwrap_or(into)).map_err(io)?;
            std::os::unix::fs::symlink(target, path).map_err(io)?;
        }
        Ok(())
    }
}
