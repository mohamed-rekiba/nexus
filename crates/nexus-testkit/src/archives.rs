//! A fake unpacker: the test says what each archive holds, and whether it fails or waits. It
//! plans each unpack with the core's [`plan_unpack`], so it refuses what the app's adapter
//! refuses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use nexus_core::{ArchiveEntry, ArchiveError, ArchiveKind, Archives, Clock, EntryKind, UnpackProgress, plan_unpack};
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
    failures: BTreeMap<String, ArchiveError>,
    /// The first parts' file names of the archives whose entries need a password.
    locked: BTreeSet<String>,
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

    /// Unpacking the archive whose first part is named `first_part` fails with `why`, as a
    /// damaged archive does.
    pub fn fails(&self, first_part: &str, why: &str) {
        self.fails_with(first_part, ArchiveError::Corrupt(why.to_owned()));
    }

    /// Unpacking the archive whose first part is named `first_part` fails with `error`.
    pub fn fails_with(&self, first_part: &str, error: ArchiveError) {
        self.lock().failures.insert(first_part.to_owned(), error);
    }

    /// The entries of the archive whose first part is named `first_part` need a password.
    pub fn locked(&self, first_part: &str) {
        self.lock().locked.insert(first_part.to_owned());
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
        free: Option<u64>,
        progress: UnpackProgress,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError> {
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
                () = cancelled => return Err(ArchiveError::Cancelled),
            }
        }
        if self.lock().cuts != cuts {
            std::future::pending::<()>().await;
        }
        if let Some(error) = self.lock().failures.get(&first).cloned() {
            return Err(error);
        }
        // The listing, as a format reader gives it: the files, then the folders, then the links.
        let (listing, bytes) = {
            let state = self.lock();
            let encrypted = state.locked.contains(&first);
            let mut listing = Vec::new();
            let mut bytes = Vec::new();
            for (path, data) in state.contents.get(&first).into_iter().flatten() {
                listing.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(path, data.len() as u64, EntryKind::File) });
                bytes.push(data.clone());
            }
            for folder in state.folders.get(&first).into_iter().flatten() {
                listing.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(folder, 0, EntryKind::Folder) });
                bytes.push(Vec::new());
            }
            for (path, target) in state.links.get(&first).into_iter().flatten() {
                listing.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(path, 0, EntryKind::Link(target.into())) });
                bytes.push(Vec::new());
            }
            (listing, bytes)
        };
        let plan = plan_unpack(listing, into, free)?;
        let io = |e: std::io::Error| ArchiveError::Io(e.to_string());
        let mut done = 0;
        for (entry, data) in plan.entries.iter().zip(bytes) {
            let out = plan.out(entry);
            match &entry.kind {
                EntryKind::File => {
                    std::fs::create_dir_all(out.parent().unwrap_or(into)).map_err(io)?;
                    std::fs::write(&out, &data).map_err(io)?;
                    done += data.len() as u64;
                    progress(done, plan.total);
                }
                EntryKind::Folder => std::fs::create_dir_all(out).map_err(io)?,
                // Made on Unix only, as the app's adapter does.
                EntryKind::Link(target) => {
                    #[cfg(unix)]
                    {
                        std::fs::create_dir_all(out.parent().unwrap_or(into)).map_err(io)?;
                        std::os::unix::fs::symlink(target, out).map_err(io)?;
                    }
                    #[cfg(not(unix))]
                    let _ = (out, target);
                }
                EntryKind::HardLink(_) | EntryKind::Other => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::*;
    use crate::contract::{self, ArchiveFixture, Sample};

    /// Declares each archive the contract asks for on the fake, under a new file name.
    struct Declared {
        fake: FakeArchives,
        made: AtomicU32,
    }

    impl Declared {
        /// A new first part in `dir`: the fake knows an archive by its first part's name.
        fn part(&self, dir: &Path) -> (String, PathBuf) {
            let name = format!("archive{}.rar", self.made.fetch_add(1, Ordering::Relaxed));
            let path = dir.join(&name);
            std::fs::write(&path, b"declared").unwrap();
            (name, path)
        }
    }

    impl ArchiveFixture for Declared {
        fn archive(&self, dir: &Path, entries: &[Sample]) -> (ArchiveKind, Vec<PathBuf>) {
            let (name, path) = self.part(dir);
            let mut files: Vec<(&str, &[u8])> = Vec::new();
            let mut links: Vec<(&str, &str)> = Vec::new();
            for entry in entries {
                match entry {
                    Sample::File(path, bytes) => files.push((path, bytes)),
                    Sample::Link(path, target) => links.push((path, target)),
                }
            }
            self.fake.holds(&name, &files);
            self.fake.also_holds(&name, &links, &[]);
            (ArchiveKind::Rar, vec![path])
        }

        fn locked(&self, dir: &Path) -> Option<(ArchiveKind, Vec<PathBuf>)> {
            let (name, path) = self.part(dir);
            self.fake.holds(&name, &[("Game/eboot.bin", b"elf")]);
            self.fake.locked(&name);
            Some((ArchiveKind::Rar, vec![path]))
        }
    }

    #[tokio::test]
    async fn the_fake_keeps_the_archives_contract() {
        let fake = FakeArchives::default();
        contract::archives(&fake, &Declared { fake: fake.clone(), made: AtomicU32::new(0) }).await;
    }
}
