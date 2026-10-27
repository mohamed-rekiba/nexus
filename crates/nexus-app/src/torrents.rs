//! The host's BitTorrent: [`nexus_core::Torrents`] on librqbit 9.0.1.
//!
//! One session, made at the first use, with no local peer discovery, at most 50 peers, and
//! uploads capped at [`SEED_UPLOAD_LIMIT`]. A torrent's metadata is kept as `<info hash>.torrent`
//! in the data folder's `torrents/`, so a start after a restart doesn't ask the swarm for it
//! again. The core saves the downloads; librqbit keeps only which pieces each one was checked
//! to have, in `torrents/resume/`, so a launch reads one piece of each file instead of every
//! byte (fastresume). Stopping a torrent forgets its pieces, and its next start checks it all.
//!
//! A route binds every socket of the session to one network interface
//! (`SessionOptions::bind_device_name`): peers over TCP and uTP, the listener, DHT, UDP
//! trackers and UPnP. HTTP trackers go through reqwest's `interface`. On Linux that is
//! SO_BINDTODEVICE, on macOS IP_BOUND_IF. A new route, and [`Torrents::stop_all`], stop the
//! session; the next start makes a new one.
//!
//! A paused start adds the torrent paused, which checks the pieces on disk without
//! downloading. The adapter only reports what librqbit says ([`Torrents::stats`]): the core
//! decides what a finished or a paused torrent means, and unpauses or stops it.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions, TorrentStatsState,
};
use nexus_api::{PluginError, PluginResult};
use nexus_core::{SEED_UPLOAD_LIMIT, TorrentFile, TorrentInfo, TorrentStats, Torrents, TransferState};

type ManagedTorrentHandle = Arc<ManagedTorrent>;

/// How long reading a torrent's files from the swarm may take.
const RESOLVE_LIMIT: Duration = Duration::from_secs(180);
/// How long a stopped session's torrents get to write down which pieces they have.
const FLUSH_TIME: Duration = Duration::from_millis(200);

struct Running {
    handle: ManagedTorrentHandle,
}

pub struct RqbitTorrents {
    dir: PathBuf,
    /// Extra session settings: tests run without DHT, trackers or the network.
    local: Option<Local>,
    session: Mutex<Option<Arc<Session>>>,
    /// Held while a session is made or stopped, so there is one at a time.
    making: tokio::sync::Mutex<()>,
    /// The interface every socket binds to; `None` for any.
    bind: Mutex<Option<String>>,
    running: Mutex<HashMap<String, Running>>,
}

/// A session that only talks to `peers` on this computer, for tests.
#[derive(Clone)]
pub struct Local {
    pub peers: Vec<std::net::SocketAddr>,
}

fn error(e: impl std::fmt::Display) -> PluginError {
    PluginError::Network(e.to_string())
}

impl RqbitTorrents {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            local: None,
            session: Default::default(),
            making: Default::default(),
            bind: Default::default(),
            running: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn local(dir: PathBuf, peers: Vec<std::net::SocketAddr>) -> Self {
        Self { local: Some(Local { peers }), ..Self::new(dir) }
    }

    /// The session now, if there is one.
    fn current(&self) -> Option<Arc<Session>> {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    async fn session(&self) -> PluginResult<Arc<Session>> {
        let _making = self.making.lock().await;
        if let Some(session) = self.current() {
            return Ok(session);
        }
        // librqbit keeps the pieces each torrent was checked to have, by its info hash, so a
        // launch doesn't read every finished download again. Its list of torrents goes first:
        // the core starts the ones it wants.
        let resume = self.dir.join("resume");
        if let Err(e) = tokio::fs::remove_file(resume.join("session.json")).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "could not clear the torrent session's list");
        }
        let mut opts = SessionOptions {
            persistence: Some(librqbit::SessionPersistenceConfig::Json { folder: Some(resume) }),
            fastresume: true,
            disable_local_service_discovery: true,
            peer_limit: Some(50),
            concurrent_init_limit: Some(2),
            ratelimits: librqbit::limits::LimitsConfig {
                upload_bps: NonZeroU32::new(SEED_UPLOAD_LIMIT as u32),
                download_bps: None,
            },
            bind_device_name: self.bind.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            ..Default::default()
        };
        if self.local.is_some() {
            opts.dht = None;
            opts.disable_trackers = true;
            opts.listen = Some(librqbit::ListenerOptions {
                mode: librqbit::ListenerMode::TcpOnly,
                listen_addr: "127.0.0.1:0".parse().expect("an address"),
                enable_upnp_port_forwarding: false,
                ..Default::default()
            });
        }
        let session = Session::new_with_opts(self.dir.join("unused-default-output"), opts).await.map_err(error)?;
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(session.clone());
        Ok(session)
    }

    fn metadata_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.torrent"))
    }

    fn peers(&self) -> Option<Vec<std::net::SocketAddr>> {
        self.local.as_ref().map(|l| l.peers.clone())
    }

    fn handle(&self, id: &str) -> Option<ManagedTorrentHandle> {
        self.running.lock().unwrap_or_else(|p| p.into_inner()).get(id).map(|r| r.handle.clone())
    }
}

#[async_trait]
impl Torrents for RqbitTorrents {
    async fn resolve(&self, magnet: &str) -> PluginResult<TorrentInfo> {
        let session = self.session().await?;
        let options = AddTorrentOptions { list_only: true, initial_peers: self.peers(), ..Default::default() };
        let answer =
            tokio::time::timeout(RESOLVE_LIMIT, session.add_torrent(AddTorrent::from_url(magnet), Some(options)))
                .await
                .map_err(|_| PluginError::Network("no peer answered in 3 minutes: try again later".into()))?
                .map_err(error)?;
        let AddTorrentResponse::ListOnly(listed) = answer else {
            return Err(PluginError::Other("the torrent engine didn't list the files".into()));
        };
        let id = listed.info_hash.as_string();
        if let Err(e) = tokio::fs::create_dir_all(&self.dir).await {
            tracing::warn!(error = %e, "could not make the torrents folder");
        }
        if let Err(e) = tokio::fs::write(self.metadata_path(&id), &listed.torrent_bytes).await {
            tracing::warn!(error = %e, "could not keep a torrent's metadata");
        }
        let files = listed
            .info
            .iter_file_details()
            .map(|f| TorrentFile { path: f.filename.to_vec().join("/"), size: f.len })
            .collect();
        let name = listed.info.info().name.as_ref().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned());
        Ok(TorrentInfo { name: name.unwrap_or_default(), files })
    }

    async fn start(&self, id: &str, magnet: &str, folder: &Path, paused: bool) -> PluginResult<()> {
        let session = self.session().await?;
        let add = match tokio::fs::read(self.metadata_path(id)).await {
            Ok(bytes) => AddTorrent::from_bytes(bytes),
            Err(_) => AddTorrent::from_url(magnet),
        };
        let trackers = librqbit::Magnet::parse(magnet).map(|m| m.trackers).unwrap_or_default();
        let options = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: Some(folder.to_string_lossy().into_owned()),
            trackers: Some(trackers),
            initial_peers: self.peers(),
            ..Default::default()
        };
        let handle = session
            .add_torrent(add, Some(options))
            .await
            .map_err(error)?
            .into_handle()
            .ok_or_else(|| PluginError::Other("the torrent engine gave no handle".into()))?;
        // A torrent the session already has: a resume.
        if !paused && matches!(handle.stats().state, TorrentStatsState::Paused) {
            session.unpause(&handle).await.map_err(error)?;
        }
        self.running.lock().unwrap_or_else(|p| p.into_inner()).insert(id.to_owned(), Running { handle });
        Ok(())
    }

    async fn unpause(&self, id: &str) -> PluginResult<()> {
        let (Some(handle), Some(session)) = (self.handle(id), self.current()) else { return Ok(()) };
        // Already live: librqbit can start a torrent on its own between the stats read and
        // this, and then says "already live", which is no fault.
        if !handle.is_paused() {
            return Ok(());
        }
        match session.unpause(&handle).await {
            Err(e) if !handle.is_paused() => {
                tracing::debug!(error = %e, "a torrent was already live");
                Ok(())
            }
            done => done.map_err(error),
        }
    }

    async fn stop(&self, id: &str) -> PluginResult<()> {
        let Some(running) = self.running.lock().unwrap_or_else(|p| p.into_inner()).remove(id) else { return Ok(()) };
        let Some(session) = self.current() else { return Ok(()) };
        if matches!(running.handle.stats().state, TorrentStatsState::Live | TorrentStatsState::Initializing { .. }) {
            let _ = session.pause(&running.handle).await;
        }
        // The files stay: only the session forgets it.
        session.delete(running.handle.id().into(), false).await.map_err(error)
    }

    fn stats(&self, id: &str) -> Option<TorrentStats> {
        let stats = self.handle(id)?.stats();
        let live = stats.live.as_ref();
        let rate = |mbps: f64| (mbps * 1024.0 * 1024.0) as u64;
        Some(TorrentStats {
            state: match stats.state {
                TorrentStatsState::Initializing { .. } => TransferState::Checking,
                TorrentStatsState::Live => TransferState::Live,
                TorrentStatsState::Paused => TransferState::Paused,
                TorrentStatsState::Error => TransferState::Error,
            },
            finished: stats.finished,
            done: stats.progress_bytes,
            total: stats.total_bytes,
            download_rate: live.map(|l| rate(l.download_speed.mbps)).unwrap_or(0),
            upload_rate: live.map(|l| rate(l.upload_speed.mbps)).unwrap_or(0),
            peers: live.map(|l| l.snapshot.peer_stats.live).unwrap_or(0),
            error: stats.error.clone().unwrap_or_default(),
        })
    }

    async fn route(&self, interface: Option<String>) -> PluginResult<()> {
        {
            let mut bind = self.bind.lock().unwrap_or_else(|p| p.into_inner());
            if *bind == interface {
                return Ok(());
            }
            *bind = interface;
        }
        self.stop_all().await
    }

    async fn stop_all(&self) -> PluginResult<()> {
        let _making = self.making.lock().await;
        self.running.lock().unwrap_or_else(|p| p.into_inner()).clear();
        let session = self.session.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(session) = session {
            // It pauses every torrent and closes the listener, DHT and trackers. It waits a
            // second by itself.
            if tokio::time::timeout(Duration::from_secs(5), session.stop()).await.is_err() {
                tracing::warn!("the torrent session took more than 5 s to stop");
            }
            // Its torrents go with it, and each hands its pieces to a task that writes them
            // down: a moment for that before the caller goes on, maybe to quit.
            drop(session);
            tokio::time::sleep(FLUSH_TIME).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in, `NEXUS_TORRENT_TESTS=1`: a real transfer between two librqbit sessions on this
    /// computer, with no DHT, trackers or other network. The payload is made-up bytes.
    #[test]
    fn a_torrent_downloads_finishes_and_seeds_again_only_when_its_files_are_whole() {
        if std::env::var_os("NEXUS_TORRENT_TESTS").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("nexus-torrent-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let mut x = 0x1234_5678u32;
            let payload: Vec<u8> = (0..512 * 1024)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let source = root.join("seed/fixture.bin");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, &payload).unwrap();
            let seeder = Session::new_with_opts(
                root.join("seed"),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    persistence: None,
                    fastresume: false,
                    listen: Some(librqbit::ListenerOptions {
                        mode: librqbit::ListenerMode::TcpOnly,
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let (made, _served) = seeder
                .create_and_serve_torrent(
                    &source,
                    librqbit::CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(65536) },
                )
                .await
                .unwrap();
            let magnet = made.as_magnet().to_string();
            let id = made.info_hash().as_string();
            let torrents = RqbitTorrents::local(root.join("leech/torrents"), vec![seeder.listen_addr().unwrap()]);

            let info = torrents.resolve(&magnet).await.unwrap();
            assert_eq!(info.files, [TorrentFile { path: "fixture.bin".into(), size: payload.len() as u64 }]);

            let out = root.join("leech/out");
            async fn until(
                torrents: &RqbitTorrents,
                id: &str,
                what: &str,
                done: fn(&TorrentStats) -> bool,
            ) -> TorrentStats {
                for _ in 0..600 {
                    if let Some(stats) = torrents.stats(id).filter(done) {
                        return stats;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                panic!("never {what}: {:?}", torrents.stats(id));
            }
            let checked = |s: &TorrentStats| s.state != TransferState::Checking;
            torrents.start(&id, &magnet, &out, false).await.unwrap();
            until(&torrents, &id, "finished", |s| s.finished && s.state == TransferState::Live).await;
            assert_eq!(std::fs::read(out.join("fixture.bin")).unwrap(), payload);
            torrents.stop(&id).await.unwrap();

            // Again, paused: the files are whole, so its check finishes it, and it connects
            // only when the core unpauses it.
            torrents.start(&id, &magnet, &out, true).await.unwrap();
            let whole = until(&torrents, &id, "checked", checked).await;
            assert_eq!((whole.state, whole.finished), (TransferState::Paused, true));
            torrents.unpause(&id).await.unwrap();
            until(&torrents, &id, "live", |s| s.state == TransferState::Live).await;
            torrents.stop(&id).await.unwrap();

            // A file changed: the check finds a piece missing, and nothing downloads.
            let mut broken = payload.clone();
            broken[1000] ^= 0xff;
            std::fs::write(out.join("fixture.bin"), &broken).unwrap();
            torrents.start(&id, &magnet, &out, true).await.unwrap();
            let missing = until(&torrents, &id, "checked", checked).await;
            assert_eq!((missing.state, missing.finished), (TransferState::Paused, false));
            torrents.stop(&id).await.unwrap();
            assert_eq!(std::fs::read(out.join("fixture.bin")).unwrap(), broken, "nothing was downloaded");

            // Whole again, and the launcher closed: the next launch trusts the pieces it checked
            // before instead of reading every file again. It reads one piece of each file, so a
            // change in another piece shows only here, as a check that finds it whole.
            std::fs::write(out.join("fixture.bin"), &payload).unwrap();
            torrents.start(&id, &magnet, &out, true).await.unwrap();
            until(&torrents, &id, "checked", checked).await;
            torrents.stop_all().await.unwrap();
            let mut last_piece_changed = payload.clone();
            last_piece_changed[payload.len() - 1000] ^= 0xff;
            std::fs::write(out.join("fixture.bin"), &last_piece_changed).unwrap();
            let relaunched = RqbitTorrents::local(root.join("leech/torrents"), vec![seeder.listen_addr().unwrap()]);
            relaunched.start(&id, &magnet, &out, true).await.unwrap();
            let trusted = until(&relaunched, &id, "checked", checked).await;
            assert_eq!((trusted.state, trusted.finished), (TransferState::Paused, true), "not checked again");
            relaunched.stop_all().await.unwrap();
            let _ = std::fs::remove_dir_all(&root);
        });
    }
}
