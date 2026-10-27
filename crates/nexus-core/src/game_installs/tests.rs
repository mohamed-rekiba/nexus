//! The subsystem on its own, over fake ports: no kernel, no plugins, no core.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nexus_api::{GameInstaller, HostPath, InstallJob, Installable, InstalledGame, PluginError, PluginId, RelPath};

use super::*;
use crate::archives::{ArchiveSet, Unpack};
use crate::changes::{Change, Changes};
use crate::config::ConfigService;
use crate::{ArchiveError, ArchiveKind, EventBus, NoHardware, TokioClock, UnpackProgress};

fn plugin() -> PluginId {
    PluginId::new("emu-g").unwrap()
}

fn at(folder: &str, path: &str) -> HostPath {
    HostPath::new(folder, 0, RelPath::new(path).unwrap())
}

/// The downloads, kept in memory.
#[derive(Default)]
struct FakeDownloads(Mutex<BTreeMap<String, TorrentJob>>);

impl TorrentDownloads for FakeDownloads {
    fn job(&self, id: &str) -> Option<TorrentJob> {
        self.0.lock().unwrap().get(id).cloned()
    }

    fn jobs(&self) -> Vec<TorrentJob> {
        self.0.lock().unwrap().values().cloned().collect()
    }

    fn hand_off(&self, id: &str, change: &mut dyn FnMut(&TorrentState, &mut HandOff)) -> bool {
        let mut jobs = self.0.lock().unwrap();
        let Some(job) = jobs.get_mut(id) else { return false };
        change(&job.state, &mut job.hand_off);
        true
    }
}

/// Writes one file for each set, named after its first part.
struct FakeArchives;

#[async_trait]
impl Archives for FakeArchives {
    async fn unpack(
        &self,
        _kind: ArchiveKind,
        parts: &[PathBuf],
        into: &Path,
        _free: Option<u64>,
        progress: UnpackProgress,
        _cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError> {
        let name = parts[0].file_name().unwrap();
        std::fs::write(into.join(name), b"game").unwrap();
        progress(1, 1);
        Ok(())
    }
}

/// Installs a package into `games/<name>`, through the work folder `games/<name>.part`. A
/// package called `broken` fails and leaves its work folder behind.
struct FakeInstaller {
    games: PathBuf,
}

#[async_trait]
impl GameInstaller for FakeInstaller {
    async fn candidates(&self) -> nexus_api::PluginResult<Vec<Installable>> {
        Ok(Vec::new())
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> nexus_api::PluginResult<HostPath> {
        let name = source.path.as_str().rsplit('/').next().unwrap().to_owned();
        let work = self.games.join(format!("{name}.part"));
        std::fs::create_dir_all(&work).unwrap();
        let games = self.games.clone();
        let title = name.clone();
        tokio::spawn(async move {
            job.progress(1, 2);
            if title == "broken" {
                job.finished(Err(PluginError::Other("the package is broken".into())));
                return;
            }
            std::fs::rename(work, games.join(&title)).unwrap();
            job.finished(Ok(InstalledGame { dest: at("games", &title), title }));
        });
        Ok(at("games", &format!("{name}.part")))
    }
}

/// What the core heard, with the job's phase at that moment.
type Heard = Arc<Mutex<Vec<(String, Option<GamePhase>)>>>;

struct Setup {
    installs: GameInstallsSubsystem,
    downloads: Arc<FakeDownloads>,
    config: Arc<ConfigService>,
    changes: Changes,
    heard: Heard,
    /// Told each time the core hears something.
    told: Arc<tokio::sync::Notify>,
    games: tempfile::TempDir,
    packages: tempfile::TempDir,
}

fn setup() -> Setup {
    let config = Arc::new(ConfigService::in_memory());
    let (games, packages) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let folders = serde_json::json!({
        "games": [games.path().display().to_string()],
        "packages": [packages.path().display().to_string()],
    });
    config.set(&["app", "folders"], folders).unwrap();
    let downloads = Arc::new(FakeDownloads::default());
    let changes = Changes::new();
    let heard = Heard::default();
    let told = Arc::new(tokio::sync::Notify::new());
    // The installs are read through this, once the subsystem is made.
    let reader: Arc<Mutex<Option<GameInstallsSubsystem>>> = Arc::default();
    let events: EventSink = {
        let (heard, told, reader) = (heard.clone(), told.clone(), reader.clone());
        Arc::new(move |event| {
            let installs = reader.lock().unwrap().clone().unwrap();
            let phase = installs.snapshot().jobs.values().next().map(|j| j.phase);
            let name = match event {
                InstallEvent::Installed => "installed".to_owned(),
                InstallEvent::Failed(error) => format!("failed: {error}"),
                InstallEvent::Unpacked { source, into, title, .. } => {
                    format!("unpacked {title} from {} into {:?}", source.path, into.map(|i| i.path.to_string()))
                }
            };
            heard.lock().unwrap().push((name, phase));
            told.notify_one();
            Box::pin(async {})
        })
    };
    let installs = GameInstallsSubsystem::new(
        InstallPorts {
            archives: Arc::new(FakeArchives),
            hardware: Arc::new(NoHardware),
            clock: Arc::new(TokioClock),
            downloads: downloads.clone(),
            call_timeout: Duration::from_secs(5),
        },
        Records::new(config.clone()),
        Notifier::new(changes.clone(), EventBus::new(16)),
        events,
    );
    *reader.lock().unwrap() = Some(installs.clone());
    Setup { installs, downloads, config, changes, heard, told, games, packages }
}

impl Setup {
    fn folders(&self) -> HostFolders {
        let permissions = serde_json::from_value(serde_json::json!({
            "filesystem": [{ "folder": "games", "write": true }, { "folder": "packages", "write": true }],
        }))
        .unwrap();
        HostFolders { plugin: plugin(), permissions, config: self.config.clone(), home: None, platform: None }
    }

    fn new_install(&self, id: u64, source: HostPath) -> NewInstall {
        NewInstall {
            id: JobId(id),
            plugin: plugin(),
            source,
            into: None,
            installer: Arc::new(FakeInstaller { games: self.games.path().to_owned() }),
            folders: self.folders(),
        }
    }

    fn phase(&self, id: u64) -> Option<GamePhase> {
        self.installs.snapshot().jobs.get(&JobId(id)).map(|j| j.phase)
    }

    fn heard(&self) -> Vec<(String, Option<GamePhase>)> {
        self.heard.lock().unwrap().clone()
    }

    /// A finished download of one archive, `game.zip`, in `packages/torrent-abc`.
    fn finished_download(&self) -> PathBuf {
        let folder = self.packages.path().join("torrent-abc");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("game.zip"), b"zip").unwrap();
        let set =
            ArchiveSet { name: "game".into(), kind: ArchiveKind::Zip, parts: vec!["game.zip".into()], problem: None };
        let job = TorrentJob {
            id: "abc".into(),
            offer: nexus_api::GameKey::new(PluginId::new("catalog").unwrap(), "astro"),
            emulator: plugin(),
            title: "Astro".into(),
            version: String::new(),
            region: String::new(),
            magnet: String::new(),
            source: at("packages", "torrent-abc"),
            folder: folder.clone(),
            state: TorrentState::Complete,
            done: 3,
            total: 3,
            files: Vec::new(),
            file_count: 1,
            error: String::new(),
            download_rate: 0,
            upload_rate: 0,
            peers: 0,
            seeding: false,
            verifying: false,
            waiting_vpn: false,
            start: None,
            hand_off: HandOff {
                unpack: Unpack { sets: vec![set], phase: UnpackPhase::Waiting, ..Unpack::default() },
                ..HandOff::default()
            },
        };
        self.downloads.0.lock().unwrap().insert(job.id.clone(), job);
        folder
    }
}

/// Waits until `done` holds, looking after each change of the installs.
async fn until(changes: &Changes, mut done: impl FnMut() -> bool) {
    let mut rx = changes.subscribe();
    while !done() {
        match rx.recv().await {
            Ok(Change { .. }) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(e) => panic!("the changes stopped: {e}"),
        }
    }
}

#[test]
fn install_starts_at_once_unpacks_first_or_waits_by_the_unpack_phase() {
    use UnpackPhase as U;
    let table = [
        (U::None, Step::Install),
        (U::Unpacked, Step::Install),
        (U::Waiting, Step::Unpack),
        (U::Failed, Step::Unpack),
        (U::Cancelled, Step::Unpack),
        (U::Unpacking, Step::Nothing),
    ];
    for (phase, step) in table {
        assert_eq!(first_step(phase), step, "{phase:?}");
    }
}

#[tokio::test]
async fn an_install_shows_as_done_only_after_the_core_heard_of_the_game() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "astro"))).await.unwrap();
    until(&s.changes, || s.phase(1) == Some(GamePhase::Done)).await;

    assert_eq!(s.heard(), [("installed".to_owned(), Some(GamePhase::Installing))]);
    assert!(s.games.path().join("astro").is_dir());
    let records = s.config.get(&["app", "game_installs"]);
    assert!(records.as_object().is_none_or(|r| r.is_empty()), "no work folder to clean up: {records}");
    let job = s.installs.snapshot().jobs[&JobId(1)].clone();
    assert_eq!((job.done, job.total), (2, 2), "a finished job is full");
}

#[tokio::test]
async fn a_failed_install_tells_the_player_first_and_removes_its_work_folder() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "broken"))).await.unwrap();
    until(&s.changes, || s.phase(1) == Some(GamePhase::Failed)).await;

    let heard = s.heard();
    assert_eq!(heard, [("failed: the package is broken".to_owned(), Some(GamePhase::Installing))]);
    assert!(!s.games.path().join("broken.part").exists(), "the host removes what the plugin left");
}

#[tokio::test]
async fn the_same_source_doesn_t_install_twice_at_once() {
    let s = setup();
    s.installs.begin(s.new_install(1, at("packages", "astro"))).await.unwrap();
    // The plugin's work runs once this test waits.
    let again = s.installs.begin(s.new_install(2, at("packages", "astro"))).await;
    assert!(again.unwrap_err().to_string().contains("already installing"));
    assert_eq!(s.phase(2), None, "the refused install isn't listed");
}

#[tokio::test]
async fn install_unpacks_a_download_s_archives_first_then_asks_the_core_to_install() {
    let s = setup();
    let folder = s.finished_download();

    assert_eq!(s.installs.install_download("abc", at("games", "PS5")), None, "it unpacks first");
    // The hand-off changes publish as the downloads' own, which the fake doesn't: the event
    // says the unpack ended.
    s.told.notified().await;

    assert_eq!(s.downloads.job("abc").unwrap().hand_off.unpack.phase, UnpackPhase::Unpacked);
    assert_eq!(std::fs::read(folder.join("unpacked/game/game.zip")).unwrap(), b"game");
    assert_eq!(s.heard(), [("unpacked Astro from torrent-abc into Some(\"PS5\")".to_owned(), None)]);
    let job = s.downloads.job("abc").unwrap();
    assert_eq!(job.hand_off.unpack.progress, 1.0);
    assert_eq!(job.hand_off.install_into, Some(at("games", "PS5")));

    // Unpacked: Install starts the install at once.
    let now = s.installs.install_download("abc", at("games", "PS5"));
    assert_eq!(now, Some((plugin(), at("packages", "torrent-abc"))));
}

#[test]
fn the_work_folder_a_crash_left_goes_at_the_next_start() {
    let s = setup();
    std::fs::create_dir_all(s.games.path().join("astro.part")).unwrap();
    let records = serde_json::json!({ "7": { "plugin": "emu-g", "work": at("games", "astro.part") } });
    s.config.set(&["app", "game_installs"], records).unwrap();

    s.installs.clean_up(|p| (p == &plugin()).then(|| s.folders()));

    assert!(!s.games.path().join("astro.part").exists());
    assert_eq!(s.config.get(&["app", "game_installs"]), serde_json::json!({}));
}
