//! Named host folders: what a plugin's `permissions.filesystem` grants reach.
//!
//! The player maps each name to real folders, saved under `app.folders.<name>` as a list of
//! paths. Until then, the grant's `default` (with `~/` for the home folder) is the only folder,
//! or, for a list, each folder in it that exists.
//! Until then, a grant with `listed_in` also has the folders a file in those lists: Steam's
//! `steamapps/libraryfolders.vdf` names its libraries on other drives.
//! Each folder is a [`DirStorage`] root, so a symlink out of it is refused like one out of the
//! data folder.
//!
//! The core also asks here where a console's downloads and installs go
//! ([`HostFolders::place`]), and saves the player's pick ([`HostFolders::remember`]). A
//! [`Purpose`] says which of the two it is.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nexus_api::{
    DirEntry, FolderRoot, Folders, GAMES_FOLDER, HostPath, MAX_FOLDER_READ, PACKAGES_FOLDER, PathGrant, Permissions,
    PluginError, PluginId, PluginManifest, PluginResult, RelPath, Storage,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{ConfigService, CoreError, CoreResult, DirStorage};

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
    /// The plugin's name, for what the player reads.
    pub name: String,
    pub permissions: Permissions,
    pub config: Arc<ConfigService>,
    pub home: Option<PathBuf>,
    /// The system this runs on, for the folders' defaults.
    pub platform: Option<nexus_api::Platform>,
}

/// What a folder of a console is for. It picks the grant the folder lies in, the key the last
/// pick is saved under, and the words of an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// Where the catalogs' downloads go: inside a `packages` folder.
    Download,
    /// Where a downloaded game is installed: inside a `games` folder.
    Install,
}

impl Purpose {
    fn grant(self) -> &'static str {
        match self {
            Purpose::Download => PACKAGES_FOLDER,
            Purpose::Install => GAMES_FOLDER,
        }
    }

    /// The key under `app`, then the console, that holds the last pick. Players have these saved.
    fn saved(self) -> &'static str {
        match self {
            Purpose::Download => "download_folder",
            Purpose::Install => "install_folder",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Purpose::Download => "packages folder",
            Purpose::Install => "game folder",
        }
    }
}

impl HostFolders {
    pub(crate) fn new(manifest: &PluginManifest, config: &Arc<ConfigService>, host: &crate::HostConfig) -> Self {
        Self {
            plugin: manifest.id.clone(),
            name: manifest.name.clone(),
            permissions: manifest.permissions.clone(),
            config: config.clone(),
            home: host.home.clone(),
            platform: host.platform,
        }
    }

    /// The folder of `console` for `purpose`, inside one of the plugin's grant folders:
    /// `chosen`, or else the one last picked for the console, or else `<console>` in the first
    /// grant folder. A saved pick that is in no grant folder any more reads as none.
    pub(crate) fn place(
        &self,
        purpose: Purpose,
        console: &str,
        chosen: Option<&Path>,
    ) -> CoreResult<(HostPath, PathBuf)> {
        let (grant, noun) = (purpose.grant(), purpose.noun());
        let home = self.home.as_deref();
        let roots =
            self.permissions.folder(grant).map(|g| roots(&self.config, home, self.platform, g)).unwrap_or_default();
        // The root that holds `folder`, and the path from that root.
        let inside = |folder: &Path| {
            roots.iter().enumerate().find_map(|(i, root)| {
                let rest = folder.strip_prefix(root).ok()?;
                let parts: Vec<String> =
                    rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                Some((i as u32, parts))
            })
        };
        let (root, parts) = match chosen {
            Some(path) => {
                let path = expand(&path.to_string_lossy(), home)
                    .ok_or_else(|| CoreError::Invalid(format!("{} isn't a full path", path.display())))?;
                inside(&path).ok_or_else(|| {
                    CoreError::Invalid(format!(
                        "{} isn't in a {noun}. Pick a folder inside a {noun}, or add it in Settings",
                        path.display()
                    ))
                })?
            }
            None => self
                .config
                .get(&["app", purpose.saved(), console])
                .as_str()
                .and_then(|saved| inside(Path::new(saved)))
                .unwrap_or_else(|| (0, vec![console.to_owned()])),
        };
        let rel = RelPath::new(parts.join("/")).map_err(|e| CoreError::Invalid(e.to_string()))?;
        let at = HostPath::new(grant, root, rel);
        let folder = resolve(&self.config, home, self.platform, &self.permissions, &at)
            .map_err(|e| CoreError::Invalid(format!("{} has no {noun}: {e}", self.name)))?;
        Ok((at, folder))
    }

    /// Saves `folder` as the last one picked for `console` and `purpose`.
    pub(crate) fn remember(&self, purpose: Purpose, console: &str, folder: &Path) {
        let value = Value::String(folder.display().to_string());
        if let Err(e) = self.config.set(&["app", purpose.saved(), console], value) {
            tracing::warn!(error = %e, "could not save the folder");
        }
    }

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
    use super::*;

    /// The folders of a plugin that asks for `packages` and `games`, with the player's `packages`
    /// and `games` mapped to the given folders.
    fn host_folders(packages: &[&Path], games: &[&Path]) -> HostFolders {
        let config = Arc::new(ConfigService::in_memory());
        for (name, roots) in [("packages", packages), ("games", games)] {
            let list: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
            config.set(&["app", "folders", name], serde_json::json!(list)).unwrap();
        }
        let permissions = serde_json::from_value(serde_json::json!({
            "filesystem": [{ "folder": "packages", "write": true }, { "folder": "games", "write": true }],
        }))
        .unwrap();
        HostFolders {
            plugin: PluginId::new("emu").unwrap(),
            name: "Emu".into(),
            permissions,
            config,
            home: None,
            platform: None,
        }
    }

    #[test]
    fn a_download_goes_to_the_console_s_folder_in_the_first_packages_folder() {
        let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let folders = host_folders(&[first.path(), second.path()], &[]);
        let (at, folder) = folders.place(Purpose::Download, "ps5", None).unwrap();
        assert_eq!(folder, first.path().join("ps5"));
        assert_eq!((at.folder.as_str(), at.root, at.path.as_str()), ("packages", 0, "ps5"));
    }

    #[test]
    fn a_chosen_folder_is_used_when_it_is_inside_a_grant_folder_and_refused_when_it_is_not() {
        let (first, second, outside) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let folders = host_folders(&[first.path(), second.path()], &[]);
        let chosen = second.path().join("mine/ps4");
        let (at, folder) = folders.place(Purpose::Download, "ps4", Some(&chosen)).unwrap();
        assert_eq!(folder, chosen);
        assert_eq!((at.root, at.path.as_str()), (1, "mine/ps4"));

        let refused = folders.place(Purpose::Download, "ps4", Some(outside.path())).unwrap_err().to_string();
        assert!(refused.contains("Pick a folder inside a packages folder"), "{refused}");
        let relative = folders.place(Purpose::Download, "ps4", Some(Path::new("games/ps4"))).unwrap_err().to_string();
        assert!(relative.contains("isn't a full path"), "{relative}");
    }

    #[test]
    fn a_chosen_folder_can_be_the_grant_folder_itself() {
        let root = tempfile::tempdir().unwrap();
        let folders = host_folders(&[root.path()], &[]);
        let (at, folder) = folders.place(Purpose::Download, "ps5", Some(root.path())).unwrap();
        assert_eq!((folder, at.root, at.path.as_str()), (root.path().to_owned(), 0, ""));
    }

    #[test]
    fn the_last_pick_is_used_until_it_is_in_no_grant_folder_any_more() {
        let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let folders = host_folders(&[first.path(), second.path()], &[]);
        folders.remember(Purpose::Download, "ps5", &second.path().join("lib/ps5"));
        let (at, folder) = folders.place(Purpose::Download, "ps5", None).unwrap();
        assert_eq!((folder, at.root), (second.path().join("lib/ps5"), 1));

        // The player unmapped that folder: the console's folder in the first one is used.
        folders.config.set(&["app", "folders", "packages"], serde_json::json!([first.path()])).unwrap();
        let (at, folder) = folders.place(Purpose::Download, "ps5", None).unwrap();
        assert_eq!((folder, at.root), (first.path().join("ps5"), 0));
    }

    #[test]
    fn a_download_and_an_install_use_their_own_grant_and_their_own_saved_pick() {
        let (packages, games) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let folders = host_folders(&[packages.path()], &[games.path()]);
        folders.remember(Purpose::Install, "ps5", &games.path().join("chosen"));
        let (_, download) = folders.place(Purpose::Download, "ps5", None).unwrap();
        let (at, install) = folders.place(Purpose::Install, "ps5", None).unwrap();
        assert_eq!(download, packages.path().join("ps5"), "the install's pick doesn't move a download");
        assert_eq!((install, at.folder.as_str()), (games.path().join("chosen"), "games"));
        assert_eq!(
            folders.config.get(&["app", "install_folder", "ps5"]),
            serde_json::json!(games.path().join("chosen"))
        );
        assert!(folders.config.get(&["app", "download_folder", "ps5"]).is_null());
    }

    #[test]
    fn a_plugin_without_the_grant_has_no_folder_to_place() {
        let mut folders = host_folders(&[], &[]);
        folders.permissions = serde_json::from_value(serde_json::json!({})).unwrap();
        let refused = folders.place(Purpose::Install, "ps5", None).unwrap_err().to_string();
        assert!(refused.contains("Emu has no game folder"), "{refused}");
    }

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
