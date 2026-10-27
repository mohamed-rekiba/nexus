//! The host's archives port: unpacking the archives a download holds, off the async runtime.
//!
//! Each archive is read twice. The first pass lists it with a format reader, and the core's
//! plan ([`plan_unpack`]) refuses before anything is written: an entry that needs a password,
//! a path that leaves the folder (absolute, `..`, or a link that points out), and an archive
//! bigger than the free space. The second pass writes the plan's entries only, checks each
//! path again on the disk (nothing is written through a link), reports progress and stops soon
//! after a cancel.
//!
//! Readers: `zip` for zip archives and GOG installers (a script with a zip at its end), `tar`
//! and `flate2` for tar and tar.gz, `sevenz-rust2` for 7z (a split 7z is one stream cut into
//! parts, read as one), and libarchive for RAR and the rest (see `libarchive.rs`). A RAR, or a
//! script installer's archive, that libarchive can't read ([`ArchiveError::Unsupported`]) goes
//! to `unar` (The Unarchiver) when it is installed: a RAR 5 with a dictionary bigger than
//! libarchive takes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use nexus_core::{
    ArchiveEntry, ArchiveError, ArchiveKind, Archives, EntryKind, Escape, Plan, Planned, UnpackProgress, plan_unpack,
    room, stays_inside,
};

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
        free: Option<u64>,
        progress: UnpackProgress,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError> {
        let (parts, into) = (parts.to_vec(), into.to_path_buf());
        let work = Work { into, free, progress, cancel, done: 0, total: 0 };
        match tokio::task::spawn_blocking(move || unpack(kind, &parts, work)).await {
            Ok(result) => result,
            Err(e) => Err(ArchiveError::Io(format!("the unpacking stopped: {e}"))),
        }
    }
}

/// An unpack in progress.
struct Work {
    into: PathBuf,
    /// The free bytes on the disk when the unpack began; `None` when it doesn't say.
    free: Option<u64>,
    progress: UnpackProgress,
    cancel: Arc<AtomicBool>,
    done: u64,
    total: u64,
}

impl Work {
    fn check_cancel(&self) -> Result<(), ArchiveError> {
        if self.cancel.load(Ordering::Acquire) { Err(ArchiveError::Cancelled) } else { Ok(()) }
    }

    /// Where a planned entry goes, after checking on the disk that nothing on the way is a link.
    fn place(&self, entry: &Planned) -> Result<PathBuf, ArchiveError> {
        let escapes = |how| ArchiveError::Escapes { entry: entry.name.clone(), how };
        let out = self.into.join(&entry.path);
        if std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(escapes(Escape::ThroughLink));
        }
        if !stays_inside(&out, &self.into).map_err(|e| ArchiveError::Io(e.to_string()))? {
            return Err(escapes(Escape::LeavesThroughLink));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(&entry.path, e))?;
        }
        Ok(out)
    }

    /// Writes `reader` into the file `entry`, reporting progress as it goes.
    fn write(&mut self, entry: &Planned, reader: &mut dyn Read) -> Result<(), ArchiveError> {
        let out = self.place(entry)?;
        let mut file = File::create(&out).map_err(|e| io(&entry.path, e))?;
        let mut buffer = vec![0; PIECE];
        loop {
            self.check_cancel()?;
            let n = reader.read(&mut buffer).map_err(|e| read_failed(&entry.path, e))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| io(&entry.path, e))?;
            self.done += n as u64;
            (self.progress)(self.done, self.total);
        }
        Ok(())
    }

    fn folder(&self, entry: &Planned) -> Result<(), ArchiveError> {
        let out = self.place(entry)?;
        std::fs::create_dir_all(out).map_err(|e| io(&entry.path, e))
    }

    /// Another name for the entry at `target` (from the folder's top), which the plan checked.
    fn hard_link(&self, entry: &Planned, target: &Path) -> Result<(), ArchiveError> {
        let out = self.place(entry)?;
        std::fs::hard_link(self.into.join(target), out).map_err(|e| io(&entry.path, e))
    }

    /// The file's Unix mode from the archive, so a program stays a program. Nothing on Windows.
    fn mode(&self, path: &Path, mode: u32) -> Result<(), ArchiveError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode & 0o777);
            std::fs::set_permissions(self.into.join(path), perms).map_err(|e| io(path, e))?;
        }
        #[cfg(not(unix))]
        let _ = (path, mode);
        Ok(())
    }

    /// A link the plan checked, made only where links are files of their own.
    fn link(&self, entry: &Planned, target: &Path) -> Result<(), ArchiveError> {
        let out = self.place(entry)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, out).map_err(|e| io(&entry.path, e))?;
        #[cfg(not(unix))]
        let _ = (out, target);
        Ok(())
    }

    /// Writes one planned entry whose data, for a file, is `data`.
    fn put(&mut self, entry: &Planned, data: &mut dyn Read) -> Result<(), ArchiveError> {
        match &entry.kind {
            EntryKind::Folder => self.folder(entry),
            EntryKind::File => self.write(entry, data),
            EntryKind::Link(target) => self.link(entry, target),
            EntryKind::HardLink(target) => self.hard_link(entry, target),
            EntryKind::Other => Ok(()),
        }
    }
}

/// A file in the folder that couldn't be written.
fn io(path: &Path, e: std::io::Error) -> ArchiveError {
    ArchiveError::Io(format!("{}: {e}", path.display()))
}

/// An archive a Rust reader couldn't read: damaged, or cut short.
fn read_error(path: &Path, e: &dyn std::fmt::Display) -> ArchiveError {
    ArchiveError::Corrupt(format!("{} couldn't be read: {e}", path.display()))
}

/// An entry's data that couldn't be read. libarchive's errors carry their [`ArchiveError`],
/// which keeps its kind, so an archive libarchive can't read still goes to unar.
fn read_failed(path: &Path, e: std::io::Error) -> ArchiveError {
    let inner = e.get_ref().and_then(|inner| inner.downcast_ref::<ArchiveError>());
    match inner {
        Some(ArchiveError::Unsupported(why)) => {
            ArchiveError::Unsupported(format!("{} couldn't be read: {why}", path.display()))
        }
        Some(ArchiveError::Corrupt(why)) => read_error(path, why),
        Some(other) => other.clone(),
        None => read_error(path, &e),
    }
}

/// The error for an archive the second pass doesn't find as the first one listed it.
fn changed() -> ArchiveError {
    ArchiveError::Corrupt("the archive changed while it was unpacked".into())
}

fn no_parts() -> ArchiveError {
    ArchiveError::Io("an archive with no parts".into())
}

/// Which reader unpacks an archive.
enum Via {
    Zip,
    Tar,
    SevenZ,
    Libarchive,
}

fn unpack(kind: ArchiveKind, parts: &[PathBuf], mut work: Work) -> Result<(), ArchiveError> {
    let error = match unpack_set(kind, parts, &mut work) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    // A RAR or script installer's archive libarchive can't read: unar may. Not one it refused
    // for a password, a path or the room.
    let ArchiveError::Unsupported(why) = &error else { return Err(error) };
    if !matches!(kind, ArchiveKind::Rar | ArchiveKind::Script) || work.cancel.load(Ordering::Acquire) {
        return Err(error);
    }
    let Some(tools) = Unar::find() else {
        let by_hand = if kind == ArchiveKind::Script { ", or run the installer by hand" } else { "" };
        return Err(ArchiveError::Missing(format!("{why}. Install unar to unpack it{by_hand}")));
    };
    // What libarchive wrote before it stopped.
    std::fs::remove_dir_all(&work.into)
        .and_then(|()| std::fs::create_dir_all(&work.into))
        .map_err(|e| io(&work.into, e))?;
    work.done = 0;
    work.total = 0;
    tools.unpack(kind, parts, work)
}

fn unpack_set(kind: ArchiveKind, parts: &[PathBuf], work: &mut Work) -> Result<(), ArchiveError> {
    let first = parts.first().ok_or_else(no_parts)?;
    // The Rust readers first; libarchive for the rest, and for a 7z with a codec
    // `sevenz-rust2` isn't built with.
    let (reader, listing) = match kind {
        ArchiveKind::Zip | ArchiveKind::Gog => (Via::Zip, zip_entries(first)?),
        ArchiveKind::Tar | ArchiveKind::TarGz => (Via::Tar, tar_entries(kind, first)?),
        ArchiveKind::SevenZ => match seven_entries(parts)? {
            Some(listing) => (Via::SevenZ, listing),
            None => (Via::Libarchive, lib_entries(kind, parts)?),
        },
        ArchiveKind::Rar
        | ArchiveKind::TarXz
        | ArchiveKind::TarBz2
        | ArchiveKind::TarZst
        | ArchiveKind::Iso
        | ArchiveKind::Cab
        | ArchiveKind::Script => (Via::Libarchive, lib_entries(kind, parts)?),
    };
    let plan = plan_unpack(listing, &work.into, work.free)?;
    work.total = plan.total;
    match reader {
        Via::Zip => zip_write(first, &plan, work),
        Via::Tar => tar_write(kind, first, &plan, work),
        Via::SevenZ => seven_write(parts, &plan, work),
        Via::Libarchive => lib_write(kind, parts, &plan, work),
    }?;
    (work.progress)(work.total, work.total);
    Ok(())
}

// Zip, and GOG installers: the reader finds a zip's start from its end, past a script before it.

fn open_zip(path: &Path) -> Result<zip::ZipArchive<File>, ArchiveError> {
    let file = File::open(path).map_err(|e| io(path, e))?;
    zip::ZipArchive::new(file).map_err(|e| read_error(path, &e))
}

fn zip_entries(path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut zip = open_zip(path)?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index_raw(i).map_err(|e| read_error(path, &e))?;
        let name = file.name().map_err(|e| read_error(path, &e))?.into_owned();
        let encrypted = file.encrypted();
        let kind = if file.is_dir() {
            EntryKind::Folder
        } else if file.is_symlink() && !encrypted {
            let mut target = String::new();
            // A raw entry: a link's target is stored, or the reader can't read it.
            file.read_to_string(&mut target).map_err(|e| read_error(path, &e))?;
            EntryKind::Link(target.into())
        } else {
            EntryKind::File
        };
        out.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(name, file.size(), kind) });
    }
    Ok(out)
}

fn zip_write(path: &Path, plan: &Plan, work: &mut Work) -> Result<(), ArchiveError> {
    let mut zip = open_zip(path)?;
    if zip.len() != plan.entries.len() {
        return Err(changed());
    }
    for (i, entry) in plan.entries.iter().enumerate() {
        work.check_cancel()?;
        let mut file = zip.by_index(i).map_err(|e| read_error(path, &e))?;
        if *file.name().map_err(|e| read_error(path, &e))? != *entry.name {
            return Err(changed());
        }
        work.put(entry, &mut file)?;
        if entry.kind == EntryKind::File
            && let Some(mode) = file.unix_mode()
        {
            work.mode(&entry.path, mode)?;
        }
    }
    Ok(())
}

// Tar and tar.gz.

fn open_tar(kind: ArchiveKind, path: &Path) -> Result<tar::Archive<Box<dyn Read>>, ArchiveError> {
    let file = File::open(path).map_err(|e| io(path, e))?;
    let reader: Box<dyn Read> =
        if kind == ArchiveKind::TarGz { Box::new(flate2::read::GzDecoder::new(file)) } else { Box::new(file) };
    Ok(tar::Archive::new(reader))
}

/// A tar entry as the archive lists it.
fn tar_entry(path: &Path, e: &tar::Entry<'_, Box<dyn Read>>) -> Result<ArchiveEntry, ArchiveError> {
    let name = e.path().map_err(|err| read_error(path, &err))?.to_string_lossy().into_owned();
    let kind = e.header().entry_type();
    let size = e.header().size().unwrap_or(0);
    let target = || -> Result<PathBuf, ArchiveError> {
        Ok(e.link_name().map_err(|err| read_error(path, &err))?.unwrap_or_default().into_owned())
    };
    let kind = if kind.is_hard_link() {
        EntryKind::HardLink(target()?)
    } else if kind.is_dir() {
        EntryKind::Folder
    } else if kind.is_file() {
        EntryKind::File
    } else if kind.is_symlink() {
        EntryKind::Link(target()?)
    } else {
        EntryKind::Other
    };
    Ok(ArchiveEntry::new(name, size, kind))
}

fn tar_entries(kind: ArchiveKind, path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut tar = open_tar(kind, path)?;
    let mut out = Vec::new();
    for e in tar.entries().map_err(|e| read_error(path, &e))? {
        let e = e.map_err(|e| read_error(path, &e))?;
        out.push(tar_entry(path, &e)?);
    }
    Ok(out)
}

fn tar_write(kind: ArchiveKind, path: &Path, plan: &Plan, work: &mut Work) -> Result<(), ArchiveError> {
    let mut tar = open_tar(kind, path)?;
    let mut planned = plan.entries.iter();
    for e in tar.entries().map_err(|e| read_error(path, &e))? {
        work.check_cancel()?;
        let mut e = e.map_err(|e| read_error(path, &e))?;
        let entry = planned.next().ok_or_else(changed)?;
        if tar_entry(path, &e)?.name != entry.name {
            return Err(changed());
        }
        work.put(entry, &mut e)?;
        if entry.kind == EntryKind::File
            && let Ok(mode) = e.header().mode()
        {
            work.mode(&entry.path, mode)?;
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
    fn open(paths: &[PathBuf]) -> Result<Self, ArchiveError> {
        let mut parts = Vec::new();
        for path in paths {
            let file = File::open(path).map_err(|e| io(path, e))?;
            let len = file.metadata().map_err(|e| io(path, e))?.len();
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

/// A `sevenz-rust2` error as an archive error: a password, a codec it is built without, or a
/// damaged archive.
fn seven_error(e: sevenz_rust2::Error, damaged: impl FnOnce(sevenz_rust2::Error) -> ArchiveError) -> ArchiveError {
    match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => ArchiveError::Password,
        sevenz_rust2::Error::UnsupportedCompressionMethod(method) => {
            ArchiveError::Unsupported(format!("the 7z reader is built without {method}"))
        }
        e => damaged(e),
    }
}

fn open_seven(parts: &[PathBuf]) -> Result<sevenz_rust2::ArchiveReader<Joined>, ArchiveError> {
    let name = parts.first().map(|p| p.as_path()).unwrap_or(Path::new("archive"));
    sevenz_rust2::ArchiveReader::new(Joined::open(parts)?, sevenz_rust2::Password::empty())
        .map_err(|e| seven_error(e, |e| read_error(name, &e)))
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
fn seven_entries(parts: &[PathBuf]) -> Result<Option<Vec<ArchiveEntry>>, ArchiveError> {
    let mut reader = match open_seven(parts) {
        Ok(reader) => reader,
        // A codec this reader lacks in the listing itself, such as the AES of an encrypted one.
        Err(ArchiveError::Unsupported(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let files = reader.archive().files.clone();
    let mut out = Vec::new();
    let mut links = Vec::new();
    for file in &files {
        let mut methods = Vec::new();
        let mut encrypted = false;
        if file.has_stream {
            if reader.file_compression_methods(&file.name, &mut methods).is_err()
                || methods.iter().any(|m| SEVEN_ELSEWHERE.contains(&m.id()))
            {
                return Ok(None);
            }
            encrypted = methods.contains(&sevenz_rust2::EncoderMethod::AES256_SHA256);
        }
        if seven_is_link(file) && !encrypted {
            links.push(file.name.clone());
            continue;
        }
        let kind = if file.is_directory { EntryKind::Folder } else { EntryKind::File };
        out.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(&file.name, file.size, kind) });
    }
    // A link's target is its contents.
    for name in links {
        let target = reader.read_file(&name).map_err(|e| read_error(Path::new(&name), &e))?;
        out.push(ArchiveEntry::new(&name, 0, EntryKind::Link(String::from_utf8_lossy(&target).into_owned().into())));
    }
    Ok(Some(out))
}

fn seven_write(parts: &[PathBuf], plan: &Plan, work: &mut Work) -> Result<(), ArchiveError> {
    let mut reader = open_seven(parts)?;
    // The reader gives the entries block by block, not in the listing's order: each is found
    // by its name.
    let planned: HashMap<&str, &Planned> = plan.entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut failure = None;
    let result = reader.for_each_entries(|file, data| {
        let step = (|| -> Result<(), ArchiveError> {
            work.check_cancel()?;
            let entry = planned.get(file.name.as_str()).ok_or_else(changed)?;
            work.put(entry, data)?;
            // The next entry's data follows this one's in the block: read past what wasn't used.
            std::io::copy(data, &mut std::io::sink()).map_err(|e| read_error(&entry.path, &e))?;
            Ok(())
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
    result.map_err(|e| seven_error(e, |e| ArchiveError::Corrupt(format!("the 7z archive couldn't be read: {e}"))))
}

// Through libarchive: RAR, tar.xz, tar.bz2, tar.zst, ISO, cab, and 7z with other codecs. It
// reads a set's parts as one stream, in the order given.

fn lib_api(kind: ArchiveKind) -> Result<&'static crate::libarchive::Api, ArchiveError> {
    crate::libarchive::api().ok_or_else(|| {
        ArchiveError::Missing(format!(
            "{} archives need libarchive, which isn't here: install {}",
            serde_name(kind),
            crate::libarchive::install_hint()
        ))
    })
}

/// A kind's name as the archive's ending: `rar`, `tar.xz`.
fn serde_name(kind: ArchiveKind) -> String {
    nexus_api::serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_uppercase)).unwrap_or_default()
}

/// libarchive on the archive cut into `parts`; a script installer's from its end.
fn lib_open(kind: ArchiveKind, parts: &[PathBuf]) -> Result<crate::libarchive::Reader, ArchiveError> {
    let api = lib_api(kind)?;
    if kind != ArchiveKind::Script {
        return crate::libarchive::Reader::open(api, parts);
    }
    let script = parts.first().ok_or_else(no_parts)?;
    let offset =
        nexus_core::embedded_archive(script).ok_or_else(|| read_error(script, &"it doesn't say its archive's size"))?;
    crate::libarchive::Reader::open_at(api, script, offset)
}

fn lib_entries(kind: ArchiveKind, parts: &[PathBuf]) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut reader = lib_open(kind, parts)?;
    let mut out = Vec::new();
    while let Some(entry) = reader.next()? {
        out.push(entry);
        reader.skip()?;
    }
    Ok(out)
}

fn lib_write(kind: ArchiveKind, parts: &[PathBuf], plan: &Plan, work: &mut Work) -> Result<(), ArchiveError> {
    let mut reader = lib_open(kind, parts)?;
    // The second pass reads the entries the first one listed, in the same order.
    for entry in &plan.entries {
        work.check_cancel()?;
        if reader.next()?.is_none_or(|header| header.name != entry.name) {
            return Err(changed());
        }
        match entry.kind {
            EntryKind::Other => reader.skip()?,
            _ => work.put(entry, &mut reader)?,
        }
    }
    Ok(())
}

// unar, for what libarchive can't read. It reads only a whole file: a script installer's
// archive is copied out first, beside the folder, and removed after.

/// The Unarchiver's programs: `lsar` lists, `unar` unpacks.
struct Unar {
    lsar: PathBuf,
    unar: PathBuf,
}

/// The Unix modes of the listing's files, by their place in the listing.
type Modes = Vec<(usize, u32)>;

/// A file removed when it goes out of scope.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Unar {
    fn find() -> Option<Self> {
        use nexus_core::ProcessSpawner;
        let which = |name| nexus_core::TokioSpawner.which(name);
        Some(Self { lsar: which("lsar")?, unar: which("unar")? })
    }

    fn unpack(&self, kind: ArchiveKind, parts: &[PathBuf], mut work: Work) -> Result<(), ArchiveError> {
        let first = parts.first().ok_or_else(no_parts)?;
        let scratch = if kind == ArchiveKind::Script { Some(copy_out(first, &mut work)?) } else { None };
        let archive = scratch.as_ref().map_or(first.as_path(), |(s, _)| s.0.as_path());
        // The copy takes room on the same disk.
        let free = work.free.map(|free| free.saturating_sub(scratch.as_ref().map_or(0, |(_, size)| *size)));
        let (listing, modes) = self.entries(archive)?;
        let plan = plan_unpack(listing, &work.into, free)?;
        work.total = plan.total;
        work.done = 0;
        let sizes: HashMap<String, u64> =
            plan.entries.iter().map(|e| (e.path.to_string_lossy().into_owned(), e.size)).collect();
        // `-D`: the entries as the archive has them; `-nr`: an archive inside stays packed.
        let mut child = std::process::Command::new(&self.unar)
            .args(["-f", "-D", "-nr", "-o"])
            .arg(&work.into)
            .arg(archive)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ArchiveError::Io(format!("unar: {e}")))?;
        let stdout = child.stdout.take().ok_or_else(|| ArchiveError::Io("unar: no output".into()))?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        // Each line names a file it wrote: "  Game/run.sh  (175 B)... OK."
        let status = loop {
            while let Ok(line) = rx.try_recv() {
                if let Some(name) = line.trim_start().split("  (").next()
                    && let Some(size) = sizes.get(name)
                {
                    work.done += size;
                    (work.progress)(work.done, work.total);
                }
            }
            if work.check_cancel().is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return work.check_cancel();
            }
            if let Some(status) = child.try_wait().map_err(|e| ArchiveError::Io(format!("unar: {e}")))? {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        if !status.success() {
            let mut errors = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut errors);
            }
            return Err(ArchiveError::Corrupt(format!("unar couldn't unpack {}: {}", first.display(), errors.trim())));
        }
        // unar leaves a program's flag off.
        for (i, mode) in modes {
            work.mode(&plan.entries[i].path, mode)?;
        }
        Ok(())
    }

    /// The archive's entries as lsar lists them, and the Unix modes it keeps.
    fn entries(&self, archive: &Path) -> Result<(Vec<ArchiveEntry>, Modes), ArchiveError> {
        use nexus_api::serde_json::Value;
        let out = std::process::Command::new(&self.lsar)
            .args(["-j", "-jss", "-nr"])
            .arg(archive)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| ArchiveError::Io(format!("lsar: {e}")))?;
        if !out.status.success() {
            return Err(read_error(archive, &String::from_utf8_lossy(&out.stdout).trim()));
        }
        let listing: Value = nexus_api::serde_json::from_slice(&out.stdout).map_err(|e| read_error(archive, &e))?;
        let mut entries = Vec::new();
        let mut modes = Vec::new();
        for e in listing["lsarContents"].as_array().ok_or_else(|| read_error(archive, &"lsar listed nothing"))? {
            let name = e["XADFileName"].as_str().ok_or_else(|| read_error(archive, &"an entry with no name"))?;
            let size = e["XADFileSize"].as_u64().unwrap_or(0);
            let kind = if e["XADIsDirectory"].as_u64().unwrap_or(0) != 0 {
                EntryKind::Folder
            } else if e["XADIsLink"].as_u64().unwrap_or(0) != 0 {
                EntryKind::Link(e["XADLinkDestination"].as_str().unwrap_or_default().into())
            } else {
                EntryKind::File
            };
            // A RAR 5 made on Unix keeps the file's mode.
            if kind == EntryKind::File
                && e["RAR5OS"].as_u64() == Some(1)
                && let Some(attributes) = e["RAR5Attributes"].as_u64()
            {
                modes.push((entries.len(), attributes as u32));
            }
            let encrypted = e["XADIsEncrypted"].as_u64().unwrap_or(0) != 0;
            entries.push(ArchiveEntry { encrypted, ..ArchiveEntry::new(name, size, kind) });
        }
        Ok((entries, modes))
    }
}

/// Copies a script installer's archive out of it, beside the unpack folder: unar reads only a
/// whole file. Returns the copy and its size.
fn copy_out(script: &Path, work: &mut Work) -> Result<(Scratch, u64), ArchiveError> {
    let offset =
        nexus_core::embedded_archive(script).ok_or_else(|| read_error(script, &"it doesn't say its archive's size"))?;
    let mut file = File::open(script).map_err(|e| io(script, e))?;
    let size = file.metadata().map_err(|e| io(script, e))?.len() - offset;
    room(size, work.free)?;
    file.seek(SeekFrom::Start(offset)).map_err(|e| io(script, e))?;
    let scratch = Scratch(work.into.with_extension("archive"));
    let mut out = File::create(&scratch.0).map_err(|e| io(&scratch.0, e))?;
    let mut buffer = vec![0; PIECE];
    loop {
        work.check_cancel()?;
        let n = file.read(&mut buffer).map_err(|e| read_error(script, &e))?;
        if n == 0 {
            break;
        }
        out.write_all(&buffer[..n]).map_err(|e| io(&scratch.0, e))?;
    }
    Ok((scratch, size))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use nexus_testkit::contract::{self, ArchiveFixture, Sample};

    use super::*;

    /// A new empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nexus-archives-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn testdata() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
    }

    /// Unpacks into `<dir>/out`, and returns the last progress it reported.
    fn run(kind: ArchiveKind, parts: &[PathBuf], dir: &Path) -> Result<(u64, u64), ArchiveError> {
        let into = dir.join("out");
        std::fs::create_dir_all(&into).unwrap();
        let last = Arc::new(Mutex::new((0, 0)));
        let progress: UnpackProgress = {
            let last = last.clone();
            Arc::new(move |done, total| *last.lock().unwrap() = (done, total))
        };
        let work = Work { into, free: None, progress, cancel: Arc::new(AtomicBool::new(false)), done: 0, total: 0 };
        unpack(kind, parts, work)?;
        Ok(*last.lock().unwrap())
    }

    fn nothing_in(folder: &Path) -> bool {
        std::fs::read_dir(folder).unwrap().next().is_none()
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

    /// Real zip files for the archives contract, written as each sample says, hostile names
    /// and all.
    struct Zips;

    impl ArchiveFixture for Zips {
        fn archive(&self, dir: &Path, entries: &[Sample]) -> (ArchiveKind, Vec<PathBuf>) {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default();
            for entry in entries {
                match entry {
                    Sample::File(name, bytes) => {
                        zip.start_file(name, options).unwrap();
                        zip.write_all(bytes).unwrap();
                    }
                    Sample::Link(name, target) => zip.add_symlink(name, target, options).unwrap(),
                }
            }
            let path = dir.join("contract.zip");
            std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
            (ArchiveKind::Zip, vec![path])
        }

        fn locked(&self, dir: &Path) -> Option<(ArchiveKind, Vec<PathBuf>)> {
            let path = dir.join("crypted.zip");
            std::fs::copy(testdata().join("crypted.zip"), &path).unwrap();
            Some((ArchiveKind::Zip, vec![path]))
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_keeps_the_archives_contract() {
        contract::archives(&HostArchives, &Zips).await;
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
        assert_eq!(refused, ArchiveError::Escapes { entry: "../evil.txt".into(), how: Escape::Leaves });
        assert_eq!(refused.to_string(), "../evil.txt leaves the folder");
        assert!(!dir.join("out/good.txt").exists(), "nothing written");
        assert!(!dir.join("evil.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_encrypted_zip_or_7z_is_refused_before_anything_is_written() {
        let dir = folder("crypted-zip");
        let refused = run(ArchiveKind::Zip, &[testdata().join("crypted.zip")], &dir).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
        assert!(nothing_in(&dir.join("out")), "nothing written");
        std::fs::remove_dir_all(&dir).unwrap();

        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        // A 7z whose listing itself is encrypted: `sevenz-rust2` is built without AES, so it
        // goes to libarchive, which says it saw encryption.
        let seven = folder("crypted-7z");
        let refused = run(ArchiveKind::SevenZ, &[testdata().join("crypted-header.7z")], &seven).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
        assert!(nothing_in(&seven.join("out")), "nothing written");
        std::fs::remove_dir_all(&seven).unwrap();
    }

    #[test]
    fn libarchive_says_password_for_an_archive_whose_listing_is_encrypted() {
        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        // libarchive reports the encryption it saw, not only an error text.
        let refused = lib_entries(ArchiveKind::SevenZ, &[testdata().join("crypted-header.7z")]).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
    }

    #[test]
    fn a_data_error_from_libarchive_keeps_its_kind_so_unar_still_gets_the_archive() {
        let path = Path::new("Game/data.bin");
        let unsupported = std::io::Error::other(ArchiveError::Unsupported("the archive couldn't be read: X".into()));
        assert_eq!(
            read_failed(path, unsupported),
            ArchiveError::Unsupported("Game/data.bin couldn't be read: the archive couldn't be read: X".into())
        );
        assert_eq!(read_failed(path, std::io::Error::other(ArchiveError::Password)), ArchiveError::Password);
        assert_eq!(
            read_failed(path, std::io::Error::other("cut short")),
            ArchiveError::Corrupt("Game/data.bin couldn't be read: cut short".into())
        );
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
    fn a_script_installer_unpacks_the_archive_its_header_sizes() {
        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        let dir = folder("script");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        tar.append_data(&mut header, "Game/run.sh", &b"#!/"[..]).unwrap();
        let payload = tar.into_inner().unwrap().finish().unwrap();
        // The script, then the programs it unpacks with, then the archive.
        let mut installer = format!("#!/usr/bin/env sh\narcsz={}\nexit 0\n", payload.len()).into_bytes();
        installer.resize(4096, b'#');
        installer.extend_from_slice(&[0x7f; 3000]);
        installer.extend_from_slice(&payload);
        std::fs::write(dir.join("game.run"), &installer).unwrap();
        run(ArchiveKind::Script, &[dir.join("game.run")], &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("out/Game/run.sh")).unwrap(), b"#!/");

        // An archive libarchive can't read: the player runs the installer.
        let mut unreadable = b"#!/usr/bin/env sh\narcsz=3000\n".to_vec();
        unreadable.resize(4096 + 3000, 0x7f);
        std::fs::write(dir.join("other.run"), &unreadable).unwrap();
        let bad = folder("script-bad");
        let refused = run(ArchiveKind::Script, &[dir.join("other.run")], &bad).unwrap_err();
        if Unar::find().is_none() {
            assert!(matches!(refused, ArchiveError::Missing(_)), "{refused:?}");
            assert!(
                refused.to_string().ends_with("Install unar to unpack it, or run the installer by hand"),
                "{refused}"
            );
        } else {
            assert!(matches!(refused, ArchiveError::Corrupt(_)), "unar can't read it either: {refused:?}");
            let copy = bad.join("out.archive");
            assert!(refused.to_string().starts_with(&format!("{} couldn't be read", copy.display())), "{refused}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&bad).unwrap();
    }

    /// Unpacks with unar into `<dir>/out`, and returns the last progress it reported.
    fn run_unar(tools: &Unar, kind: ArchiveKind, parts: &[PathBuf], dir: &Path) -> Result<(u64, u64), ArchiveError> {
        let into = dir.join("out");
        std::fs::create_dir_all(&into).unwrap();
        let last = Arc::new(Mutex::new((0, 0)));
        let progress: UnpackProgress = {
            let last = last.clone();
            Arc::new(move |done, total| *last.lock().unwrap() = (done, total))
        };
        let work = Work { into, free: None, progress, cancel: Arc::new(AtomicBool::new(false)), done: 0, total: 0 };
        tools.unpack(kind, parts, work)?;
        Ok(*last.lock().unwrap())
    }

    #[test]
    fn unar_unpacks_a_rar_and_a_script_installer_s_archive_and_leaves_no_copy() {
        let Some(tools) = Unar::find() else {
            eprintln!("SKIPPED: no unar on this computer");
            return;
        };
        let data = testdata();
        let dir = folder("unar-rar");
        run_unar(&tools, ArchiveKind::Rar, &[data.join("version.rar")], &dir).unwrap();
        assert!(!std::fs::read(dir.join("out/VERSION")).unwrap().is_empty());

        // An installer with a RAR at its end, as Cuphead's.
        let dir = folder("unar-script");
        let payload = std::fs::read(data.join("version.rar")).unwrap();
        let mut installer = format!("#!/usr/bin/env sh\narcsz={}\n", payload.len()).into_bytes();
        installer.resize(4096, b'#');
        installer.extend_from_slice(&payload);
        std::fs::write(dir.join("game.run"), &installer).unwrap();
        let (done, total) = run_unar(&tools, ArchiveKind::Script, &[dir.join("game.run")], &dir).unwrap();
        assert!(total > 0 && done == total, "progress to the end: {done} of {total}");
        assert!(!std::fs::read(dir.join("out/VERSION")).unwrap().is_empty());
        assert!(!dir.join("out.archive").exists(), "the archive's copy is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unar_refuses_by_the_same_plan() {
        let Some(tools) = Unar::find() else {
            eprintln!("SKIPPED: no unar on this computer");
            return;
        };
        let dir = folder("unar-room");
        let into = dir.join("out");
        std::fs::create_dir_all(&into).unwrap();
        let work = Work {
            into: into.clone(),
            free: Some(0),
            progress: Arc::new(|_, _| {}),
            cancel: Arc::new(AtomicBool::new(false)),
            done: 0,
            total: 0,
        };
        let refused = tools.unpack(ArchiveKind::Rar, &[testdata().join("version.rar")], work).unwrap_err();
        assert!(matches!(refused, ArchiveError::NoRoom { free: 0, .. }), "{refused:?}");
        assert!(nothing_in(&into), "nothing written");

        let locked = folder("unar-locked");
        let refused = run_unar(&tools, ArchiveKind::Rar, &[testdata().join("crypted.rar")], &locked).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
        assert!(nothing_in(&locked.join("out")), "nothing written");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&locked).unwrap();
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
        assert_eq!(refused, ArchiveError::Escapes { entry: "Game/passwd".into(), how: Escape::LinksOut });
        assert_eq!(refused.to_string(), "Game/passwd links out of the folder");
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
        let data = testdata();
        let dir = folder("rar");
        run(ArchiveKind::Rar, &[data.join("version.rar")], &dir).unwrap();
        assert!(!std::fs::read(dir.join("out/VERSION")).unwrap().is_empty());

        let locked = folder("rar-locked");
        let refused = run(ArchiveKind::Rar, &[data.join("crypted.rar")], &locked).unwrap_err();
        assert_eq!(refused, ArchiveError::Password);
        assert_eq!(refused.to_string(), "This archive needs a password, which Nexus can't ask for");
        assert!(nothing_in(&locked.join("out")), "nothing written");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&locked).unwrap();
    }

    #[test]
    fn libarchive_reads_what_the_rust_readers_don_t() {
        if crate::libarchive::api().is_none() {
            eprintln!("SKIPPED: no libarchive with RAR 5 on this computer");
            return;
        }
        let data = testdata();
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
        let work = Work { into, free: None, progress: Arc::new(|_, _| {}), cancel, done: 0, total: 0 };
        let stopped = unpack(ArchiveKind::Zip, &[dir.join("game.zip")], work).unwrap_err();
        assert_eq!(stopped, ArchiveError::Cancelled);
        assert_eq!(stopped.to_string(), "The unpacking was cancelled");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
