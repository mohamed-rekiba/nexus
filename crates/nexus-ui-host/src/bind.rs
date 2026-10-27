//! Turning view models, tokens and images into Slint values on the theme's globals.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use nexus_ui_api::*;
use slint::{Model, ModelRc, VecModel};
use slint_interpreter::{Brush, Color, ComponentInstance, Image, SetPropertyError, SharedString, Value};

use crate::values::Values;

/// Images the view refers to. Unknown ones show as empty until loaded.
pub trait ImageSource {
    /// The image at `size`, with `crop` cut off, if it is in, else an empty one; queued for
    /// loading when `want`.
    fn image(&self, image: &ImageRef, size: crate::ImageSize, crop: crate::Crop, want: bool) -> Image;
    /// Whether the image at `size`, with `crop` cut off, couldn't load.
    fn failed(&self, image: &ImageRef, size: crate::ImageSize, crop: crate::Crop) -> bool;
    /// Whether card `index` of the grid named `grid` is on screen or near it.
    fn near(&self, grid: &str, index: usize) -> bool;
}

fn text(s: impl AsRef<str>) -> Value {
    Value::String(SharedString::from(s.as_ref()))
}

fn number(n: impl Into<f64>) -> Value {
    Value::Number(n.into())
}

/// The crop of a game's cover: a PS4 cover loses the blue "PS4" band of its box art.
pub(crate) fn cover_crop(platform: &str) -> crate::Crop {
    if platform == "PS4" { crate::Crop::Ps4Band } else { crate::Crop::None }
}

/// A card's cover with `crop`; loaded only when `want`.
fn cover(images: &dyn ImageSource, r: &Option<ImageRef>, crop: crate::Crop, want: bool) -> Value {
    Value::Image(r.as_ref().map(|r| images.image(r, crate::ImageSize::Thumb, crop, want)).unwrap_or_default())
}

/// `GameCard.cover-state` for the cover `shown`: "loading", "ready" or "none".
pub(crate) fn cover_state(images: &dyn ImageSource, r: &Option<ImageRef>, crop: crate::Crop, shown: &Value) -> Value {
    let state = match r {
        None => "none",
        Some(_) if matches!(shown, Value::Image(i) if i.size().width > 0) => "ready",
        Some(r) if images.failed(r, crate::ImageSize::Thumb, crop) => "none",
        Some(_) => "loading",
    };
    text(state)
}

/// The long lists of cards (the Catalog's, the Library's, Home's), each as last shown: the card
/// data and its cover's state, and the list the theme has. A render converts only the cards
/// that changed, and changes only their rows: the rest, often thousands, stay as they are.
/// Each card still asks for its cover, so the covers it shows stay in and load.
#[derive(Default)]
pub struct CardModels(RefCell<HashMap<String, CardList>>);

#[derive(Default)]
struct CardList {
    shown: Vec<(GameCardVm, bool, Value)>,
    model: Rc<VecModel<Value>>,
}

impl CardModels {
    /// A new theme: its lists start empty.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// The list `name` of `view`: `home` (Home's games), `library` or `catalog` (the grids').
    /// Home loads every cover; a grid only those of the cards near the screen.
    pub(crate) fn list(&self, name: &str, images: &dyn ImageSource, view: &AppView) -> Value {
        let (games, every): (&[GameCardVm], bool) = match name {
            "home" => (&view.home.games, true),
            "library" => (&view.home.library.games, false),
            "catalog" => (&view.catalog.games.games, false),
            _ => {
                tracing::error!(list = name, "no such list of cards");
                return Value::Model(ModelRc::default());
            }
        };
        let mut lists = self.0.borrow_mut();
        let list = lists.entry(name.to_owned()).or_default();
        for (i, g) in games.iter().enumerate() {
            let want = every || images.near(name, i);
            let crop = cover_crop(&g.platform);
            let shown = cover(images, &g.cover, crop, want);
            let loaded = matches!(&shown, Value::Image(image) if image.size().width > 0);
            let state = cover_state(images, &g.cover, crop, &shown);
            if list.shown.get(i).is_some_and(|(old, was, old_state)| old == g && *was == loaded && *old_state == state)
            {
                continue;
            }
            let row = Values { images, cards: None, want_covers: want }.of(g).unwrap_or_else(|e| {
                tracing::error!(card = %g.key, error = %e, "a card has no Slint form");
                Value::Void
            });
            if i < list.model.row_count() {
                list.model.set_row_data(i, row);
                list.shown[i] = (g.clone(), loaded, state);
            } else {
                list.model.push(row);
                list.shown.push((g.clone(), loaded, state));
            }
        }
        while list.model.row_count() > games.len() {
            list.model.remove(list.model.row_count() - 1);
        }
        list.shown.truncate(games.len());
        Value::Model(ModelRc::from(list.model.clone()))
    }
}

/// Every `Vm` property for `view`, by name, in the order of its fields.
pub fn vm_values(view: &AppView, images: &dyn ImageSource, cards: &CardModels) -> Result<Vec<(String, Value)>, String> {
    Values { images, cards: Some((cards, view)), want_covers: true }.fields(view).map_err(|e| e.to_string())
}

/// The window, as the theme sees it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    /// How big everything is drawn: the player's "Size of everything", or NEXUS_SCALE.
    pub size: f32,
    /// `width` over `height`; 0 before the window has a size.
    pub aspect: f32,
    pub breakpoint: String,
}

/// The `Vm` properties the host gives, not the core: the window, the input, the clock, the
/// monitors and the trailer player.
#[derive(serde::Serialize)]
pub(crate) struct HostVm<'a> {
    pub viewport: Viewport,
    pub input_mode: &'a str,
    pub clock: &'a str,
    pub can_play_trailers: bool,
    pub pad_name: &'a str,
    pub monitors: &'a [String],
    pub monitor: String,
    pub has_trailer: bool,
    pub trailer: TrailerVm,
}

/// The trailer playing, as the contract's `Trailer`.
#[derive(Default, serde::Serialize)]
pub(crate) struct TrailerVm {
    pub playing: bool,
    pub loading: bool,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
}

/// Every `Vm` property the host gives, by name.
pub(crate) fn host_values(host: &HostVm, images: &dyn ImageSource) -> Result<Vec<(String, Value)>, String> {
    Values::new(images).fields(host).map_err(|e| e.to_string())
}

pub fn token_value(value: &TokenValue) -> Value {
    match value {
        TokenValue::Color(rgba) => {
            let [r, g, b, a] = rgba.to_be_bytes();
            Value::Brush(Brush::SolidColor(Color::from_argb_u8(a, r, g, b)))
        }
        TokenValue::Length(n) | TokenValue::Duration(n) => number(*n),
        TokenValue::Int(n) => number(*n),
        TokenValue::Text(s) => text(s),
    }
}

/// Sets one global property. A property the theme doesn't have is skipped; a value of the
/// wrong type is an error.
pub fn set(instance: &ComponentInstance, global: &str, name: &str, value: Value) -> Result<(), String> {
    // `Void` means "leave the default", for optional structs.
    if matches!(value, Value::Void) {
        return Ok(());
    }
    let value = match instance.get_global_property(global, name) {
        Ok(old) => {
            let value = reconcile(&old, value);
            if value == old {
                return Ok(());
            }
            value
        }
        Err(_) => value,
    };
    match instance.set_global_property(global, name, value) {
        Ok(()) | Err(SetPropertyError::NoSuchProperty) => Ok(()),
        Err(e) => Err(format!("{global}.{name}: {e}")),
    }
}

/// `new`, with the lists of `old` kept and changed in place. A repeater makes its rows again
/// when it gets another list, so without this every view update would restart the theme's
/// entrance animations and stop its transitions.
fn reconcile(old: &Value, new: Value) -> Value {
    reconcile_at(old, new, Within::Property)
}

/// Where a value is: a property of its own, a field of a struct property, or in a list's row.
#[derive(Clone, Copy, PartialEq)]
enum Within {
    Property,
    Struct,
    Row,
}

/// The `id` or `key` field of a list's row, when it has one.
fn row_key(row: &Value) -> Option<Value> {
    let Value::Struct(fields) = row else { return None };
    fields.get_field("id").or_else(|| fields.get_field("key")).cloned()
}

/// The `id` field of a list's row, when it has one.
fn row_id(row: &Value) -> Option<Value> {
    let Value::Struct(fields) = row else { return None };
    fields.get_field("id").cloned()
}

/// Lists are kept and changed in place, with two exceptions, because a repeater doesn't
/// always see a change made in place to a list that is not a property of its own:
/// - A list inside a list's row (a card's buttons) is kept only while it is the same.
/// - A list inside a struct (a Game Hub's releases) is replaced when its rows are other rows,
///   by their `id` or `key`. A repeater whose struct was set again since it last drew skips
///   a row changed in place, so the Game Hub of one game would show another game's releases.
fn reconcile_at(old: &Value, new: Value, within: Within) -> Value {
    // The same list (a `CardModels` list): it already has its rows.
    if let (Value::Model(a), Value::Model(b)) = (old, &new)
        && a == b
    {
        return new;
    }
    match (old, new) {
        (Value::Model(old_model), Value::Model(new_model)) if within == Within::Row => {
            let same = old_model.row_count() == new_model.row_count()
                && (0..new_model.row_count()).all(|i| old_model.row_data(i) == new_model.row_data(i));
            Value::Model(if same { old_model.clone() } else { new_model })
        }
        (Value::Model(old_model), Value::Model(new_model)) => {
            let Some(rows) = old_model.as_any().downcast_ref::<VecModel<Value>>() else {
                return Value::Model(new_model);
            };
            let count = new_model.row_count();
            let other_rows = (0..count.min(rows.row_count())).any(|i| {
                rows.row_data(i).as_ref().and_then(row_key) != new_model.row_data(i).as_ref().and_then(row_key)
            });
            if within == Within::Struct && other_rows {
                return Value::Model(new_model);
            }
            // In a list of its own, a row whose `id` is gone is removed first. The rows after it
            // then keep their place in the repeater, and their state, such as a notice's timers.
            if within == Within::Property {
                let ids: Vec<Value> =
                    (0..count).filter_map(|i| new_model.row_data(i).as_ref().and_then(row_id)).collect();
                if !ids.is_empty() {
                    for i in (0..rows.row_count()).rev() {
                        if rows.row_data(i).as_ref().and_then(row_id).is_some_and(|id| !ids.contains(&id)) {
                            rows.remove(i);
                        }
                    }
                }
            }
            for i in 0..count {
                let new_row = new_model.row_data(i).unwrap_or_default();
                match rows.row_data(i) {
                    Some(old_row) => {
                        let row = reconcile_at(&old_row, new_row, Within::Row);
                        if row != old_row {
                            rows.set_row_data(i, row);
                        }
                    }
                    None => rows.push(new_row),
                }
            }
            while rows.row_count() > count {
                rows.remove(rows.row_count() - 1);
            }
            Value::Model(old_model.clone())
        }
        (Value::Struct(old_fields), Value::Struct(new_fields)) => {
            let inner = if within == Within::Row { Within::Row } else { Within::Struct };
            Value::Struct(
                new_fields
                    .iter()
                    .map(|(name, value)| {
                        let value = match old_fields.get_field(name) {
                            Some(old) => reconcile_at(old, value.clone(), inner),
                            None => value.clone(),
                        };
                        (name.to_owned(), value)
                    })
                    .collect(),
            )
        }
        (_, new) => new,
    }
}

pub fn set_options(instance: &ComponentInstance, values: &BTreeMap<String, OptionValue>) -> Result<(), String> {
    for (name, value) in values {
        let value = match value {
            OptionValue::Choice(choice) => text(choice),
            OptionValue::Color(rgba) => token_value(&TokenValue::Color(*rgba)),
        };
        set(instance, "Options", name, value)?;
    }
    Ok(())
}

pub fn set_tokens(instance: &ComponentInstance, tokens: &BTreeMap<String, TokenValue>) -> Result<(), String> {
    for (name, value) in tokens {
        set(instance, "Tokens", name, token_value(value))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<const N: usize>(fields: [(&str, Value); N]) -> Value {
        Value::Struct(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    fn model(values: Vec<Value>) -> Value {
        Value::Model(ModelRc::new(VecModel::from(values)))
    }

    fn releases(ids: &[&str], size: &str) -> Value {
        record([("releases", model(ids.iter().map(|id| record([("id", text(id)), ("size", text(size))])).collect()))])
    }

    fn list_of(value: &Value) -> ModelRc<Value> {
        let Value::Struct(fields) = value else { panic!("not a struct") };
        let Some(Value::Model(list)) = fields.get_field("releases") else { panic!("no releases") };
        list.clone()
    }

    #[test]
    fn a_list_in_a_struct_is_replaced_by_other_rows_and_changed_in_place_for_the_same_rows() {
        let old = releases(&["a"], "1 GB");
        let same = reconcile(&old, releases(&["a"], "2 GB"));
        assert_eq!(list_of(&same), list_of(&old), "the same rows: the list is kept");
        assert_eq!(list_of(&old).row_data(0), list_of(&releases(&["a"], "2 GB")).row_data(0), "and changed");
        let more = reconcile(&old, releases(&["a", "b"], "2 GB"));
        assert_eq!(list_of(&more), list_of(&old), "a row more: the list is kept");
        let other = reconcile(&old, releases(&["c"], "2 GB"));
        assert_ne!(list_of(&other), list_of(&old), "other rows: another list");
    }

    #[test]
    fn a_cover_is_loading_until_it_is_in_and_none_without_one_or_when_it_fails() {
        let mut images = crate::images::ImageCache::default();
        let art = |n: u32| {
            Some(ImageRef::Url { plugin: "store".parse().unwrap(), url: format!("https://art.example/{n}.png") })
        };
        let state = |images: &crate::images::ImageCache, r: &Option<ImageRef>| {
            let shown = cover(images, r, crate::Crop::None, true);
            cover_state(images, r, crate::Crop::None, &shown)
        };
        assert_eq!(state(&images, &None), text("none"));
        assert_eq!(state(&images, &art(1)), text("loading"));

        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(64, 96, image::Rgb([200, 60, 40]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        images.insert((art(1).unwrap(), crate::ImageSize::Thumb, crate::Crop::None), Ok(png.into_inner()));
        assert_eq!(state(&images, &art(1)), text("ready"));

        images.insert((art(2).unwrap(), crate::ImageSize::Thumb, crate::Crop::None), Err("HTTP 404".into()));
        assert_eq!(state(&images, &art(2)), text("none"), "it couldn't load");
    }
}
