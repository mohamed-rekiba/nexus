//! The player's Steam library, for the Catalog: the games in their Steam collections.
//!
//! Steam keeps each account's collections in `userdata/<account>/config/cloudstorage/`, in
//! `cloud-storage-namespace-<n>.json` files: a list of `[key, entry]`, where a collection's key
//! starts `user-collections.` and its entry's `value` is JSON with the collection's `id`, `name`
//! and the app ids `added`. Changes Steam hasn't sent yet are in the matching `.modified.json`,
//! which wins. `favorite` and `hidden` are Steam's Favorites and Hidden. Every file of the
//! folder is read, whatever its number: Steam's numbering isn't promised.

use std::collections::{BTreeMap, BTreeSet};

use nexus_api::serde_json::{self, Value};
use nexus_api::*;

use crate::{ART, COVER, HEADER, PLATFORM, STEAM, Steam, appinfo, vdf};

/// The most bytes of a collections or users file read.
const MAX_FILE: u64 = 8 * 1024 * 1024;
/// Steam IDs of individual accounts are this plus the account's number (its `userdata` folder).
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;
/// The link that asks Steam to install a game: its install dialog.
pub(crate) const INSTALL: &str = "steam://install/";

/// A game in the player's collections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Owned {
    pub favorite: bool,
    pub hidden: bool,
}

/// The app ids of the player's collections, each with whether it is a favorite and hidden,
/// from the collections files' text (`files`, each with its `.modified.json` after it).
pub(crate) fn collections<'a>(files: impl IntoIterator<Item = &'a str>) -> BTreeMap<u32, Owned> {
    // By key: the last file to name it wins, so a `.modified.json` after its file.
    let mut entries: BTreeMap<String, Value> = BTreeMap::new();
    for text in files {
        let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else { continue };
        for pair in list {
            if let Some(key) = pair.get(0).and_then(Value::as_str).filter(|k| k.starts_with("user-collections."))
                && let Some(entry) = pair.get(1)
            {
                entries.insert(key.to_owned(), entry.clone());
            }
        }
    }
    let mut games: BTreeMap<u32, Owned> = BTreeMap::new();
    for entry in entries.values() {
        if entry.get("is_deleted").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(value) = entry.get("value").and_then(Value::as_str) else { continue };
        let Ok(collection) = serde_json::from_str::<Value>(value) else { continue };
        let id = collection.get("id").and_then(Value::as_str).unwrap_or_default();
        let ids = |name: &str| -> Vec<u32> {
            let list = collection.get(name).and_then(Value::as_array);
            list.into_iter().flatten().filter_map(Value::as_u64).filter_map(|n| u32::try_from(n).ok()).collect()
        };
        let removed: BTreeSet<u32> = ids("removed").into_iter().collect();
        for app in ids("added").into_iter().filter(|a| !removed.contains(a)) {
            let game = games.entry(app).or_default();
            game.favorite |= id == "favorite";
            game.hidden |= id == "hidden";
        }
    }
    games
}

impl Steam {
    /// Steam's own folder, among the `steam` folders: the one with a `userdata` folder.
    async fn steam_root(&self) -> Option<u32> {
        for library in self.libraries().await.ok()? {
            let userdata = HostPath::new(STEAM, library.root, RelPath::new("userdata").ok()?);
            if !self.ctx.folders().list(&userdata).await.unwrap_or_default().is_empty() {
                return Some(library.root);
            }
        }
        None
    }

    /// The signed-in account's number, its `userdata` folder: the only one, else the one
    /// `config/loginusers.vdf` marks most recent.
    async fn account(&self, root: u32) -> Option<String> {
        let folders = self.ctx.folders();
        let userdata = HostPath::new(STEAM, root, RelPath::new("userdata").ok()?);
        let accounts: Vec<String> = folders
            .list(&userdata)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.is_dir && !e.name.is_empty() && e.name.bytes().all(|b| b.is_ascii_digit()) && e.name != "0")
            .map(|e| e.name)
            .collect();
        if accounts.len() <= 1 {
            return accounts.into_iter().next();
        }
        let users = HostPath::new(STEAM, root, RelPath::new("config/loginusers.vdf").ok()?);
        let bytes = folders.read_range(&users, 0, MAX_FILE).await.ok()?;
        let file = vdf::parse(&String::from_utf8_lossy(&bytes))?;
        let recent = file.block("users")?.0.iter().find_map(|(id, user)| {
            let vdf::Value::Block(user) = user else { return None };
            (user.text("MostRecent") == Some("1")).then(|| id.parse::<u64>().ok()).flatten()
        })?;
        let account = recent.checked_sub(STEAM_ID_BASE)?.to_string();
        accounts.into_iter().find(|a| *a == account)
    }

    /// The games in the signed-in account's collections, by app id. Empty without Steam.
    pub(crate) async fn owned(&self) -> BTreeMap<u32, Owned> {
        let Some(root) = self.steam_root().await else { return BTreeMap::new() };
        let Some(account) = self.account(root).await else { return BTreeMap::new() };
        let Ok(dir) = RelPath::new(format!("userdata/{account}/config/cloudstorage")) else { return BTreeMap::new() };
        let dir = HostPath::new(STEAM, root, dir);
        let folders = self.ctx.folders();
        let mut names: Vec<String> = folders
            .list(&dir)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.name)
            .filter(|n| {
                n.starts_with("cloud-storage-namespace-") && n.ends_with(".json") && !n.ends_with(".modified.json")
            })
            .filter(|n| n != "cloud-storage-namespaces.json")
            .collect();
        names.sort();
        let mut texts = Vec::new();
        for name in names {
            for file in [name.clone(), name.replace(".json", ".modified.json")] {
                let Ok(file) = dir.join(&file) else { continue };
                if let Ok(bytes) = folders.read_range(&file, 0, MAX_FILE).await {
                    texts.push(String::from_utf8_lossy(&bytes).into_owned());
                }
            }
        }
        collections(texts.iter().map(String::as_str))
    }

    /// The Catalog's Steam games: one release each, which Steam installs.
    pub(crate) async fn library_releases(&self) -> PluginResult<ReleaseList> {
        let owned = self.owned().await;
        let mut names = BTreeMap::new();
        if let Some(root) = self.steam_root().await {
            let cache = HostPath::new(STEAM, root, RelPath::new("appcache/appinfo.vdf")?);
            let wanted: BTreeSet<u32> = owned.keys().copied().collect();
            names = appinfo::names(self.ctx.folders().as_ref(), &cache, &wanted).await;
        }
        let releases = owned
            .into_iter()
            .map(|(app, game)| {
                let art = |file: &str| Some(ImageSource::Url(format!("{ART}/{app}/{file}")));
                Release {
                    id: format!("steam-{app}"),
                    title_id: app.to_string(),
                    platform: PLATFORM.into(),
                    // A game the cache doesn't name yet still shows, by its number.
                    title: names.get(&app).cloned().unwrap_or_else(|| format!("Steam app {app}")),
                    store: format!("{INSTALL}{app}"),
                    hidden: game.hidden,
                    favorite: game.favorite,
                    // Only what Steam's server has for every game: a guessed hero or logo is
                    // missing for an older game, so the header is the background.
                    metadata: Some(GameMetadata {
                        cover: art(COVER),
                        background: art(HEADER),
                        store_url: format!("https://store.steampowered.com/app/{app}/"),
                        ..GameMetadata::default()
                    }),
                    ..Release::default()
                }
            })
            .collect();
        Ok(ReleaseList { releases, ..ReleaseList::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A collections file as Steam writes it: `[key, entry]` pairs, each collection's value JSON
    /// in a string.
    fn file(collections: &[(&str, &str, &str, bool)]) -> String {
        let pairs: Vec<Value> = collections
            .iter()
            .map(|(id, name, added, deleted)| {
                let value = format!(r#"{{"id":"{id}","name":"{name}","added":{added},"removed":[]}}"#);
                let mut entry = serde_json::json!({ "key": format!("user-collections.{id}"), "timestamp": 1 });
                if *deleted {
                    entry["is_deleted"] = true.into();
                } else {
                    entry["value"] = value.into();
                }
                serde_json::json!([format!("user-collections.{id}"), entry])
            })
            .chain([serde_json::json!(["showcases.1", { "key": "showcases.1", "value": "{}" }])])
            .collect();
        serde_json::to_string(&pairs).unwrap()
    }

    #[test]
    fn the_library_is_every_collection_s_games_with_favorites_and_hidden_marked() {
        let base = file(&[
            ("favorite", "Favorites", "[440, 730]", false),
            ("hidden", "Hidden", "[2500]", false),
            ("uc-1", "Co-op", "[730, 620]", false),
            ("uc-old", "Gone", "[999]", true),
        ]);
        // Not sent yet: 35700 is now a favorite too.
        let modified = file(&[("favorite", "Favorites", "[440, 730, 35700]", false)]);
        let games = collections([base.as_str(), modified.as_str()]);

        let ids: Vec<u32> = games.keys().copied().collect();
        assert_eq!(ids, [440, 620, 730, 2500, 35700], "no game of a deleted collection");
        assert!(games[&35700].favorite && games[&440].favorite && !games[&620].favorite);
        assert!(games[&2500].hidden && !games[&730].hidden);
    }
}
