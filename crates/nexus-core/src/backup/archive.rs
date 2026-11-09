//! A backup's file: `nexus-backup-<date>.tar.gz`. Its entries, in order:
//!
//! - `backup.json`: the [`Manifest`]. It comes first, so the list of backups reads only the
//!   start of each file.
//! - `config.json`: the app's settings without anything that names this computer's folders
//!   ([`config_subset`]).
//! - `items/<plugin>/<item>`: an item that is a file, or `items/<plugin>/<item>/…` the files of
//!   an item that is a folder. Linked files and folders are left out.
//!
//! A restore reads the file twice. The first pass checks everything and writes nothing: each
//! entry is a file or a folder inside its item, with a name every system takes; each item is
//! no bigger than the manifest says, fits the free space where it goes, and is a file or a
//! folder there as it is in the backup. The second pass writes, each file through a temporary
//! file that replaces the one there only once it is whole and on the disk. Nothing is written
//! through a link below an item's place.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The archive format this build writes and reads. A backup of a later format is listed, but
/// not restored: its parts may mean something this build doesn't know.
pub(crate) const FORMAT: u32 = 1;
const MANIFEST: &str = "backup.json";
const CONFIG: &str = "config.json";
const ITEMS: &str = "items";
/// A backup's file name starts with this, and ends with [`EXTENSION`].
pub(crate) const PREFIX: &str = "nexus-backup-";
pub(crate) const EXTENSION: &str = ".tar.gz";
/// In the name of the backup a restore makes of what was there first.
const BEFORE_RESTORE: &str = "-before-restore";
/// The most a manifest or the settings may take, read into memory.
const MAX_JSON: u64 = 64 * 1024 * 1024;

/// An item: its plugin and its name, the same on every computer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ItemKey {
    pub plugin: String,
    pub name: String,
}

impl std::fmt::Display for ItemKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.plugin, self.name)
    }
}

/// What a backup holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub format: u32,
    /// The Nexus version that made it.
    #[serde(default)]
    pub nexus: String,
    /// The name of the computer that made it. Empty in a backup from before Nexus kept it.
    #[serde(default)]
    pub computer: String,
    /// When, in seconds since 1970 (UTC).
    #[serde(default)]
    pub made: u64,
    #[serde(default)]
    pub items: Vec<ItemRecord>,
}

/// One item in a backup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ItemRecord {
    pub plugin: String,
    pub name: String,
    pub title: String,
    /// A folder; else a file.
    pub folder: bool,
    pub bytes: u64,
}

impl ItemRecord {
    pub fn key(&self) -> ItemKey {
        ItemKey { plugin: self.plugin.clone(), name: self.name.clone() }
    }
}

/// An item to back up, where it is on this computer.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub key: ItemKey,
    pub title: String,
    pub path: PathBuf,
    /// Paths below `path` left out, such as `cache` or `a/log`.
    pub skip: Vec<String>,
}

/// Which backup a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// One the player asked for.
    Backup,
    /// The one a restore makes of what was there first.
    BeforeRestore,
}

/// The app's settings that a backup holds, under `app`: none names a folder of this computer.
/// Every plugin's settings come along whole.
pub(crate) const APP_KEYS: [&str; 13] = [
    "playtime",
    "hidden_games",
    "results",
    "result_builds",
    "result_skipped",
    "game_art",
    "disabled_plugins",
    "enabled_plugins",
    "sounds",
    "return_on_exit",
    "scale",
    "auto_update",
    "theme",
];

/// The part of the config a backup holds: `{ "app": { …APP_KEYS }, "plugins": … }`.
pub(crate) fn config_subset(app: &Value, plugins: &Value) -> Value {
    let app: serde_json::Map<String, Value> =
        APP_KEYS.iter().filter_map(|key| app.get(*key).map(|v| ((*key).to_owned(), v.clone()))).collect();
    serde_json::json!({ "app": app, "plugins": plugins })
}

/// A backup's file name for `secs` (UTC): `nexus-backup-2026-11-09-143005.tar.gz`, with
/// `-before-restore` for [`Kind::BeforeRestore`].
pub(crate) fn file_name(secs: u64, kind: Kind) -> String {
    let (y, m, d, hh, mm, ss) = crate::display::utc(secs);
    let tag = if kind == Kind::BeforeRestore { BEFORE_RESTORE } else { "" };
    format!("{PREFIX}{y:04}-{m:02}-{d:02}-{hh:02}{mm:02}{ss:02}{tag}{EXTENSION}")
}

/// Which backup the file `name` is: `None` when it isn't one.
pub(crate) fn kind_of(name: &str) -> Option<Kind> {
    let stem = name.strip_prefix(PREFIX)?.strip_suffix(EXTENSION)?;
    Some(if stem.ends_with(BEFORE_RESTORE) { Kind::BeforeRestore } else { Kind::Backup })
}

/// A file or folder of a source, with its path in the item (`/` between names).
struct Entry {
    path: PathBuf,
    name: String,
    folder: bool,
    bytes: u64,
}

/// What of `source` goes in: whether it is a folder, and its files and folders, without links
/// and the paths it skips. No entries when it isn't there.
fn walk(source: &Source) -> io::Result<(bool, Vec<Entry>)> {
    let Ok(meta) = std::fs::metadata(&source.path) else { return Ok((false, Vec::new())) };
    if !meta.is_dir() {
        let file = meta.is_file().then(|| Entry {
            path: source.path.clone(),
            name: String::new(),
            folder: false,
            bytes: meta.len(),
        });
        return Ok((false, file.into_iter().collect()));
    }
    let mut out = Vec::new();
    let mut left = vec![(source.path.clone(), String::new())];
    while let Some((dir, name)) = left.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let Some(file) = child.file_name().to_str().map(str::to_owned) else { continue };
            let rel = if name.is_empty() { file } else { format!("{name}/{file}") };
            if source.skip.contains(&rel) || child.file_type()?.is_symlink() {
                continue;
            }
            let meta = child.metadata()?;
            if meta.is_dir() {
                out.push(Entry { path: child.path(), name: rel.clone(), folder: true, bytes: 0 });
                left.push((child.path(), rel));
            } else if meta.is_file() {
                out.push(Entry { path: child.path(), name: rel, folder: false, bytes: meta.len() });
            }
        }
    }
    Ok((true, out))
}

/// Removes `path`, a file that a failed write left; a failure goes to the log.
fn remove_left(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), error = %e, "a part-written file stayed");
    }
}

/// Writes a backup of `sources` and `config` to `dest`, through `<dest>.part`, which becomes
/// `dest` only once it is whole. `progress` gets the bytes written and the total. A source that
/// isn't there is left out.
pub(crate) fn write(
    dest: &Path,
    nexus: &str,
    computer: &str,
    made: u64,
    config: &Value,
    sources: &[Source],
    progress: &dyn Fn(u64, u64),
) -> io::Result<Manifest> {
    let mut items = Vec::new();
    let mut walked = Vec::new();
    for source in sources {
        let (folder, entries) = walk(source)?;
        if entries.is_empty() && !folder {
            continue;
        }
        let bytes = entries.iter().map(|e| e.bytes).sum();
        items.push(ItemRecord {
            plugin: source.key.plugin.clone(),
            name: source.key.name.clone(),
            title: source.title.clone(),
            folder,
            bytes,
        });
        walked.push((source, entries));
    }
    let manifest = Manifest { format: FORMAT, nexus: nexus.to_owned(), computer: computer.to_owned(), made, items };
    let total: u64 = manifest.items.iter().map(|i| i.bytes).sum();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_file_name(format!("{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("backup")));
    let written = (|| {
        let gz = flate2::write::GzEncoder::new(BufWriter::new(File::create(&part)?), flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        tar.follow_symlinks(false);
        append_bytes(&mut tar, MANIFEST, &serde_json::to_vec_pretty(&manifest)?)?;
        append_bytes(&mut tar, CONFIG, &serde_json::to_vec(config)?)?;
        let mut done = 0;
        for (source, entries) in walked {
            let base = format!("{ITEMS}/{}", source.key);
            for entry in entries {
                let name = if entry.name.is_empty() { base.clone() } else { format!("{base}/{}", entry.name) };
                if entry.folder {
                    let mut header = tar::Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_mode(0o755);
                    header.set_size(0);
                    tar.append_data(&mut header, &name, io::empty())?;
                } else {
                    tar.append_file(&name, &mut File::open(&entry.path)?)?;
                    done += entry.bytes;
                    progress(done, total);
                }
            }
        }
        let file = tar.into_inner()?.finish()?.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.sync_all()?;
        std::fs::rename(&part, dest)
    })();
    if written.is_err() {
        remove_left(&part);
    }
    written.map(|()| manifest)
}

fn append_bytes<W: Write>(tar: &mut tar::Builder<W>, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    tar.append_data(&mut header, name, bytes)
}

fn open(path: &Path) -> io::Result<tar::Archive<flate2::read::GzDecoder<BufReader<File>>>> {
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(File::open(path)?))))
}

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

/// The JSON of an entry, refused over [`MAX_JSON`].
fn json<T: serde::de::DeserializeOwned>(entry: impl Read) -> io::Result<T> {
    let mut bytes = Vec::new();
    entry.take(MAX_JSON + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(invalid("a JSON entry is too big"));
    }
    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))
}

/// The manifest at the start of the backup at `path`, which this build restores.
pub(crate) fn read_manifest(path: &Path) -> io::Result<Manifest> {
    let manifest = peek_manifest(path)?;
    if manifest.format > FORMAT {
        return Err(invalid(NEWER));
    }
    Ok(manifest)
}

/// Why a backup of a later format isn't restored.
pub(crate) const NEWER: &str = "a newer Nexus made this backup: update Nexus to restore it";

/// The manifest at the start of the backup at `path`, of any format from 1 up: for the list.
/// A later format's manifest gives what it shares with this one.
pub(crate) fn peek_manifest(path: &Path) -> io::Result<Manifest> {
    let mut archive = open(path)?;
    let mut entries = archive.entries()?;
    let first = entries.next().ok_or_else(|| invalid("the file is empty"))??;
    if first.path()?.as_ref() != Path::new(MANIFEST) {
        return Err(invalid("this isn't a Nexus backup"));
    }
    let manifest: Manifest = json(first)?;
    if manifest.format == 0 {
        return Err(invalid("backup format 0 isn't one this Nexus reads"));
    }
    Ok(manifest)
}

/// A name every system takes for a file: Windows refuses `<>:"|?*`, a name that ends in a dot
/// or a space, and the names of its devices.
fn portable(name: &str) -> bool {
    const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let numbered =
        stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit();
    !name.is_empty()
        && !name.contains(['<', '>', ':', '"', '|', '?', '*', '\\'])
        && !name.ends_with(['.', ' '])
        && !DEVICES.contains(&stem.as_str())
        && !numbered
}

/// Where an entry goes: its item and its path in the item ("" for the item itself). `None` for
/// the manifest and the settings.
fn place(path: &Path) -> io::Result<Option<(ItemKey, PathBuf)>> {
    let mut parts = path.components();
    match parts.next() {
        Some(Component::Normal(first)) if first == ITEMS => {}
        Some(Component::Normal(first)) if first == MANIFEST || first == CONFIG => return Ok(None),
        _ => return Err(invalid(format!("an entry outside the backup's items: {}", path.display()))),
    }
    let mut name = |what: &str| match parts.next() {
        Some(Component::Normal(n)) => {
            n.to_str().map(str::to_owned).ok_or_else(|| invalid(format!("a {what} name that isn't text")))
        }
        _ => Err(invalid(format!("an entry with no {what}: {}", path.display()))),
    };
    let key = ItemKey { plugin: name("plugin")?, name: name("item")? };
    let rest: PathBuf = parts.clone().collect();
    for part in parts {
        let fine = matches!(part, Component::Normal(n) if n.to_str().is_some_and(portable));
        if !fine {
            return Err(invalid(format!(
                "an entry that leaves its item, or with a name some systems refuse: {}",
                path.display()
            )));
        }
    }
    Ok(Some((key, rest)))
}

/// What a restore did.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    /// The backup's settings, for the caller to put back.
    pub config: Value,
    /// The items this computer has no place for.
    pub left_out: Vec<ItemKey>,
}

/// The free space on the disk of `path`, or of its nearest folder that is there.
pub(crate) type FreeSpace<'a> = &'a dyn Fn(&Path) -> Option<u64>;

/// The first pass: the settings, after checking every entry and each item's place. Nothing is
/// written.
fn check(
    path: &Path,
    manifest: &Manifest,
    targets: &BTreeMap<ItemKey, PathBuf>,
    free: FreeSpace<'_>,
) -> io::Result<Value> {
    let declared: BTreeMap<ItemKey, &ItemRecord> = manifest.items.iter().map(|i| (i.key(), i)).collect();
    let mut sizes: BTreeMap<ItemKey, u64> = BTreeMap::new();
    let mut config = Value::Null;
    for entry in open(path)?.entries()? {
        let entry = entry?;
        let kind = entry.header().entry_type();
        let entry_path = entry.path()?.into_owned();
        if entry_path == Path::new(CONFIG) {
            config = json(entry)?;
            continue;
        }
        let Some((key, _)) = place(&entry_path)? else { continue };
        if !(kind.is_file() || kind.is_dir()) {
            return Err(invalid(format!("an entry that is a link or a device: {}", entry_path.display())));
        }
        let record = declared.get(&key).ok_or_else(|| invalid(format!("{key} isn't in the backup's list")))?;
        let size = sizes.entry(key.clone()).or_default();
        *size += entry.size();
        if *size > record.bytes {
            return Err(invalid(format!("{key} is bigger than the backup's list says: the file is damaged")));
        }
    }
    for record in &manifest.items {
        let Some(target) = targets.get(&record.key()) else { continue };
        if let Ok(meta) = std::fs::metadata(target)
            && meta.is_dir() != record.folder
        {
            let (here, there) = if record.folder { ("a file", "a folder") } else { ("a folder", "a file") };
            return Err(invalid(format!("{} is {here} here and {there} in the backup", target.display())));
        }
        if let Some(room) = free(target)
            && room < record.bytes
        {
            return Err(invalid(format!(
                "{} needs {} and its disk has {}",
                record.title,
                crate::display::size_text(record.bytes),
                crate::display::size_text(room)
            )));
        }
    }
    Ok(config)
}

/// Puts the backup at `path` back: each item into `targets[key]`, a file or a folder as the
/// backup has it. A file there is replaced; files the backup doesn't have stay. `free` is the
/// free space at a path; `progress` gets the bytes written and the total.
pub(crate) fn restore(
    path: &Path,
    targets: &BTreeMap<ItemKey, PathBuf>,
    free: FreeSpace<'_>,
    progress: &dyn Fn(u64, u64),
) -> io::Result<Restored> {
    let manifest = read_manifest(path)?;
    let config = check(path, &manifest, targets, free)?;
    let total: u64 = manifest.items.iter().filter(|i| targets.contains_key(&i.key())).map(|i| i.bytes).sum();
    let mut restored = Restored { config, ..Restored::default() };
    let mut done = 0;
    for entry in open(path)?.entries()? {
        let mut entry = entry?;
        let Some((key, rest)) = place(&entry.path()?)? else { continue };
        let Some(root) = targets.get(&key) else {
            if !restored.left_out.contains(&key) {
                restored.left_out.push(key);
            }
            continue;
        };
        let target = if rest.as_os_str().is_empty() { root.clone() } else { root.join(&rest) };
        no_link_between(root, &target)?;
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        done += replace(&target, &mut entry)?;
        progress(done, total);
    }
    Ok(restored)
}

/// Writes `from` to `target` through `.<name>.restoring` beside it, which takes `target`'s
/// place only once it is whole and on the disk. The bytes written.
fn replace(target: &Path, from: &mut impl Read) -> io::Result<u64> {
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let part = target.with_file_name(format!(".{name}.restoring"));
    match std::fs::symlink_metadata(&part) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_dir() => {
            return Err(invalid(format!("{} is in the way", part.display())));
        }
        Ok(_) => remove_left(&part),
        Err(_) => {}
    }
    let written = (|| {
        let mut out = BufWriter::new(std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?);
        let bytes = io::copy(from, &mut out)?;
        out.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
        std::fs::rename(&part, target)?;
        Ok(bytes)
    })();
    if written.is_err() {
        remove_left(&part);
    }
    written
}

/// Refuses a `target` under `root` that a link on the way would send elsewhere. The root
/// itself may be a link: the player's folder can be one.
fn no_link_between(root: &Path, target: &Path) -> io::Result<()> {
    let Ok(rest) = target.strip_prefix(root) else { return Err(invalid("a path outside its item")) };
    let mut at = root.to_path_buf();
    for part in rest.components() {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(invalid(format!("{} is a link: nothing is written through it", at.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(plugin: &str, name: &str) -> ItemKey {
        ItemKey { plugin: plugin.into(), name: name.into() }
    }

    fn source(plugin: &str, name: &str, path: &Path) -> Source {
        Source { key: key(plugin, name), title: name.into(), path: path.into(), skip: vec!["GAME/cache".into()] }
    }

    fn room(_: &Path) -> Option<u64> {
        None
    }

    #[test]
    fn a_name_has_the_date_and_time_in_utc_and_tells_its_kind() {
        assert_eq!(file_name(1_794_234_605, Kind::Backup), "nexus-backup-2026-11-09-143005.tar.gz");
        let before = file_name(0, Kind::BeforeRestore);
        assert_eq!(before, "nexus-backup-1970-01-01-000000-before-restore.tar.gz");
        assert_eq!(kind_of(&before), Some(Kind::BeforeRestore));
        assert_eq!(kind_of("nexus-backup-x.tar.gz"), Some(Kind::Backup));
        assert_eq!(kind_of("notes.tar.gz"), None);
    }

    #[test]
    fn the_settings_leave_out_what_names_this_computer_s_folders() {
        let app = serde_json::json!({ "playtime": { "a": 1 }, "folders": { "games": ["/mnt/g"] }, "theme": "default", "monitor": "HDMI-1" });
        let subset = config_subset(&app, &serde_json::json!({ "emu": { "settings": { "x": 1 } } }));
        assert_eq!(subset["app"], serde_json::json!({ "playtime": { "a": 1 }, "theme": "default" }));
        assert_eq!(subset["plugins"]["emu"]["settings"]["x"], 1);
    }

    #[test]
    fn a_name_some_systems_refuse_is_refused() {
        for bad in ["a:b", "con", "COM1.txt", "end.", "end ", "q?"] {
            assert!(!portable(bad), "{bad}");
        }
        for good in ["save.dat", "sce_sdmemory", "CONSOLE", "COM.txt", ".hidden"] {
            assert!(portable(good), "{good}");
        }
    }

    #[test]
    fn a_backup_goes_back_into_other_folders_without_what_it_skips() {
        let from = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(from.path().join("saves/GAME/cache")).unwrap();
        std::fs::create_dir_all(from.path().join("saves/OTHER/cache")).unwrap();
        std::fs::write(from.path().join("saves/GAME/slot.dat"), b"progress").unwrap();
        std::fs::write(from.path().join("saves/GAME/cache/big"), b"skip me").unwrap();
        std::fs::write(from.path().join("saves/OTHER/cache/kept"), b"a save named cache").unwrap();
        std::fs::create_dir_all(from.path().join("saves/EMPTY")).unwrap();
        std::fs::write(from.path().join("settings.ini"), b"a=1").unwrap();
        let file = from.path().join("out").join(file_name(10, Kind::Backup));
        let sources = [
            source("emu", "saves", &from.path().join("saves")),
            source("emu", "settings", &from.path().join("settings.ini")),
            source("emu", "gone", &from.path().join("missing")),
        ];
        let config = serde_json::json!({ "app": { "theme": "default" } });
        let manifest = write(&file, "1.0.0", "gaming-pc", 10, &config, &sources, &|_, _| {}).unwrap();
        assert_eq!(manifest.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["saves", "settings"]);
        assert_eq!(read_manifest(&file).unwrap(), manifest);
        assert_eq!(manifest.computer, "gaming-pc");

        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([
            (key("emu", "saves"), to.path().join("elsewhere/saves")),
            (key("emu", "settings"), to.path().join("config.ini")),
        ]);
        let restored = restore(&file, &targets, &room, &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(to.path().join("elsewhere/saves/GAME/slot.dat")).unwrap(), b"progress");
        assert!(to.path().join("elsewhere/saves/EMPTY").is_dir());
        assert!(!to.path().join("elsewhere/saves/GAME/cache").exists(), "skipped");
        assert!(to.path().join("elsewhere/saves/OTHER/cache/kept").is_file(), "only the path it names is skipped");
        assert_eq!(std::fs::read(to.path().join("config.ini")).unwrap(), b"a=1");
        assert_eq!(restored.config, config);
        assert!(restored.left_out.is_empty());
        let leftovers: Vec<_> = walkdir(to.path()).into_iter().filter(|p| p.ends_with(".restoring")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    fn walkdir(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            }
            out.push(path.display().to_string());
        }
        out
    }

    #[test]
    fn an_item_with_no_place_here_is_left_out() {
        let from = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("keys"), b"k").unwrap();
        let file = from.path().join("b.tar.gz");
        write(&file, "1", "", 0, &Value::Null, &[source("old", "keys", &from.path().join("keys"))], &|_, _| {})
            .unwrap();
        let restored = restore(&file, &BTreeMap::new(), &room, &|_, _| {}).unwrap();
        assert_eq!(restored.left_out, [key("old", "keys")]);
    }

    /// A backup of one folder item, `emu/saves`, with `saves/a` in it.
    fn one_save() -> (tempfile::TempDir, PathBuf) {
        let from = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(from.path().join("saves")).unwrap();
        std::fs::write(from.path().join("saves/a"), b"a save").unwrap();
        let file = from.path().join("b.tar.gz");
        write(&file, "1", "", 0, &Value::Null, &[source("emu", "saves", &from.path().join("saves"))], &|_, _| {})
            .unwrap();
        (from, file)
    }

    #[test]
    fn a_restore_that_wouldn_t_fit_writes_nothing() {
        let (_from, file) = one_save();
        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        let error = restore(&file, &targets, &|_| Some(2), &|_, _| {}).unwrap_err();
        assert!(error.to_string().contains("its disk has"), "{error}");
        assert!(!to.path().join("saves").exists());
    }

    #[test]
    fn a_folder_item_where_a_file_is_here_writes_nothing() {
        let (_from, file) = one_save();
        let to = tempfile::tempdir().unwrap();
        std::fs::write(to.path().join("saves"), b"a file").unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        let error = restore(&file, &targets, &room, &|_, _| {}).unwrap_err();
        assert!(error.to_string().contains("is a file here and a folder in the backup"), "{error}");
        assert_eq!(std::fs::read(to.path().join("saves")).unwrap(), b"a file");
    }

    #[cfg(unix)]
    #[test]
    fn links_stay_out_of_a_backup_and_a_restore_writes_through_none() {
        let from = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(from.path().join("saves")).unwrap();
        std::fs::write(from.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(from.path().join("secret"), from.path().join("saves/link")).unwrap();
        std::fs::write(from.path().join("saves/a"), b"a").unwrap();
        let file = from.path().join("b.tar.gz");
        let manifest =
            write(&file, "1", "", 0, &Value::Null, &[source("emu", "saves", &from.path().join("saves"))], &|_, _| {})
                .unwrap();
        assert_eq!(manifest.items[0].bytes, 1, "the link is left out");

        let to = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(to.path().join("saves")).unwrap();
        std::os::unix::fs::symlink(outside.path(), to.path().join("saves/a")).unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        assert!(restore(&file, &targets, &room, &|_, _| {}).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none(), "nothing written through the link");

        // A link where the temporary file goes is refused too.
        std::fs::remove_file(to.path().join("saves/a")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("x"), to.path().join("saves/.a.restoring")).unwrap();
        assert!(restore(&file, &targets, &room, &|_, _| {}).is_err());
        assert!(!outside.path().join("x").exists());
    }

    /// A backup with the manifest `manifest` and the raw entries `entries` (name, bytes), as a
    /// damaged or hostile file would have them.
    fn raw(dir: &Path, manifest: &Manifest, entries: &[(&[u8], &[u8])]) -> PathBuf {
        let file = dir.join("raw.tar.gz");
        let gz = flate2::write::GzEncoder::new(File::create(&file).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        append_bytes(&mut tar, MANIFEST, &serde_json::to_vec(manifest).unwrap()).unwrap();
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            // `append_data` refuses `..`; the raw name gets it in, as a hostile file would.
            header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
            header.set_cksum();
            tar.append(&header, *bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        file
    }

    fn saves_record(bytes: u64) -> Manifest {
        let item =
            ItemRecord { plugin: "emu".into(), name: "saves".into(), title: "Saves".into(), folder: true, bytes };
        Manifest { format: FORMAT, nexus: "1".into(), computer: String::new(), made: 0, items: vec![item] }
    }

    #[test]
    fn a_backup_of_a_later_format_is_listed_but_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        let mut later = saves_record(0);
        later.format = FORMAT + 1;
        later.computer = "new-pc".into();
        let file = raw(dir.path(), &later, &[]);
        assert_eq!(peek_manifest(&file).unwrap().computer, "new-pc");
        assert_eq!(read_manifest(&file).unwrap_err().to_string(), NEWER);
        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        assert!(restore(&file, &targets, &room, &|_, _| {}).is_err());

        let mut none = saves_record(0);
        none.format = 0;
        assert!(peek_manifest(&raw(dir.path(), &none, &[])).is_err(), "format 0 is no format");
    }

    #[test]
    fn an_entry_that_leaves_its_item_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let file =
            raw(dir.path(), &saves_record(3), &[(b"items/emu/saves/ok", b"ok"), (b"items/emu/saves/../../bad", b"x")]);
        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        assert!(restore(&file, &targets, &room, &|_, _| {}).is_err());
        assert!(!to.path().join("saves").exists(), "nothing written");
    }

    #[test]
    fn an_item_bigger_than_its_list_says_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = raw(dir.path(), &saves_record(2), &[(b"items/emu/saves/a", b"ab"), (b"items/emu/saves/b", b"cd")]);
        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([(key("emu", "saves"), to.path().join("saves"))]);
        let error = restore(&file, &targets, &room, &|_, _| {}).unwrap_err();
        assert!(error.to_string().contains("damaged"), "{error}");
        assert!(!to.path().join("saves").exists());
    }
}
