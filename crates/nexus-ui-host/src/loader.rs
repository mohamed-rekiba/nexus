//! The image loader: images load off the UI thread, a few at a time, from a disk cache when
//! it can, as the old launcher's image pool did. Big art is made smaller once, and the smaller
//! copy kept in `cache/thumbs`, so a warm start decodes small JPEGs instead of 4K PNGs.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::images::{Crop, ImageKey, Prepared, StillWanted, prepare};

/// How many images are fetched at once, and how many decode at once.
const FETCH_AT_ONCE: usize = 8;
const DECODE_AT_ONCE: usize = 4;

pub type Pixels = slint_interpreter::SharedPixelBuffer<slint_interpreter::Rgba8Pixel>;

pub struct ImageLoader {
    thumbs: PathBuf,
    fetching: tokio::sync::Semaphore,
    decoding: tokio::sync::Semaphore,
}

impl ImageLoader {
    /// A loader that keeps its smaller copies in `thumbs`.
    pub fn new(thumbs: PathBuf) -> Self {
        Self {
            thumbs,
            fetching: tokio::sync::Semaphore::new(FETCH_AT_ONCE),
            decoding: tokio::sync::Semaphore::new(DECODE_AT_ONCE),
        }
    }

    /// The smaller copy of an image at a size: `<hash>-card.jpg` or `<hash>-full.jpg`, and
    /// `<hash>-cut-card.jpg` or `<hash>-cut-full.jpg` for one with a crop.
    fn thumb(&self, (image, size, crop): &ImageKey) -> PathBuf {
        let mut hash = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&format!("{image:?}"), &mut hash);
        let cut = if *crop == Crop::None { "" } else { "cut-" };
        self.thumbs.join(format!("{:016x}-{cut}{}.jpg", std::hash::Hasher::finish(&hash), size.as_str()))
    }

    /// Loads `key`, with `fetch` for its bytes when no smaller copy is kept. `None` when the
    /// image left the screen before its turn: nothing is fetched or decoded then.
    pub async fn load<F, Fetched>(
        &self,
        fetch: F,
        key: &ImageKey,
        still: &StillWanted,
    ) -> Option<Result<Pixels, String>>
    where
        F: FnOnce() -> Fetched,
        Fetched: Future<Output = Result<Vec<u8>, String>>,
    {
        if !still.contains(key) {
            return None;
        }
        let thumb = self.thumb(key);
        let (bytes, from_thumb) = match tokio::fs::read(&thumb).await {
            Ok(bytes) => (bytes, true),
            Err(_) => {
                let turn = match self.fetching.acquire().await {
                    Ok(turn) => turn,
                    Err(e) => return Some(Err(e.to_string())),
                };
                // It may have left the screen while it waited.
                if !still.contains(key) {
                    return None;
                }
                let fetched = fetch().await;
                drop(turn);
                match fetched {
                    Ok(bytes) => (bytes, false),
                    Err(e) => return Some(Err(e)),
                }
            }
        };
        let turn = match self.decoding.acquire().await {
            Ok(turn) => turn,
            Err(e) => return Some(Err(e.to_string())),
        };
        if !still.contains(key) {
            return None;
        }
        let size = key.1;
        // The kept copy is cut already.
        let crop = if from_thumb { Crop::None } else { key.2 };
        let prepared = tokio::task::spawn_blocking(move || prepare(&bytes, size, crop)).await;
        drop(turn);
        Some(self.keep(prepared, &thumb, from_thumb).await)
    }

    async fn keep(
        &self,
        prepared: Result<Result<Prepared, String>, tokio::task::JoinError>,
        thumb: &Path,
        from_thumb: bool,
    ) -> Result<Pixels, String> {
        let prepared = prepared.map_err(|e| e.to_string())??;
        if !from_thumb && let Some(smaller) = &prepared.smaller {
            let _ = tokio::fs::create_dir_all(&self.thumbs).await;
            if let Err(e) = tokio::fs::write(thumb, smaller).await {
                tracing::debug!(error = %e, "could not keep a smaller copy");
            }
        }
        Ok(prepared.pixels)
    }
}

/// Deletes the files in the cover cache `dir` whose names [`ImageLoader`] doesn't make.
/// Folders, and everything outside `dir`, stay. Blocking.
pub fn sweep_thumbs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let kept = name.to_str().is_some_and(|name| {
            let hash = name.strip_suffix("-card.jpg").or_else(|| name.strip_suffix("-full.jpg"));
            let hash = hash.map(|h| h.strip_suffix("-cut").unwrap_or(h));
            hash.is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        });
        if !kept && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nexus_ui_api::ImageRef;

    use super::*;
    use crate::images::ImageSize;

    fn key() -> ImageKey {
        let plugin = nexus_api::PluginId::new("store").unwrap();
        (ImageRef::Url { plugin, url: "https://art.example/cover.png".into() }, ImageSize::Thumb, Crop::None)
    }

    /// A 4 x 6 PNG.
    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(4, 6).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn an_image_that_left_the_screen_is_neither_fetched_nor_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let loader = ImageLoader::new(dir.path().join("thumbs"));
        let fetched = Arc::new(AtomicUsize::new(0));
        let fetch = || async {
            fetched.fetch_add(1, Ordering::SeqCst);
            Ok(png())
        };
        let gone = StillWanted::default();
        assert!(loader.load(fetch, &key(), &gone).await.is_none(), "dropped");
        assert_eq!(fetched.load(Ordering::SeqCst), 0, "not fetched");

        let wanted = StillWanted::with([key()]);
        let pixels = loader.load(fetch, &key(), &wanted).await.expect("loaded").unwrap();
        assert_eq!((pixels.width(), pixels.height()), (4, 6));
        assert_eq!(fetched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_fetch_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let loader = ImageLoader::new(dir.path().join("thumbs"));
        let wanted = StillWanted::with([key()]);
        let loaded = loader.load(|| async { Err("404".to_owned()) }, &key(), &wanted).await;
        assert_eq!(loaded.map(|p| p.map(|_| ())), Some(Err("404".to_owned())));
    }

    #[test]
    fn the_cover_cache_keeps_only_the_names_the_loader_makes() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let names = [
            "0123456789abcdef-card.jpg",
            "0123456789abcdef-full.jpg",
            "0123456789abcdef-cut-card.jpg",
            // Thumbnails from before they covered the card: too small.
            "0123456789abcdef-thumb.jpg",
            "0123456789abcdef-cut-thumb.jpg",
            "0123456789abcdef.jpg",
            "0123456789abcdef-cut.jpg",
            "notes.txt",
        ];
        for name in names {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::write(dir.join("folder/0123456789abcdef.jpg"), b"x").unwrap();

        sweep_thumbs(dir);
        let mut left: Vec<_> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(
            left,
            ["0123456789abcdef-card.jpg", "0123456789abcdef-cut-card.jpg", "0123456789abcdef-full.jpg", "folder"]
        );
        assert!(dir.join("folder/0123456789abcdef.jpg").exists(), "a folder is left as it is");
    }
}
