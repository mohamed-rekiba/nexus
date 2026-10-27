//! Emulator build downloads: checks, resumes, the queue, pauses and redirects.

use super::*;

#[tokio::test]
async fn installing_downloads_checks_and_installs_the_build() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let mut events = t.core.events();

    install(&t.core).await;

    let installed = emu.installed().await.unwrap();
    assert_eq!(installed[0].version, "1.0");
    let emulators = t.core.view().emulators;
    assert_eq!(emulators.emulators[0].installed, vec!["1.0"]);
    assert!(!emulators.emulators[0].busy);
    let mut phases = vec![];
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.next()).await {
        if let Event::InstallChanged { state, .. } = event {
            phases.push(state);
        }
    }
    assert_eq!(phases, [InstallPhase::Downloading, InstallPhase::Installing, InstallPhase::Installed]);
}

#[tokio::test]
async fn an_interrupted_download_resumes_from_where_it_stopped() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, vec![7u8; 20_000]);
    host.http().cut_next_after(&emu.build_url, 9_000);
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    let first = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    assert!(first.is_err(), "the first attempt fails mid-way");
    install(&t.core).await;

    let offsets: Vec<u64> = t.http.requests().into_iter().map(|(_, offset)| offset).collect();
    assert_eq!(offsets, [0, 9_000]);
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_server_that_ignores_ranges_gets_a_clean_restart() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve(&emu.build_url, (0..20_000u32).map(|i| i as u8).collect::<Vec<_>>());
    host.http().cut_next_after(&emu.build_url, 9_000);
    host.http().ignore_ranges();
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    let _ = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    install(&t.core).await;

    assert_eq!(emu.installed().await.unwrap().len(), 1, "the checksum matched after the restart");
}

#[tokio::test]
async fn a_file_with_the_wrong_checksum_is_deleted_and_never_installed() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    host.http().serve(&emu.build_url, vec![1u8; 100]);
    emu.build_sha = "00".repeat(32);
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("checksum")), "{err}");
    assert!(emu.installed().await.unwrap().is_empty());
    let plugin_dir = t.data_dir().join("plugins/emu-a/downloads");
    assert!(std::fs::read_dir(&plugin_dir).map(|d| d.count()).unwrap_or(0) == 0, "no partial or bad file is left");
    assert_eq!(t.core.view().downloads.jobs[0].state, "failed");
}

#[tokio::test]
async fn a_download_from_a_host_outside_the_allowlist_is_refused() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_url = "https://evil.example/emu.bin".into();
    emu.build_sha = host.http().serve(&emu.build_url, vec![1u8; 10]);
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("evil.example")), "{err}");
    assert!(t.http.requests().is_empty(), "nothing was fetched");
}

#[tokio::test]
async fn an_install_a_crash_interrupted_resumes_at_the_next_start() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    let body = vec![7u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    // What the last run left: a half-downloaded part file and the saved install phase.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body[..5_000]).unwrap();
    std::fs::write(
        host.data_dir().join("config.json"),
        r#"{"app": {"installs": {"emu-a": {"version": "1.0", "from": null, "phase": "downloading"}}}}"#,
    )
    .unwrap();
    let (builder, dir, http, spawner, _) = host.plugin(emu.plugin("emu-a")).into_builder();
    let (kernel, _) = builder.boot().await;
    let mut events = kernel.events().stream();

    let _core = Core::start(kernel).await;

    loop {
        match events.next().await {
            Some(Event::InstallChanged { state: InstallPhase::Installed, .. }) => break,
            Some(_) => {}
            None => panic!("the events ended before the install did"),
        }
    }
    assert_eq!(http.requests(), [(emu.build_url.clone(), 5_000)], "it continued the part file");
    assert_eq!(emu.installed().await.unwrap()[0].version, "1.0");
    drop((dir, spawner));
}

#[tokio::test]
async fn a_download_a_crash_interrupted_resumes_at_the_next_start() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    let body = vec![9u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    // What the last run left: half the file, and the saved download.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body[..5_000]).unwrap();
    let record = serde_json::json!({"plugin": "emu-a", "request": {"type": "emulator_build", "version": null}});
    let config = serde_json::json!({"app": {"downloads": {"pending": record}}});
    std::fs::write(host.data_dir().join("config.json"), config.to_string()).unwrap();
    let (builder, dir, http, spawner, _) = host.plugin(emu.plugin("emu-a")).into_builder();
    let (kernel, _) = builder.boot().await;
    let mut events = kernel.events().stream();

    let core = Core::start(kernel).await;

    let finished = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(Event::DownloadFinished { .. }) => return,
                Some(Event::DownloadFailed { error, .. }) => panic!("{error}"),
                Some(_) => {}
                None => panic!("the events ended before the download did"),
            }
        }
    });
    finished.await.expect("the download resumed and finished");
    assert_eq!(http.requests(), [(emu.build_url.clone(), 5_000)], "it continued the part file");
    assert_eq!(std::fs::read(downloads.join("emu-1.0.bin")).unwrap(), body);
    let saved = core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_none_or(|m| m.is_empty()), "the finished download is no longer saved: {saved}");
    drop((dir, spawner));
}

/// Waits for the first event that `pick` maps to something.
async fn next_event<T>(
    events: &mut futures::stream::BoxStream<'static, Event>,
    pick: impl Fn(&Event) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.next().await.expect("the bus lives as long as the core");
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    })
    .await
    .expect("the event came")
}

fn job_state(core: &Core, job: u64) -> String {
    core.view().downloads.jobs.iter().find(|j| j.id == job).map(|j| j.state.clone()).unwrap_or_default()
}

#[tokio::test]
async fn a_download_is_saved_while_it_runs_and_forgotten_when_it_ends() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let gate = t.http.hold_after(&emu.build_url, 0);
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };

    t.core.dispatch(Command::Download { plugin: id("emu-a"), request }).await.unwrap();

    let saved = t.core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_some_and(|m| m.len() == 1), "dispatch returns while it runs: {saved}");
    gate.release();
    next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    let saved = t.core.kernel().config().get(&["app", "downloads"]);
    assert!(saved.as_object().is_some_and(|m| m.is_empty()), "{saved}");
}

#[tokio::test]
async fn downloads_beyond_the_limit_wait_in_the_queue() {
    let host = FakeHost::new().max_downloads(1);
    let mut a = FakeEmulator::new(vec!["A"]);
    a.build_sha = host.http().serve(&a.build_url, vec![1u8; 10_000]);
    let mut b = FakeEmulator::new(vec!["B"]);
    b.build_url = "https://builds.example/b-1.0.bin".into();
    b.build_sha = host.http().serve(&b.build_url, vec![2u8; 10_000]);
    let gate = host.http().hold_after(&a.build_url, 0);
    let t = host.plugin(a.plugin("emu-a")).plugin(b.plugin("emu-b")).boot_core().await;
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };

    t.core.dispatch(Command::Download { plugin: id("emu-a"), request: request.clone() }).await.unwrap();
    t.core.dispatch(Command::Download { plugin: id("emu-b"), request }).await.unwrap();
    next_event(&mut events, |e| matches!(e, Event::DownloadQueued { job: JobId(2), .. }).then_some(())).await;

    assert_eq!(job_state(&t.core, 2), "queued", "one runs at a time");
    gate.release();
    for _ in 0..2 {
        next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    }
    assert_eq!(t.http.requests().len(), 2);
    assert_eq!(job_state(&t.core, 2), "done");
}

#[tokio::test]
async fn a_paused_download_keeps_its_part_and_resumes_where_it_stopped() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let _gate = t.http.hold_after(&emu.build_url, 8_192);
    let mut events = t.core.events();
    let request = DownloadRequest::EmulatorBuild { version: None };
    t.core.dispatch(Command::Download { plugin: id("emu-a"), request }).await.unwrap();
    let job = next_event(&mut events, |e| match e {
        Event::DownloadProgress { job, received: 8_192, .. } => Some(*job),
        _ => None,
    })
    .await;

    t.core.dispatch(Command::PauseDownload(job)).await.unwrap();

    next_event(&mut events, |e| matches!(e, Event::DownloadPaused { .. }).then_some(())).await;
    assert_eq!(job_state(&t.core, job.0), "paused");
    let part = t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin.part");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), 8_192, "the part file stays");

    t.core.dispatch(Command::ResumeDownload(job)).await.unwrap();

    next_event(&mut events, |e| matches!(e, Event::DownloadFinished { .. }).then_some(())).await;
    let offsets: Vec<u64> = t.http.requests().into_iter().map(|(_, offset)| offset).collect();
    assert_eq!(offsets, [0, 8_192]);
    assert_eq!(std::fs::read(t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin")).unwrap(), vec![7u8; 20_000]);
}

#[tokio::test]
async fn cancelling_a_download_deletes_its_part_and_fails_the_install_that_waits_for_it() {
    let (t, emu) = core_with(FakeEmulator::new(vec!["A"])).await;
    let _gate = t.http.hold_after(&emu.build_url, 4_096);
    let mut events = t.core.events();
    let core = t.core.clone();
    let install =
        tokio::spawn(
            async move { core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await },
        );
    let job = next_event(&mut events, |e| match e {
        Event::DownloadProgress { job, received: 4_096, .. } => Some(*job),
        _ => None,
    })
    .await;

    t.core.dispatch(Command::CancelDownload(job)).await.unwrap();

    let err = install.await.unwrap().unwrap_err();
    assert!(matches!(err, CoreError::Cancelled(j) if j == job), "{err}");
    assert_eq!(job_state(&t.core, job.0), "cancelled");
    assert!(!t.data_dir().join("plugins/emu-a/downloads/emu-1.0.bin.part").exists(), "the part file is gone");
    assert!(emu.installed().await.unwrap().is_empty());
    let resumed = t.core.dispatch(Command::ResumeDownload(job)).await;
    assert!(resumed.is_err(), "a cancelled download can't resume");
}

#[tokio::test]
async fn a_part_file_that_already_has_every_byte_is_checked_without_a_request() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    let body = vec![7u8; 20_000];
    emu.build_sha = host.http().serve(&emu.build_url, body.clone());
    emu.build_size = Some(body.len() as u64);
    // The last run stopped after the last byte, before the check.
    let downloads = host.data_dir().join("plugins/emu-a/downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("emu-1.0.bin.part"), &body).unwrap();
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    install(&t.core).await;

    assert!(t.http.requests().is_empty(), "no request: {:?}", t.http.requests());
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_download_follows_redirects_inside_the_allowlist_and_keeps_its_range() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    let mirror = "https://cdn.builds.example/files/emu-1.0.bin";
    emu.build_sha = host.http().serve(mirror, vec![3u8; 20_000]);
    host.http().redirect(&emu.build_url, "//cdn.builds.example/files/emu-1.0.bin");
    host.http().cut_next_after(mirror, 9_000);
    let t = host.plugin(emu.plugin_with("emu-a", "network = [\"*.builds.example\"]")).boot_core().await;

    let _ = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await;
    install(&t.core).await;

    let requests = t.http.requests();
    let expected = [
        (emu.build_url.clone(), 0),
        (mirror.to_owned(), 0),
        (emu.build_url.clone(), 9_000),
        (mirror.to_owned(), 9_000),
    ];
    assert_eq!(requests, expected, "each hop asks for the same range");
    assert_eq!(emu.installed().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_out_of_the_allowlist_is_refused_before_it_is_fetched() {
    let host = FakeHost::new();
    let mut emu = FakeEmulator::new(vec!["A"]);
    emu.build_sha = host.http().serve("https://evil.example/emu.bin", vec![1u8; 10]);
    host.http().redirect(&emu.build_url, "https://evil.example/emu.bin");
    let t = host.plugin(emu.plugin("emu-a")).boot_core().await;

    let err = t.core.dispatch(Command::InstallEmulator { plugin: id("emu-a"), version: None }).await.unwrap_err();

    assert!(matches!(&err, CoreError::Download(m) if m.contains("evil.example")), "{err}");
    let fetched: Vec<String> = t.http.requests().into_iter().map(|(url, _)| url).collect();
    assert_eq!(fetched, ["https://builds.example/emu-1.0.bin"], "nothing past the refused hop");
}

#[tokio::test]
async fn a_plugin_s_own_requests_follow_redirects_only_inside_its_allowlist() {
    let host = FakeHost::new();
    host.http().serve("https://cdn.builds.example/index.json", b"{}".to_vec());
    host.http().redirect("https://builds.example/index.json", "https://cdn.builds.example/index.json");
    host.http().redirect("https://builds.example/away", "https://evil.example/x");
    let http: Arc<Mutex<Option<Arc<dyn HttpClient>>>> = Arc::default();
    let slot = http.clone();
    let plugin = StubPlugin::from_toml(
        "id = \"web\"\nkind = \"emulator\"\nname = \"W\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         [permissions]\nnetwork = [\"*.builds.example\"]",
    )
    .provides(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.http().clone());
        Ok(Provided::default())
    });
    let t = host.plugin(plugin).boot_core().await;
    t.core.kernel().provided(&id("web")).await.unwrap();
    let http = http.lock().unwrap().clone().unwrap();

    let ok = http.send(HttpRequest::get("https://builds.example/index.json")).await.unwrap();
    assert_eq!((ok.status, ok.body), (200, b"{}".to_vec()));
    let err = http.send(HttpRequest::get("https://builds.example/away")).await.unwrap_err();
    assert!(matches!(&err, PluginError::PermissionDenied(m) if m.contains("evil.example")), "{err:?}");
    assert!(!t.http.sent().iter().any(|u| u.contains("evil")), "{:?}", t.http.sent());
}
