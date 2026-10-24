//! The host's archives port: unpacking the archives a download holds, off the async runtime.
//!
//! Each archive is read twice. The first pass lists it and refuses, before anything is
//! written: an entry that needs a password, a path that leaves the folder (absolute, `..`, or
//! a link that points out), and an archive bigger than the free space. The second pass writes,
//! checks each path again on the disk (nothing is written through a link), reports progress and
//! stops soon after a cancel.
//!
//! Readers: `zip` for zip archives and GOG installers (a script with a zip at its end), `tar`
//! and `flate2` for tar and tar.gz, `sevenz-rust2` for 7z (a split 7z is one stream cut into
//! parts, read as one), and libarchive for RAR and the rest (see `libarchive.rs`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use nexus_api::{PluginError, PluginResult};
use nexus_core::{ArchiveKind, Archives, UnpackProgress, inside, link_stays_inside, stays_inside};

/// How much is copied between two progress reports and cancel checks.
const PIECE: usize = 1024 * 1024;

pub struct HostArchives;

#[async_trait]
impl Archives for HostArchives {
    async fn unpack(
        &self,
        kind: ArchiveKind,
        parts: &[PathBuf],
        into: &Path,
        progress: UnpackProgress,
        cancel: Arc<AtomicBool>,
    ) -> PluginResult<()> {
        let (parts, into) = (parts.to_vec(), into.to_path_buf());
        let work = Work { into, progress, cancel, done: 0, total: 0 };
        match tokio::task::spawn_blocking(move || unpack(kind, &parts, work)).await {
            Ok(result) => result.map_err(PluginError::Other),
            Err(e) => Err(PluginError::Other(format!("the unpacking stopped: {e}"))),
        }
    }
}

/// What an entry is.
#[derive(Debug, PartialEq)]
enum Item {
    File,
    Folder,
    /// A link to its target, from the link's folder.
    Link(PathBuf),
    /// Another name for an entry, from the archive's top.
    HardLink(PathBuf),
    /// Something to leave out: a device, a pipe.
    Other,
}

/// One entry of an archive, checked: its path is inside the folder.
#[derive(Debug)]
struct Entry {
    path: PathBuf,
    size: u64,
    item: Item,
}

/// An unpack in progress.
struct Work {
    into: PathBuf,
    progress: UnpackProgress,
    cancel: Arc<AtomicBool>,
    done: u64,
    total: u64,
}

impl Work {
    fn check_cancel(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) { Err("The unpacking was cancelled".into()) } else { Ok(()) }
    }

    /// Where an entry goes, after checking on the disk that nothing on the way is a link.
    fn place(&self, path: &Path) -> Result<PathBuf, String> {
        let out = self.into.join(path);
        if std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} would write through a link", path.display()));
        }
        if !stays_inside(&out, &self.into).map_err(|e| e.to_string())? {
            return Err(format!("{} leaves the folder through a link", path.display()));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(out)
    }

    /// Writes `reader` into the file at `path`, reporting progress as it goes.
    fn write(&mut self, path: &Path, reader: &mut dyn Read) -> Result<(), String> {
        let out = self.place(path)?;
        let mut file = File::create(&out).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut buffer = vec![0; PIECE];
        loop {
            self.check_cancel()?;
            let n = reader.read(&mut buffer).map_err(|e| read_error(path, &e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| format!("{}: {e}", path.display()))?;
            self.done += n as u64;
            (self.progress)(self.done, self.total);
        }
        Ok(())
    }

    fn folder(&self, path: &Path) -> Result<(), String> {
        let out = self.place(path)?;
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Another name for the entry at `target` (from the folder's top), which the listing checked.
    fn hard_link(&self, path: &Path, target: &Path) -> Result<(), String> {
        let out = self.place(path)?;
        std::fs::hard_link(self.into.join(target), out).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The file's Unix mode from the archive, so a program stays a program. Nothing on Windows.
    fn mode(&self, path: &Path, mode: u32) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode & 0o777);
            std::fs::set_permissions(self.into.join(path), perms).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        #[cfg(not(unix))]
        let _ = (path, mode);
        Ok(())
    }

    /// A link the listing checked, made only where links are files of their own.
    fn link(&self, path: &Path, target: &Path) -> Result<(), String> {
        let out = self.place(path)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, out).map_err(|e| format!("{}: {e}", path.display()))?;
        #[cfg(not(unix))]
        let _ = (out, target);
        Ok(())
    }
}

fn read_error(path: &Path, e: &str) -> String {
    format!("{} couldn't be read: {e}", path.display())
}

/// The error for an archive that needs a password.
pub(crate) fn password() -> String {
    "This archive needs a password, which Nexus can't ask for".into()
}

/// A checked entry, or why the archive is refused.
fn entry(name: &str, size: u64, item: Item) -> Result<Entry, String> {
    let path = inside(Path::new(name)).ok_or_else(|| format!("{name} leaves the folder"))?;
    if let Item::Link(target) = &item
        && !link_stays_inside(&path, target)
    {
        return Err(format!("{name} links out of the folder"));
    }
    Ok(Entry { path, size, item })
}

/// Which reader unpacks an archive.
enum Via {
    Zip,
    Tar,
    SevenZ,
    Libarchive,
}

fn unpack(kind: ArchiveKind, parts: &[PathBuf], mut work: Work) -> Result<(), String> {
    let first = parts.first().ok_or("an archive with no parts")?;
    // The Rust readers first; libarchive for the rest, and for a 7z with a codec
    // `sevenz-rust2` isn't built with.
    let (reader, entries) = match kind {
        ArchiveKind::Zip | ArchiveKind::Gog => (Via::Zip, zip_entries(first)?),
        ArchiveKind::Tar | ArchiveKind::TarGz => (Via::Tar, tar_entries(kind, first)?),
        ArchiveKind::SevenZ => match seven_entries(parts)? {
            Some(entries) => (Via::SevenZ, entries),
            None => (Via::Libarchive, lib_entries(kind, parts)?),
        },
        ArchiveKind::Rar
        | ArchiveKind::TarXz
        | ArchiveKind::TarBz2
        | ArchiveKind::TarZst
        | ArchiveKind::Iso
        | ArchiveKind::Cab => (Via::Libarchive, lib_entries(kind, parts)?),
    };
    work.total = entries.iter().filter(|e| e.item == Item::File).map(|e| e.size).sum();
    crate::disk::room(work.total, crate::disk::free_bytes(&work.into))?;
    match reader {
        Via::Zip => zip_write(first, &mut work),
        Via::Tar => tar_write(kind, first, &mut work),
        Via::SevenZ => seven_write(parts, &mut work),
        Via::Libarchive => lib_write(kind, parts, &entries, &mut work),
    }?;
    (work.progress)(work.total, work.total);
    Ok(())
}

// Zip, and GOG installers: the reader finds a zip's start from its end, past a script before it.

fn open_zip(path: &Path) -> Result<zip::ZipArchive<File>, String> {
    let file = File::open(path).map_err(|e| read_error(path, &e.to_string()))?;
    zip::ZipArchive::new(file).map_err(|e| read_error(path, &e.to_string()))
}

fn zip_entries(path: &Path) -> Result<Vec<Entry>, String> {
    let mut zip = open_zip(path)?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index_raw(i).map_err(|e| read_error(path, &e.to_string()))?;
        if file.encrypted() {
            return Err(password());
        }
        let name = file.name().map_err(|e| read_error(path, &e.to_string()))?.into_owned();
        let item = if file.is_dir() {
            Item::Folder
        } else if file.is_symlink() {
            let mut target = String::new();
            // A raw entry: a link's target is stored, or the reader can't read it.
            file.read_to_string(&mut target).map_err(|e| read_error(path, &e.to_string()))?;
            Item::Link(target.into())
        } else {
            Item::File
        };
        out.push(entry(&name, file.size(), item)?);
    }
    Ok(out)
}

fn zip_write(path: &Path, work: &mut Work) -> Result<(), String> {
    let mut zip = open_zip(path)?;
    for i in 0..zip.len() {
        work.check_cancel()?;
        let mut file = zip.by_index(i).map_err(|e| read_error(path, &e.to_string()))?;
        let name = file.name().map_err(|e| read_error(path, &e.to_string()))?.into_owned();
        let rel = inside(Path::new(&name)).ok_or_else(|| format!("{name} leaves the folder"))?;
        if file.is_dir() {
            work.folder(&rel)?;
        } else if file.is_symlink() {
            let mut target = String::new();
            file.read_to_string(&mut target).map_err(|e| read_error(&rel, &e.to_string()))?;
            if !link_stays_inside(&rel, Path::new(&target)) {
                return Err(format!("{name} links out of the folder"));
            }
            work.link(&rel, Path::new(&target))?;
        } else {
            work.write(&rel, &mut file)?;
            if let Some(mode) = file.unix_mode() {
                work.mode(&rel, mode)?;
            }
        }
    }
    Ok(())
}

// Tar and tar.gz.

fn open_tar(kind: ArchiveKind, path: &Path) -> Result<tar::Archive<Box<dyn Read>>, String> {
    let file = File::open(path).map_err(|e| read_error(path, &e.to_string()))?;
    let reader: Box<dyn Read> =
        if kind == ArchiveKind::TarGz { Box::new(flate2::read::GzDecoder::new(file)) } else { Box::new(file) };
    Ok(tar::Archive::new(reader))
}

/// A tar entry's checked path and what it is; `None` for the archive's own `./`.
fn tar_entry(e: &tar::Entry<'_, Box<dyn Read>>) -> Result<Option<Entry>, String> {
    let path = e.path().map_err(|err| err.to_string())?.into_owned();
    if path.components().all(|c| c == std::path::Component::CurDir) {
        return Ok(None);
    }
    let name = path.to_string_lossy().into_owned();
    let kind = e.header().entry_type();
    let size = e.header().size().unwrap_or(0);
    let target = || -> Result<PathBuf, String> {
        Ok(e.link_name().map_err(|err| err.to_string())?.unwrap_or_default().into_owned())
    };
    if kind.is_hard_link() {
        // A hard link names another entry from the archive's top.
        let path = inside(&path).ok_or_else(|| format!("{name} leaves the folder"))?;
        let target = inside(&target()?).ok_or_else(|| format!("{name} links out of the folder"))?;
        return Ok(Some(Entry { path, size, item: Item::HardLink(target) }));
    }
    let item = if kind.is_dir() {
        Item::Folder
    } else if kind.is_file() {
        Item::File
    } else if kind.is_symlink() {
        Item::Link(target()?)
    } else {
        Item::Other
    };
    entry(&name, size, item).map(Some)
}

fn tar_entries(kind: ArchiveKind, path: &Path) -> Result<Vec<Entry>, String> {
    let mut tar = open_tar(kind, path)?;
    let mut out = Vec::new();
    for e in tar.entries().map_err(|e| read_error(path, &e.to_string()))? {
        let e = e.map_err(|e| read_error(path, &e.to_string()))?;
        if let Some(entry) = tar_entry(&e)? {
            out.push(entry);
        }
    }
    Ok(out)
}

fn tar_write(kind: ArchiveKind, path: &Path, work: &mut Work) -> Result<(), String> {
    let mut tar = open_tar(kind, path)?;
    for e in tar.entries().map_err(|e| read_error(path, &e.to_string()))? {
        work.check_cancel()?;
        let mut e = e.map_err(|e| read_error(path, &e.to_string()))?;
        let Some(checked) = tar_entry(&e)? else { continue };
        match &checked.item {
            Item::Folder => work.folder(&checked.path)?,
            Item::File => {
                work.write(&checked.path, &mut e)?;
                if let Ok(mode) = e.header().mode() {
                    work.mode(&checked.path, mode)?;
                }
            }
            Item::HardLink(target) => work.hard_link(&checked.path, target)?,
            Item::Link(target) => work.link(&checked.path, target)?,
            Item::Other => {}
        }
    }
    Ok(())
}

// 7z. A split 7z (`game.7z.001`, `game.7z.002`) is one archive cut into parts.

/// The parts of a split file, read as one.
struct Joined {
    parts: Vec<(File, u64)>,
    len: u64,
    at: u64,
}

impl Joined {
    fn open(paths: &[PathBuf]) -> Result<Self, String> {
        let mut parts = Vec::new();
        for path in paths {
            let file = File::open(path).map_err(|e| read_error(path, &e.to_string()))?;
            let len = file.metadata().map_err(|e| read_error(path, &e.to_string()))?.len();
            parts.push((file, len));
        }
        let len = parts.iter().map(|(_, l)| l).sum();
        Ok(Self { parts, len, at: 0 })
    }
}

impl Read for Joined {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut start = 0;
        for (file, len) in &mut self.parts {
            if self.at < start + *len {
                file.seek(SeekFrom::Start(self.at - start))?;
                let want = buf.len().min((start + *len - self.at) as usize);
                let n = file.read(&mut buf[..want])?;
                self.at += n as u64;
                return Ok(n);
            }
            start += *len;
        }
        Ok(0)
    }
}

impl Seek for Joined {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let at = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(n) => self.len.checked_add_signed(n),
            SeekFrom::Current(n) => self.at.checked_add_signed(n),
        };
        self.at = at.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a seek before the start"))?;
        Ok(self.at)
    }
}

fn open_seven(parts: &[PathBuf]) -> Result<sevenz_rust2::ArchiveReader<Joined>, String> {
    let name = parts.first().map(|p| p.as_path()).unwrap_or(Path::new("archive"));
    sevenz_rust2::ArchiveReader::new(Joined::open(parts)?, sevenz_rust2::Password::empty()).map_err(|e| match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => password(),
        e => read_error(name, &e.to_string()),
    })
}

/// Whether a 7z entry is a link: its attributes carry a Unix mode in the high 16 bits.
fn seven_is_link(entry: &sevenz_rust2::ArchiveEntry) -> bool {
    let attributes = entry.windows_attributes;
    entry.has_windows_attributes && attributes & 0x8000 != 0 && (attributes >> 16) & 0o170000 == 0o120000
}

/// The codecs `sevenz-rust2` is built without here: a 7z that uses one goes to libarchive.
const SEVEN_ELSEWHERE: [&[u8]; 7] = [
    sevenz_rust2::EncoderMethod::ID_BZIP2,
    sevenz_rust2::EncoderMethod::ID_ZSTD,
    sevenz_rust2::EncoderMethod::ID_BROTLI,
    sevenz_rust2::EncoderMethod::ID_LZ4,
    sevenz_rust2::EncoderMethod::ID_LZS,
    sevenz_rust2::EncoderMethod::ID_LIZARD,
    sevenz_rust2::EncoderMethod::ID_DEFLATE64,
];

/// A 7z's entries, or `None` when one uses a codec this reader lacks.
fn seven_entries(parts: &[PathBuf]) -> Result<Option<Vec<Entry>>, String> {
    let mut reader = open_seven(parts)?;
    let files = reader.archive().files.clone();
    let mut out = Vec::new();
    let mut links = Vec::new();
    for file in &files {
        let mut methods = Vec::new();
        if file.has_stream {
            if reader.file_compression_methods(&file.name, &mut methods).is_err()
                || methods.iter().any(|m| SEVEN_ELSEWHERE.contains(&m.id()))
            {
                return Ok(None);
            }
            if methods.contains(&sevenz_rust2::EncoderMethod::AES256_SHA256) {
                return Err(password());
            }
        }
        if seven_is_link(file) {
            links.push(file.name.clone());
            continue;
        }
        let item = if file.is_directory { Item::Folder } else { Item::File };
        out.push(entry(&file.name, file.size, item)?);
    }
    // A link's target is its contents.
    for name in links {
        let target = reader.read_file(&name).map_err(|e| read_error(Path::new(&name), &e.to_string()))?;
        out.push(entry(&name, 0, Item::Link(String::from_utf8_lossy(&target).into_owned().into()))?);
    }
    Ok(Some(out))
}

fn seven_write(parts: &[PathBuf], work: &mut Work) -> Result<(), String> {
    let mut reader = open_seven(parts)?;
    let mut failure = None;
    let result = reader.for_each_entries(|file, data| {
        let step = (|| {
            work.check_cancel()?;
            let rel = inside(Path::new(&file.name)).ok_or_else(|| format!("{} leaves the folder", file.name))?;
            if file.is_directory {
                work.folder(&rel)
            } else if seven_is_link(file) {
                let mut target = String::new();
                data.read_to_string(&mut target).map_err(|e| read_error(&rel, &e.to_string()))?;
                if !link_stays_inside(&rel, Path::new(&target)) {
                    return Err(format!("{} links out of the folder", file.name));
                }
                work.link(&rel, Path::new(&target))
            } else {
                work.write(&rel, data)
            }
        })();
        match step {
            Ok(()) => Ok(true),
            Err(e) => {
                failure = Some(e);
                Ok(false)
            }
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    result.map_err(|e| match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => password(),
        e => format!("the 7z archive couldn't be read: {e}"),
    })
}

// Through libarchive: RAR, tar.xz, tar.bz2, tar.zst, ISO, cab, and 7z with other codecs. It
// reads a set's parts as one stream, in the order given.

fn lib_api(kind: ArchiveKind) -> Result<&'static crate::libarchive::Api, String> {
    crate::libarchive::api().ok_or_else(|| {
        format!(
            "{} archives need libarchive, which isn't here: install {}",
            serde_name(kind),
            crate::libarchive::install_hint()
        )
    })
}

/// A kind's name as the archive's ending: `rar`, `tar.xz`.
fn serde_name(kind: ArchiveKind) -> String {
    nexus_api::serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_uppercase)).unwrap_or_default()
}

fn lib_entries(kind: ArchiveKind, parts: &[PathBuf]) -> Result<Vec<Entry>, String> {
    use crate::libarchive::Kind;
    let mut reader = crate::libarchive::Reader::open(lib_api(kind)?, parts)?;
    let mut out = Vec::new();
    while let Some(header) = reader.next()? {
        if header.encrypted {
            return Err(password());
        }
        out.push(match header.kind {
            Kind::HardLink(target) => {
                let target = inside(&target).ok_or_else(|| format!("{} links out of the folder", header.path))?;
                let path =
                    inside(Path::new(&header.path)).ok_or_else(|| format!("{} leaves the folder", header.path))?;
                Entry { path, size: header.size, item: Item::HardLink(target) }
            }
            Kind::File => entry(&header.path, header.size, Item::File)?,
            Kind::Folder => entry(&header.path, header.size, Item::Folder)?,
            Kind::Link(target) => entry(&header.path, header.size, Item::Link(target))?,
            Kind::Other => entry(&header.path, header.size, Item::Other)?,
        });
        reader.skip()?;
    }
    Ok(out)
}

fn lib_write(kind: ArchiveKind, parts: &[PathBuf], entries: &[Entry], work: &mut Work) -> Result<(), String> {
    let mut reader = crate::libarchive::Reader::open(lib_api(kind)?, parts)?;
    // The second pass reads the entries the first one checked, in the same order.
    for checked in entries {
        work.check_cancel()?;
        if reader.next()?.is_none() {
            return Err("the archive changed while it was unpacked".into());
        }
        match &checked.item {
            Item::Folder => work.folder(&checked.path)?,
            Item::File => work.write(&checked.path, &mut reader)?,
            Item::Link(target) => work.link(&checked.path, target)?,
            Item::HardLink(target) => work.hard_link(&checked.path, target)?,
            Item::Other => reader.skip()?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;

    /// A new empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nexus-archives-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Unpacks into `<dir>/out`, and returns the last progress it reported.
    fn run(kind: ArchiveKind, parts: &[PathBuf], dir: &Path) -> Result<(u64, u64), String> {
        let into = dir.join("out");
        std::fs::create_dir_all(&into).unwrap();
        let last = Arc::new(Mutex::new((0, 0)));
        let progress: UnpackProgress = {
            let last = last.clone();
            Arc::new(move |done, total| *last.lock().unwrap() = (done, total))
        };
        let work = Work { into, progress, cancel: Arc::new(AtomicBool::new(false)), done: 0, total: 0 };
        unpack(kind, parts, work)?;
        Ok(*last.lock().unwrap())
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    fn seven_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut seven = sevenz_rust2::ArchiveWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
        for (name, bytes) in entries {
            seven.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file(name), Some(*bytes)).unwrap();
        }
        seven.finish().unwrap().into_inner()
    }

    #[test]
    fn a_zip_unpacks_its_files_and_folders_with_progress() {
        let dir = folder("zip");
        std::fs::write(
            dir.join("game.zip"),
            zip_of(&[("Game/", b""), ("Game/eboot.bin", b"elf"), ("readme.txt", b"hi")]),
        )
        .unwrap();
        assert_eq!(run(ArchiveKind::Zip, &[dir.join("game.zip")], &dir).unwrap(), (5, 5));
        assert_eq!(std::fs::read(dir.join("out/Game/eboot.bin")).unwrap(), b"elf");
        assert_eq!(std::fs::read(dir.join("out/readme.txt")).unwrap(), b"hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_that_leaves_the_folder_is_refused_before_anything_is_written() {
        let dir = folder("escape");
        std::fs::write(dir.join("bad.zip"), zip_of(&[("good.txt", b"ok"), ("../evil.txt", b"no")])).unwrap();
        let refused = run(ArchiveKind::Zip, &[dir.join("bad.zip")], &dir).unwrap_err();
        assert!(refused.contains("leaves the folder"), "{refused}");
        assert!(!dir.join("out/good.txt").exists(), "nothing written");
        assert!(!dir.join("evil.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gog_installer_unpacks_the_zip_after_its_script() {
        let dir = folder("gog");
        let mut gog = b"#!/bin/sh\necho this never runs\n".to_vec();
        gog.resize(2048, b'#');
        gog.extend(zip_of(&[("data/noarch/game/start.sh", b"#!/bin/sh\n./game\n")]));
        std::fs::write(dir.join("game.sh"), gog).unwrap();
        run(ArchiveKind::Gog, &[dir.join("game.sh")], &dir).unwrap();
        assert!(dir.join("out/data/noarch/game/start.sh").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tar_gz_unpacks_its_files_and_hard_links_and_refuses_a_link_out() {
        let dir = folder("tar");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        tar.append_data(&mut header, "Game/game.x86_64", &b"elf"[..]).unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Link);
        link.set_size(0);
        tar.append_link(&mut link, "Game/again", "Game/game.x86_64").unwrap();
        std::fs::write(dir.join("game.tar.gz"), tar.into_inner().unwrap().finish().unwrap()).unwrap();
        run(ArchiveKind::TarGz, &[dir.join("game.tar.gz")], &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("out/Game/game.x86_64")).unwrap(), b"elf");
        assert_eq!(std::fs::read(dir.join("out/Game/again")).unwrap(), b"elf");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("out/Game/game.x86_64")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "a program stays a program");
        }

        let mut bad = tar::Builder::new(Vec::new());
        let mut out = tar::Header::new_gnu();
        out.set_entry_type(tar::EntryType::Symlink);
        out.set_size(0);
        bad.append_link(&mut out, "Game/passwd", "../../../etc/passwd").unwrap();
        std::fs::write(dir.join("bad.tar"), bad.into_inner().unwrap()).unwrap();
        let refused = run(ArchiveKind::Tar, &[dir.join("bad.tar")], &folder("tar-bad")).unwrap_err();
        assert!(refused.contains("links out of the folder"), "{refused}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_7z_unpacks_whole_and_cut_into_parts() {
        let dir = folder("7z");
        let seven = seven_of(&[("Game/eboot.bin", b"elf"), ("Game/data.bin", &[7; 5000])]);
        std::fs::write(dir.join("game.7z"), &seven).unwrap();
        assert_eq!(run(ArchiveKind::SevenZ, &[dir.join("game.7z")], &dir).unwrap(), (5003, 5003));
        assert_eq!(std::fs::read(dir.join("out/Game/eboot.bin")).unwrap(), b"elf");

        let split = folder("7z-split");
        let (one, two) = seven.split_at(seven.len() / 2);
        std::fs::write(split.join("game.7z.001"), one).unwrap();
        std::fs::write(split.join("game.7z.002"), two).unwrap();
        run(ArchiveKind::SevenZ, &[split.join("game.7z.001"), split.join("game.7z.002")], &split).unwrap();
        assert_eq!(std::fs::read(split.join("out/Game/data.bin")).unwrap(), vec![7; 5000]);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&split).unwrap();
    }

    #[test]
    fn a_rar_unpacks_and_one_that_needs_a_password_is_refused() {
        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let dir = folder("rar");
        run(ArchiveKind::Rar, &[data.join("version.rar")], &dir).unwrap();
        assert!(!std::fs::read(dir.join("out/VERSION")).unwrap().is_empty());

        let locked = folder("rar-locked");
        let refused = run(ArchiveKind::Rar, &[data.join("crypted.rar")], &locked).unwrap_err();
        assert!(refused.contains("needs a password"), "{refused}");
        let left: Vec<_> = std::fs::read_dir(locked.join("out")).unwrap().collect();
        assert!(left.is_empty(), "nothing written");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&locked).unwrap();
    }

    #[test]
    fn libarchive_reads_what_the_rust_readers_don_t() {
        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        for (kind, file) in [
            (ArchiveKind::TarXz, "game.tar.xz"),
            (ArchiveKind::TarBz2, "game.tar.bz2"),
            (ArchiveKind::TarZst, "game.tar.zst"),
            // A 7z whose codec, BZip2, `sevenz-rust2` isn't built with here.
            (ArchiveKind::SevenZ, "bzip2.7z"),
        ] {
            let dir = folder(&format!("lib-{file}"));
            run(kind, &[data.join(file)], &dir).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(std::fs::read(dir.join("out/Game/eboot.bin")).unwrap(), b"elf", "{file}");
            assert_eq!(std::fs::read(dir.join("out/Game/data.bin")).unwrap(), b"data", "{file}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_cancel_stops_the_unpacking() {
        let dir = folder("cancel");
        std::fs::write(dir.join("game.zip"), zip_of(&[("a.bin", &[1; 10]), ("b.bin", &[2; 10])])).unwrap();
        let into = dir.join("out");
        std::fs::create_dir_all(&into).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let work = Work { into, progress: Arc::new(|_, _| {}), cancel, done: 0, total: 0 };
        let stopped = unpack(ArchiveKind::Zip, &[dir.join("game.zip")], work).unwrap_err();
        assert!(stopped.contains("cancelled"), "{stopped}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
