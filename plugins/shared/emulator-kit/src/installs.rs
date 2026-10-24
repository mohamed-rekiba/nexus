//! Installing a game from a package or disc image into a game folder: the work a plugin's
//! [`nexus_api::GameInstaller`] runs in the background. The tree is read through the
//! folders facade and written into a `.part` work folder, which moves into place at the end.

use std::sync::Arc;

use nexus_api::{Folders, HostPath, InstallJob, MAX_FOLDER_READ, PluginError, PluginResult, RelPath};

use crate::fail::{Context, Error, Result};
use crate::read_at::ReadAt;
use crate::{ensure, exfat, pkg5};

/// A file or folder in a package or image, with `/` between the parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub size: u64,
}

/// Something that holds a tree of files: a package or a disc image.
pub trait Tree {
    fn entries(&mut self, cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>>;
    /// Streams entry `index` (from the last [`Tree::entries`]) to `sink`.
    fn copy(&mut self, index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()>;
}

/// A PS5 package's tree.
pub struct Ps5Package<R: ReadAt> {
    package: pkg5::Package<R>,
    nodes: Vec<pkg5::Node>,
}

impl<R: ReadAt> Ps5Package<R> {
    pub fn open(file: R, cancel: &dyn Fn() -> Result<()>) -> Result<Self> {
        Ok(Self { package: pkg5::Package::open(file, cancel)?, nodes: Vec::new() })
    }
}

impl<R: ReadAt> Tree for Ps5Package<R> {
    fn entries(&mut self, cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>> {
        self.nodes = self.package.list(cancel)?;
        Ok(self.nodes.iter().map(|n| Entry { path: n.path.clone(), directory: n.directory, size: n.size }).collect())
    }

    fn copy(&mut self, index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let node = self.nodes.get(index).context("No such file in the package")?.clone();
        self.package.copy(&node, sink)
    }
}

/// An exFAT disc image's tree.
pub struct ExfatImage<R: ReadAt> {
    volume: exfat::Volume<R>,
    nodes: Vec<exfat::Node>,
}

impl<R: ReadAt> ExfatImage<R> {
    pub fn open(file: R) -> Result<Self> {
        Ok(Self { volume: exfat::Volume::open(file)?, nodes: Vec::new() })
    }
}

impl<R: ReadAt> Tree for ExfatImage<R> {
    fn entries(&mut self, cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>> {
        self.nodes = self.volume.list(cancel)?;
        Ok(self.nodes.iter().map(|n| Entry { path: n.path.clone(), directory: n.directory, size: n.size }).collect())
    }

    fn copy(&mut self, index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let node = self.nodes.get(index).context("No such file in the image")?.clone();
        self.volume.copy(&node, sink)
    }
}

/// What a package is, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A PS5 package (`7F 'F' 'I' 'H'`).
    Ps5Package,
    /// A PS4 package (`7F 'C' 'N' 'T'`).
    Ps4Package,
    Exfat,
}

/// The format of a file whose first 512 bytes are `head`, if it is one the readers know.
pub fn format(head: &[u8]) -> Option<Format> {
    if head.starts_with(b"\x7fFIH") {
        Some(Format::Ps5Package)
    } else if head.starts_with(b"\x7fCNT") {
        Some(Format::Ps4Package)
    } else if head.get(3..11) == Some(b"EXFAT   ") {
        Some(Format::Exfat)
    } else {
        None
    }
}

/// How deep a [`Download`] is looked into: past any game's folders, short of a folder link
/// that loops. An install copies every file it lists, so a deeper download is refused.
const SCAN_DEPTH: usize = 64;
/// The most files a [`Download`] lists. A download with more is refused, as above.
const SCAN_FILES: usize = 1_000_000;
/// How many of a download's biggest files [`Download::packages`] reads the start of, beside
/// every file named as a package or an image.
const PICK_FILES: usize = 20;

/// The folder a game goes in: the one the player picked for this install, else `console` in
/// the first games folder.
pub async fn games_folder(folders: &dyn Folders, job: &dyn InstallJob, console: &str) -> PluginResult<HostPath> {
    if let Some(picked) = job.games_folder().filter(|p| p.folder == nexus_api::GAMES_FOLDER) {
        return Ok(picked);
    }
    let root = folders
        .roots(nexus_api::GAMES_FOLDER)
        .await?
        .first()
        .map(|r| r.root)
        .ok_or_else(|| PluginError::Precondition("add a game folder in Settings first".into()))?;
    let console = RelPath::new(console).map_err(|e| PluginError::InvalidInput(e.to_string()))?;
    Ok(HostPath::new(nexus_api::GAMES_FOLDER, root, console))
}

/// The size of the file at `path`, from its folder's listing; 0 when it isn't there.
pub async fn file_size(folders: &dyn Folders, path: &HostPath) -> PluginResult<u64> {
    let (parent, name) = match path.path.as_str().rsplit_once('/') {
        Some((dir, name)) => (RelPath::new(dir).map_err(|e| PluginError::InvalidInput(e.to_string()))?, name),
        None => (RelPath::root(), path.path.as_str()),
    };
    Ok(folders
        .list(&HostPath::new(&path.folder, path.root, parent))
        .await?
        .into_iter()
        .find(|e| e.name == name)
        .map(|e| e.size)
        .unwrap_or(0))
}

/// A file in a download, with its path from the download's folder.
#[derive(Clone, Debug)]
pub struct DownloadFile {
    pub path: HostPath,
    pub rel: String,
    pub size: u64,
}

impl DownloadFile {
    /// How many folders down it is.
    pub fn depth(&self) -> usize {
        self.rel.matches('/').count()
    }
}

/// A download: a folder of files, every one of them, up to [`SCAN_DEPTH`] folders down. It can
/// hold packages and images, or a game folder that is already unpacked.
pub struct Download {
    pub source: HostPath,
    pub files: Vec<DownloadFile>,
}

impl Download {
    /// The download at `source`. `None` when `source` is a file. A download deeper than
    /// [`SCAN_DEPTH`] or with more than [`SCAN_FILES`] files is refused: an install would leave
    /// the rest out.
    pub async fn scan(folders: &dyn Folders, source: &HostPath) -> PluginResult<Option<Self>> {
        let mut files = Vec::new();
        let mut dirs = vec![(source.clone(), String::new(), 0)];
        let mut is_dir = false;
        while let Some((dir, rel, depth)) = dirs.pop() {
            let Ok(entries) = folders.list(&dir).await else { continue };
            if dir == *source {
                is_dir = !entries.is_empty();
            }
            for e in entries {
                let path = dir.join(&e.name)?;
                let name = format!("{rel}{}", e.name);
                if e.is_dir {
                    if depth + 1 >= SCAN_DEPTH {
                        return Err(PluginError::InvalidInput(format!(
                            "{name} is more than {SCAN_DEPTH} folders down in {}: a folder link may loop",
                            source.path
                        )));
                    }
                    dirs.push((path, format!("{name}/"), depth + 1));
                } else if files.len() < SCAN_FILES {
                    files.push(DownloadFile { path, rel: name, size: e.size });
                } else {
                    return Err(PluginError::InvalidInput(format!("{} has more than {SCAN_FILES} files", source.path)));
                }
            }
        }
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(is_dir.then(|| Self { source: source.clone(), files }))
    }

    /// The packages and images in it whose format `accept` takes, read by their first bytes:
    /// every file named `.pkg`, `.exfat` or `.img`, and the biggest others.
    pub async fn packages(
        &self,
        folders: &dyn Folders,
        accept: impl Fn(Format) -> bool,
    ) -> PluginResult<Vec<(DownloadFile, Format)>> {
        let named = |f: &DownloadFile| {
            let lower = f.rel.to_ascii_lowercase();
            [".pkg", ".exfat", ".img"].iter().any(|ext| lower.ends_with(ext))
        };
        let mut others: Vec<&DownloadFile> = self.files.iter().filter(|f| !named(f)).collect();
        others.sort_by_key(|f| std::cmp::Reverse(f.size));
        let mut out = Vec::new();
        for file in self.files.iter().filter(|f| named(f)).chain(others.into_iter().take(PICK_FILES)) {
            let head = folders.read_range(&file.path, 0, 512).await?;
            if let Some(format) = format(&head).filter(|f| accept(*f)) {
                out.push((file.clone(), format));
            }
        }
        out.sort_by(|a, b| a.0.rel.cmp(&b.0.rel));
        Ok(out)
    }

    /// The unpacked game folders in it, nearest the top first: each holds `sce_sys/<marker>`
    /// and a non-empty `eboot.bin`. A folder is its path with a `/` at the end, or `""` at the
    /// top.
    pub fn game_folders(&self, marker: &str) -> Vec<String> {
        let suffix = format!("sce_sys/{marker}");
        let mut found: Vec<String> = self
            .files
            .iter()
            .filter_map(|f| f.rel.strip_suffix(&suffix))
            .filter(|prefix| prefix.is_empty() || prefix.ends_with('/'))
            .filter(|prefix| self.files.iter().any(|f| f.size > 0 && f.rel == format!("{prefix}eboot.bin")))
            .map(str::to_owned)
            .collect();
        found.sort_by_key(|prefix| (prefix.matches('/').count(), prefix.clone()));
        found
    }

    /// A small file of it, such as `param.sfo`, read whole.
    pub async fn read(&self, folders: &dyn Folders, rel: &str, limit: u64) -> PluginResult<Vec<u8>> {
        let file = self
            .files
            .iter()
            .find(|f| f.rel == rel)
            .ok_or_else(|| PluginError::NotFound(format!("{rel} in {}", self.source.path)))?;
        if file.size > limit {
            return Err(PluginError::InvalidInput(format!("{rel} is too large")));
        }
        folders.read_range(&file.path, 0, file.size).await
    }

    /// The files under `prefix` as a tree, with their paths from `prefix`. Call it from async
    /// code; read the tree from blocking code.
    pub fn tree(&self, folders: Arc<dyn Folders>, prefix: &str) -> HostTree {
        let files = self
            .files
            .iter()
            .filter_map(|f| {
                let rel = f.rel.strip_prefix(prefix)?;
                Some(DownloadFile { path: f.path.clone(), rel: rel.to_owned(), size: f.size })
            })
            .collect();
        HostTree { folders, files, runtime: tokio::runtime::Handle::current() }
    }

    /// Why nothing in it installs.
    pub fn nothing_to_install(&self) -> PluginError {
        PluginError::InvalidInput(format!(
            "{} holds no package, disc image or unpacked game to install",
            self.source.path
        ))
    }
}

/// A folder of files in the host folders, read as a [`Tree`]. For blocking code, as
/// [`crate::read_at::HostFile`] is.
pub struct HostTree {
    folders: Arc<dyn Folders>,
    files: Vec<DownloadFile>,
    runtime: tokio::runtime::Handle,
}

impl HostTree {
    /// A tree of these files, each at its `rel` path in the tree. Call it from async code.
    pub fn of(folders: Arc<dyn Folders>, files: Vec<DownloadFile>) -> Self {
        Self { folders, files, runtime: tokio::runtime::Handle::current() }
    }
}

impl Tree for HostTree {
    fn entries(&mut self, _cancel: &dyn Fn() -> Result<()>) -> Result<Vec<Entry>> {
        Ok(self.files.iter().map(|f| Entry { path: f.rel.clone(), directory: false, size: f.size }).collect())
    }

    fn copy(&mut self, index: usize, sink: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let file = self.files.get(index).context("No such file in the folder")?.clone();
        let mut at = 0;
        while at < file.size {
            let want = (file.size - at).min(FLUSH as u64);
            let piece = self
                .runtime
                .block_on(self.folders.read_range(&file.path, at, want))
                .map_err(|e| Error::msg(e.to_string()))?;
            ensure!(!piece.is_empty(), "{} changed while installing", file.rel);
            at += piece.len() as u64;
            sink(&piece)?;
        }
        Ok(())
    }
}

/// Whether a tree's files hold a game: a `sce_sys/param.json` and a non-empty `eboot.bin`
/// beside its `sce_sys`.
pub fn holds_game(entries: &[Entry]) -> bool {
    game_folder(entries, "param.json").is_some_and(|prefix| {
        entries.iter().any(|e| !e.directory && e.size > 0 && e.path == format!("{prefix}eboot.bin"))
    })
}

/// Writes into host folders from blocking code, in pieces of a few MiB.
pub struct Writer {
    pub folders: Arc<dyn Folders>,
    pub runtime: tokio::runtime::Handle,
}

const FLUSH: usize = 4 * 1024 * 1024;

impl Writer {
    fn io<T>(&self, call: impl std::future::Future<Output = PluginResult<T>>) -> Result<T> {
        self.runtime.block_on(call).map_err(|e| Error::msg(e.to_string()))
    }

    pub fn exists(&self, path: &HostPath) -> Result<bool> {
        self.io(self.folders.exists(path))
    }

    pub fn remove(&self, path: &HostPath) -> Result<()> {
        self.io(self.folders.remove(path))
    }

    pub fn rename(&self, from: &HostPath, to: &HostPath) -> Result<()> {
        self.io(self.folders.rename(from, to))
    }

    /// Writes entry `index` of `tree` to `path`, adding what it writes to `done`.
    pub fn write_entry(
        &self,
        tree: &mut dyn Tree,
        index: usize,
        path: &HostPath,
        job: &dyn InstallJob,
        done: &mut u64,
        total: u64,
    ) -> Result<()> {
        let mut buffer = Vec::with_capacity(FLUSH);
        let mut at = 0u64;
        let flush = |buffer: &mut Vec<u8>, at: &mut u64| -> Result<()> {
            self.io(self.folders.write_at(path, *at, buffer))?;
            *at += buffer.len() as u64;
            buffer.clear();
            Ok(())
        };
        // An empty file is still a file.
        flush(&mut buffer, &mut at)?;
        tree.copy(index, &mut |piece| {
            ensure!(!job.cancelled(), "The install was cancelled");
            buffer.extend_from_slice(piece);
            *done += piece.len() as u64;
            if buffer.len() >= FLUSH.min(MAX_FOLDER_READ as usize) {
                flush(&mut buffer, &mut at)?;
                job.progress(*done, total);
            }
            Ok(())
        })?;
        flush(&mut buffer, &mut at)?;
        job.progress(*done, total);
        Ok(())
    }

    /// Reads entry `index` into memory: for small files such as `param.json`.
    pub fn read_entry(tree: &mut dyn Tree, index: usize, limit: u64) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        tree.copy(index, &mut |piece| {
            ensure!(out.len() as u64 + piece.len() as u64 <= limit, "A metadata file is too large");
            out.extend_from_slice(piece);
            Ok(())
        })?;
        Ok(out)
    }
}

/// The game folder inside a tree: where its `sce_sys/<marker>` is, nearest the top. `""`
/// when it is at the top.
pub fn game_folder(entries: &[Entry], marker: &str) -> Option<String> {
    let suffix = format!("sce_sys/{marker}");
    entries
        .iter()
        .filter(|e| !e.directory && (e.path == suffix || e.path.ends_with(&format!("/{suffix}"))))
        .map(|e| e.path[..e.path.len() - suffix.len()].to_owned())
        .min_by_key(|prefix| prefix.matches('/').count())
}

/// Writes every file of `tree` under `prefix` into `work`, without the prefix. Folders
/// come with the files in them.
pub fn write_tree(
    writer: &Writer,
    tree: &mut dyn Tree,
    entries: &[Entry],
    prefix: &str,
    work: &HostPath,
    job: &dyn InstallJob,
) -> Result<()> {
    let total = tree_size(entries, prefix);
    let mut done = 0;
    job.progress(0, total);
    write_tree_counted(writer, tree, entries, prefix, work, job, (&mut done, total))
}

/// The bytes [`write_tree`] writes for `prefix`.
pub fn tree_size(entries: &[Entry], prefix: &str) -> u64 {
    entries.iter().filter(|e| !e.directory && e.path.starts_with(prefix)).map(|e| e.size).sum()
}

/// [`write_tree`] as one part of a larger job: `done` of `total` bytes are written before it.
pub fn write_tree_counted(
    writer: &Writer,
    tree: &mut dyn Tree,
    entries: &[Entry],
    prefix: &str,
    work: &HostPath,
    job: &dyn InstallJob,
    (done, total): (&mut u64, u64),
) -> Result<()> {
    let files: Vec<usize> =
        (0..entries.len()).filter(|&i| !entries[i].directory && entries[i].path.starts_with(prefix)).collect();
    for i in files {
        ensure!(!job.cancelled(), "The install was cancelled");
        let path = RelPath::new(&entries[i].path[prefix.len()..]).map_err(|e| Error::msg(e.to_string()))?;
        let dest = HostPath::new(
            &work.folder,
            work.root,
            work.path.join(path.as_str()).map_err(|e| Error::msg(e.to_string()))?,
        );
        writer.write_entry(tree, i, &dest, job, done, total).context(format!("Install {}", entries[i].path))?;
    }
    Ok(())
}

/// Runs `work` and reports its end: on an error the work folder goes, and the job fails.
/// `work` returns where the game went and its title.
pub fn finish(
    writer: &Writer,
    work_folder: &HostPath,
    job: &dyn InstallJob,
    work: impl FnOnce() -> Result<(HostPath, String)>,
) {
    match work() {
        Ok((dest, title)) => job.finished(Ok(nexus_api::InstalledGame { dest, title })),
        Err(e) => {
            // If this fails too, the folder stays as `<name>.part`, which no game index reads.
            let _ = writer.remove(work_folder);
            job.finished(Err(PluginError::Other(e.to_string())));
        }
    }
}

/// Runs `work` on a blocking thread with a writer into the host folders, and reports its end
/// through `job`: on an error `work_folder` goes. Call it from the plugin's `begin`, which then
/// returns at once.
pub fn spawn(
    folders: Arc<dyn Folders>,
    work_folder: HostPath,
    job: Arc<dyn InstallJob>,
    work: impl FnOnce(&Writer, &dyn InstallJob) -> Result<(HostPath, String)> + Send + 'static,
) {
    let writer = Writer { folders, runtime: tokio::runtime::Handle::current() };
    tokio::task::spawn_blocking(move || {
        finish(&writer, &work_folder, job.as_ref(), || work(&writer, job.as_ref()));
    });
}

/// Runs short blocking work (opening a package reads its header and keys) off the runtime,
/// and waits for it.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> PluginResult<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(PluginError::from),
        Err(e) => Err(PluginError::Other(format!("reading the package stopped: {e}"))),
    }
}

/// A `cancel` callback for the readers: fails once the player cancelled.
pub fn cancel_check(job: &dyn InstallJob) -> impl Fn() -> Result<()> + '_ {
    move || {
        ensure!(!job.cancelled(), "The install was cancelled");
        Ok(())
    }
}
