//! Command fields that hold text the player typed, such as a folder: how a command reads them.
//!
//! A theme sends a command as JSON (`Host.command` in the theme contract), with the player's
//! text as it was typed. These read that text the way the launcher always has: trimmed, and
//! a blank folder is the default one.

use std::path::PathBuf;

use serde::{Deserialize, Deserializer};

/// A folder the player may leave blank: trimmed, and `None` when blank or `null`.
pub(crate) fn folder<'de, D: Deserializer<'de>>(de: D) -> Result<Option<PathBuf>, D::Error> {
    let text: Option<String> = Option::deserialize(de)?;
    Ok(text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(PathBuf::from))
}

/// A path the player must give: trimmed, and refused when blank.
pub(crate) fn path<'de, D: Deserializer<'de>>(de: D) -> Result<PathBuf, D::Error> {
    let text = String::deserialize(de)?;
    match text.trim() {
        "" => Err(serde::de::Error::custom("a blank path")),
        trimmed => Ok(PathBuf::from(trimmed)),
    }
}

/// Folders as a list, or as the text of a Settings field: separated by `;`, each trimmed,
/// blank ones left out.
pub(crate) fn folder_list<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<PathBuf>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Folders {
        Text(String),
        List(Vec<PathBuf>),
    }
    Ok(match Folders::deserialize(de)? {
        Folders::Text(text) => split_folders(&text),
        Folders::List(paths) => paths,
    })
}

/// The folders in a Settings text: separated by `;`, each trimmed, empty ones left out.
fn split_folders(text: &str) -> Vec<PathBuf> {
    text.split(';').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct Fields {
        #[serde(default, deserialize_with = "folder")]
        folder: Option<PathBuf>,
        #[serde(default, deserialize_with = "folder_list")]
        folders: Vec<PathBuf>,
    }

    fn read(json: serde_json::Value) -> Fields {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn folders_are_separated_by_semicolons() {
        let paths = split_folders(" /games/a ; /mnt/b c;; ");
        assert_eq!(paths, [PathBuf::from("/games/a"), "/mnt/b c".into()]);
    }

    #[test]
    fn a_folder_is_trimmed_and_a_blank_one_is_none() {
        assert_eq!(read(serde_json::json!({ "folder": " ~/games " })).folder, Some("~/games".into()));
        assert_eq!(read(serde_json::json!({ "folder": "  " })).folder, None);
        assert_eq!(read(serde_json::json!({ "folder": null })).folder, None);
        assert_eq!(read(serde_json::json!({})).folder, None);
    }

    #[test]
    fn folders_come_as_settings_text_or_as_a_list() {
        assert_eq!(read(serde_json::json!({ "folders": "/a; /b" })).folders, [PathBuf::from("/a"), "/b".into()]);
        assert_eq!(read(serde_json::json!({ "folders": ["/a"] })).folders, [PathBuf::from("/a")]);
        assert!(read(serde_json::json!({ "folders": "" })).folders.is_empty(), "the plugins' default");
    }

    #[test]
    fn a_blank_path_is_refused() {
        #[derive(Debug, Deserialize)]
        struct File(#[serde(deserialize_with = "path")] PathBuf);
        assert_eq!(
            serde_json::from_value::<File>(serde_json::json!(" ~/a.conf ")).unwrap().0,
            PathBuf::from("~/a.conf")
        );
        assert!(serde_json::from_value::<File>(serde_json::json!("  ")).is_err());
    }
}
