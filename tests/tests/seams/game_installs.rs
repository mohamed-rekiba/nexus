//! Seam 1, core app API: installing a game from a package the player has. The plugin decodes
//! in the background; the host owns the job: it shows progress in Downloads, cancels it, and
//! cleans up after a crash.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_api::*;
use nexus_core::{Area, Clock, Command};
use nexus_testkit::{FakeHost, StubPlugin, TestCore};

const MANIFEST: &str = "id = \"emu-g\"\nkind = \"emulator\"\nname = \"G\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
    capabilities = [\"catalog\", \"slots\", \"game_installer\"]\n\
    [[permissions.filesystem]]\nfolder = \"packages\"\n\
    [[permissions.filesystem]]\nfolder = \"games\"\nwrite = true\n";

/// A "package" is a file that starts with `PKG!`. Installing one copies it, 64 KiB at a time,
/// into `games/<name>/data.bin`. A package called `panic.pkg` panics half way; one called
/// `burst.pkg` reports its progress 10,000 times first, as fast as it can.
struct Packages {
    ctx: PluginContext,
    hold: Hold,
}

/// What the next install waits for after its first piece: it goes on when the test sends, or
/// drops the sender.
type Hold = Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>>;

fn at(folder: &str, path: &str) -> HostPath {
    HostPath::new(folder, 0, RelPath::new(path).unwrap())
}

#[async_trait]
impl Catalog for Packages {
    async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
        let entries = self.ctx.folders().list(&HostPath::new("games", 0, RelPath::root())).await.unwrap_or_default();
        let items = entries
            .into_iter()
            .filter(|e| e.is_dir && !e.name.ends_with(".part"))
            .map(|e| GameSummary {
                id: e.name.clone(),
                title: e.name,
                platform: "PS5".into(),
                cover: None,
                tags: vec![],
                release_year: None,
                title_id: None,
                content_id: None,
            })
            .collect();
        Ok(Page { items, total: None })
    }
    async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        Ok(vec![])
    }
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        Err(PluginError::NotFound(game.into()))
    }
    async fn artwork(&self, _: &str, _: ArtworkKind) -> PluginResult<Option<ImageSource>> {
        Ok(None)
    }
}

#[async_trait]
impl Slots for Packages {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        if request.slot != SlotId::EmulatorPanel {
            return Ok(vec![]);
        }
        let mut items = Vec::new();
        for found in self.candidates().await? {
            items.push(SlotItem {
                id: format!("install-{}", found.label),
                order: Order::default(),
                content: SlotContent::Action {
                    label: format!("Install {}", found.label),
                    action: HOST_INSTALL_GAME.into(),
                    args: serde_json::to_value(&found.source).unwrap(),
                    style: ActionStyle::Secondary,
                    icon: None,
                },
            });
        }
        Ok(items)
    }

    async fn run_action(&self, action: &str, _: Option<&str>, _: &serde_json::Value) -> PluginResult<()> {
        Err(PluginError::NotFound(format!("action {action}: the host runs host: actions")))
    }
}

#[async_trait]
impl GameInstaller for Packages {
    async fn candidates(&self) -> PluginResult<Vec<Installable>> {
        let root = HostPath::new("packages", 0, RelPath::root());
        let mut out = Vec::new();
        for entry in self.ctx.folders().list(&root).await? {
            let Some(name) = entry.name.strip_suffix(".pkg") else { continue };
            out.push(Installable { source: at("packages", &entry.name), label: name.into() });
        }
        Ok(out)
    }

    async fn begin(&self, source: &HostPath, job: Arc<dyn InstallJob>) -> PluginResult<HostPath> {
        let folders = self.ctx.folders().clone();
        if folders.read_range(source, 0, 4).await? != b"PKG!" {
            return Err(PluginError::InvalidInput(format!("{} isn't a package", source.path)));
        }
        let size = folders.list(&HostPath::new("packages", 0, RelPath::root())).await?;
        let size = size.iter().find(|e| e.name == source.path.as_str()).map(|e| e.size).unwrap_or(0);
        let name = source.path.as_str().trim_end_matches(".pkg").to_owned();
        let (part, dest) = (at("games", &format!("{name}.part")), at("games", &name));
        let (source, work) = (source.clone(), part.clone());
        let handle = tokio::runtime::Handle::current();
        let hold = self.hold.lock().unwrap().take();
        tokio::task::spawn_blocking(move || {
            if name == "burst" {
                for n in 1..=10_000 {
                    job.progress(n, 10_000);
                }
            }
            let file = work.join("data.bin").unwrap();
            let mut done = 0;
            while done < size {
                if job.cancelled() {
                    let _ = handle.block_on(folders.remove(&work));
                    job.finished(Err(PluginError::Other("cancelled".into())));
                    return;
                }
                let piece = handle.block_on(folders.read_range(&source, done, 64 * 1024)).unwrap();
                handle.block_on(folders.write_at(&file, done, &piece)).unwrap();
                done += piece.len() as u64;
                job.progress(done, size);
                if name == "panic" && done * 2 >= size {
                    panic!("the decoder broke");
                }
                if let Some(hold) = &hold {
                    // Sent or dropped, it goes on.
                    let _ = hold.recv();
                }
            }
            handle.block_on(folders.rename(&work, &dest)).unwrap();
            job.finished(Ok(InstalledGame { dest, title: name }));
        });
        Ok(part)
    }
}

struct Booted {
    hold: Hold,
    test: TestCore,
    packages: tempfile::TempDir,
    games: tempfile::TempDir,
}

async fn boot_with(host: FakeHost, games: tempfile::TempDir) -> Booted {
    let hold = Hold::default();
    let held = hold.clone();
    let plugin = StubPlugin::from_toml(MANIFEST).provides(move |ctx| {
        let p = Arc::new(Packages { ctx: ctx.clone(), hold: held.clone() });
        Ok(Provided {
            catalog: Some(p.clone()),
            slots: Some(p.clone()),
            game_installer: Some(p),
            ..Provided::default()
        })
    });
    let test = host.plugin(plugin).boot_core().await;
    let packages = tempfile::tempdir().unwrap();
    for (name, folder) in [("packages", packages.path()), ("games", games.path())] {
        test.core.dispatch(Command::SetFolders { name: name.into(), paths: vec![folder.into()] }).await.unwrap();
    }
    Booted { hold, test, packages, games }
}

impl Booted {
    /// The next install stops after its first piece until the test sends on what this gives.
    fn hold(&self) -> std::sync::mpsc::Sender<()> {
        let (go, wait) = std::sync::mpsc::channel();
        *self.hold.lock().unwrap() = Some(wait);
        go
    }
}

async fn boot() -> Booted {
    boot_with(FakeHost::new(), tempfile::tempdir().unwrap()).await
}

fn package(b: &Booted, name: &str, size: usize) {
    let mut bytes = b"PKG!".to_vec();
    bytes.resize(size, 7);
    std::fs::write(b.packages.path().join(name), bytes).unwrap();
}

fn plugin() -> PluginId {
    PluginId::new("emu-g").unwrap()
}

fn install(source: &str) -> Command {
    Command::RunAction {
        plugin: plugin(),
        action: HOST_INSTALL_GAME.into(),
        game: None,
        args: serde_json::to_value(at("packages", source)).unwrap(),
    }
}

/// Waits until the install job is over, and returns its state.
async fn wait_for_end(t: &TestCore) -> String {
    let state = |core: &nexus_core::Core| core.view().downloads.jobs.first().map(|job| job.state.clone());
    t.until(Area::Installs, |core| state(core).is_some_and(|s| !matches!(s.as_str(), "queued" | "installing"))).await;
    state(&t.core).unwrap()
}

#[tokio::test]
async fn a_package_installs_through_its_action_and_shows_in_downloads_and_the_library() {
    let b = boot().await;
    package(&b, "astro.pkg", 300 * 1024);

    b.test.core.dispatch(install("astro.pkg")).await.unwrap();
    let job = b.test.core.view().downloads.jobs[0].clone();
    assert_eq!((job.label.as_str(), job.can_pause, job.can_cancel), ("Install astro.pkg", false, true));

    assert_eq!(wait_for_end(&b.test).await, "done");
    let copied = std::fs::read(b.games.path().join("astro/data.bin")).unwrap();
    assert_eq!(copied.len(), 300 * 1024);
    assert!(!b.games.path().join("astro.part").exists());
    assert_eq!(b.test.core.view().downloads.jobs[0].progress, 1.0);
    let titles: Vec<_> = b.test.core.view().home.games.into_iter().map(|g| g.title).collect();
    assert_eq!(titles, ["astro"], "the library shows the new game");
}

#[tokio::test]
async fn a_file_that_isn_t_a_package_fails_at_once() {
    let b = boot().await;
    std::fs::write(b.packages.path().join("notes.pkg"), b"hello").unwrap();
    let err = b.test.core.dispatch(install("notes.pkg")).await.unwrap_err();
    assert!(err.to_string().contains("isn't a package"), "{err}");
    assert!(b.test.core.view().downloads.jobs.is_empty());
}

#[tokio::test]
async fn cancelling_an_install_leaves_nothing_behind() {
    let b = boot().await;
    package(&b, "big.pkg", 4 * 1024 * 1024);
    let go = b.hold();
    b.test.core.dispatch(install("big.pkg")).await.unwrap();
    let id = b.test.core.view().downloads.jobs[0].id;
    // Part of it is written.
    b.test.until(Area::Installs, |core| core.view().downloads.jobs[0].progress > 0.0).await;
    b.test.core.dispatch(Command::CancelDownload(JobId(id))).await.unwrap();
    drop(go);

    assert_eq!(wait_for_end(&b.test).await, "cancelled");
    assert!(!b.games.path().join("big.part").exists());
    assert!(!b.games.path().join("big").exists());
}

#[tokio::test]
async fn an_install_whose_decoder_panics_fails_and_the_app_goes_on() {
    let b = boot().await;
    package(&b, "panic.pkg", 512 * 1024);
    b.test.core.dispatch(install("panic.pkg")).await.unwrap();

    assert_eq!(wait_for_end(&b.test).await, "failed");
    assert!(!b.test.core.view().notices.is_empty(), "the player is told");
    assert!(!b.games.path().join("panic.part").exists(), "the host removes what the broken install wrote");
    package(&b, "fine.pkg", 64 * 1024);
    b.test.core.dispatch(install("fine.pkg")).await.unwrap();
    let fine = |core: &nexus_core::Core| {
        core.view().downloads.jobs.iter().any(|j| j.label == "Install fine.pkg" && j.state == "done")
    };
    b.test.until(Area::Installs, fine).await;
    assert!(b.games.path().join("fine/data.bin").is_file(), "the next install ran");
}

#[tokio::test]
async fn a_work_folder_left_by_a_crash_is_removed_at_the_next_start() {
    let games = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(games.path().join("astro.part")).unwrap();
    std::fs::write(games.path().join("astro.part/data.bin"), b"half").unwrap();
    let host = FakeHost::new();
    // What the last run saved before it stopped.
    let record = serde_json::json!({ "1": { "plugin": "emu-g", "work": { "folder": "games", "root": 0, "path": "astro.part" } } });
    let config = host.data_dir().join("config.json");
    std::fs::write(
        &config,
        serde_json::json!({ "app": {
        "folders": { "games": [games.path().display().to_string()] },
        "game_installs": record,
    } })
        .to_string(),
    )
    .unwrap();

    let b = boot_with(host, games).await;
    assert!(!b.games.path().join("astro.part").exists(), "the half-written folder is gone");
    assert!(b.test.core.kernel().config().get(&["app", "game_installs"]).as_object().is_none_or(|m| m.is_empty()));
}

#[tokio::test]
async fn the_same_package_doesn_t_install_twice_at_once() {
    let b = boot().await;
    package(&b, "big.pkg", 4 * 1024 * 1024);
    let go = b.hold();
    b.test.core.dispatch(install("big.pkg")).await.unwrap();
    let err = b.test.core.dispatch(install("big.pkg")).await.unwrap_err();
    assert!(err.to_string().contains("already installing"), "{err}");
    drop(go);

    assert_eq!(wait_for_end(&b.test).await, "done");
    assert_eq!(std::fs::read(b.games.path().join("big/data.bin")).unwrap().len(), 4 * 1024 * 1024);
}

#[tokio::test]
async fn an_install_s_progress_never_waits_for_the_screen_and_redraws_a_few_times_a_second() {
    use futures::StreamExt;
    let b = boot().await;
    package(&b, "burst.pkg", 64 * 1024);
    let mut events = b.test.core.events();
    let started = b.test.clock.now();
    b.test.core.dispatch(install("burst.pkg")).await.unwrap();
    assert_eq!(wait_for_end(&b.test).await, "done");
    let took = b.test.clock.now() - started;
    let mut changes = 0;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.next()).await {
        if matches!(event, Event::StateChanged) {
            changes += 1;
        }
    }
    // 10,000 reports in a burst: a handful of redraws, at most five a second, not one each.
    let most = 20 + 5 * took.as_secs() as usize;
    assert!(changes <= most, "{changes} state changes in {took:?}");
}
