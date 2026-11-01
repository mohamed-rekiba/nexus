//! Steam's app cache, `appcache/appinfo.vdf`: what Steam knows of each app, in binary. Only
//! the names of the apps asked for are read.
//!
//! The file is a header, then one entry per app until an app id of 0:
//!
//! - header: a magic number whose last byte is the version, the universe, and from version
//!   0x29 the offset of a string table at the file's end;
//! - entry: the app id and the size of what follows (both `u32`), 60 bytes of state, change
//!   number and checksums, then the app's keys in Valve's binary format, where a key is an
//!   index into the string table (0x29) or a zero-ended string (0x28).
//!
//! The file can be over 100 MB on a big account, so it is read in pieces, and an entry that
//! isn't wanted is skipped by its size.

use std::collections::{BTreeMap, BTreeSet};

use nexus_api::*;

/// The magic numbers of the versions read: 0x28 has keys inline, 0x29 a string table.
const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;
/// An entry's state, change number and checksums, before its keys.
const ENTRY_HEAD: usize = 4 + 4 + 8 + 20 + 4 + 20;
/// The most bytes one app's entry may have to be read: a game's is a few kilobytes.
const MAX_ENTRY: u32 = 4 * 1024 * 1024;
/// How much is read at once.
const PIECE: u64 = 4 * 1024 * 1024;

/// A file read in pieces through [`Folders`].
struct Pieces<'a> {
    folders: &'a dyn Folders,
    file: &'a HostPath,
    /// The piece in memory, and where it starts in the file.
    piece: Vec<u8>,
    start: u64,
}

impl<'a> Pieces<'a> {
    /// `len` bytes at `at`; `None` past the end or when the file can't be read.
    async fn bytes(&mut self, at: u64, len: usize) -> Option<Vec<u8>> {
        let end = at + len as u64;
        let inside = at >= self.start && end <= self.start + self.piece.len() as u64;
        if !inside {
            let want = PIECE.max(len as u64).min(MAX_FOLDER_READ);
            self.piece = self.folders.read_range(self.file, at, want).await.ok()?;
            self.start = at;
        }
        let from = (at - self.start) as usize;
        self.piece.get(from..from + len).map(<[u8]>::to_vec)
    }

    async fn u32_at(&mut self, at: u64) -> Option<u32> {
        let b = self.bytes(at, 4).await?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// The names Steam's cache `file` gives the apps `wanted`. An app it has no entry for, or no
/// name, is left out; an unknown version gives none.
pub(crate) async fn names(folders: &dyn Folders, file: &HostPath, wanted: &BTreeSet<u32>) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let mut pieces = Pieces { folders, file, piece: Vec::new(), start: 0 };
    let Some(magic) = pieces.u32_at(0).await else { return out };
    let (strings, mut at) = match magic {
        V29 => {
            let Some(offset) = pieces.bytes(8, 8).await else { return out };
            let offset = u64::from_le_bytes(offset.try_into().unwrap_or_default());
            let Some(strings) = string_table(&mut pieces, offset).await else { return out };
            (Some(strings), 16)
        }
        V28 => (None, 8),
        _ => return out,
    };
    while out.len() < wanted.len() {
        let (Some(app), Some(size)) = (pieces.u32_at(at).await, pieces.u32_at(at + 4).await) else { break };
        if app == 0 {
            break;
        }
        if wanted.contains(&app)
            && size <= MAX_ENTRY
            && let Some(entry) = pieces.bytes(at + 8, size as usize).await
            && let Some(name) = entry.get(ENTRY_HEAD..).and_then(|keys| name_in(keys, strings.as_deref()))
        {
            out.insert(app, name);
        }
        at += 8 + u64::from(size);
    }
    out
}

/// The string table at `offset`: a count, then that many zero-ended strings.
async fn string_table(pieces: &mut Pieces<'_>, offset: u64) -> Option<Vec<String>> {
    let count = pieces.u32_at(offset).await?;
    let mut strings = Vec::with_capacity(count.min(1 << 20) as usize);
    let mut at = offset + 4;
    let mut word = Vec::new();
    while strings.len() < count as usize {
        let piece = pieces.bytes(at, 1).await?;
        at += 1;
        if piece[0] == 0 {
            strings.push(String::from_utf8_lossy(&word).into_owned());
            word.clear();
        } else {
            word.push(piece[0]);
        }
    }
    Some(strings)
}

/// `appinfo` → `common` → `name` in an app's keys.
fn name_in(keys: &[u8], strings: Option<&[String]>) -> Option<String> {
    let mut reader = Keys { bytes: keys, at: 0, strings };
    reader.find(&["appinfo", "common", "name"], 0)
}

/// Valve's binary keys, read for one text value.
struct Keys<'a> {
    bytes: &'a [u8],
    at: usize,
    strings: Option<&'a [String]>,
}

impl Keys<'_> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        (self.at + n <= self.bytes.len()).then(|| self.at += n)
    }

    fn text(&mut self) -> Option<String> {
        let end = self.bytes.get(self.at..)?.iter().position(|b| *b == 0)?;
        let text = String::from_utf8_lossy(&self.bytes[self.at..self.at + end]).into_owned();
        self.at += end + 1;
        Some(text)
    }

    fn key(&mut self) -> Option<String> {
        match self.strings {
            Some(strings) => {
                let b = self.bytes.get(self.at..self.at + 4)?;
                self.at += 4;
                strings.get(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).cloned()
            }
            None => self.text(),
        }
    }

    /// The text at `path` from `depth` in the block being read, which is read to its end.
    fn find(&mut self, path: &[&str], depth: usize) -> Option<String> {
        let mut found = None;
        loop {
            let kind = self.byte()?;
            if kind == 8 || kind == 11 {
                return found;
            }
            let key = self.key()?;
            let on_path = found.is_none() && path.get(depth).is_some_and(|p| *p == key);
            match kind {
                0 => {
                    let inner = self.find(path, depth + 1);
                    if on_path {
                        found = inner;
                    }
                }
                1 => {
                    let value = self.text()?;
                    if on_path && depth + 1 == path.len() {
                        found = Some(value);
                    }
                }
                2..=4 | 6 => self.skip(4)?,
                7 | 10 => self.skip(8)?,
                // UTF-16, ended by two zero bytes.
                5 => {
                    while self.bytes.get(self.at..self.at + 2)? != [0, 0] {
                        self.at += 2;
                    }
                    self.at += 2;
                }
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A cache file of version `magic` with an app of each `(id, name)`.
    pub(crate) fn cache(magic: u32, apps: &[(u32, &str)]) -> Vec<u8> {
        let table = ["appinfo", "common", "name", "type", "oslist"];
        let index = |k: &str| table.iter().position(|t| *t == k).unwrap() as u32;
        let key = |out: &mut Vec<u8>, k: &str| {
            if magic == V29 {
                out.extend(index(k).to_le_bytes());
            } else {
                out.extend(k.as_bytes());
                out.push(0);
            }
        };
        let mut body = Vec::new();
        body.extend(magic.to_le_bytes());
        body.extend(1u32.to_le_bytes());
        if magic == V29 {
            body.extend(0u64.to_le_bytes());
        }
        for (id, name) in apps {
            let mut keys = Vec::new();
            keys.push(0);
            key(&mut keys, "appinfo");
            keys.push(2);
            key(&mut keys, "type");
            keys.extend(7u32.to_le_bytes());
            keys.push(0);
            key(&mut keys, "common");
            keys.push(1);
            key(&mut keys, "oslist");
            keys.extend(b"linux\0");
            keys.push(1);
            key(&mut keys, "name");
            keys.extend(name.as_bytes());
            keys.push(0);
            keys.extend([8, 8, 8]);
            body.extend(id.to_le_bytes());
            body.extend(((ENTRY_HEAD + keys.len()) as u32).to_le_bytes());
            body.extend(vec![0u8; ENTRY_HEAD]);
            body.extend(keys);
        }
        body.extend(0u32.to_le_bytes());
        if magic == V29 {
            let offset = body.len() as u64;
            body[8..16].copy_from_slice(&offset.to_le_bytes());
            body.extend((table.len() as u32).to_le_bytes());
            for t in table {
                body.extend(t.as_bytes());
                body.push(0);
            }
        }
        body
    }

    #[test]
    fn an_app_s_name_is_read_from_its_keys_in_either_version() {
        for magic in [V28, V29] {
            let bytes = cache(magic, &[(440, "Team Fortress 2")]);
            let strings = (magic == V29).then(|| {
                vec!["appinfo", "common", "name", "type", "oslist"].into_iter().map(String::from).collect::<Vec<_>>()
            });
            let keys = &bytes[(if magic == V29 { 16 } else { 8 }) + 8 + ENTRY_HEAD..];
            assert_eq!(name_in(keys, strings.as_deref()).as_deref(), Some("Team Fortress 2"), "{magic:#x}");
        }
    }
}
