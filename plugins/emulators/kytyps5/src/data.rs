//! KytyPS5 writes saves and caches relative to its working folder, which is the build's
//! folder. When the current build changes, those folders move to the new build, so saves and
//! shader caches survive updates and rollbacks. Leaving your own build copies its folders in.

use emulator_kit::builds::OWN_DATA;
use emulator_kit::host::Host;
use emulator_kit::versions;
use nexus_api::*;

use crate::{MAC_BUNDLE, OWN_BUILD, path};

/// Folders KytyPS5 writes relative to its working folder.
pub const DATA_DIRS: [&str; 6] = ["_SaveData", "_PipelineCache", "_DownloadData", "_TempData", "_Textures", "_Patches"];
/// Where the data folders wait while no build is installed.
const PARKED: &str = "data";

pub fn binary_name(platform: Option<Platform>) -> &'static str {
    if platform == Some(Platform::Windows) { "kyty_emulator.exe" } else { "kyty_emulator" }
}

/// The emulator binary in a build's folder: at its top, one folder down, or in the macOS
/// app bundle.
pub async fn binary(storage: &dyn Storage, tag: &str, platform: Option<Platform>) -> PluginResult<RelPath> {
    let build = versions::folder(tag)?;
    let binary = binary_name(platform);
    let top = build.join(binary)?;
    if storage.exists(&top).await? {
        return Ok(top);
    }
    let bundled = path(format!("{build}/{MAC_BUNDLE}/{binary}"))?;
    if storage.exists(&bundled).await? {
        return Ok(bundled);
    }
    for name in storage.list(&build).await? {
        let inner = build.join(&name)?.join(binary)?;
        if storage.exists(&inner).await? {
            return Ok(inner);
        }
    }
    Err(PluginError::NotFound(format!("no {binary} in KytyPS5 {tag}")))
}

/// The folder KytyPS5 runs in for a build: where its binary is, or for a macOS app bundle
/// the build's folder, so saves don't go inside the signed bundle.
pub async fn work_dir(storage: &dyn Storage, tag: &str, platform: Option<Platform>) -> PluginResult<RelPath> {
    let binary = binary(storage, tag, platform).await?;
    let parent = binary.as_str().rsplit_once('/').map(|(dir, _)| dir).unwrap_or_default();
    match parent.strip_suffix(&format!("/{MAC_BUNDLE}")) {
        Some(build) => path(build),
        None => path(parent),
    }
}

/// The data folders' home with `tag` current: its folder, or the parking folder when there
/// is no build or its binary is missing.
pub async fn home(storage: &dyn Storage, tag: Option<&str>, platform: Option<Platform>) -> PluginResult<RelPath> {
    match tag {
        Some(tag) => match work_dir(storage, tag, platform).await {
            Ok(dir) => Ok(dir),
            Err(_) => path(PARKED),
        },
        None => path(PARKED),
    }
}

/// Moves the data folders from `from` to `to`. When `to` already has one (a build that
/// ships it), the entries it lacks move into it; an entry both have stays in `from`, so
/// nothing is lost.
pub async fn move_data(host: &Host, from: &RelPath, to: &RelPath) -> PluginResult<()> {
    let storage = host.storage.as_ref();
    for dir in DATA_DIRS {
        let (source, target) = (from.join(dir)?, to.join(dir)?);
        if !storage.exists(&source).await? {
            continue;
        }
        if !storage.exists(&target).await? {
            storage.rename(&source, &target).await?;
            continue;
        }
        for name in storage.list(&source).await? {
            let (from_entry, to_entry) = (source.join(&name)?, target.join(&name)?);
            if storage.exists(&to_entry).await? {
                host.log.warn(&format!("{to_entry} already exists; {from_entry} stays where it is"));
            } else {
                storage.rename(&from_entry, &to_entry).await?;
            }
        }
    }
    Ok(())
}

/// Copies your own build's data folders ([`DATA_DIRS`]: saves, the shader cache and the
/// rest) into `to`, each one only when `to` has none of it yet. Your build keeps its own
/// copy. `own`: your own build's binary, when it is there.
pub async fn adopt_own_data(host: &Host, own: Option<&Location>, to: &RelPath) -> PluginResult<()> {
    let storage = host.storage.as_ref();
    let empty = |target: RelPath| async move {
        PluginResult::Ok(!(storage.exists(&target).await? && !storage.list(&target).await?.is_empty()))
    };
    let binary = match own {
        Some(Location::Folder(binary)) => binary,
        // From PATH it ran in the plugin's own folder: its data is already in the data
        // folder, so it moves instead of copying.
        Some(_) => {
            for name in DATA_DIRS {
                let (from, target) = (path(format!("{OWN_DATA}/{name}"))?, to.join(name)?);
                if storage.exists(&from).await? && empty(target.clone()).await? {
                    storage.remove(&target).await?;
                    storage.rename(&from, &target).await?;
                }
            }
            return Ok(());
        }
        None => return Ok(()),
    };
    let dir = binary.path.as_str().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let dir = match dir.strip_suffix(&format!("/{MAC_BUNDLE}")).or(if dir == MAC_BUNDLE { Some("") } else { None }) {
        Some(build) => build.to_owned(),
        None => dir.to_owned(),
    };
    for name in DATA_DIRS {
        let from = if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
        let source = HostPath::new(OWN_BUILD, binary.root, path(from)?);
        let target = to.join(name)?;
        if !host.folders.exists(&source).await? {
            continue;
        }
        if !empty(target.clone()).await? {
            host.log.warn(&format!("official build already has {name}; your own build's was not copied"));
            continue;
        }
        copy_tree(host, &source, &target).await?;
    }
    Ok(())
}

/// Copies a folder from a host folder into the data folder, file by file, each file in
/// pieces of at most [`MAX_FOLDER_READ`], so a big cache never fills the memory.
async fn copy_tree(host: &Host, from: &HostPath, to: &RelPath) -> PluginResult<()> {
    let (folders, storage) = (&host.folders, &host.storage);
    let mut pending = vec![(from.clone(), to.clone())];
    while let Some((dir, target)) = pending.pop() {
        for entry in folders.list(&dir).await? {
            let source = HostPath::new(&dir.folder, dir.root, dir.path.join(&entry.name)?);
            let dest = target.join(&entry.name)?;
            if entry.is_dir {
                pending.push((source, dest));
                continue;
            }
            // An empty file is written too.
            storage.write(&dest, &[]).await?;
            let mut done = 0u64;
            loop {
                let piece = folders.read_range(&source, done, MAX_FOLDER_READ).await?;
                storage.write_at(&dest, done, &piece).await?;
                done += piece.len() as u64;
                if (piece.len() as u64) < MAX_FOLDER_READ {
                    break;
                }
            }
        }
    }
    Ok(())
}
