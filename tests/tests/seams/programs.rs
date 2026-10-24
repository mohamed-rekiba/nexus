//! Seam 1, core app API: a plugin finds and briefly runs the programs its manifest may launch,
//! for example to read a build's version. The host applies the launch rules, caps the time and
//! the output, and searches the system's `PATH` itself.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_api::*;
use nexus_testkit::{FakeHost, StubPlugin, TestCore};

const MANIFEST: &str = "id = \"prober\"\nkind = \"emulator\"\nname = \"P\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
    [permissions]\nprocess = [\"folder:builds\", \"system:emu\"]\n\
    [[permissions.filesystem]]\nfolder = \"builds\"\n\
    [[permissions.filesystem]]\nfolder = \"other\"\n";

async fn boot(host: FakeHost) -> (TestCore, Arc<dyn Programs>) {
    let programs: Arc<Mutex<Option<Arc<dyn Programs>>>> = Arc::default();
    let slot = programs.clone();
    let plugin = StubPlugin::from_toml(MANIFEST).provides(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.programs().clone());
        Ok(Provided::default())
    });
    let t = host.plugin(plugin).boot_core().await;
    t.core.kernel().provided(&PluginId::new("prober").unwrap()).await.unwrap();
    let programs = programs.lock().unwrap().clone().expect("init ran");
    (t, programs)
}

fn map(t: &TestCore, name: &str, root: &Path) {
    t.core.kernel().config().set(&["app", "folders", name], serde_json::json!([root.to_str().unwrap()])).unwrap();
}

fn in_folder(folder: &str, path: &str) -> Location {
    Location::Folder(HostPath::new(folder, 0, RelPath::new(path).unwrap()))
}

#[tokio::test]
async fn a_program_in_a_granted_folder_is_found_and_its_output_read() {
    let (t, programs) = boot(FakeHost::new()).await;
    let builds = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(builds.path().join("bin")).unwrap();
    std::fs::write(builds.path().join("bin/emu"), b"#!/bin/sh\n").unwrap();
    map(&t, "builds", builds.path());
    let real = builds.path().join("bin/emu");
    t.spawner.prints(&real, b"emu git = 6799ecb, date = 2026.09.29\n");

    let program = in_folder("builds", "bin/emu");
    assert_eq!(programs.find(&program).await.unwrap(), Some(real.display().to_string()));
    assert_eq!(programs.find(&in_folder("builds", "bin/missing")).await.unwrap(), None);
    let out = programs.output(&program, &["--help".into()], Duration::from_secs(3)).await.unwrap();
    assert_eq!(out, b"emu git = 6799ecb, date = 2026.09.29\n");
    let run = t.spawner.ran();
    assert_eq!(run.last().unwrap().args, ["--help"]);
    assert_eq!(run.last().unwrap().cwd, builds.path().join("bin"), "it runs in its own folder");
}

#[tokio::test]
async fn only_programs_the_manifest_may_launch_are_found_or_run() {
    let (t, programs) = boot(FakeHost::new()).await;
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("tool"), b"x").unwrap();
    map(&t, "other", other.path());
    t.spawner.on_path("wine", Path::new("/usr/bin/wine"));

    let denied = |r: PluginResult<Option<String>>| matches!(r, Err(PluginError::PermissionDenied(_)));
    assert!(denied(programs.find(&in_folder("other", "tool")).await), "no folder:other rule");
    assert!(denied(programs.find(&Location::System("wine".into())).await), "no system:wine rule");
    let run = programs.output(&in_folder("other", "tool"), &[], Duration::from_secs(1)).await;
    assert!(matches!(run, Err(PluginError::PermissionDenied(_))));
    assert!(t.spawner.ran().is_empty());
}

#[tokio::test]
async fn a_system_program_is_found_on_the_path_by_the_host() {
    let (t, programs) = boot(FakeHost::new()).await;
    let system = Location::System("emu".into());
    assert_eq!(programs.find(&system).await.unwrap(), None, "not on PATH");

    t.spawner.on_path("emu", Path::new("/opt/emu/bin/emu"));
    t.spawner.prints(Path::new("/opt/emu/bin/emu"), b"v1");
    assert_eq!(programs.find(&system).await.unwrap(), Some("/opt/emu/bin/emu".into()));
    assert_eq!(programs.output(&system, &[], Duration::from_secs(1)).await.unwrap(), b"v1");
    assert_eq!(t.spawner.ran().last().unwrap().cwd, Path::new("/opt/emu/bin"));
}

#[tokio::test]
async fn a_program_that_runs_too_long_is_stopped_and_long_output_is_cut() {
    let (t, programs) = boot(FakeHost::new()).await;
    let builds = tempfile::tempdir().unwrap();
    std::fs::write(builds.path().join("slow"), b"x").unwrap();
    std::fs::write(builds.path().join("chatty"), b"x").unwrap();
    map(&t, "builds", builds.path());
    t.spawner.hangs(&builds.path().join("slow"));
    t.spawner.prints(&builds.path().join("chatty"), &vec![b'a'; 200 * 1024]);

    // A plugin can't ask for more than the host's limit.
    let slow = programs.output(&in_folder("builds", "slow"), &[], Duration::from_secs(3600)).await;
    assert!(matches!(slow, Err(PluginError::Timeout(ms)) if ms == MAX_PROGRAM_TIME.as_millis() as u64), "{slow:?}");
    let chatty = programs.output(&in_folder("builds", "chatty"), &[], Duration::from_secs(1)).await.unwrap();
    assert_eq!(chatty.len(), MAX_PROGRAM_OUTPUT);
}
