//! The real KytyPS5 and shadPS4 releases, downloaded from GitHub and installed through the
//! plugins with the app's own HTTP transport and process spawner. Opt-in, because it needs
//! the network and downloads hundreds of megabytes: set `NEXUS_REAL_RELEASES=1`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nexus_api::{Platform, PluginId};
use nexus_core::{Command, Core, HostConfig, Kernel, ProcessSpawner, Route, SpawnCommand, TokioSpawner};

fn wanted() -> bool {
    let on = std::env::var_os("NEXUS_REAL_RELEASES").is_some();
    if !on {
        eprintln!("SKIPPED: set NEXUS_REAL_RELEASES=1 to download the real releases");
    }
    on
}

/// A fresh data folder under the system's temporary folder.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("launcher-real-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn core(dir: &Path) -> Core {
    let mut host = HostConfig::new(dir);
    // Downloads of a few hundred megabytes need more than the default call timeout.
    host.call_timeout = Duration::from_secs(600);
    let (kernel, report) = Kernel::builder(host)
        .http(Arc::new(crate::http::ReqwestTransport::new().unwrap()))
        .built_in_plugins()
        .boot()
        .await;
    // A plugin with no build for this system is left out on purpose.
    let broken: Vec<_> =
        report.rejected.iter().filter(|r| !matches!(r.error, nexus_core::LoadError::Platform { .. })).collect();
    assert!(broken.is_empty(), "{broken:?}");
    Core::start(kernel).await
}

/// Installs the newest build of `plugin` and returns what the Emulators screen shows.
async fn install(core: &Core, plugin: &str) -> Vec<String> {
    let plugin = PluginId::new(plugin).unwrap();
    core.dispatch(Command::InstallEmulator { plugin: plugin.clone(), version: None }).await.unwrap();
    core.dispatch(Command::Navigate(Route::Emulators)).await.unwrap();
    let view = core.view();
    view.emulators.emulators.into_iter().find(|e| e.plugin == plugin).map(|e| e.installed).unwrap_or_default()
}

/// The first file called `name` under `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == name) && path.is_file() {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find(&path, name)
        {
            return Some(found);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_kytyps5_installs_and_prints_its_version() {
    if !wanted() {
        return;
    }
    // This system's default release repo: on macOS, the macOS test builds.
    kytyps5_from(None, "kyty").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_upstream_kytyps5_installs_and_prints_its_version() {
    if !wanted() {
        return;
    }
    kytyps5_from(Some("KytyPS5/KytyPS5"), "kyty-upstream").await;
}

/// Installs the newest KytyPS5 from `repo` (or the default one) and runs its `--help`.
async fn kytyps5_from(repo: Option<&str>, name: &str) {
    let dir = data_dir(name);
    let core = core(&dir).await;
    if let Some(repo) = repo {
        let plugin = PluginId::new("emu-kyty").unwrap();
        let set = Command::SetConfigField {
            plugin,
            scope: nexus_api::ConfigScope::Emulator,
            key: "release_repo".into(),
            value: repo.into(),
        };
        core.dispatch(set).await.unwrap();
    }
    let installed = install(&core, "emu-kyty").await;
    assert_eq!(installed.len(), 1, "{installed:?}");
    eprintln!("KytyPS5 installed: {installed:?}");

    let builds = dir.join("plugins/emu-kyty/builds");
    let binary = find(&builds, "kyty_emulator").or_else(|| find(&builds, "kyty_emulator.exe")).expect("the binary");
    let command = SpawnCommand {
        program: binary.clone(),
        args: vec!["--help".into()],
        env: Default::default(),
        cwd: binary.parent().unwrap().into(),
        log: None,
    };
    let out = TokioSpawner.output(command, Duration::from_secs(10), 64 * 1024).await.unwrap();
    let banner = String::from_utf8_lossy(out.as_deref().unwrap_or_default()).into_owned();
    eprintln!("KytyPS5 --help: {:?}", banner.lines().next());
    assert!(emu_kyty::banner_version(&banner).is_some(), "the banner names its build: {banner:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_shadps4_installs_where_it_has_builds() {
    if !wanted() {
        return;
    }
    if Platform::current().is_none() {
        eprintln!("SKIPPED: shadPS4 has no official build for this system in the plugin");
        return;
    }
    let dir = data_dir("shad");
    let core = core(&dir).await;
    let installed = install(&core, "emu-shadps4").await;
    assert_eq!(installed.len(), 1, "{installed:?}");
    let builds = dir.join("plugins/emu-shadps4/builds");
    let program = find(&builds, "AppRun").or_else(|| find(&builds, "shadPS4.exe")).or_else(|| find(&builds, "shadps4"));
    let program = program.expect("the program");
    eprintln!("shadPS4 installed: {installed:?} at {}", program.display());
    let _ = std::fs::remove_dir_all(&dir);
}
