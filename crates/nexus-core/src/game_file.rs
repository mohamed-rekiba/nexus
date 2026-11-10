//! The game file: everything Nexus knows about a game, kept in the game's own folder, so the
//! Library shows the game whole without asking any store again, and the game takes its card
//! with it when its folder moves to another computer.
//!
//! The folder of a game holds `nexus_game.json` in its root and the art in `nexus_art/`:
//!
//! ```text
//! <the game's folder>/
//! ├── nexus_game.json
//! └── nexus_art/
//!     ├── cover.jpg
//!     ├── background.jpg
//!     ├── logo.png
//!     └── screenshot_1.jpg … screenshot_6.jpg
//! ```
//!
//! Every name Nexus writes is lowercase snake_case, and starts with `nexus_` in the game's
//! root. Nexus never renames or deletes the player's own files. The file names its art by paths
//! relative to the game's folder, and only inside `nexus_art/`.
//!
//! The file fits every kind of game. A plugin may keep its own keys in it, such as the PC
//! plugin's `starts` (the files that start the game): the core keeps every key it doesn't own
//! when it writes, and a plugin keeps the core's.

use std::path::Path;

use nexus_api::{GameMetadata, HostPath, ImageSource, Rating, RelPath};
use serde::{Deserialize, Serialize};

/// The file's name, in the game's folder.
pub(crate) const FILE: &str = "nexus_game.json";
/// The folder of the art, in the game's folder.
pub(crate) const ART_DIR: &str = "nexus_art";
/// The file format this build writes and reads. A file of a later format isn't read.
pub(crate) const FORMAT: u32 = 1;
/// The most screenshots a file keeps.
pub(crate) const MAX_SCREENSHOTS: usize = 6;

/// Where the game came from, when Nexus downloaded it: the Catalog's offer and its release.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Source {
    pub offer: String,
    pub release: String,
}

/// The art's paths, relative to the game's folder: `nexus_art/cover.jpg`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Art {
    pub cover: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub screenshots: Vec<String>,
}

/// One piece of art, by what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtKind {
    Cover,
    Background,
    Logo,
    /// The screenshot at this place, from 1.
    Screenshot(usize),
}

impl ArtKind {
    fn stem(self) -> String {
        match self {
            ArtKind::Cover => "cover".into(),
            ArtKind::Background => "background".into(),
            ArtKind::Logo => "logo".into(),
            ArtKind::Screenshot(n) => format!("screenshot_{n}"),
        }
    }
}

/// `nexus_game.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GameFile {
    pub format: u32,
    /// The Nexus that wrote it: "Nexus 0.14.0".
    pub written_by: String,
    pub title: String,
    /// The game's console, as its plugin names it: "PS5", "Switch".
    pub platform: String,
    pub title_id: Option<String>,
    pub description: String,
    pub tagline: String,
    pub publisher: String,
    pub genres: Vec<String>,
    /// `YYYY-MM-DD`.
    pub release: String,
    pub rating: Option<Rating>,
    pub age_rating: String,
    pub store_url: String,
    pub trailer: String,
    pub source: Option<Source>,
    pub art: Art,
}

impl GameFile {
    /// The file for a game: its title, console and title id, what the stores and the catalogs
    /// say about it (`about`), where it came from, and the art the folder will hold.
    pub fn new(
        title: &str,
        platform: &str,
        title_id: Option<&str>,
        about: &GameMetadata,
        source: Option<Source>,
        art: Art,
    ) -> Self {
        Self {
            format: FORMAT,
            written_by: format!("Nexus {}", env!("CARGO_PKG_VERSION")),
            title: title.to_owned(),
            platform: platform.to_owned(),
            title_id: title_id.filter(|t| !t.is_empty()).map(str::to_owned),
            description: about.description.clone(),
            tagline: about.tagline.clone(),
            publisher: about.publisher.clone(),
            genres: about.genres.clone(),
            release: about.release.clone(),
            rating: about.rating,
            age_rating: about.age_rating.clone(),
            store_url: about.store_url.clone(),
            trailer: about.trailer.clone(),
            source,
            art,
        }
    }

    /// What the file says, as a metadata layer. Its art is in the game's folder, `folder`.
    pub fn metadata(&self, folder: &HostPath) -> GameMetadata {
        let image = |path: &String| -> Option<ImageSource> {
            let rel = RelPath::new(format!("{}/{path}", folder.path.as_str())).ok()?;
            Some(ImageSource::Folder(HostPath::new(folder.folder.clone(), folder.root, rel)))
        };
        GameMetadata {
            title: self.title.clone(),
            description: self.description.clone(),
            tagline: self.tagline.clone(),
            publisher: self.publisher.clone(),
            genres: self.genres.clone(),
            release: self.release.clone(),
            rating: self.rating,
            age_rating: self.age_rating.clone(),
            icon: None,
            cover: self.art.cover.as_ref().and_then(image),
            background: self.art.background.as_ref().and_then(image),
            logo: self.art.logo.as_ref().and_then(image),
            screenshots: self.art.screenshots.iter().filter_map(image).collect(),
            store_url: self.store_url.clone(),
            trailer: self.trailer.clone(),
        }
    }

    /// The core wrote its part: the file says more than a plugin's title, if only where the
    /// game came from. A file with only a plugin's part (the PC plugin's `title` and `starts`)
    /// gets the core's.
    pub fn has_details(&self) -> bool {
        let art = &self.art;
        !(self.source.is_none()
            && self.description.is_empty()
            && self.tagline.is_empty()
            && self.publisher.is_empty()
            && self.genres.is_empty()
            && self.release.is_empty()
            && self.rating.is_none()
            && self.age_rating.is_empty()
            && self.store_url.is_empty()
            && self.trailer.is_empty()
            && art.cover.is_none()
            && art.background.is_none()
            && art.logo.is_none()
            && art.screenshots.is_empty())
    }

    /// Drops any art path that isn't a plain file in `nexus_art/`, such as `../x` or a folder
    /// beyond it: the file names only its own art.
    fn keep_own_art(mut self) -> Self {
        let own = |p: &String| is_art_path(p);
        self.art.cover = self.art.cover.filter(own);
        self.art.background = self.art.background.filter(own);
        self.art.logo = self.art.logo.filter(own);
        self.art.screenshots.retain(own);
        self.art.screenshots.truncate(MAX_SCREENSHOTS);
        self
    }
}

/// The art's path for `kind`, from its bytes: `nexus_art/cover.jpg`. `None` when the bytes are
/// no picture this build knows (JPEG, PNG, WebP or GIF).
pub(crate) fn art_path(kind: ArtKind, bytes: &[u8]) -> Option<String> {
    let extension = picture_extension(bytes)?;
    Some(format!("{ART_DIR}/{}.{extension}", kind.stem()))
}

/// The extension of a picture, from its first bytes.
fn picture_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else {
        None
    }
}

/// `nexus_art/<name>`: lowercase letters, digits and `_`, then a picture's extension.
fn is_art_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix(&format!("{ART_DIR}/")) else { return false };
    let Some((stem, extension)) = name.rsplit_once('.') else { return false };
    !stem.is_empty()
        && stem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && matches!(extension, "jpg" | "png" | "webp" | "gif")
}

/// The game file in the folder `dir`. `None` when there is none, it doesn't read, or it is of a
/// later format.
pub(crate) fn read(dir: &Path) -> Option<GameFile> {
    let text = std::fs::read(dir.join(FILE)).ok()?;
    let file: GameFile = match serde_json::from_slice(&text) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!(error = %e, dir = %dir.display(), "a game file doesn't read");
            return None;
        }
    };
    (file.format >= 1 && file.format <= FORMAT).then(|| file.keep_own_art())
}

/// Writes the game file and its art into the folder `dir`. Each file goes through a temporary
/// file, and the game file goes last: a game file on disk names only art that is there. The
/// keys of the file on disk that the core doesn't own, such as a plugin's `starts`, stay.
/// `art`: each piece's path, as [`art_path`] gives it, and its bytes.
pub(crate) fn write(dir: &Path, file: &GameFile, art: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    if !art.is_empty() {
        std::fs::create_dir_all(dir.join(ART_DIR))?;
    }
    for (path, bytes) in art {
        if !is_art_path(path) {
            return Err(std::io::Error::other(format!("{path} is not a path in {ART_DIR}/")));
        }
        replace(&dir.join(path), bytes)?;
    }
    let mut merged: serde_json::Map<String, serde_json::Value> = std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let ours = serde_json::to_value(file).map_err(std::io::Error::other)?;
    if let serde_json::Value::Object(ours) = ours {
        merged.extend(ours);
    }
    let mut text = serde_json::to_vec_pretty(&serde_json::Value::Object(merged)).map_err(std::io::Error::other)?;
    text.push(b'\n');
    replace(&dir.join(FILE), &text)
}

/// Writes `bytes` to `path` through a temporary file beside it.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = std::path::PathBuf::from(temporary);
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2];
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn about() -> GameMetadata {
        GameMetadata {
            title: "Store Title".into(),
            description: "A football game.".into(),
            publisher: "Nintendo".into(),
            genres: vec!["Sports".into()],
            release: "2022-06-10".into(),
            rating: Some(Rating { score: 4.5, total: 120 }),
            age_rating: "PEGI 3".into(),
            ..GameMetadata::default()
        }
    }

    fn folder() -> HostPath {
        HostPath::new("games", 0, RelPath::new("SWITCH/Mario Strikers").unwrap())
    }

    #[test]
    fn art_is_named_by_what_it_is_and_by_its_bytes() {
        assert_eq!(art_path(ArtKind::Cover, JPG).as_deref(), Some("nexus_art/cover.jpg"));
        assert_eq!(art_path(ArtKind::Logo, PNG).as_deref(), Some("nexus_art/logo.png"));
        assert_eq!(art_path(ArtKind::Screenshot(3), JPG).as_deref(), Some("nexus_art/screenshot_3.jpg"));
        assert_eq!(
            art_path(ArtKind::Background, b"RIFF\0\0\0\0WEBPVP8 ").as_deref(),
            Some("nexus_art/background.webp")
        );
        assert_eq!(art_path(ArtKind::Cover, b"<html>").as_deref(), None, "a page is no picture");
    }

    #[test]
    fn a_written_file_reads_back_with_its_art() {
        let dir = tempfile::tempdir().unwrap();
        let art = Art {
            cover: Some("nexus_art/cover.jpg".into()),
            logo: Some("nexus_art/logo.png".into()),
            ..Art::default()
        };
        let source = Source { offer: "release-catalog:switch/1".into(), release: "6223729".into() };
        let file = GameFile::new("Mario Strikers", "Switch", Some("010019401051C000"), &about(), Some(source), art);

        write(
            dir.path(),
            &file,
            &[("nexus_art/cover.jpg".into(), JPG.to_vec()), ("nexus_art/logo.png".into(), PNG.to_vec())],
        )
        .unwrap();

        assert_eq!(read(dir.path()), Some(file));
        assert_eq!(std::fs::read(dir.path().join("nexus_art/cover.jpg")).unwrap(), JPG);
        let names: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(names.iter().all(|n| n.starts_with("nexus_")), "no temporary file is left: {names:?}");
    }

    #[test]
    fn its_metadata_points_into_the_game_s_folder() {
        let art = Art {
            cover: Some("nexus_art/cover.jpg".into()),
            screenshots: vec!["nexus_art/screenshot_1.jpg".into()],
            ..Art::default()
        };
        let file = GameFile::new("Mario Strikers", "Switch", None, &about(), None, art);

        let meta = file.metadata(&folder());

        let at = |path: &str| ImageSource::Folder(HostPath::new("games", 0, RelPath::new(path).unwrap()));
        assert_eq!(meta.cover, Some(at("SWITCH/Mario Strikers/nexus_art/cover.jpg")));
        assert_eq!(meta.screenshots, [at("SWITCH/Mario Strikers/nexus_art/screenshot_1.jpg")]);
        assert_eq!(meta.background, None);
        assert_eq!(meta.title, "Mario Strikers", "the game's own title, not the store's");
        assert_eq!(meta.publisher, "Nintendo");
        assert_eq!(meta.rating, Some(Rating { score: 4.5, total: 120 }));
    }

    #[test]
    fn a_file_names_only_art_in_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let text = serde_json::json!({
            "format": 1,
            "title": "A",
            "art": {
                "cover": "../../etc/passwd.jpg",
                "background": "nexus_art/Back Ground.jpg",
                "logo": "nexus_art/logo.png",
                "screenshots": ["nexus_art/sub/screenshot_1.jpg", "nexus_art/screenshot_2.jpg"]
            }
        });
        std::fs::write(dir.path().join(FILE), text.to_string()).unwrap();

        let file = read(dir.path()).unwrap();

        assert_eq!(file.art.cover, None, "outside its folder");
        assert_eq!(file.art.background, None, "not snake_case");
        assert_eq!(file.art.logo.as_deref(), Some("nexus_art/logo.png"));
        assert_eq!(file.art.screenshots, ["nexus_art/screenshot_2.jpg"]);
    }

    #[test]
    fn a_file_of_a_later_format_or_that_does_not_read_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None, "no file");
        std::fs::write(dir.path().join(FILE), r#"{"format": 2, "title": "A"}"#).unwrap();
        assert_eq!(read(dir.path()), None, "a later format");
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert_eq!(read(dir.path()), None, "broken");
        std::fs::write(dir.path().join(FILE), r#"{"format": 1, "title": "A", "new_field": true}"#).unwrap();
        assert_eq!(read(dir.path()).map(|f| f.title).as_deref(), Some("A"), "a field it doesn't know is left out");
    }

    #[test]
    fn writing_keeps_the_keys_of_the_rest_of_nexus() {
        let dir = tempfile::tempdir().unwrap();
        let pc = r#"{"format": 1, "title": "Cuphead", "starts": [{"kind": "native", "path": "Cuphead.x86_64"}]}"#;
        std::fs::write(dir.path().join(FILE), pc).unwrap();
        assert!(!read(dir.path()).unwrap().has_details(), "the PC plugin's part alone");

        let file = GameFile::new("Cuphead", "PC", None, &about(), None, Art::default());
        write(dir.path(), &file, &[]).unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(FILE)).unwrap()).unwrap();
        assert_eq!(written["starts"], serde_json::json!([{"kind": "native", "path": "Cuphead.x86_64"}]));
        assert_eq!(written["publisher"], "Nintendo");
        assert!(read(dir.path()).unwrap().has_details());
    }

    #[test]
    fn writing_refuses_art_outside_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = GameFile::new("A", "PS5", None, &GameMetadata::default(), None, Art::default());
        assert!(write(dir.path(), &file, &[("../cover.jpg".into(), JPG.to_vec())]).is_err());
        assert!(!dir.path().join(FILE).exists(), "nothing is written");
    }
}
