//! Unpacking downloaded archives into a plugin's data folder.
//!
//! Every entry's path is checked before anything is written: absolute paths, `..` and links
//! that point out of the folder are refused, so an archive can't write outside it. The work is
//! blocking, so it runs on Tokio's blocking pool.

use std::path::{Component, Path, PathBuf};

use nexus_api::ArchiveFormat;

use crate::{CoreError, CoreResult, Kernel, SpawnCommand};

/// The longest an AppImage may take to unpack itself.
const EXTRACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

fn failed(message: impl std::fmt::Display) -> CoreError {
    CoreError::Download(format!("could not unpack: {message}"))
}

/// `path` as a relative path with only normal parts, or `None`.
pub fn inside(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A link at `link` (relative to the folder) pointing at `target` stays inside the folder.
pub fn link_stays_inside(link: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth: i64 = link.components().count() as i64 - 1;
    for part in target.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => depth -= 1,
            Component::CurDir => {}
            _ => return false,
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

/// Whether `path` is inside `root` once links on the way are followed: the deepest part of it
/// that exists is checked.
pub fn stays_inside(path: &Path, root: &Path) -> CoreResult<bool> {
    let root = root.canonicalize().map_err(failed)?;
    let mut probe = path;
    while std::fs::symlink_metadata(probe).is_err() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return Ok(false),
        }
    }
    Ok(probe.canonicalize().map_err(failed)?.starts_with(&root))
}

pub(crate) fn untar_gz(archive: &Path, into: &Path) -> CoreResult<()> {
    let file = std::fs::File::open(archive).map_err(failed)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    for entry in tar.entries().map_err(failed)? {
        let mut entry = entry.map_err(failed)?;
        let path = entry.path().map_err(failed)?.into_owned();
        // `./`, the archive's own root (KytyPS5's Linux builds start with it).
        if path.components().all(|c| c == Component::CurDir) {
            continue;
        }
        let rel = inside(&path).ok_or_else(|| failed(format!("{} leaves the folder", path.display())))?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry.link_name().map_err(failed)?.unwrap_or_default().into_owned();
            let ok = if kind.is_hard_link() { inside(&target).is_some() } else { link_stays_inside(&rel, &target) };
            if !ok {
                return Err(failed(format!("{} links out of the folder", path.display())));
            }
        }
        // `unpack_in` also refuses paths that leave `into`, and writing through links.
        if !entry.unpack_in(into).map_err(failed)? {
            return Err(failed(format!("{} leaves the folder", path.display())));
        }
    }
    Ok(())
}

pub(crate) fn unzip(archive: &Path, into: &Path, only_suffix: Option<&str>) -> CoreResult<Vec<PathBuf>> {
    let file = std::fs::File::open(archive).map_err(failed)?;
    let mut zip = zip::ZipArchive::new(file).map_err(failed)?;
    let mut written = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(failed)?;
        let name = entry.name().map_err(failed)?.into_owned();
        let rel = entry
            .enclosed_name()
            .and_then(|p| inside(&p))
            .ok_or_else(|| failed(format!("{name} leaves the folder")))?;
        if only_suffix.is_some_and(|suffix| !name.ends_with(suffix)) {
            continue;
        }
        let out = into.join(&rel);
        // An earlier entry may have made a link on the way; nothing is written through one.
        if std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(failed(format!("{name} would write through a link")));
        }
        if !stays_inside(&out, into)? {
            return Err(failed(format!("{name} leaves the folder through a link")));
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(failed)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        if entry.is_symlink() {
            let mut target = String::new();
            std::io::Read::read_to_string(&mut entry, &mut target).map_err(failed)?;
            if !link_stays_inside(&rel, Path::new(&target)) {
                return Err(failed(format!("{name} links out of the folder")));
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &out).map_err(failed)?;
            continue;
        }
        let mut file = std::fs::File::create(&out).map_err(failed)?;
        std::io::copy(&mut entry, &mut file).map_err(failed)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777)).map_err(failed)?;
        }
        written.push(out);
    }
    Ok(written)
}

/// Unpacks `archive` into `into`, replacing what was there. Unpacks into a sibling folder
/// first, so a failure leaves the old contents.
/// `may_run` says whether the plugin may run a program at a path in its data folder: an
/// AppImage runs to unpack itself only with the plugin's process permission.
pub(crate) async fn unpack(
    kernel: &Kernel,
    may_run: &(dyn Fn(&Path) -> bool + Sync),
    format: ArchiveFormat,
    archive: &Path,
    into: &Path,
) -> CoreResult<()> {
    let mut name = into.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".unpacking");
    let staging = into.with_file_name(name);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(failed)?;
    let result = unpack_into(kernel, may_run, format, archive, &staging, into).await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(e);
    }
    let _ = tokio::fs::remove_dir_all(into).await;
    tokio::fs::rename(&staging, into).await.map_err(failed)
}

async fn unpack_into(
    kernel: &Kernel,
    may_run: &(dyn Fn(&Path) -> bool + Sync),
    format: ArchiveFormat,
    archive: &Path,
    dir: &Path,
    into: &Path,
) -> CoreResult<()> {
    let (archive, dir) = (archive.to_path_buf(), dir.to_path_buf());
    match format {
        ArchiveFormat::TarGz => blocking(move || untar_gz(&archive, &dir)).await,
        ArchiveFormat::Zip => blocking(move || unzip(&archive, &dir, None).map(|_| ())).await,
        ArchiveFormat::AppImageInZip => {
            if kernel.host().platform != Some(nexus_api::Platform::Linux) {
                return Err(failed("AppImages run on Linux only"));
            }
            let found = {
                let (archive, dir) = (archive.clone(), dir.clone());
                blocking(move || unzip(&archive, &dir, Some(".AppImage"))).await?
            };
            let [appimage] = found.as_slice() else {
                return Err(failed(format!("expected one AppImage in the zip, found {}", found.len())));
            };
            // Checked where it will be once unpacked, as the manifest names it.
            let name = appimage.file_name().unwrap_or_default();
            if !may_run(&into.join(name)) {
                return Err(failed(format!(
                    "the plugin may not run {}: add it to permissions.process",
                    name.to_string_lossy()
                )));
            }
            set_executable(appimage)?;
            // The file passed its checks; running it to unpack is no worse than running it to play.
            let command = SpawnCommand {
                program: appimage.clone(),
                args: vec!["--appimage-extract".into()],
                env: Default::default(),
                cwd: dir.clone(),
                log: None,
                keep_running: false,
            };
            let mut process = kernel.spawner().spawn(command).await.map_err(failed)?;
            let exit = match tokio::time::timeout(EXTRACT_TIMEOUT, process.wait()).await {
                Ok(exit) => exit.map_err(failed)?,
                Err(_) => {
                    let _ = process.kill().await;
                    return Err(failed("the AppImage's --appimage-extract took too long"));
                }
            };
            if !exit.success {
                return Err(failed(format!("the AppImage's --appimage-extract failed (code {:?})", exit.code)));
            }
            let _ = tokio::fs::remove_file(appimage).await;
            if !dir.join("squashfs-root").is_dir() {
                return Err(failed("the AppImage left no squashfs-root folder"));
            }
            Ok(())
        }
    }
}

fn set_executable(path: &Path) -> CoreResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).map_err(failed)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms).map_err(failed)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CoreResult<T> + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(failed)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_must_stay_inside_the_folder() {
        assert!(link_stays_inside(Path::new("lib/libx.so"), Path::new("libx.so.1")));
        assert!(link_stays_inside(Path::new("a/b/link"), Path::new("../c")));
        assert!(!link_stays_inside(Path::new("link"), Path::new("../outside")));
        assert!(!link_stays_inside(Path::new("a/link"), Path::new("../../outside")));
        assert!(!link_stays_inside(Path::new("link"), Path::new("/etc/passwd")));
        assert_eq!(inside(Path::new("../x")), None);
        assert_eq!(inside(Path::new("/x")), None);
        assert_eq!(inside(Path::new("./a/b")), Some(PathBuf::from("a/b")));
    }
}
