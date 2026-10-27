//! A fake unpacker: the test says what each archive holds, and whether it fails or waits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use nexus_api::{PluginError, PluginResult};
use nexus_core::{ArchiveKind, Archives, Clock, UnpackProgress};
use tokio::sync::watch;

use crate::{FakeClock, Gate};

/// How often a held unpack reads whether it was cancelled.
const CANCEL_READ: Duration = Duration::from_millis(5);

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
    /// Unpacks wait for this gate.
    held: Option<watch::Receiver<bool>>,
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

    /// Until the gate is released, unpacks wait (and stop when cancelled).
    pub fn hold(&self) -> Gate {
        let (gate, held) = Gate::new();
        self.lock().held = Some(held);
        gate
    }

    /// The unpacks that run now never end, as in an app that closed during them, and the
    /// hold comes off for the unpacks that begin after.
    pub fn cut(&self) {
        let mut state = self.lock();
        state.cuts += 1;
        state.held = None;
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
        let (cuts, held) = {
            let mut state = self.lock();
            state.unpacked.push((kind, parts.iter().map(|p| file_name(p)).collect()));
            (state.cuts, state.held.clone())
        };
        if let Some(mut held) = held {
            // The port says a cancel only by a flag: it is read on the fake clock's time.
            let mut ticks = FakeClock::default().interval(CANCEL_READ);
            let cancelled = async {
                while !cancel.load(Ordering::Acquire) {
                    ticks.next().await;
                }
            };
            tokio::select! {
                _ = held.wait_for(|open| *open) => {}
                () = cancelled => return Err(PluginError::Other("The unpacking was cancelled".into())),
            }
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
