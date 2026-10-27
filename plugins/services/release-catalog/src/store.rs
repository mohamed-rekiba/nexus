//! The copies the plugin keeps in its storage, one folder for each kind: `fetched/` for the
//! catalogs fetched from the catalog address, `refreshed/` for the refreshed catalogs, and
//! `sources/` for the catalog sources.
//!
//! For each name (`ps5`), a folder has a meta file, `<name>.meta.json`, which says where and
//! when the copy came from and which file it is in. A copy that changes is written under a new
//! name, `<name>.<time in nanoseconds>.json`, before the meta file that names it, so a reader
//! of the meta file never gets another write's copy. The copy named before stays for a reader
//! that read the meta file before; [`CopyStore::prune`] removes the older ones. The catalog
//! sources keep one copy, `<name>.json`, which each write replaces.

use std::sync::Arc;

use nexus_api::serde::Serialize;
use nexus_api::serde::de::DeserializeOwned;
use nexus_api::serde_json;
use nexus_api::{Logger, RelPath, Storage};

/// The copy a [`CopyStore::write`] writes before its meta file.
#[derive(Clone, Copy, Debug)]
pub enum CopyFile<'a> {
    /// A new copy, in the file `body` names: see [`CopyStore::new_body`].
    New { body: &'a str, bytes: &'a [u8] },
    /// None: the copy the meta file named before stays.
    Kept,
    /// The one copy of the name, `<name>.json`.
    Plain(&'a [u8]),
}

/// One folder of kept copies in the plugin's storage.
pub struct CopyStore {
    storage: Arc<dyn Storage>,
    log: Logger,
    /// `fetched`, `refreshed` or `sources`.
    folder: &'static str,
}

impl CopyStore {
    pub fn new(storage: Arc<dyn Storage>, log: Logger, folder: &'static str) -> Self {
        Self { storage, log, folder }
    }

    /// A new file for a copy of `name`: `<name>.<time in nanoseconds>.json`.
    pub fn new_body(name: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        format!("{name}.{nanos}.json")
    }

    /// The path of the copy `body` of `name`, when `body` names one: `<name>.<digits>.json`.
    pub fn path(&self, name: &str, body: &str) -> Result<RelPath, String> {
        let plain = body.strip_prefix(&format!("{name}.")).and_then(|rest| rest.strip_suffix(".json"));
        if !plain.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            return Err(format!("the {} {name} catalog's meta file names no copy of it", self.folder));
        }
        RelPath::new(format!("{}/{body}", self.folder)).map_err(|e| e.to_string())
    }

    /// The path of `<name><part>.json`: the one copy for `part` empty, the meta file for
    /// `.meta`.
    fn own_path(&self, name: &str, part: &str) -> Result<RelPath, String> {
        RelPath::new(format!("{}/{name}{part}.json", self.folder)).map_err(|e| e.to_string())
    }

    /// What the meta file of `name` says, when it reads as an `M`.
    pub async fn latest<M: DeserializeOwned>(&self, name: &str) -> Option<M> {
        let bytes = self.storage.read(&self.own_path(name, ".meta").ok()?).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The bytes of the copy `body` of `name`.
    pub async fn read(&self, name: &str, body: &str) -> Result<Vec<u8>, String> {
        let path = self.path(name, body)?;
        self.storage.read(&path).await.map_err(|e| e.to_string())
    }

    /// The bytes of the one copy of `name`, `<name>.json`.
    pub async fn read_plain(&self, name: &str) -> Result<Vec<u8>, String> {
        let path = self.own_path(name, "")?;
        self.storage.read(&path).await.map_err(|e| e.to_string())
    }

    /// Writes `copy`, then the meta file `meta` of `name`. It fails when either can't be
    /// written: a copy with no meta file is no change, as the next read wouldn't find it.
    pub async fn write<M: Serialize>(&self, name: &str, copy: CopyFile<'_>, meta: &M) -> Result<(), String> {
        match copy {
            CopyFile::New { body, bytes } => {
                let path = self.path(name, body)?;
                self.storage.write(&path, bytes).await.map_err(|e| e.to_string())?;
            }
            CopyFile::Kept => {}
            CopyFile::Plain(bytes) => {
                let path = self.own_path(name, "")?;
                self.storage.write(&path, bytes).await.map_err(|e| e.to_string())?;
            }
        }
        let meta = serde_json::to_vec(meta).map_err(|e| e.to_string())?;
        let path = self.own_path(name, ".meta")?;
        self.storage.write(&path, &meta).await.map_err(|e| e.to_string())
    }

    /// Removes the copies of `name` other than the ones in `keep`: the one the meta file names,
    /// and the one it named before, which a reader that read the meta file before may still
    /// read. A copy that can't be removed is logged.
    pub async fn prune(&self, name: &str, keep: &[&str]) {
        let Ok(path) = RelPath::new(self.folder) else { return };
        let names = self.storage.list(&path).await.unwrap_or_default();
        let meta_file = format!("{name}.meta.json");
        for file in names {
            let copy = file.starts_with(&format!("{name}.")) && file.ends_with(".json") && file != meta_file;
            if !copy || keep.contains(&file.as_str()) {
                continue;
            }
            let removed = match RelPath::new(format!("{}/{file}", self.folder)) {
                Ok(path) => self.storage.remove(&path).await,
                Err(e) => Err(e.into()),
            };
            if let Err(e) = removed {
                self.log.warn(&format!("could not remove the old {} copy {file}: {e}", self.folder));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nexus_api::{LogLevel, LogSink};
    use nexus_testkit::MemStorage;

    struct Quiet;
    impl LogSink for Quiet {
        fn log(&self, _level: LogLevel, _message: &str) {}
    }

    fn store(folder: &'static str) -> (Arc<MemStorage>, CopyStore) {
        let memory = Arc::new(MemStorage::default());
        (memory.clone(), CopyStore::new(memory, Logger::new(Arc::new(Quiet)), folder))
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }

    #[test]
    fn a_meta_file_names_only_a_copy_of_its_own_catalog() {
        let (_, store) = store("refreshed");
        let path = store.path("ps5", "ps5.1791400000000000000.json").unwrap();
        assert_eq!(path.as_str(), "refreshed/ps5.1791400000000000000.json");
        for body in ["", "ps4.1.json", "ps5..json", "ps5.meta.json", "ps5.1/../../x.json"] {
            assert!(store.path("ps5", body).is_err(), "{body}");
        }
    }

    #[test]
    fn a_copy_is_written_before_its_meta_file_and_read_back() {
        runtime().block_on(async {
            let (memory, store) = store("fetched");
            assert_eq!(store.latest::<String>("ps5").await, None, "nothing kept");
            let body = CopyStore::new_body("ps5");
            store.write("ps5", CopyFile::New { body: &body, bytes: b"one" }, &body).await.unwrap();
            let named: String = store.latest("ps5").await.unwrap();
            assert_eq!(store.read("ps5", &named).await.unwrap(), b"one");
            store.write("ps5", CopyFile::Kept, &named).await.unwrap();
            assert_eq!(memory.paths().len(), 2, "the copy and its meta file");
            assert!(store.read("ps5", "ps5.meta.json").await.is_err(), "not a copy");
        });
    }

    #[test]
    fn the_one_copy_of_a_name_is_replaced() {
        runtime().block_on(async {
            let (_, store) = store("sources");
            store.write("catalog-sources", CopyFile::Plain(b"one"), &1).await.unwrap();
            store.write("catalog-sources", CopyFile::Plain(b"two"), &2).await.unwrap();
            assert_eq!(store.read_plain("catalog-sources").await.unwrap(), b"two");
            assert_eq!(store.latest::<u32>("catalog-sources").await, Some(2));
        });
    }

    #[test]
    fn a_prune_keeps_the_copies_named_and_other_names() {
        runtime().block_on(async {
            let (memory, store) = store("refreshed");
            for file in ["ps5.1.json", "ps5.2.json", "ps5.3.json", "ps5.meta.json", "ps4.1.json"] {
                memory.put(&format!("refreshed/{file}"), &[]);
            }
            store.prune("ps5", &["ps5.3.json", "ps5.2.json"]).await;
            let left = memory.paths();
            assert_eq!(
                left,
                ["refreshed/ps4.1.json", "refreshed/ps5.2.json", "refreshed/ps5.3.json", "refreshed/ps5.meta.json"]
            );
        });
    }
}
