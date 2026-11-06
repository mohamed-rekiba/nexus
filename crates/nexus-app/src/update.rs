//! The launcher's own update at startup, and starting the launcher again.

use std::path::PathBuf;

/// The installed launcher this binary is part of: the AppImage file, `Nexus.app`, or the
/// package folder (with its LICENSE and extensions/). `None` for a build that isn't
/// installed, such as `cargo run`'s: an update never replaces that.
pub fn install_root() -> Option<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(PathBuf::from(appimage));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if dir.ends_with("Contents/MacOS") {
        return dir.parent()?.parent().filter(|app| app.extension().is_some_and(|e| e == "app")).map(PathBuf::from);
    }
    (dir.join("LICENSE").is_file() && dir.join("extensions").is_dir()).then(|| dir.to_path_buf())
}

/// The program the system starts at login: the AppImage file, or this binary in an installed
/// package or app bundle. `None` for a build that isn't installed.
pub fn login_program() -> Option<PathBuf> {
    let root = install_root()?;
    if root.is_file() { Some(root) } else { std::env::current_exe().ok() }
}

/// A launcher update kept by the last run goes in now, and the new version starts instead.
pub fn apply_launcher_update(data_dir: &std::path::Path) {
    use nexus_core::launcher_update::{Install, apply_staged, clean_old_files, staged};
    // A package manager or Nexus OS owns these files: nothing is ever swapped in here.
    if matches!(Install::detect().method(), "package" | "system") {
        return;
    }
    let Some(root) = install_root() else {
        if staged(data_dir).is_some() {
            tracing::warn!("a launcher update is waiting, but this build isn't installed from a package");
        }
        return;
    };
    clean_old_files(&root);
    if let Some(update) = staged(data_dir).filter(|s| s.file.to_string_lossy().ends_with("-setup.exe")) {
        run_installer(data_dir, &update.file);
        return;
    }
    // Where this binary sits in the install, for the new one at the same place.
    let exe = std::env::current_exe().ok();
    let inside = exe.as_deref().and_then(|e| e.strip_prefix(&root).ok()).map(PathBuf::from);
    match apply_staged(data_dir, &root) {
        Ok(Some(version)) => {
            tracing::info!(%version, "launcher updated; starting the new version");
            // The old binary was moved away: start the new one in its place.
            restart_as(inside.map(|p| root.join(p)));
        }
        Ok(None) => {}
        Err(error) => tracing::error!(%error, "the launcher update didn't go in; this version runs"),
    }
}

/// Windows' installer put this launcher in: the new installer replaces it, silently, and starts
/// the new version when it's done (its `[Run]` entry), so this one ends. Tried once.
fn run_installer(data_dir: &std::path::Path, installer: &std::path::Path) {
    let _ = std::fs::remove_file(data_dir.join("updates/ready.json"));
    match std::process::Command::new(installer).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).spawn() {
        Ok(_) => {
            tracing::info!(installer = %installer.display(), "updating with the new installer");
            std::process::exit(0);
        }
        Err(error) => tracing::error!(%error, "the new installer didn't start; this version runs"),
    }
}

/// Starts this launcher again with the same arguments, and ends this one.
pub fn restart() {
    restart_as(None);
}

/// Like [`restart`], with `exe` in place of this binary when it moved.
fn restart_as(exe: Option<PathBuf>) {
    let exe = exe.or_else(|| std::env::var_os("APPIMAGE").map(PathBuf::from)).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    match std::process::Command::new(&exe).args(std::env::args_os().skip(1)).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => tracing::error!(%error, "could not start the launcher again"),
    }
}
