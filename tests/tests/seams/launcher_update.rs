//! Seam 1, core app API: the launcher's own update. The release's latest.json says the
//! newest version and each kind of install's package; the core downloads this install's,
//! checks its SHA-256, and puts it in the way that install takes: kept for the app to swap in
//! at the next start, handed to the system's package manager, or left to Nexus OS.

use std::io::Write;

use nexus_core::launcher_update::{Install, apply_staged, package_key, staged};
use nexus_core::{Command, PackageError, PackageManager};
use nexus_testkit::FakeHost;

const LATEST: &str = "https://github.com/owner/nexus/releases/latest/download/latest.json";

/// A package archive as the release makes it: one top folder holding the launcher.
fn package(marker: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(marker.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "nexus-linux-x86_64/VERSION", marker.as_bytes()).unwrap();
        tar.into_inner().unwrap().finish().unwrap().flush().unwrap();
    }
    bytes
}

/// This system's package for an unpacked install, as the release names it, and its bytes: a
/// zip on Windows (its files go in one by one), a tar.gz elsewhere (the folder is swapped).
fn folder_package(marker: &str) -> (&'static str, Vec<u8>) {
    if !cfg!(windows) {
        let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-linux-x86_64.tar.gz";
        return (url, package(marker));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("nexus-windows-x86_64/VERSION", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(marker.as_bytes()).unwrap();
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-windows-x86_64.zip";
    (url, zip.finish().unwrap().into_inner())
}

fn manifest(version: &str, url: &str, sha256: &str) -> Vec<u8> {
    manifest_for(Install::Folder, version, url, sha256)
}

fn manifest_for(install: Install, version: &str, url: &str, sha256: &str) -> Vec<u8> {
    let mut assets = serde_json::Map::new();
    assets.insert(package_key(install).unwrap(), serde_json::json!({ "url": url, "sha256": sha256 }));
    serde_json::json!({ "version": version, "assets": assets }).to_string().into_bytes()
}

#[tokio::test]
async fn a_newer_release_is_found_downloaded_checked_and_kept_for_the_next_start() {
    let host = FakeHost::new();
    let (url, bytes) = folder_package("9.9.9");
    let sha = host.http().serve(url, bytes);
    host.http().serve(LATEST, manifest("9.9.9", url, &sha));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!((update.state.as_str(), update.version.as_str()), ("available", "9.9.9"));

    t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "ready", "{}", update.text);
    let kept = staged(t.data_dir()).expect("kept for the next start");
    assert_eq!(kept.version, "9.9.9");

    // The next start: the kept package replaces the install folder.
    let install = tempfile::tempdir().unwrap();
    let root = install.path().join("nexus-linux-x86_64");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("VERSION"), "0.1.0").unwrap();
    assert_eq!(apply_staged(t.data_dir(), &root).unwrap(), Some("9.9.9".into()));
    assert_eq!(std::fs::read_to_string(root.join("VERSION")).unwrap(), "9.9.9");
    assert!(staged(t.data_dir()).is_none(), "it is tried once");
    // The old version, still running, stays until the start after: beside the folder, or on
    // Windows, where a running program's folder can't be renamed, each file beside itself.
    let old = if cfg!(windows) { root.join("VERSION.old") } else { install.path().join("nexus-linux-x86_64.old") };
    let old_version = if cfg!(windows) { old.clone() } else { old.join("VERSION") };
    assert_eq!(std::fs::read_to_string(old_version).unwrap(), "0.1.0");
    nexus_core::launcher_update::clean_old_files(&root);
    assert!(!old.exists(), "the next start removes it");
}

#[tokio::test]
async fn a_download_that_doesn_t_match_its_sha256_is_refused() {
    let host = FakeHost::new();
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-linux-x86_64.tar.gz";
    host.http().serve(url, package("9.9.9"));
    host.http().serve(LATEST, manifest("9.9.9", url, &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!(update.state, "failed");
    assert!(update.text.contains("SHA-256"), "{}", update.text);
    assert!(staged(t.data_dir()).is_none(), "nothing kept");
}

#[tokio::test]
async fn an_empty_release_repo_turns_checks_off_and_an_older_release_is_no_update() {
    let host = FakeHost::new();
    host.http().serve(LATEST, manifest("0.0.1", "https://github.com/owner/nexus/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "".into()).unwrap();
    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "off");

    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();
    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "current");
}

#[tokio::test]
async fn without_a_setting_the_launcher_reads_nexus_s_own_releases() {
    let host = FakeHost::new();
    let own = format!(
        "https://github.com/{}/releases/latest/download/latest.json",
        nexus_core::launcher_update::DEFAULT_REPO
    );
    host.http().serve(&own, manifest("0.0.1", "https://github.com/mohamed-rekiba/nexus/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "current", "read from {own}");
}

#[test]
fn each_kind_of_install_reads_its_own_entry_in_latest_json() {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    let arch = std::env::consts::ARCH;
    assert_eq!(package_key(Install::Folder), Some(format!("{os}-{arch}")));
    assert_eq!(package_key(Install::MacApp), Some(format!("{os}-{arch}")));
    assert_eq!(package_key(Install::AppImage), Some(format!("linux-{arch}-appimage")));
    assert_eq!(package_key(Install::Rpm), Some(format!("linux-{arch}-rpm")));
    assert_eq!(package_key(Install::Deb), Some(format!("linux-{arch}-deb")));
    assert_eq!(package_key(Install::Arch), Some(format!("linux-{arch}-arch")));
    assert_eq!(package_key(Install::WindowsSetup), Some(format!("windows-{arch}-setup")));
    assert_eq!(package_key(Install::NexusOs), None, "Nexus OS updates the launcher itself");
}

/// A packaged install's release: the rpm, served at its release address.
async fn packaged(install: Install, file: &str) -> (FakeHost, String) {
    let host = FakeHost::new().install(install);
    let url = format!("https://github.com/owner/nexus/releases/download/v9.9.9/{file}");
    let sha = host.http().serve(&url, b"a package".to_vec());
    host.http().serve(LATEST, manifest_for(install, "9.9.9", &url, &sha));
    (host, url)
}

#[tokio::test]
async fn a_packaged_install_hands_the_checked_package_to_its_package_manager_then_offers_a_restart() {
    for (install, file, manager) in [
        (Install::Rpm, "nexus-9.9.9.x86_64.rpm", PackageManager::Dnf),
        (Install::Deb, "nexus_9.9.9_amd64.deb", PackageManager::Apt),
        (Install::Arch, "nexus-9.9.9-1-x86_64.pkg.tar.zst", PackageManager::Pacman),
    ] {
        let (host, _) = packaged(install, file).await;
        let login_session = host.login_session().clone();
        let t = host.boot_core().await;
        t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

        t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!((update.state.as_str(), update.method.as_str()), ("available", "package"));

        t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!(update.state, "installed", "{}", update.text);
        assert!(update.text.contains("restart"), "{}", update.text);
        let handed = login_session.packages();
        assert_eq!(handed.len(), 1, "{install:?}");
        assert_eq!(handed[0].1, manager);
        assert_eq!(handed[0].0.file_name().unwrap(), file);
        assert!(staged(t.data_dir()).is_none(), "the package manager put it in: nothing for the next start");
    }
}

#[tokio::test]
async fn a_closed_password_prompt_or_a_package_manager_error_leaves_this_version_and_says_why() {
    for (refusal, says) in [
        (PackageError::Cancelled, "password"),
        (PackageError::Failed("nothing provides libmpv".into()), "nothing provides libmpv"),
    ] {
        let (host, _) = packaged(Install::Rpm, "nexus-9.9.9.x86_64.rpm").await;
        host.login_session().refuse_packages(refusal);
        let t = host.boot_core().await;
        t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

        t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
        t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
        let update = t.core.view().settings.launcher_update;
        assert_eq!(update.state, "failed");
        assert!(update.text.contains(says), "{}", update.text);
        assert!(staged(t.data_dir()).is_none());
    }
}

#[tokio::test]
async fn a_package_that_doesn_t_match_its_sha256_never_reaches_the_package_manager() {
    let host = FakeHost::new().install(Install::Deb);
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus_9.9.9_amd64.deb";
    host.http().serve(url, b"altered".to_vec());
    host.http().serve(LATEST, manifest_for(Install::Deb, "9.9.9", url, &"0".repeat(64)));
    let login_session = host.login_session().clone();
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "failed");
    assert!(login_session.packages().is_empty());
}

#[tokio::test]
async fn an_installer_install_keeps_the_new_installer_for_the_next_start() {
    let host = FakeHost::new().install(Install::WindowsSetup);
    let url = "https://github.com/owner/nexus/releases/download/v9.9.9/nexus-windows-x86_64-setup.exe";
    let sha = host.http().serve(url, b"an installer".to_vec());
    host.http().serve(LATEST, manifest_for(Install::WindowsSetup, "9.9.9", url, &sha));
    let login_session = host.login_session().clone();
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.method, "installer");
    t.core.dispatch(Command::DownloadLauncherUpdate).await.unwrap();
    assert_eq!(t.core.view().settings.launcher_update.state, "ready");
    let kept = staged(t.data_dir()).expect("kept for the next start");
    assert_eq!(kept.file.file_name().unwrap(), "nexus-windows-x86_64-setup.exe");
    assert!(login_session.packages().is_empty());
}

#[tokio::test]
async fn on_nexus_os_a_newer_release_comes_with_the_next_system_update() {
    let host = FakeHost::new().install(Install::NexusOs);
    host.http().serve(LATEST, manifest("9.9.9", "https://github.com/owner/nexus/x.tar.gz", &"0".repeat(64)));
    let t = host.boot_core().await;
    t.core.kernel().config().set(&["app", "update_repo"], "owner/nexus".into()).unwrap();

    t.core.dispatch(Command::CheckLauncherUpdate).await.unwrap();
    let update = t.core.view().settings.launcher_update;
    assert_eq!((update.state.as_str(), update.method.as_str(), update.version.as_str()), ("system", "system", "9.9.9"));
    assert!(update.text.contains("system update"), "{}", update.text);
    assert!(t.core.dispatch(Command::DownloadLauncherUpdate).await.is_err(), "nothing for the launcher to download");
}

#[tokio::test]
async fn a_folder_or_app_install_swaps_its_update_in_at_the_next_start() {
    for install in [Install::Folder, Install::AppImage, Install::MacApp, Install::WindowsZip] {
        let t = FakeHost::new().install(install).boot_core().await;
        assert_eq!(t.core.view().settings.launcher_update.method, "swap", "{install:?}");
    }
}

#[test]
fn the_install_is_told_by_the_files_beside_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path();
    assert_eq!(Install::of(bin, false), Install::Unknown, "a build that isn't installed");

    std::fs::write(bin.join("LICENSE"), "").unwrap();
    std::fs::create_dir(bin.join("extensions")).unwrap();
    let folder = if cfg!(windows) { Install::WindowsZip } else { Install::Folder };
    assert_eq!(Install::of(bin, false), folder);

    std::fs::write(bin.join("installed-by-setup"), "").unwrap();
    assert_eq!(Install::of(bin, false), Install::WindowsSetup);

    for (format, install) in [("rpm\n", Install::Rpm), ("deb", Install::Deb), ("arch", Install::Arch)] {
        std::fs::write(bin.join("packaged"), format).unwrap();
        assert_eq!(Install::of(bin, false), install);
    }
    assert_eq!(Install::of(bin, true), Install::NexusOs, "Nexus OS's packaged launcher");

    let app = dir.path().join("Nexus.app/Contents/MacOS");
    std::fs::create_dir_all(&app).unwrap();
    assert_eq!(Install::of(&app, false), Install::MacApp);
}
