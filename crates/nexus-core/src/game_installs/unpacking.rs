//! The hand-off from a finished download: its archive sets ([`crate::archives`]) unpack
//! through the [`crate::Archives`] port into `unpacked/<folder>/<name>/` in the download, one
//! `.part` folder at a time, before the emulator installs. The progress and the outcome are
//! the download's hand-off ([`crate::torrents::HandOff`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::{GameInstallsSubsystem, InstallEvent};
use crate::archives::{UNPACKED, UnpackPhase, unpacked_folder, work_folder};
use crate::torrents::TorrentState;

impl GameInstallsSubsystem {
    /// Unpacks the download's archives in the background, then, when `install`, has the core
    /// install it ([`InstallEvent::Unpacked`]). Sets already unpacked stay as they are.
    pub fn unpack(&self, id: &str, install: bool) {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut started = false;
        self.hand_off(id, |state, h| {
            if *state != TorrentState::Complete || h.unpack.phase == UnpackPhase::Unpacking {
                return;
            }
            h.unpack.phase = UnpackPhase::Unpacking;
            h.unpack.error.clear();
            h.unpack.progress = 0.0;
            h.unpack.cancel = Some(cancel.clone());
            started = true;
        });
        if !started {
            return;
        }
        let this = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let (phase, error) = match this.run_unpack(&id, cancel.clone()).await {
                Ok(()) => (UnpackPhase::Unpacked, String::new()),
                Err(_) if cancel.load(Ordering::Acquire) => (UnpackPhase::Cancelled, String::new()),
                Err(e) => (UnpackPhase::Failed, e),
            };
            this.hand_off(&id, |_, h| {
                h.unpack.phase = phase;
                h.unpack.error.clone_from(&error);
                h.unpack.cancel = None;
                if phase == UnpackPhase::Unpacked {
                    h.unpack.progress = 1.0;
                }
            });
            if let Some(job) = this.downloads().job(&id).filter(|_| install && phase == UnpackPhase::Unpacked) {
                this.emit(InstallEvent::Unpacked {
                    plugin: job.emulator,
                    source: job.source,
                    into: job.hand_off.install_into,
                    title: job.title,
                })
                .await;
            }
        });
    }

    async fn run_unpack(&self, id: &str, cancel: Arc<AtomicBool>) -> Result<(), String> {
        let Some(job) = self.downloads().job(id) else { return Err("no such download".into()) };
        let sets = job.hand_off.unpack.sets.clone();
        if let Some(problem) = sets.iter().find_map(|s| s.problem.clone()) {
            return Err(problem);
        }
        let ports = &self.inner.ports;
        let count = sets.len().max(1) as f32;
        // What this run unpacked, removed again when a later set fails.
        let mut written: Vec<PathBuf> = Vec::new();
        for (i, set) in sets.iter().enumerate() {
            let dest = unpacked_folder(&job.folder, set);
            if dest.exists() {
                continue;
            }
            let work = work_folder(&job.folder, set);
            let _ = tokio::fs::remove_dir_all(&work).await;
            tokio::fs::create_dir_all(&work).await.map_err(|e| format!("The unpack folder couldn't be made: {e}"))?;
            let parts: Vec<PathBuf> = set.parts.iter().map(|p| job.folder.join(p)).collect();
            // Saved at each whole percent, not at each piece.
            let shown = Arc::new(AtomicU32::new(0));
            let progress: crate::UnpackProgress = {
                let this = self.clone();
                let id = id.to_owned();
                Arc::new(move |done, total| {
                    let part = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    let percent = (((i as f32 + part.min(1.0)) / count) * 100.0) as u32;
                    if shown.swap(percent, Ordering::AcqRel) != percent {
                        this.hand_off(&id, |_, h| h.unpack.progress = percent as f32 / 100.0);
                    }
                })
            };
            // The folder exists now, so the disk that holds it can say its free space.
            let free = ports.hardware.free_bytes(&work);
            let unpacked = ports.archives.unpack(set.kind, &parts, &work, free, progress, cancel.clone()).await;
            let moved = match unpacked {
                Ok(()) => tokio::fs::rename(&work, &dest).await.map_err(|e| format!("{}: {e}", set.name)),
                Err(e) => Err(format!("{}: {e}", set.name)),
            };
            if let Err(e) = moved {
                let _ = tokio::fs::remove_dir_all(&work).await;
                for dest in &written {
                    let _ = tokio::fs::remove_dir_all(dest).await;
                }
                return Err(e);
            }
            written.push(dest);
        }
        Ok(())
    }

    /// Deletes the download's unpacked files. The torrent's own files stay.
    pub async fn remove_unpacked(&self, id: &str) {
        let Some(job) = self.downloads().job(id) else { return };
        if job.hand_off.unpack.phase == UnpackPhase::Unpacking {
            return;
        }
        let _ = tokio::fs::remove_dir_all(job.folder.join(UNPACKED)).await;
        self.hand_off(id, |_, h| {
            h.unpack.phase = UnpackPhase::Waiting;
            h.unpack.progress = 0.0;
            h.unpack.error.clear();
        });
    }
}
