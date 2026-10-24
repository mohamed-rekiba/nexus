//! The real KytyPS5 and shadPS4 release archives, installed through the plugins and the host's
//! unpacker on a fake web. Opt-in: set `NEXUS_REAL_ARCHIVES` to a folder with the
//! downloaded archives (names as GitHub has them). Each archive that isn't there is skipped.

use std::path::{Path, PathBuf};

use nexus_api::*;
use nexus_core::Command;
use nexus_testkit::{FakeHost, TestCore};

fn archive(prefix: &str, suffix: &str) -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("NEXUS_REAL_ARCHIVES") else {
        eprintln!("SKIPPED: set NEXUS_REAL_ARCHIVES to a folder of release archives");
        return None;
    };
    let found = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        name.starts_with(prefix) && name.ends_with(suffix)
    });
    if found.is_none() {
        eprintln!("SKIPPED: no {prefix}…{suffix} there");
    }
    found
}

/// Serves `file` as the only asset of release `tag` at `api`, and boots `plugin` on `platform`.
async fn host(file: &Path, api: &str, tag: &str, platform: Platform, plugin: nexus_api::PluginByKind) -> TestCore {
    let host = FakeHost::new().platform(Some(platform));
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    let url = format!("https://github.com/x/releases/download/{tag}/{name}");
    let sha = host.http().serve(&url, std::fs::read(file).unwrap());
    let release = serde_json::json!([{ "tag_name": tag, "published_at": "2026-10-16T00:00:00Z", "assets": [
        { "name": name, "browser_download_url": url, "size": 0, "digest": format!("sha256:{sha}") }] }]);
    host.http().serve(api, serde_json::to_vec(&release).unwrap());
    host.built_in(plugin).boot_core().await
}

fn build(test: &TestCore, plugin: &str, tag: &str) -> PathBuf {
    test.data_dir().join("plugins").join(plugin).join("builds").join(tag)
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 == 0o111)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

async fn install_kyty(file: PathBuf, platform: Platform) -> (TestCore, PathBuf) {
    let tag = "KytyPS5-2026-10-16-d7974dc";
    let test =
        host(&file, emu_kyty::RELEASES, tag, platform, nexus_api::PluginByKind::emulator(emu_kyty::KytyPlugin)).await;
    let plugin = PluginId::new("emu-kyty").unwrap();
    let dir = build(&test, "emu-kyty", tag);
    // The fake spawner doesn't run the build's test run: it prints a banner where each
    // system's build keeps its binary.
    for binary in ["kyty_emulator", "kyty_emulator.exe", "KytyPS5.app/Contents/MacOS/kyty_emulator"] {
        test.spawner.prints(&dir.join(binary), b"KytyPS5 git = d7974dc, date = 2026.10.16\n");
    }
    test.core.dispatch(Command::InstallEmulator { plugin, version: None }).await.unwrap();
    (test, dir)
}

#[tokio::test]
async fn the_real_kyty_linux_build_unpacks_with_its_binary_at_the_top() {
    let Some(file) = archive("KytyPS5-", "-Linux-x86_64.tar.gz") else { return };
    let (_test, dir) = install_kyty(file, Platform::Linux).await;
    assert!(executable(&dir.join("kyty_emulator")));
    assert!(dir.join("plugins/platforms/libqlinuxfb.so").is_file());
}

#[tokio::test]
async fn the_real_kyty_macos_build_unpacks_its_app_bundle_with_its_links() {
    let Some(file) = archive("KytyPS5-", "-macOS-x86_64.zip") else { return };
    let (_test, dir) = install_kyty(file, Platform::Macos).await;
    assert!(executable(&dir.join("KytyPS5.app/Contents/MacOS/kyty_emulator")));
    #[cfg(unix)]
    {
        let current = dir.join("KytyPS5.app/Contents/Frameworks/QtGui.framework/Versions/Current");
        assert!(std::fs::symlink_metadata(&current).unwrap().file_type().is_symlink());
        assert!(current.canonicalize().unwrap().starts_with(dir.canonicalize().unwrap()), "the link stays inside");
    }
}

#[tokio::test]
async fn the_real_kyty_windows_build_unpacks_with_its_exe_at_the_top() {
    let Some(file) = archive("KytyPS5-", "-Windows-x64.zip") else { return };
    let (_test, dir) = install_kyty(file, Platform::Windows).await;
    assert!(dir.join("kyty_emulator.exe").is_file());
}

#[tokio::test]
async fn the_real_shadps4_windows_build_unpacks_its_exe() {
    let Some(file) = archive("shadps4-win64", ".zip") else { return };
    let tag = "v.0.20.0";
    let test = host(
        &file,
        emu_shadps4::RELEASES,
        tag,
        Platform::Windows,
        nexus_api::PluginByKind::emulator(emu_shadps4::ShadPlugin),
    )
    .await;
    let plugin = PluginId::new("emu-shadps4").unwrap();
    test.spawner.prints(&build(&test, "emu-shadps4", tag).join("shadPS4.exe"), b"shadPS4 v0.20.0\n");
    test.core.dispatch(Command::InstallEmulator { plugin, version: None }).await.unwrap();
    assert!(build(&test, "emu-shadps4", tag).join("shadPS4.exe").is_file());
}

#[tokio::test]
async fn the_real_shadps4_linux_zip_gives_up_its_appimage_to_run() {
    let Some(file) = archive("shadps4-linux", ".zip") else { return };
    let tag = "v.0.20.0";
    let test = host(
        &file,
        emu_shadps4::RELEASES,
        tag,
        Platform::Linux,
        nexus_api::PluginByKind::emulator(emu_shadps4::ShadPlugin),
    )
    .await;
    test.spawner.prints(&build(&test, "emu-shadps4", tag).join("squashfs-root/AppRun"), b"shadPS4 v0.20.0\n");
    let core = test.core.clone();
    let install = tokio::spawn(async move {
        core.dispatch(Command::InstallEmulator { plugin: PluginId::new("emu-shadps4").unwrap(), version: None }).await
    });

    // The fake spawner doesn't run it; check what the host would run, then play its part.
    let process = test.spawner.wait_for(1).await;
    assert_eq!(process.command.program.file_name().unwrap(), "Shadps4-sdl.AppImage");
    let head = std::fs::read(&process.command.program).unwrap();
    assert_eq!(&head[..4], b"\x7fELF", "a real program came out of the zip");
    assert!(executable(&process.command.program));
    std::fs::create_dir_all(process.command.cwd.join("squashfs-root")).unwrap();
    std::fs::write(process.command.cwd.join("squashfs-root/AppRun"), b"#!/bin/sh\n").unwrap();
    process.exit(0);
    install.await.unwrap().unwrap();
}
