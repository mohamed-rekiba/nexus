use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use image::GenericImageView;
use nexus_ui_api::ImageRef;
use slint_interpreter::{Image, Rgba8Pixel, SharedPixelBuffer};

/// How big an image is made, by where it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageSize {
    /// Cards, tiles and badges: enough to cover [`THUMB_WIDTH`] by [`THUMB_HEIGHT`], a 2:3 cover
    /// for a card on a 2x screen (see [`scale`]).
    Thumb,
    /// The Game Hub's cover, backgrounds, logos and screenshots: at most what a 1080p screen
    /// shows.
    Full,
}

pub const THUMB_WIDTH: u32 = 512;
pub const THUMB_HEIGHT: u32 = 768;
pub const MAX_WIDTH: u32 = 1920;
pub const MAX_HEIGHT: u32 = 1080;

impl ImageSize {
    pub fn limits(self) -> (u32, u32) {
        match self {
            Self::Thumb => (THUMB_WIDTH, THUMB_HEIGHT),
            Self::Full => (MAX_WIDTH, MAX_HEIGHT),
        }
    }

    /// For file names: `card` or `full`. (Before thumbnails covered the card they were
    /// `thumb`: those copies are too small, and the cache sweep deletes them.)
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thumb => "card",
            Self::Full => "full",
        }
    }

    /// The most kept in memory; the ones used longest ago go first.
    fn kept(self) -> usize {
        match self {
            Self::Thumb => 90,
            Self::Full => 12,
        }
    }
}

/// What is cut off an image before it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Crop {
    None,
    /// The blue "PS4" band across the top of PS4 box art, when the art has one, as the old
    /// launcher cut it.
    Ps4Band,
}

/// An image at a size, with its cut: what the cache keeps and loads.
pub type ImageKey = (ImageRef, ImageSize, Crop);

/// An image made ready off the UI thread: its pixels, and when it had to be made smaller, the
/// smaller copy as a JPEG to keep, so the next start decodes that instead.
pub struct Prepared {
    pub pixels: SharedPixelBuffer<Rgba8Pixel>,
    pub smaller: Option<Vec<u8>>,
}

/// How much `size` scales an image of `width` × `height`, at most 1. A full image fits inside a
/// 1080p screen. A thumbnail covers the card's 512 × 768: cards crop art of any shape to fill
/// them, so a wide picture is made as tall as the card, else the card would stretch a short
/// copy. Its long side stays at most a screen's width.
fn scale(size: ImageSize, width: u32, height: u32) -> f64 {
    let (w, h) = (f64::from(width), f64::from(height));
    let (max_w, max_h) = size.limits();
    let scale = match size {
        ImageSize::Full => f64::min(f64::from(max_w) / w, f64::from(max_h) / h),
        ImageSize::Thumb => f64::max(f64::from(max_w) / w, f64::from(max_h) / h).min(f64::from(MAX_WIDTH) / w.max(h)),
    };
    scale.min(1.0)
}

/// Decodes PNG or JPEG, makes it no bigger than `size` needs ([`scale`]) and cuts `crop` off. Slow
/// for big art: call it on a worker thread, as the old launcher's image pool did. The copy
/// to keep is already cut: load it again with [`Crop::None`].
pub fn prepare(bytes: &[u8], size: ImageSize, crop: Crop) -> Result<Prepared, String> {
    let mut decoded = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (width, height) = (decoded.width(), decoded.height());
    let scale = scale(size, width, height);
    let mut changed = false;
    if scale < 1.0 {
        let (w, h) = ((f64::from(width) * scale).round() as u32, (f64::from(height) * scale).round() as u32);
        decoded = decoded.resize_exact(w.max(1), h.max(1), image::imageops::FilterType::Triangle);
        changed = true;
    }
    if crop == Crop::Ps4Band {
        let (w, h) = decoded.dimensions();
        let top = (ps4_band(&decoded) * h as f32).round() as u32;
        if top > 0 {
            decoded = decoded.crop_imm(0, top, w, h - top);
            changed = true;
        }
    }
    let mut smaller = None;
    // Art without transparency keeps well as a JPEG.
    if changed && !decoded.color().has_alpha() {
        let mut out = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        if decoded.to_rgb8().write_with_encoder(encoder).is_ok() {
            smaller = Some(out);
        }
    }
    let rgba = decoded.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(Prepared { pixels: SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), w, h), smaller })
}

/// Decodes on this thread: for tests, and small images.
pub fn decode(bytes: &[u8], size: ImageSize, crop: Crop) -> Result<Image, String> {
    prepare(bytes, size, crop).map(|p| Image::from_rgba8(p.pixels))
}

/// How much of the art's height, from the top, the blue "PS4" band of PS4 box art takes, and
/// a little more; 0 when there is none. The band is rows of mostly PlayStation blue with the
/// white "PS4" lettering, which end within the top fifth.
pub fn ps4_band(art: &impl GenericImageView<Pixel = image::Rgba<u8>>) -> f32 {
    let (w, h) = art.dimensions();
    // Only portrait box art has the band.
    if w < 32 || h < w {
        return 0.0;
    }
    let mut rows = 0;
    for y in 0..h / 5 {
        let (mut blue, mut white, mut n) = (0u32, 0u32, 0u32);
        for x in (0..w).step_by(2) {
            let [r, g, b, _] = art.get_pixel(x, y).0.map(i32::from);
            if b > r + 60 && b > 100 {
                blue += 1;
            } else if r > 200 && g > 200 && b > 200 {
                white += 1;
            }
            n += 1;
        }
        if blue * 2 >= n && (blue + white) * 100 >= n * 85 {
            rows = y + 1;
        } else if rows > 0 || y as f32 > h as f32 * 0.02 {
            // The band ended, or there is none at the very top.
            break;
        }
    }
    let band = rows as f32 / h as f32;
    if (0.04..=0.16).contains(&band) { band + 0.004 } else { 0.0 }
}

/// The images the last render showed and doesn't have: a load still in this set when its
/// turn comes goes on, and one that left it is dropped. Shared with the loader's threads.
#[derive(Clone, Default)]
pub struct StillWanted(Arc<Mutex<HashSet<ImageKey>>>);

impl StillWanted {
    /// A set of `keys`, as a render would leave it.
    #[cfg(test)]
    pub fn with(keys: impl IntoIterator<Item = ImageKey>) -> Self {
        Self(Arc::new(Mutex::new(keys.into_iter().collect())))
    }

    pub fn contains(&self, key: &ImageKey) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).contains(key)
    }
}

/// A grid's cards on screen, as the theme says: `first` to `last`, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub first: usize,
    pub last: usize,
}

impl Window {
    /// Before the theme says: the first screens of a grid.
    pub const START: Window = Window { first: 0, last: 29 };

    /// On screen, or within one screen of it.
    pub fn near(&self, index: usize) -> bool {
        let screen = self.last.saturating_sub(self.first) + 1;
        index + screen >= self.first && index <= self.last + screen
    }
}

/// Images the view models refer to. A missing one shows as empty and is queued for loading,
/// when it is wanted: a card far from the screen asks for nothing.
#[derive(Default)]
pub struct ImageCache {
    loaded: HashMap<ImageKey, Image>,
    failed: HashSet<ImageKey>,
    /// Asked for by the render going on, in the order asked: the first are on screen.
    wanted: RefCell<Vec<ImageKey>>,
    /// Asked for, and not in yet.
    pending: HashSet<ImageKey>,
    /// When each loaded image was last shown, by render.
    used: RefCell<HashMap<ImageKey, u64>>,
    frame: Cell<u64>,
    still: StillWanted,
    /// Each grid's cards on screen, by the grid's name (`library`, `catalog`). Shared with
    /// the theme's `Host.grid-window` callback.
    windows: Windows,
}

/// Each grid's cards on screen, by the grid's name.
pub type Windows = std::rc::Rc<RefCell<HashMap<String, Window>>>;

impl ImageCache {
    /// Starts a render: what it asks for replaces what the last one asked for.
    pub fn begin_render(&self) {
        self.frame.set(self.frame.get() + 1);
        self.wanted.borrow_mut().clear();
    }

    /// Ends a render: lets go of the images used longest ago, past what is kept.
    pub fn end_render(&mut self) {
        let wanted: HashSet<ImageKey> = self.wanted.borrow().iter().cloned().collect();
        *self.still.0.lock().unwrap_or_else(|p| p.into_inner()) = wanted;
        let frame = self.frame.get();
        for size in [ImageSize::Thumb, ImageSize::Full] {
            let mut of_size: Vec<(u64, ImageKey)> = self
                .loaded
                .keys()
                .filter(|(_, s, _)| *s == size)
                .map(|k| (self.used.borrow().get(k).copied().unwrap_or(0), k.clone()))
                .collect();
            if of_size.len() <= size.kept() {
                continue;
            }
            of_size.sort_by_key(|(at, _)| *at);
            let over = of_size.len() - size.kept();
            // Never what this render shows.
            for (_, key) in of_size.into_iter().take(over).filter(|(at, _)| *at < frame) {
                self.loaded.remove(&key);
                self.used.borrow_mut().remove(&key);
            }
        }
    }

    /// The set the loader checks before it loads.
    pub fn still_wanted(&self) -> StillWanted {
        self.still.clone()
    }

    /// Where the theme's reports of the grids' windows go.
    pub fn windows(&self) -> Windows {
        self.windows.clone()
    }

    /// The theme's report of a grid's cards on screen.
    #[cfg(test)]
    pub fn set_window(&self, grid: &str, window: Window) {
        self.windows.borrow_mut().insert(grid.to_owned(), window);
    }

    /// Whether card `index` of `grid` is near the screen.
    pub fn near(&self, grid: &str, index: usize) -> bool {
        self.windows.borrow().get(grid).copied().unwrap_or(Window::START).near(index)
    }

    /// Images to load, once each, on-screen ones first.
    pub fn take_wanted(&mut self) -> Vec<ImageKey> {
        let wanted: Vec<ImageKey> = self.wanted.borrow().clone();
        let mut seen = HashSet::new();
        let wanted: Vec<ImageKey> = wanted
            .into_iter()
            .filter(|k| !self.loaded.contains_key(k) && !self.failed.contains(k) && !self.pending.contains(k))
            .filter(|k| seen.insert(k.clone()))
            .collect();
        self.pending.extend(wanted.iter().cloned());
        wanted
    }

    /// Every image the screen asked for is in (or failed).
    pub fn settled(&self) -> bool {
        self.pending.is_empty()
            && self.wanted.borrow().iter().all(|k| self.loaded.contains_key(k) || self.failed.contains(k))
    }

    /// Pixels made ready on a worker thread. `None`: the load was dropped because the image
    /// left the screen first; it may be asked for again.
    pub fn insert_pixels(&mut self, key: ImageKey, pixels: Option<Result<SharedPixelBuffer<Rgba8Pixel>, String>>) {
        self.pending.remove(&key);
        match pixels {
            None => {}
            Some(Ok(pixels)) => {
                self.loaded.insert(key, Image::from_rgba8(pixels));
            }
            Some(Err(error)) => {
                tracing::warn!(?key, %error, "image not loaded");
                self.failed.insert(key);
            }
        }
    }

    /// An image's bytes, decoded on this thread: for tests.
    #[cfg(test)]
    pub fn insert(&mut self, key: ImageKey, bytes: Result<Vec<u8>, String>) {
        let (size, crop) = (key.1, key.2);
        self.pending.remove(&key);
        match bytes.and_then(|b| decode(&b, size, crop)) {
            Ok(decoded) => {
                self.loaded.insert(key, decoded);
            }
            Err(error) => {
                tracing::warn!(?key, %error, "image not loaded");
                self.failed.insert(key);
            }
        }
    }

    /// The images asked for that it still keeps track of: in memory, failed or loading.
    pub fn asked(&self) -> Vec<ImageKey> {
        self.loaded.keys().chain(&self.failed).chain(&self.pending).cloned().collect()
    }

    /// How many images are in memory.
    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }
}

impl crate::bind::ImageSource for ImageCache {
    fn image(&self, image: &ImageRef, size: ImageSize, crop: Crop, want: bool) -> Image {
        let key = (image.clone(), size, crop);
        match self.loaded.get(&key) {
            Some(i) => {
                self.used.borrow_mut().insert(key, self.frame.get());
                i.clone()
            }
            None => {
                if want && !self.failed.contains(&key) {
                    self.wanted.borrow_mut().push(key);
                }
                Image::default()
            }
        }
    }

    fn failed(&self, image: &ImageRef, size: ImageSize, crop: Crop) -> bool {
        self.failed.contains(&(image.clone(), size, crop))
    }

    fn near(&self, grid: &str, index: usize) -> bool {
        ImageCache::near(self, grid, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::ImageSource;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn art(n: usize) -> ImageRef {
        ImageRef::Url { plugin: "store".parse().unwrap(), url: format!("https://art.example/{n}.png") }
    }

    #[test]
    fn big_art_is_made_screen_sized_once_and_kept_as_a_jpeg() {
        let big = png(3840, 2160);
        let started = std::time::Instant::now();
        let prepared = prepare(&big, ImageSize::Full, Crop::None).unwrap();
        let first = started.elapsed();
        assert_eq!((prepared.pixels.width(), prepared.pixels.height()), (1920, 1080));
        let smaller = prepared.smaller.expect("a smaller copy to keep");

        let started = std::time::Instant::now();
        let again = prepare(&smaller, ImageSize::Full, Crop::None).unwrap();
        let warm = started.elapsed();
        assert_eq!((again.pixels.width(), again.pixels.height()), (1920, 1080));
        assert!(again.smaller.is_none(), "the kept copy is already small");
        eprintln!("4K PNG: {first:?}; the kept 1080p JPEG: {warm:?}");
    }

    #[test]
    fn a_card_s_cover_is_made_card_sized() {
        let cover = png(1200, 1800);
        let started = std::time::Instant::now();
        let thumb = prepare(&cover, ImageSize::Thumb, Crop::None).unwrap();
        let cold = started.elapsed();
        assert_eq!((thumb.pixels.width(), thumb.pixels.height()), (512, 768));
        let started = std::time::Instant::now();
        prepare(&thumb.smaller.expect("a card-sized copy to keep"), ImageSize::Thumb, Crop::None).unwrap();
        eprintln!("1200x1800 PNG to a card: {cold:?}; the kept card-sized JPEG: {:?}", started.elapsed());
    }

    /// A wide picture as a card's cover: the card crops its sides, so it is made as tall as
    /// the card, not as wide, and stays sharp.
    #[test]
    fn a_wide_cover_is_made_as_tall_as_the_card() {
        let wide = prepare(&png(3840, 2160), ImageSize::Thumb, Crop::None).unwrap();
        assert_eq!((wide.pixels.width(), wide.pixels.height()), (1365, 768));
        let kept = prepare(&wide.smaller.expect("a card-sized copy to keep"), ImageSize::Thumb, Crop::None).unwrap();
        assert_eq!((kept.pixels.width(), kept.pixels.height()), (1365, 768), "the kept copy loads as it is");
        assert!(kept.smaller.is_none());
        // A panorama stays at most a screen wide.
        let panorama = prepare(&png(8000, 1000), ImageSize::Thumb, Crop::None).unwrap();
        assert_eq!((panorama.pixels.width(), panorama.pixels.height()), (1920, 240));
    }

    /// 100 × 160 box art: 16 rows of PlayStation blue with a white "PS4", then red artwork.
    fn ps4_box_art() -> image::RgbaImage {
        let mut art = image::RgbaImage::from_pixel(100, 160, image::Rgba([180, 60, 40, 255]));
        for y in 0..16 {
            for x in 0..100 {
                let logo = (10..30).contains(&x) && (5..11).contains(&y);
                art.put_pixel(x, y, image::Rgba(if logo { [255, 255, 255, 255] } else { [10, 80, 190, 255] }));
            }
        }
        art
    }

    #[test]
    fn the_blue_band_is_found_only_on_ps4_box_art() {
        assert!((ps4_band(&ps4_box_art()) - (16.0 / 160.0 + 0.004)).abs() < 1e-4);
        // A blue sky over the whole top is not a band: it doesn't end in time.
        assert_eq!(ps4_band(&image::RgbaImage::from_pixel(100, 160, image::Rgba([20, 90, 200, 255]))), 0.0);
        // Square store art and wide logos stay as they are.
        assert_eq!(ps4_band(&image::RgbaImage::from_pixel(100, 100, image::Rgba([10, 80, 190, 255]))), 0.0);
        assert_eq!(ps4_band(&image::RgbaImage::from_pixel(160, 100, image::Rgba([10, 80, 190, 255]))), 0.0);
    }

    #[test]
    fn a_ps4_cover_loses_its_band_and_keeps_the_cut_copy() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(ps4_box_art()).to_rgb8().write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();

        let cut = prepare(&bytes, ImageSize::Thumb, Crop::Ps4Band).unwrap();
        assert_eq!((cut.pixels.width(), cut.pixels.height()), (100, 143), "16 rows and a little more go");
        let top = &cut.pixels.as_slice()[0];
        assert_eq!((top.r, top.g, top.b), (180, 60, 40), "the artwork starts at the top");
        assert!(cut.smaller.is_some(), "the cut copy is kept, so the next start doesn't cut again");

        let whole = prepare(&bytes, ImageSize::Thumb, Crop::None).unwrap();
        assert_eq!(whole.pixels.height(), 160, "other covers stay whole");
    }

    #[test]
    fn small_art_stays_as_it_is() {
        let prepared = prepare(&png(512, 512), ImageSize::Full, Crop::None).unwrap();
        assert_eq!((prepared.pixels.width(), prepared.pixels.height()), (512, 512));
        assert!(prepared.smaller.is_none());
    }

    #[test]
    fn only_cards_near_the_screen_are_asked_for_on_screen_first() {
        let mut cache = ImageCache::default();
        cache.set_window("catalog", Window { first: 40, last: 49 });
        cache.begin_render();
        for i in 0..100 {
            cache.image(&art(i), ImageSize::Thumb, Crop::None, cache.near("catalog", i));
        }
        cache.end_render();
        let wanted: Vec<ImageKey> = cache.take_wanted();
        assert_eq!(wanted.len(), 30, "the 10 on screen and one screen each side");
        assert_eq!(wanted[0].0, art(30));
        assert!(cache.still_wanted().contains(&(art(45), ImageSize::Thumb, Crop::None)));
        assert!(!cache.still_wanted().contains(&(art(5), ImageSize::Thumb, Crop::None)));

        // The player scrolls away: the next render wants other cards; the old ones are stale.
        cache.set_window("catalog", Window { first: 90, last: 99 });
        cache.begin_render();
        for i in 0..100 {
            cache.image(&art(i), ImageSize::Thumb, Crop::None, cache.near("catalog", i));
        }
        cache.end_render();
        assert!(!cache.still_wanted().contains(&(art(45), ImageSize::Thumb, Crop::None)), "left the screen");
        // A dropped load can be asked for again.
        cache.insert_pixels((art(45), ImageSize::Thumb, Crop::None), None);
        cache.set_window("catalog", Window { first: 40, last: 49 });
        cache.begin_render();
        cache.image(&art(45), ImageSize::Thumb, Crop::None, cache.near("catalog", 45));
        cache.end_render();
        assert!(cache.take_wanted().contains(&(art(45), ImageSize::Thumb, Crop::None)));
    }

    #[test]
    fn the_images_used_longest_ago_go_past_what_is_kept() {
        let mut cache = ImageCache::default();
        let pixels = || Some(Ok(SharedPixelBuffer::<Rgba8Pixel>::new(4, 6)));
        let kept = ImageSize::Thumb.kept();
        for i in 0..kept + 10 {
            cache.begin_render();
            cache.image(&art(i), ImageSize::Thumb, Crop::None, true);
            cache.end_render();
            cache.take_wanted();
            cache.insert_pixels((art(i), ImageSize::Thumb, Crop::None), pixels());
            // Shown once more, so it counts as used at this render.
            cache.begin_render();
            cache.image(&art(i), ImageSize::Thumb, Crop::None, true);
            cache.end_render();
        }
        assert_eq!(cache.loaded_count(), kept);
        cache.begin_render();
        assert_eq!(cache.image(&art(0), ImageSize::Thumb, Crop::None, false).size().width, 0, "the oldest went");
        assert_eq!(
            cache.image(&art(kept + 9), ImageSize::Thumb, Crop::None, false).size().width,
            4,
            "the newest stayed"
        );
    }
}
