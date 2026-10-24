//! The catalog sources: the forums each console's catalog comes from, in the format of the
//! project's `assets/catalog-sources.json`. The catalog collector reads the same file. The root
//! is each catalog, by its file name without `.json`:
//!
//! ```json
//! { "ps5": { "platform": "PS5", "forums": [
//!     { "key": "ps5", "url": "https://xyz.net/forum/viewforum.php?f=546", "tag": "PS5" }] } }
//! ```
//!
//! - A catalog's `platform` is its console, as an emulator's manifest names it.
//! - A forum's `url` is its first listing page. The plugin never fetches it: it takes the
//!   forum's `f=` number for the refresh's addresses (see [`crate::refresh::API`]) and writes the
//!   `url` into the `source` of the topics it adds.
//! - A forum has a console `tag` or a PC `system`, and `wine` is `true` for a forum whose games
//!   are packed with Wine.
//!
//! A file reads only when all of it does: every forum's `url` starts with `https://`, because
//! it goes into the `source` of each topic the refresh adds.

use std::collections::BTreeMap;

use nexus_api::Console;
use nexus_api::serde::Deserialize;
use nexus_api::serde_json;

use crate::https_host;
use crate::refresh::forum_number;

/// The forums of each console's catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(crate = "nexus_api::serde", transparent)]
pub struct Sources {
    /// Each catalog, by its file name without `.json` (`ps5`): the root of the file.
    pub catalogs: BTreeMap<String, SourceCatalog>,
}

/// One console's catalog and the forums it comes from.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct SourceCatalog {
    /// The console, as an emulator's manifest names it (`PS5`). The file names it `platform`,
    /// as the catalog collector does.
    #[serde(rename = "platform")]
    pub console: Console,
    pub forums: Vec<Forum>,
}

/// One forum.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(crate = "nexus_api::serde")]
pub struct Forum {
    /// The name the collector uses for it (`ps5`).
    pub key: String,
    /// Its first listing page, `.../viewforum.php?f=<number>`.
    pub url: String,
    /// For a console forum, the tag that starts its titles (`PS5`).
    #[serde(default)]
    pub tag: Option<String>,
    /// For a PC forum, the system its games are for (`linux`).
    #[serde(default)]
    pub system: Option<String>,
    /// Its games are packed with Wine.
    #[serde(default)]
    pub wine: bool,
}

/// Reads a catalog sources file.
pub fn read(bytes: &[u8]) -> Result<Sources, String> {
    let sources: Sources = serde_json::from_slice(bytes)
        .map_err(|e| format!("this is not a catalog sources file in the current format: {e}"))?;
    for url in sources.catalogs.values().flat_map(|c| c.forums.iter().map(|f| f.url.as_str())) {
        if https_host(url).is_none() {
            return Err(format!("not a catalog sources file: {url} is not an https:// address"));
        }
    }
    Ok(sources)
}

impl Sources {
    /// The sources with only the forums whose `url` has an `f=` number, and each forum left
    /// out, as `<key>: <url>`. A catalog with no forum left is left out.
    pub fn usable(mut self) -> (Sources, Vec<String>) {
        let mut unusable = Vec::new();
        for catalog in self.catalogs.values_mut() {
            catalog.forums.retain(|f| {
                let ok = forum_number(&f.url).is_some();
                if !ok {
                    unusable.push(format!("{}: {}", f.key, f.url));
                }
                ok
            });
        }
        self.catalogs.retain(|_, c| !c.forums.is_empty());
        (self, unusable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &str = r#"{
          "ps5": { "platform": "PS5", "forums": [{ "key": "ps5", "url": "https://xyz.net/f?f=1", "tag": "PS5" }] },
          "pc": { "platform": "PC", "forums": [
            { "key": "linux", "url": "https://xyz.net/f?f=2", "system": "linux" },
            { "key": "wine", "url": "https://xyz.net/f?t=3", "system": "linux", "wine": true }] } }"#;

    #[test]
    fn a_sources_file_reads_with_its_forums() {
        let sources = read(SOURCES.as_bytes()).unwrap();
        assert_eq!(sources.catalogs["pc"].console.as_str(), "PC");
        let wine = &sources.catalogs["pc"].forums[1];
        assert_eq!((wine.key.as_str(), wine.system.as_deref(), wine.wine), ("wine", Some("linux"), true));
        assert_eq!(sources.catalogs["ps5"].forums[0].tag.as_deref(), Some("PS5"));
        assert!(read(b"{ not json").is_err());
        assert!(read(br#"{ "api": "https://xyz.net/f/" }"#).is_err(), "the root is the catalogs");
    }

    #[test]
    fn a_sources_file_with_an_address_that_is_not_https_does_not_read() {
        let plain = SOURCES.replace("https://xyz.net/f?f=2", "http://xyz.net/f?f=2");
        assert!(read(plain.as_bytes()).unwrap_err().contains("http://xyz.net/f?f=2 is not an https:// address"));
    }

    #[test]
    fn a_forum_with_no_f_number_is_left_out_and_named() {
        let (kept, unusable) = read(SOURCES.as_bytes()).unwrap().usable();
        assert_eq!(unusable, ["wine: https://xyz.net/f?t=3"]);
        let keys: Vec<&str> = kept.catalogs["pc"].forums.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["linux"]);
        assert_eq!(kept.catalogs["ps5"].forums.len(), 1);
    }
}
