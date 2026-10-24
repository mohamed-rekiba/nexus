//! An in-memory BitTorrent swarm: the test says which torrents it has and when pieces arrive.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::{PluginError, PluginResult};
use nexus_core::{TorrentFile, TorrentInfo, TorrentPhase, TorrentStart, TorrentStatus, Torrents};

/// A torrent's files: each path and its bytes.
type Files = Vec<(String, Vec<u8>)>;

struct Running {
    folder: PathBuf,
    how: TorrentStart,
    status: TorrentStatus,
}

#[derive(Default)]
struct State {
    /// Hash -> the torrent's name and its files' bytes.
    swarm: BTreeMap<String, (String, Files)>,
    running: BTreeMap<String, Running>,
    /// Every start: hash and how.
    started: Vec<(String, TorrentStart)>,
    /// Every route asked for, in order.
    routes: Vec<Option<String>>,
    /// How many times every torrent was stopped at once.
    stopped_all: usize,
    /// Lookups wait while this is on.
    held: bool,
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

    /// While `on`, a lookup of a torrent's files waits: the download stays Resolving.
    pub fn hold(&self, on: bool) {
        self.lock().held = on;
    }

    /// The running torrent has `done` bytes.
    pub fn progress(&self, hash: &str, done: u64) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.status.phase = TorrentPhase::Downloading;
            r.status.done = done;
            r.status.download_rate = 1024 * 1024;
            r.status.peers = 3;
        }
    }

    /// The running torrent downloads `bytes` a second.
    pub fn rate(&self, hash: &str, bytes: u64) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.status.download_rate = bytes;
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
        r.status.done = r.status.total;
        r.status.download_rate = 0;
        r.status.phase = match r.how {
            TorrentStart::Download { seed: false } => TorrentPhase::Finished,
            _ => TorrentPhase::Seeding,
        };
    }

    /// The running torrent stops with `error`.
    pub fn fail(&self, hash: &str, error: &str) {
        if let Some(r) = self.lock().running.get_mut(hash) {
            r.status.phase = TorrentPhase::Failed;
            r.status.error = error.to_owned();
        }
    }

    /// Every start so far, as (hash, how).
    pub fn started(&self) -> Vec<(String, TorrentStart)> {
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
        while self.lock().held {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let state = self.lock();
        let (name, files) = state.swarm.get(&hash).ok_or_else(|| PluginError::Network("no peers have it".into()))?;
        Ok(TorrentInfo {
            name: name.clone(),
            files: files.iter().map(|(p, b)| TorrentFile { path: p.clone(), size: b.len() as u64 }).collect(),
        })
    }

    async fn start(&self, id: &str, magnet: &str, folder: &Path, how: TorrentStart) -> PluginResult<()> {
        let hash = hash_of(magnet)?;
        let mut state = self.lock();
        let files =
            state.swarm.get(&hash).map(|(_, f)| f.clone()).ok_or_else(|| PluginError::Network("no peers".into()))?;
        let total = files.iter().map(|(_, b)| b.len() as u64).sum();
        let whole = files.iter().all(|(p, b)| std::fs::read(folder.join(p)).is_ok_and(|on_disk| &on_disk == b));
        let (phase, done, error) = match how {
            _ if whole => (
                if how == (TorrentStart::Download { seed: false }) {
                    TorrentPhase::Finished
                } else {
                    TorrentPhase::Seeding
                },
                total,
                String::new(),
            ),
            TorrentStart::SeedOnly => (TorrentPhase::Failed, 0, "pieces are missing".to_owned()),
            TorrentStart::Download { .. } => (TorrentPhase::Checking, 0, String::new()),
        };
        state.started.push((id.to_owned(), how));
        state.running.insert(
            id.to_owned(),
            Running {
                folder: folder.to_owned(),
                how,
                status: TorrentStatus { phase, done, total, download_rate: 0, upload_rate: 0, peers: 0, error },
            },
        );
        Ok(())
    }

    async fn stop(&self, id: &str) -> PluginResult<()> {
        self.lock().running.remove(id);
        Ok(())
    }

    fn status(&self, id: &str) -> Option<TorrentStatus> {
        self.lock().running.get(id).map(|r| r.status.clone())
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
