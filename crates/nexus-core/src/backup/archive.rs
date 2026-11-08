//! A backup's file: `nexus-backup-<date>.tar.gz`. Its entries, in order:
//!
//! - `backup.json`: the [`Manifest`]. It comes first, so the list of backups reads only the
//!   start of each file.
//! - `config.json`: the app's settings without anything that names this computer's folders
//!   ([`config_subset`]).
//! - `items/<plugin>/<item>`: an item that is a file, or `items/<plugin>/<item>/…` the files of
//!   an item that is a folder. Linked files and folders are left out.
//!
//! A restore reads the file twice: once to check every entry, then to write. It writes nothing
//! when an entry is a link, leaves its item, or isn't a file or a folder.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The archive format this build writes and reads.
pub(crate) const FORMAT: u32 = 1;
const MANIFEST: &str = "backup.json";
const CONFIG: &str = "config.json";
const ITEMS: &str = "items";
/// A backup's file name starts with this, and ends with [`EXTENSION`].
pub(crate) const PREFIX: &str = "nexus-backup-";
pub(crate) const EXTENSION: &str = ".tar.gz";
/// The most a manifest or the settings may take, read into memory.
const MAX_JSON: u64 = 64 * 1024 * 1024;

/// What a backup holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub format: u32,
    /// The Nexus version that made it.
    pub nexus: String,
    /// When, in seconds since 1970 (UTC).
    pub made: u64,
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

/// An item to back up, where it is on this computer.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub plugin: String,
    pub name: String,
    pub title: String,
    pub path: PathBuf,
    pub skip: Vec<String>,
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
/// `-before-restore` for the one a restore makes first.
pub(crate) fn file_name(secs: u64, before_restore: bool) -> String {
    let (y, m, d) = civil(secs / 86_400);
    let t = secs % 86_400;
    let tag = if before_restore { "-before-restore" } else { "" };
    format!("{PREFIX}{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}{tag}{EXTENSION}", t / 3600, t % 3600 / 60, t % 60)
}

/// The date of day `days` since 1970-01-01 (Howard Hinnant's civil_from_days).
pub(crate) fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A file or folder of a source, with its path in the item (`/` between names).
struct Entry {
    path: PathBuf,
    name: String,
    folder: bool,
    bytes: u64,
}

/// What of `source` goes in: its files and folders, without links and the names it skips.
/// Empty when it isn't there.
fn walk(source: &Source) -> io::Result<(bool, Vec<Entry>)> {
    let Ok(meta) = std::fs::symlink_metadata(&source.path) else { return Ok((false, Vec::new())) };
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
            if source.skip.contains(&file) {
                continue;
            }
            let meta = child.metadata()?;
            let rel = if name.is_empty() { file } else { format!("{name}/{file}") };
            if child.file_type()?.is_symlink() {
                continue;
            }
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

/// Writes a backup of `sources` and `config` to `dest`, through `<dest>.part`, which becomes
/// `dest` only once it is whole. `progress` gets the bytes written and the total. A source that
/// isn't there is left out.
pub(crate) fn write(
    dest: &Path,
    nexus: &str,
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
            plugin: source.plugin.clone(),
            name: source.name.clone(),
            title: source.title.clone(),
            folder,
            bytes,
        });
        walked.push((source, entries));
    }
    let manifest = Manifest { format: FORMAT, nexus: nexus.to_owned(), made, items };
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
            let base = format!("{ITEMS}/{}/{}", source.plugin, source.name);
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
        let mut file = tar.into_inner()?.finish()?.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&part, dest)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
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

/// The manifest at the start of the backup at `path`.
pub(crate) fn read_manifest(path: &Path) -> io::Result<Manifest> {
    let mut archive = open(path)?;
    let mut entries = archive.entries()?;
    let first = entries.next().ok_or_else(|| invalid("the file is empty"))??;
    if first.path()?.as_ref() != Path::new(MANIFEST) {
        return Err(invalid("this isn't a Nexus backup"));
    }
    let manifest: Manifest = json(first)?;
    if manifest.format != FORMAT {
        return Err(invalid(format!("backup format {} isn't one this Nexus reads", manifest.format)));
    }
    Ok(manifest)
}

/// Where an entry goes: its item and its path in the item ("" for the item itself).
fn place(path: &Path) -> io::Result<Option<(String, String, PathBuf)>> {
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
    let (plugin, item) = (name("plugin")?, name("item")?);
    let rest: PathBuf = parts.clone().collect();
    if parts.any(|c| !matches!(c, Component::Normal(_))) {
        return Err(invalid(format!("an entry that leaves its item: {}", path.display())));
    }
    Ok(Some((plugin, item, rest)))
}

/// What a restore did.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    /// The backup's settings, for the caller to put back.
    pub config: Value,
    /// The items that went back.
    pub items: Vec<(String, String)>,
    /// The items this computer has no place for, as `plugin/item`.
    pub left_out: Vec<String>,
}

/// Puts the backup at `path` back: each item into `targets[(plugin, item)]`, a file or a
/// folder as the backup has it. A file there is replaced; files the backup doesn't have stay.
/// `progress` gets the bytes written and the total.
pub(crate) fn restore(
    path: &Path,
    targets: &BTreeMap<(String, String), PathBuf>,
    progress: &dyn Fn(u64, u64),
) -> io::Result<Restored> {
    let manifest = read_manifest(path)?;
    // First pass: every entry is a file or a folder inside its item.
    let mut config = Value::Null;
    for entry in open(path)?.entries()? {
        let entry = entry?;
        let kind = entry.header().entry_type();
        let path = entry.path()?.into_owned();
        if path == Path::new(CONFIG) {
            config = json(entry)?;
            continue;
        }
        if place(&path)?.is_some() && !(kind.is_file() || kind.is_dir()) {
            return Err(invalid(format!("an entry that is a link or a device: {}", path.display())));
        }
    }
    let total: u64 = manifest
        .items
        .iter()
        .filter(|i| targets.contains_key(&(i.plugin.clone(), i.name.clone())))
        .map(|i| i.bytes)
        .sum();
    let mut restored = Restored { config, ..Restored::default() };
    let mut done = 0;
    for entry in open(path)?.entries()? {
        let mut entry = entry?;
        let Some((plugin, item, rest)) = place(&entry.path()?)? else { continue };
        let key = (plugin, item);
        let Some(root) = targets.get(&key) else {
            let name = format!("{}/{}", key.0, key.1);
            if !restored.left_out.contains(&name) {
                restored.left_out.push(name);
            }
            continue;
        };
        if !restored.items.contains(&key) {
            restored.items.push(key.clone());
        }
        let target = if rest.as_os_str().is_empty() { root.clone() } else { root.join(&rest) };
        no_link_between(root, &target)?;
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = target
            .with_file_name(format!(".{}.restoring", target.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
        let bytes = io::copy(&mut entry, &mut BufWriter::new(File::create(&part)?))?;
        std::fs::rename(&part, &target)?;
        done += bytes;
        progress(done, total);
    }
    Ok(restored)
}

/// Refuses a `target` under `root` that a link on the way would send elsewhere.
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

    fn source(plugin: &str, name: &str, path: &Path) -> Source {
        Source {
            plugin: plugin.into(),
            name: name.into(),
            title: name.into(),
            path: path.into(),
            skip: vec!["cache".into()],
        }
    }

    #[test]
    fn a_name_has_the_date_and_time_in_utc() {
        assert_eq!(file_name(1_794_234_605, false), "nexus-backup-2026-11-09-143005.tar.gz");
        assert_eq!(file_name(0, true), "nexus-backup-1970-01-01-000000-before-restore.tar.gz");
    }

    #[test]
    fn the_settings_leave_out_what_names_this_computer_s_folders() {
        let app = serde_json::json!({ "playtime": { "a": 1 }, "folders": { "games": ["/mnt/g"] }, "theme": "default", "monitor": "HDMI-1" });
        let subset = config_subset(&app, &serde_json::json!({ "emu": { "settings": { "x": 1 } } }));
        assert_eq!(subset["app"], serde_json::json!({ "playtime": { "a": 1 }, "theme": "default" }));
        assert_eq!(subset["plugins"]["emu"]["settings"]["x"], 1);
    }

    #[test]
    fn a_backup_goes_back_into_other_folders_without_what_it_skips() {
        let from = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(from.path().join("saves/GAME/cache")).unwrap();
        std::fs::write(from.path().join("saves/GAME/slot.dat"), b"progress").unwrap();
        std::fs::write(from.path().join("saves/GAME/cache/big"), b"skip me").unwrap();
        std::fs::create_dir_all(from.path().join("saves/EMPTY")).unwrap();
        std::fs::write(from.path().join("settings.ini"), b"a=1").unwrap();
        let file = from.path().join("out").join(file_name(10, false));
        let sources = [
            source("emu", "saves", &from.path().join("saves")),
            source("emu", "settings", &from.path().join("settings.ini")),
            source("emu", "gone", &from.path().join("missing")),
        ];
        let config = serde_json::json!({ "app": { "theme": "default" } });
        let manifest = write(&file, "1.0.0", 10, &config, &sources, &|_, _| {}).unwrap();
        assert_eq!(manifest.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["saves", "settings"]);
        assert_eq!(read_manifest(&file).unwrap(), manifest);
        assert!(!file.with_file_name(format!("{}.part", file_name(10, false))).exists());

        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([
            (("emu".to_owned(), "saves".to_owned()), to.path().join("elsewhere/saves")),
            (("emu".to_owned(), "settings".to_owned()), to.path().join("config.ini")),
        ]);
        let restored = restore(&file, &targets, &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(to.path().join("elsewhere/saves/GAME/slot.dat")).unwrap(), b"progress");
        assert!(to.path().join("elsewhere/saves/EMPTY").is_dir());
        assert!(!to.path().join("elsewhere/saves/GAME/cache").exists(), "skipped");
        assert_eq!(std::fs::read(to.path().join("config.ini")).unwrap(), b"a=1");
        assert_eq!(restored.config, config);
        assert!(restored.left_out.is_empty());
    }

    #[test]
    fn an_item_with_no_place_here_is_left_out() {
        let from = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("keys"), b"k").unwrap();
        let file = from.path().join("b.tar.gz");
        write(&file, "1", 0, &Value::Null, &[source("old", "keys", &from.path().join("keys"))], &|_, _| {}).unwrap();
        let restored = restore(&file, &BTreeMap::new(), &|_, _| {}).unwrap();
        assert_eq!(restored.left_out, ["old/keys"]);
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
            write(&file, "1", 0, &Value::Null, &[source("emu", "saves", &from.path().join("saves"))], &|_, _| {})
                .unwrap();
        assert_eq!(manifest.items[0].bytes, 1, "the link is left out");

        let to = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(to.path().join("saves")).unwrap();
        std::os::unix::fs::symlink(outside.path(), to.path().join("saves/a")).unwrap();
        let targets = BTreeMap::from([(("emu".to_owned(), "saves".to_owned()), to.path().join("saves"))]);
        assert!(restore(&file, &targets, &|_, _| {}).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none(), "nothing written through the link");
    }

    #[test]
    fn an_archive_with_an_entry_that_leaves_its_item_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.tar.gz");
        let manifest = Manifest { format: FORMAT, nexus: "1".into(), made: 0, items: vec![] };
        let gz = flate2::write::GzEncoder::new(File::create(&file).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        append_bytes(&mut tar, MANIFEST, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        append_bytes(&mut tar, "items/emu/saves/ok", b"ok").unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        // `append_data` refuses `..`; the raw name gets it in, as a hostile file would.
        header.as_gnu_mut().unwrap().name[..25].copy_from_slice(b"items/emu/saves/../../bad");
        header.set_cksum();
        tar.append(&header, &b"x"[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();

        let to = tempfile::tempdir().unwrap();
        let targets = BTreeMap::from([(("emu".to_owned(), "saves".to_owned()), to.path().join("saves"))]);
        assert!(restore(&file, &targets, &|_, _| {}).is_err());
        assert!(!to.path().join("saves").exists(), "nothing written");
    }
}
