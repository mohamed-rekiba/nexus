//! An in-memory BitTorrent swarm: the test says which torrents it has and when pieces arrive.
//! It reports raw stats, as an engine does, and decides nothing: the core's phase rules read
//! them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::{PluginError, PluginResult};
use nexus_core::{TorrentFile, TorrentInfo, TorrentStats, Torrents, TransferState};
use tokio::sync::watch;

use crate::Gate;

/// A torrent's files: each path and its bytes.
type Files = Vec<(String, Vec<u8>)>;

struct Running {
    folder: PathBuf,
    /// Started paused, or paused since: it talks to no peer.
    paused: bool,
    stats: TorrentStats,
}

#[derive(Default)]
struct State {
    /// Hash -> the torrent's name and its files' bytes.
    swarm: BTreeMap<String, (String, Files)>,
    running: BTreeMap<String, Running>,
    /// Every start: hash, and whether it started paused.
    started: Vec<(String, bool)>,
    /// Every route asked for, in order.
    routes: Vec<Option<String>>,
    /// How many times every torrent was stopped at once.
    stopped_all: usize,
    /// Lookups wait for this gate.
    held: Option<watch::Receiver<bool>>,
}

/// A fake swarm. Torrents are known by the info hash in their magnet link.
#[derive(Clone, Default)]
pub struct FakeTorrents {
    state: Arc<Mutex<State>>,
}

fn hash_of(magnet: &str) -> PluginResult<String> {
    magnet
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("xt=urn:btih:"))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| PluginError::InvalidInput(format!("not a magnet link: {magnet}")))
}

impl FakeTorrents {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// The swarm has the torrent `hash`, named `name`, with these files.
    pub fn has(&self, hash: &str, name: &str, files: &[(&str, &[u8])]) {
        let files = files.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        self.lock().swarm.insert(hash.to_owned(), (name.to_owned(), files));
    }

    /// Until the gate is released, a lookup of a torrent's files waits: the download stays
    /// Resolving.
    pub fn hold(&self) -> Gate {
        let (gate, held) = Gate::new();
        self.lock().held = Some(held);
        gate
    }

    /// The running torrent has `done` bytes.
    pub fn progress(&self, hash: &str, done: u64) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.stats.state = TransferState::Live;
            r.stats.done = done;
            r.stats.download_rate = 1024 * 1024;
            r.stats.peers = 3;
        }
    }

    /// The running torrent downloads `bytes` a second.
    pub fn rate(&self, hash: &str, bytes: u64) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.stats.download_rate = bytes;
        }
    }

    /// Every piece arrives: the files are written into the torrent's folder.
    pub fn complete(&self, hash: &str) {
        let mut state = self.lock();
        let files = state.swarm.get(hash).map(|(_, f)| f.clone()).unwrap_or_default();
        let Some(r) = state.running.get_mut(hash) else { return };
        for (path, bytes) in files {
            let to = r.folder.join(&path);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, bytes).unwrap();
        }
        r.stats.done = r.stats.total;
        r.stats.download_rate = 0;
        r.stats.finished = true;
        r.stats.state = if r.paused { TransferState::Paused } else { TransferState::Live };
    }

    /// The running torrent stops with `error`.
    pub fn fail(&self, hash: &str, error: &str) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.stats.state = TransferState::Error;
            r.stats.error = error.to_owned();
        }
    }

    /// Every start so far, as (hash, started paused).
    pub fn started(&self) -> Vec<(String, bool)> {
        self.lock().started.clone()
    }

    /// The hashes of the torrents running now.
    pub fn running(&self) -> Vec<String> {
        self.lock().running.keys().cloned().collect()
    }

    /// Every route the core asked for, in order: the interface, or `None` for any.
    pub fn routes(&self) -> Vec<Option<String>> {
        self.lock().routes.clone()
    }

    /// How many times the core stopped every torrent at once.
    pub fn stopped_all(&self) -> usize {
        self.lock().stopped_all
    }
}

#[async_trait]
impl Torrents for FakeTorrents {
    async fn resolve(&self, magnet: &str) -> PluginResult<TorrentInfo> {
        let hash = hash_of(magnet)?;
        let held = self.lock().held.clone();
        if let Some(mut held) = held {
            // A released gate is open; a dropped one too.
            let _ = held.wait_for(|open| *open).await;
        }
        let state = self.lock();
        let (name, files) = state.swarm.get(&hash).ok_or_else(|| PluginError::Network("no peers have it".into()))?;
        Ok(TorrentInfo {
            name: name.clone(),
            files: files.iter().map(|(p, b)| TorrentFile { path: p.clone(), size: b.len() as u64 }).collect(),
        })
    }

    /// A start checks the files in the folder at once. Whole, the torrent is finished: live,
    /// or paused when it started paused. Else a live one checks until the test sends pieces
    /// ([`FakeTorrents::progress`]), and a paused one stays paused with pieces missing.
    async fn start(&self, id: &str, magnet: &str, folder: &Path, paused: bool) -> PluginResult<()> {
        let hash = hash_of(magnet)?;
        let mut state = self.lock();
        let files =
            state.swarm.get(&hash).map(|(_, f)| f.clone()).ok_or_else(|| PluginError::Network("no peers".into()))?;
        let total = files.iter().map(|(_, b)| b.len() as u64).sum();
        let whole = files.iter().all(|(p, b)| std::fs::read(folder.join(p)).is_ok_and(|on_disk| &on_disk == b));
        let state_now = match (whole, paused) {
            (_, true) => TransferState::Paused,
            (true, false) => TransferState::Live,
            (false, false) => TransferState::Checking,
        };
        state.started.push((id.to_owned(), paused));
        state.running.insert(
            id.to_owned(),
            Running {
                folder: folder.to_owned(),
                paused,
                stats: TorrentStats {
                    state: state_now,
                    finished: whole,
                    done: if whole { total } else { 0 },
                    total,
                    download_rate: 0,
                    upload_rate: 0,
                    peers: 0,
                    error: String::new(),
                },
            },
        );
        Ok(())
    }

    async fn unpause(&self, id: &str) -> PluginResult<()> {
        if let Some(r) = self.lock().running.get_mut(id)
            && r.paused
        {
            r.paused = false;
            if r.stats.state == TransferState::Paused {
                r.stats.state = TransferState::Live;
            }
        }
        Ok(())
    }

    async fn stop(&self, id: &str) -> PluginResult<()> {
        self.lock().running.remove(id);
        Ok(())
    }

    fn stats(&self, id: &str) -> Option<TorrentStats> {
        self.lock().running.get(id).map(|r| r.stats.clone())
    }

    async fn route(&self, interface: Option<String>) -> PluginResult<()> {
        self.lock().routes.push(interface);
        Ok(())
    }

    async fn stop_all(&self) -> PluginResult<()> {
        let mut state = self.lock();
        state.running.clear();
        state.stopped_all += 1;
        Ok(())
    }
}
