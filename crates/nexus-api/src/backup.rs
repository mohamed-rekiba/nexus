//! The optional backup capability: what of a plugin's data Settings → Backup saves into an
//! archive, and puts back from one.
//!
//! The plugin only names its items; the host reads and writes them. An item has the same
//! name on every computer, and its place is where it is on this one: a backup made on one
//! computer restores on another whose folders are elsewhere.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{HostPath, PluginResult, RelPath};

/// Where a backup item is on this computer: a file, or a folder and everything in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupPlace {
    /// In the plugin's data folder.
    Data(RelPath),
    /// In a host folder the manifest asks for. A restore writes there only when the grant
    /// has `write = true`.
    Folder(HostPath),
}

/// One thing a backup holds of a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupItem {
    /// Its name in the archive, the same on every computer: lowercase letters, digits and `-`,
    /// such as `saves`.
    pub name: String,
    /// What Settings calls it, such as "Saves".
    pub title: String,
    pub place: BackupPlace,
    /// Names left out wherever they are below `place`, such as `cache` and `log`.
    #[serde(default)]
    pub skip: Vec<String>,
    /// Large, such as PC games' Wine prefixes: in a backup only when the player turns on
    /// "Include large items".
    #[serde(default)]
    pub large: bool,
}

impl BackupItem {
    pub fn new(name: impl Into<String>, title: impl Into<String>, place: BackupPlace) -> Self {
        Self { name: name.into(), title: title.into(), place, skip: Vec::new(), large: false }
    }

    /// Leaves out the entries named `names` wherever they are below the item.
    pub fn skip(mut self, names: &[&str]) -> Self {
        self.skip.extend(names.iter().map(|n| (*n).to_owned()));
        self
    }

    /// Marks the item large: see [`BackupItem::large`].
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Whether `name` can be an item's name: 1 to 64 lowercase letters, digits and `-`.
    pub fn valid_name(name: &str) -> bool {
        (1..=64).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }
}

/// What of a plugin's data a backup holds.
#[async_trait]
pub trait Backup: Send + Sync {
    /// The items, as they are on this computer now. A restore asks again, and puts each item
    /// of the archive where the item of the same name is now; an item the plugin no longer
    /// names is left out.
    async fn items(&self) -> PluginResult<Vec<BackupItem>>;
}
