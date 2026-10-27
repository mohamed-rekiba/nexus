//! The archives in a finished download, which the game installs unpack through the host's
//! [`crate::Archives`] port before the emulator installs ([`crate::game_installs`]).
//!
//! An archive set is one archive, or one cut into parts: `game.part1.rar`, `game.part2.rar`;
//! `game.rar` with `game.r00`; `game.7z.001`, `game.7z.002`. Sets are found by their names, at
//! the top of the download or one folder down; a GOG installer (a script with a zip at its end)
//! by its bytes. Each set unpacks into `unpacked/<folder>/<name>/` in the download, through a
//! `.part` folder that moves into place at the end.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::ArchiveKind;

/// The folder in a download that unpacked sets go to.
pub(crate) const UNPACKED: &str = "unpacked";
/// The most parts one set can have.
const MAX_PARTS: u32 = 999;

/// One archive set in a download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArchiveSet {
    /// Its folder in the download and its name, as `a/game` for `a/game.part1.rar`: where it
    /// unpacks, under [`UNPACKED`].
    pub name: String,
    pub kind: ArchiveKind,
    /// Its parts in order, from the download's folder.
    pub parts: Vec<String>,
    /// Why it can't be unpacked, such as a missing part. Nothing is written for it then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Which set a file is a part of, and which part (the emulator kit's `archive_sets::set_name`
/// follows these rules for plugins): its kind, its set's name (folder and stem,
/// lower case) and its part number from 1. `None` for a file that isn't an archive.
pub(crate) fn part_of(path: &str) -> Option<(ArchiveKind, String, u32)> {
    let lower = path.to_ascii_lowercase();
    let number = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.parse::<u32>().ok()).flatten()
    };
    if let Some(stem) = lower.strip_suffix(".rar") {
        if let Some((name, part)) = stem.rsplit_once(".part")
            && let Some(n) = number(part)
        {
            return Some((ArchiveKind::Rar, name.to_owned(), n));
        }
        return Some((ArchiveKind::Rar, stem.to_owned(), 1));
    }
    if let Some((stem, ext)) = lower.rsplit_once('.') {
        // `game.r00` is the second part of `game.rar`.
        if let Some(digits) = ext.strip_prefix('r')
            && digits.len() == 2
            && let Some(n) = number(digits)
        {
            return Some((ArchiveKind::Rar, stem.to_owned(), n + 2));
        }
        if let Some(name) = stem.strip_suffix(".7z")
            && let Some(n) = number(ext)
        {
            return Some((ArchiveKind::SevenZ, name.to_owned(), n));
        }
    }
    for (ext, kind) in [
        (".tar.gz", ArchiveKind::TarGz),
        (".tgz", ArchiveKind::TarGz),
        (".tar.xz", ArchiveKind::TarXz),
        (".txz", ArchiveKind::TarXz),
        (".tar.bz2", ArchiveKind::TarBz2),
        (".tbz2", ArchiveKind::TarBz2),
        (".tbz", ArchiveKind::TarBz2),
        (".tar.zst", ArchiveKind::TarZst),
        (".tzst", ArchiveKind::TarZst),
        (".iso", ArchiveKind::Iso),
        (".cab", ArchiveKind::Cab),
        (".tar", ArchiveKind::Tar),
        (".7z", ArchiveKind::SevenZ),
        (".zip", ArchiveKind::Zip),
    ] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return Some((kind, stem.to_owned(), 1));
        }
    }
    None
}

/// A set's files by part number: more than one under a number is a doubled part.
type Parts = BTreeMap<u32, Vec<String>>;

/// The archive sets among `paths` (files from a download's folder, `/` between folders), at
/// the top or one folder down, in name order. A set with a missing or doubled part says so.
pub(crate) fn sets_of<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<ArchiveSet> {
    // By the lower-case name; each set keeps the name as its first part spells it.
    let mut groups: BTreeMap<(String, ArchiveKind), (String, Parts)> = BTreeMap::new();
    for path in paths {
        if path.matches('/').count() > 1 {
            continue;
        }
        if let Some((kind, name, n)) = part_of(path) {
            let spelled = path[..name.len()].to_owned();
            let group = groups.entry((name, kind)).or_insert_with(|| (spelled, BTreeMap::new()));
            group.1.entry(n).or_default().push(path.to_owned());
        }
    }
    groups
        .into_iter()
        .map(|((_, kind), (name, parts))| {
            let mut problem = None;
            let mut ordered = Vec::new();
            for (i, (n, paths)) in parts.into_iter().enumerate() {
                if paths.len() > 1 {
                    problem.get_or_insert(format!("{} and {} are the same part of one archive", paths[0], paths[1]));
                }
                if n != i as u32 + 1 || n > MAX_PARTS {
                    problem.get_or_insert(format!("part {} of {name} is missing", i + 1));
                }
                ordered.push(paths[0].clone());
            }
            ArchiveSet { name, kind, parts: ordered, problem }
        })
        .collect()
}

/// Whether the file at `path` is a GOG installer: a shell script with a zip at its end.
pub(crate) fn is_gog_installer(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return false };
    let mut head = [0u8; 2];
    if len < 1024 || file.read_exact(&mut head).is_err() || &head != b"#!" {
        return false;
    }
    // A zip's end record is in its last 64 KiB.
    let tail = len.min(64 * 1024);
    let mut buf = Vec::with_capacity(tail as usize);
    if file.seek(SeekFrom::Start(len - tail)).is_err() || file.take(tail).read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(4).any(|w| w == b"PK\x05\x06")
}

/// How far into a script installer's header its archive's size can be stated.
const SCRIPT_HEAD: u64 = 128 * 1024;

/// Where the archive a script installer carries at its end starts, when its header states the
/// archive's size on an `arcsz=` line, as Kron4ek's YAD installers do; `None` for any other file.
pub fn embedded_archive(path: &Path) -> Option<u64> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = Vec::new();
    file.take(SCRIPT_HEAD).read_to_end(&mut head).ok()?;
    if !head.starts_with(b"#!") {
        return None;
    }
    let size: u64 = head.split(|b| *b == b'\n').find_map(|line| {
        let value = line.strip_prefix(b"arcsz=")?;
        let digits = std::str::from_utf8(value).ok()?.trim().trim_matches(|c| c == '"' || c == '\'');
        digits.parse().ok()
    })?;
    // The script comes first.
    (size > 0 && size.checked_add(1024)? <= len).then(|| len - size)
}

/// Whether `path` is named as a script installer is: `.sh` or `.run`.
fn script_named(path: &str) -> Option<&str> {
    let lower = path.to_ascii_lowercase();
    [".sh", ".run"].iter().find(|ext| lower.ends_with(*ext)).map(|ext| &path[..path.len() - ext.len()])
}

/// The archive sets in the download at `folder`: by name, and script installers (GOG's, and
/// those that state their archive's size) by their bytes.
/// The `unpacked` folder is left out.
pub(crate) fn scan(folder: &Path) -> Vec<ArchiveSet> {
    let mut paths = Vec::new();
    let mut gog = Vec::new();
    let list = |dir: &Path| -> Vec<(String, bool)> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|e| Some((e.file_name().into_string().ok()?, e.file_type().ok()?.is_dir())))
            .collect()
    };
    for (name, is_dir) in list(folder) {
        if is_dir {
            if name == UNPACKED {
                continue;
            }
            for (inner, inner_dir) in list(&folder.join(&name)) {
                if !inner_dir {
                    paths.push(format!("{name}/{inner}"));
                }
            }
        } else {
            paths.push(name);
        }
    }
    for path in &paths {
        let Some(name) = script_named(path) else { continue };
        let file = folder.join(path);
        let kind = if path.to_ascii_lowercase().ends_with(".sh") && is_gog_installer(&file) {
            ArchiveKind::Gog
        } else if embedded_archive(&file).is_some() {
            ArchiveKind::Script
        } else {
            continue;
        };
        gog.push(ArchiveSet { name: name.to_owned(), kind, parts: vec![path.clone()], problem: None });
    }
    let mut sets = sets_of(paths.iter().map(String::as_str));
    sets.extend(gog);
    sets.sort_by(|a, b| a.name.cmp(&b.name));
    sets
}

/// How far a download's archives are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UnpackPhase {
    /// The download holds no archive: it installs as it is.
    #[default]
    None,
    /// Its archives aren't unpacked yet.
    Waiting,
    Unpacking,
    Unpacked,
    Failed,
    Cancelled,
}

impl UnpackPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Waiting => "waiting",
            Self::Unpacking => "unpacking",
            Self::Unpacked => "unpacked",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A download's unpacking, as it is saved.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Unpack {
    /// The archive sets found when the download finished.
    #[serde(default)]
    pub sets: Vec<ArchiveSet>,
    #[serde(default)]
    pub phase: UnpackPhase,
    /// 0.0 to 1.0 over every set.
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub error: String,
    /// Set to stop the unpacking that runs.
    #[serde(skip)]
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Unpack {
    /// The unpacking of a download that just finished in `folder`.
    pub fn of(folder: &Path) -> Self {
        let sets = scan(folder);
        let phase = if sets.is_empty() { UnpackPhase::None } else { UnpackPhase::Waiting };
        Self { sets, phase, ..Self::default() }
    }

    /// After a restart: an unpacking the app's stop cut fails, and its half folders go.
    pub fn restored(&mut self, folder: &Path) {
        if self.phase == UnpackPhase::Unpacking {
            self.phase = UnpackPhase::Failed;
            self.error = "The unpacking stopped when Nexus closed: Install unpacks again.".into();
            for set in &self.sets {
                let _ = std::fs::remove_dir_all(work_folder(folder, set));
            }
        }
    }
}

/// Where a set goes in the download at `folder`.
pub(crate) fn unpacked_folder(folder: &Path, set: &ArchiveSet) -> PathBuf {
    folder.join(UNPACKED).join(&set.name)
}

/// Where a set is written before it moves into place.
pub(crate) fn work_folder(folder: &Path, set: &ArchiveSet) -> PathBuf {
    folder.join(UNPACKED).join(format!("{}.part", set.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sets: &[ArchiveSet]) -> Vec<(String, ArchiveKind, Vec<String>, Option<String>)> {
        sets.iter().map(|s| (s.name.clone(), s.kind, s.parts.clone(), s.problem.clone())).collect()
    }

    #[test]
    fn a_part_is_known_by_its_name() {
        assert_eq!(part_of("Game.part3.rar"), Some((ArchiveKind::Rar, "game".into(), 3)));
        assert_eq!(part_of("game.rar"), Some((ArchiveKind::Rar, "game".into(), 1)));
        assert_eq!(part_of("game.r00"), Some((ArchiveKind::Rar, "game".into(), 2)));
        assert_eq!(part_of("game.r11"), Some((ArchiveKind::Rar, "game".into(), 13)));
        assert_eq!(part_of("a/game.7z.002"), Some((ArchiveKind::SevenZ, "a/game".into(), 2)));
        assert_eq!(part_of("game.7z"), Some((ArchiveKind::SevenZ, "game".into(), 1)));
        assert_eq!(part_of("game.tar.gz"), Some((ArchiveKind::TarGz, "game".into(), 1)));
        assert_eq!(part_of("game.tgz"), Some((ArchiveKind::TarGz, "game".into(), 1)));
        assert_eq!(part_of("data.tar"), Some((ArchiveKind::Tar, "data".into(), 1)));
        assert_eq!(part_of("game.tar.xz"), Some((ArchiveKind::TarXz, "game".into(), 1)));
        assert_eq!(part_of("game.tbz2"), Some((ArchiveKind::TarBz2, "game".into(), 1)));
        assert_eq!(part_of("game.tar.zst"), Some((ArchiveKind::TarZst, "game".into(), 1)));
        assert_eq!(part_of("disc.iso"), Some((ArchiveKind::Iso, "disc".into(), 1)));
        assert_eq!(part_of("data1.cab"), Some((ArchiveKind::Cab, "data1".into(), 1)));
        assert_eq!(part_of("game.zip"), Some((ArchiveKind::Zip, "game".into(), 1)));
        assert_eq!(part_of("game.pkg"), None);
        assert_eq!(part_of("readme.txt"), None);
        assert_eq!(part_of("music.rock"), None, "an r with no digits isn't a part");
    }

    #[test]
    fn parts_group_into_sets_in_order_and_each_folder_is_its_own_set() {
        let sets = sets_of(["b/game.rar", "game.part2.rar", "game.part1.rar", "a/game.rar", "a/game.r00", "x.7z.001"]);
        assert_eq!(
            names(&sets),
            [
                ("a/game".into(), ArchiveKind::Rar, vec!["a/game.rar".into(), "a/game.r00".into()], None),
                ("b/game".into(), ArchiveKind::Rar, vec!["b/game.rar".into()], None),
                ("game".into(), ArchiveKind::Rar, vec!["game.part1.rar".into(), "game.part2.rar".into()], None),
                ("x".into(), ArchiveKind::SevenZ, vec!["x.7z.001".into()], None),
            ]
        );
    }

    #[test]
    fn a_missing_or_doubled_part_is_a_problem_and_deep_archives_are_left_alone() {
        let missing = sets_of(["game.part1.rar", "game.part3.rar"]);
        assert_eq!(missing[0].problem.as_deref(), Some("part 2 of game is missing"));
        let first_missing = sets_of(["game.7z.002"]);
        assert_eq!(first_missing[0].problem.as_deref(), Some("part 1 of game is missing"));
        let doubled = sets_of(["Game.part1.rar", "game.part1.rar"]);
        assert!(doubled[0].problem.as_deref().is_some_and(|p| p.contains("the same part")), "{doubled:?}");
        assert!(sets_of(["Game/Data/levels.zip"]).is_empty(), "two folders down: the game's own data");
    }

    #[test]
    fn a_script_installer_that_states_its_archive_s_size_is_unpacked_from_its_end() {
        let dir = std::env::temp_dir().join(format!("nexus-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut installer = b"#!/usr/bin/env sh\napp=\"Game\"\narcsz=2048\nexport arcsz\nexit 0\n".to_vec();
        installer.resize(4096, b'#');
        let start = installer.len() as u64;
        installer.resize(4096 + 2048, 0);
        std::fs::write(dir.join("game_v1.2.run"), &installer).unwrap();
        // Says a size bigger than the file: not one.
        std::fs::write(dir.join("broken.sh"), b"#!/bin/sh\narcsz=99999999\n".repeat(100)).unwrap();
        let mut plain = b"#!/bin/sh\n".to_vec();
        plain.resize(4096, b' ');
        std::fs::write(dir.join("start.sh"), plain).unwrap();

        assert_eq!(embedded_archive(&dir.join("game_v1.2.run")), Some(start));
        assert_eq!(embedded_archive(&dir.join("broken.sh")), None);
        assert_eq!(embedded_archive(&dir.join("start.sh")), None);
        let sets = scan(&dir);
        assert_eq!(names(&sets), [("game_v1.2".into(), ArchiveKind::Script, vec!["game_v1.2.run".into()], None)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gog_installer_is_a_script_with_a_zip_at_its_end() {
        let dir = std::env::temp_dir().join(format!("nexus-gog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut gog = b"#!/bin/sh\necho installer\n".to_vec();
        gog.resize(4096, b'#');
        gog.extend_from_slice(b"PK\x05\x06");
        gog.extend_from_slice(&[0; 18]);
        std::fs::write(dir.join("game.sh"), &gog).unwrap();
        let mut plain = b"#!/bin/sh\n".to_vec();
        plain.resize(4096, b' ');
        std::fs::write(dir.join("start.sh"), plain).unwrap();

        let sets = scan(&dir);
        assert_eq!(names(&sets), [("game".into(), ArchiveKind::Gog, vec!["game.sh".into()], None)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
