//! Named host folders: what a plugin's `permissions.filesystem` grants reach.
//!
//! The player maps each name to real folders, saved under `app.folders.<name>` as a list of
//! paths. Until then, the grant's `default` (with `~/` for the home folder) is the only folder,
//! or, for a list, each folder in it that exists.
//! Until then, a grant with `listed_in` also has the folders a file in those lists: Steam's
//! `steamapps/libraryfolders.vdf` names its libraries on other drives.
//! Each folder is a [`DirStorage`] root, so a symlink out of it is refused like one out of the
//! data folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nexus_api::{
    DirEntry, FolderRoot, Folders, HostPath, MAX_FOLDER_READ, PathGrant, Permissions, PluginError, PluginId,
    PluginResult, Storage,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{ConfigService, DirStorage};

/// The player's folders for `grant`: the saved mapping, or else the default on `platform`, or
/// else each folder of the default's list that exists.
pub(crate) fn roots(
    config: &ConfigService,
    home: Option<&Path>,
    platform: Option<nexus_api::Platform>,
    grant: &PathGrant,
) -> Vec<PathBuf> {
    let saved = config.get(&["app", "folders", &grant.folder]);
    if let Some(list) = saved.as_array() {
        return list.iter().filter_map(Value::as_str).map(PathBuf::from).filter(|p| p.is_absolute()).collect();
    }
    let mut roots: Vec<PathBuf> = match grant.default_path(platform) {
        Some(default) => expand(default, home).into_iter().collect(),
        None => {
            // A folder that two entries name, as `~/kyty` and `~/Kyty` on a system that ignores
            // case, counts once.
            let mut seen = std::collections::HashSet::new();
            grant
                .search_paths(platform)
                .iter()
                .filter_map(|d| expand(d, home))
                .filter(|p| p.canonicalize().is_ok_and(|real| real.is_dir() && seen.insert(real)))
                .collect()
        }
    };
    if let Some(listed) = &grant.listed_in {
        let mut seen: std::collections::HashSet<PathBuf> = roots.iter().filter_map(|p| p.canonicalize().ok()).collect();
        let listed: Vec<PathBuf> = roots
            .iter()
            .flat_map(|root| listed_folders(&root.join(&listed.file), &listed.key))
            .filter(|p| p.is_absolute() && p.join(&listed.holds).is_dir())
            .filter(|p| p.canonicalize().is_ok_and(|real| seen.insert(real)))
            .collect();
        roots.extend(listed);
    }
    roots
}

/// The most of a list file read: Steam's library list is a few kilobytes.
const MAX_LIST: u64 = 256 * 1024;

/// Every value of `key` in the file at `file`, in Valve's text format: a quoted key, then its
/// quoted value (`"path"  "/mnt/Games/SteamLibrary"`, `\\` for a backslash). Empty when the
/// file isn't there.
fn listed_folders(file: &Path, key: &str) -> Vec<PathBuf> {
    use std::io::Read;
    let mut text = String::new();
    let read = std::fs::File::open(file).and_then(|f| f.take(MAX_LIST).read_to_string(&mut text));
    if read.is_err() {
        return Vec::new();
    }
    let words = quoted(&text);
    words.windows(2).filter(|pair| pair[0] == key).map(|pair| PathBuf::from(&pair[1])).collect()
}

/// The quoted words of Valve's text, in order, with `\\` and `\"` read back.
fn quoted(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut word = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => word.extend(chars.next()),
                '"' => break,
                c => word.push(c),
            }
        }
        words.push(word);
    }
    words
}

/// `~/x` under the home folder; other relative paths mean nothing.
pub(crate) fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => Some(home?.join(rest)),
        None => Some(PathBuf::from(path)).filter(|p| p.is_absolute()),
    }
}

/// One plugin's view of the host folders.
pub(crate) struct HostFolders {
    pub plugin: PluginId,
    pub permissions: Permissions,
    pub config: Arc<ConfigService>,
    pub home: Option<PathBuf>,
    /// The system this runs on, for the folders' defaults.
    pub platform: Option<nexus_api::Platform>,
}

impl HostFolders {
    /// Where `path` is on disk, checked against the plugin's grants.
    pub(crate) fn resolve(&self, path: &HostPath) -> PluginResult<PathBuf> {
        self.storage(path, false)?.resolve(&path.path)
    }

    /// Moves `from` to `to`, where the plugin may write, when both are on one disk: `false`
    /// when they aren't, or `to` is there already. The caller decides `from` may go.
    pub(crate) fn take(&self, from: &HostPath, to: &HostPath) -> PluginResult<bool> {
        let source = self.resolve(from)?;
        let target = self.storage(to, true)?.resolve(&to.path)?;
        if to.path.is_root() || target.exists() {
            return Ok(false);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PluginError::Storage(e.to_string()))?;
        }
        match std::fs::rename(&source, &target) {
            Ok(()) => Ok(true),
            // Another disk: the plugin copies.
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => Ok(false),
            Err(e) => Err(PluginError::Storage(e.to_string())),
        }
    }

    /// Deletes `path` and everything in it, where the plugin may write: never one of the
    /// player's folders itself, nor anything outside them.
    pub(crate) async fn remove(&self, path: &HostPath) -> PluginResult<()> {
        self.storage(path, true)?.remove(&path.path).await
    }

    fn grant(&self, folder: &str) -> PluginResult<&PathGrant> {
        self.permissions.folder(folder).ok_or_else(|| {
            PluginError::PermissionDenied(format!("{} did not ask for the folder {folder:?}", self.plugin))
        })
    }

    /// The root `path` is in, checked against the grant.
    fn storage(&self, path: &HostPath, write: bool) -> PluginResult<DirStorage> {
        let grant = self.grant(&path.folder)?;
        if write && !grant.write {
            return Err(PluginError::PermissionDenied(format!(
                "{} may only read the folder {:?}",
                self.plugin, path.folder
            )));
        }
        let roots = roots(&self.config, self.home.as_deref(), self.platform, grant);
        let root = roots
            .into_iter()
            .nth(path.root as usize)
            .ok_or_else(|| PluginError::NotFound(format!("folder {:?} has no root {}", path.folder, path.root)))?;
        Ok(DirStorage::new(root))
    }
}

/// The bytes of every file under `dir`, at most `MAX_SIZE_FILES` entries, without entering
/// linked folders.
fn folder_size(dir: &std::path::Path) -> u64 {
    let (mut total, mut entries, mut todo) = (0u64, 0usize, vec![dir.to_path_buf()]);
    while let Some(dir) = todo.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            entries += 1;
            if entries > nexus_api::MAX_SIZE_FILES {
                return total;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                todo.push(entry.path());
            } else if let Ok(meta) = std::fs::metadata(entry.path()) {
                // A linked file counts what it points at; a linked folder is not entered.
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn io(e: std::io::Error) -> PluginError {
    match e.kind() {
        std::io::ErrorKind::NotFound => PluginError::NotFound(e.to_string()),
        _ => PluginError::Storage(e.to_string()),
    }
}

#[async_trait]
impl Folders for HostFolders {
    async fn roots(&self, folder: &str) -> PluginResult<Vec<FolderRoot>> {
        let grant = self.grant(folder)?;
        Ok(roots(&self.config, self.home.as_deref(), self.platform, grant)
            .into_iter()
            .enumerate()
            .map(|(i, p)| FolderRoot { root: i as u32, path: p.display().to_string() })
            .collect())
    }

    async fn list(&self, dir: &HostPath) -> PluginResult<Vec<DirEntry>> {
        let full = self.storage(dir, false)?.resolve(&dir.path)?;
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(&full).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io(e)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Follows symlinks, so a link reports what it points at. Reading through one that
            // leaves the folder is still refused.
            let Ok(meta) = tokio::fs::metadata(entry.path()).await else { continue };
            out.push(DirEntry { name, is_dir: meta.is_dir(), size: if meta.is_dir() { 0 } else { meta.len() } });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn size(&self, dir: &HostPath) -> PluginResult<u64> {
        let full = self.storage(dir, false)?.resolve(&dir.path)?;
        // One blocking walk: a game can hold tens of thousands of files.
        tokio::task::spawn_blocking(move || folder_size(&full)).await.map_err(|e| PluginError::Storage(e.to_string()))
    }

    async fn exists(&self, path: &HostPath) -> PluginResult<bool> {
        Ok(self.storage(path, false)?.resolve(&path.path)?.exists())
    }

    async fn read_range(&self, path: &HostPath, offset: u64, len: u64) -> PluginResult<Vec<u8>> {
        let full = self.storage(path, false)?.resolve(&path.path)?;
        let mut file = tokio::fs::File::open(&full).await.map_err(io)?;
        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(io)?;
        let mut out = Vec::new();
        file.take(len.min(MAX_FOLDER_READ)).read_to_end(&mut out).await.map_err(io)?;
        Ok(out)
    }

    async fn write(&self, path: &HostPath, data: &[u8]) -> PluginResult<()> {
        self.storage(path, true)?.write(&path.path, data).await
    }

    async fn write_at(&self, path: &HostPath, offset: u64, data: &[u8]) -> PluginResult<()> {
        self.storage(path, true)?.write_at(&path.path, offset, data).await
    }

    async fn rename(&self, from: &HostPath, to: &HostPath) -> PluginResult<()> {
        if from.folder != to.folder || from.root != to.root {
            return Err(PluginError::InvalidInput("a rename stays inside one folder and root".into()));
        }
        let storage = self.storage(from, true)?;
        if storage.resolve(&to.path)?.exists() {
            return Err(PluginError::InvalidInput(format!("{} already exists", to.path)));
        }
        storage.rename(&from.path, &to.path).await
    }

    async fn remove(&self, path: &HostPath) -> PluginResult<()> {
        self.storage(path, true)?.remove(&path.path).await
    }

    async fn set_executable(&self, path: &HostPath) -> PluginResult<()> {
        self.storage(path, true)?.set_executable(&path.path).await
    }
}

/// The real path of a [`HostPath`], for a launch: checked against the grant and the folder's
/// symlinks like any other access.
pub(crate) fn resolve(
    config: &ConfigService,
    home: Option<&Path>,
    platform: Option<nexus_api::Platform>,
    permissions: &Permissions,
    path: &HostPath,
) -> PluginResult<PathBuf> {
    let grant = permissions
        .folder(&path.folder)
        .ok_or_else(|| PluginError::PermissionDenied(format!("the folder {:?} was not asked for", path.folder)))?;
    let root = roots(config, home, platform, grant)
        .into_iter()
        .nth(path.root as usize)
        .ok_or_else(|| PluginError::NotFound(format!("folder {:?} has no root {}", path.folder, path.root)))?;
    DirStorage::new(root).resolve(&path.path)
}

#[cfg(test)]
mod tests {
    /// A store's library list: each library's path, a Windows one with its backslashes doubled.
    #[test]
    fn a_list_file_gives_each_value_of_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("libraryfolders.vdf");
        let text = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/p/.local/share/Store\"\n\t\t\"label\"\t\t\"\"\n\t}\n\
                    \t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Store \\\"Games\\\"\"\n\t}\n}\n";
        std::fs::write(&file, text).unwrap();
        let paths: Vec<_> = super::listed_folders(&file, "path").into_iter().map(|p| p.display().to_string()).collect();
        assert_eq!(paths, ["/home/p/.local/share/Store", r#"D:\Store "Games""#]);
        assert!(super::listed_folders(&dir.path().join("missing.vdf"), "path").is_empty());
    }
}
