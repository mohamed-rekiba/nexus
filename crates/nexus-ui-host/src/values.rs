//! One converter from `serde` data to Slint values: how a view model becomes the theme's `Vm`.
//!
//! - A struct or a map becomes a `Struct`, its field names kebab-cased (`can_go_back` is
//!   `can-go-back`). A field that is `None` is left out: Slint gives it its type's default.
//! - A list becomes a model; text, numbers and flags stay as they are; an enum's unit variant
//!   is its `serde` name.
//! - The fields [`nexus_ui_api::mark`] names take a form of their own: an image, a color, an
//!   optional part with its `has-` flag, or a long list of cards kept in place.

use nexus_ui_api::{AppView, ImageRef, mark};
use serde::Serialize;
use serde::ser::{self, Impossible};
use slint::{ModelRc, VecModel};
use slint_interpreter::{Brush, Color, SharedString, Struct, Value};

use crate::bind::{CardModels, ImageSource, cover_crop, cover_state};
use crate::{Crop, ImageSize};

/// What the converter needs besides the data: the images, and the card lists it keeps.
pub(crate) struct Values<'a> {
    pub images: &'a dyn ImageSource,
    /// The long lists of cards, and the view they come from. Without them, cards convert as
    /// any list.
    pub cards: Option<(&'a CardModels, &'a AppView)>,
    /// A card's cover loads only when this is true: a card far from the screen asks for none.
    pub want_covers: bool,
}

impl Values<'_> {
    pub(crate) fn new(images: &dyn ImageSource) -> Values<'_> {
        Values { images, cards: None, want_covers: true }
    }

    /// `value` as a Slint value.
    pub(crate) fn of<T: Serialize + ?Sized>(&self, value: &T) -> Result<Value, Error> {
        Ok(value.serialize(self.at("", "", None))?.value())
    }

    /// The fields of the struct `value`, in their order, as Slint values: each a property of a
    /// global. A field left out is not there.
    pub(crate) fn fields<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<(String, Value)>, Error> {
        match value.serialize(self.at("", "", None))? {
            Out::Fields(fields) => Ok(fields),
            _ => Err(ser::Error::custom("not a struct")),
        }
    }

    fn at<'s>(&'s self, field: &str, owner: &str, platform: Option<&str>) -> Ser<'s> {
        Ser { values: self, field: field.to_owned(), owner: owner.to_owned(), platform: platform.map(str::to_owned) }
    }

    fn image(&self, image: Option<ImageRef>, size: ImageSize, crop: Crop, want: bool) -> Value {
        Value::Image(image.map(|r| self.images.image(&r, size, crop, want)).unwrap_or_default())
    }
}

/// Why a value has no Slint form.
#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Error(msg.to_string())
    }
}

/// One converted value, before its struct takes it.
enum Out {
    Value(Value),
    /// `None`: the struct leaves the field out.
    Absent,
    /// A [`mark::HAS`] field: its `has-` flag, and its value when there is one.
    Has(Option<Value>),
    /// A [`mark::COVER`] field: the image, and its `-state`.
    Cover(Value, Value),
    /// A struct's fields, in their order.
    Fields(Vec<(String, Value)>),
}

impl Out {
    fn value(self) -> Value {
        match self {
            Out::Value(value) | Out::Has(Some(value)) | Out::Cover(value, _) => value,
            Out::Fields(fields) => Value::Struct(fields.into_iter().collect::<Struct>()),
            Out::Absent | Out::Has(None) => Value::Void,
        }
    }
}

/// The serializer for one value: the field it is, the field its struct is in, and the
/// `platform` of its struct so far, which cuts a cover.
struct Ser<'s> {
    values: &'s Values<'s>,
    field: String,
    owner: String,
    platform: Option<String>,
}

/// A value inside a marked field, read back as its type.
fn read<T: serde::de::DeserializeOwned, V: Serialize + ?Sized>(value: &V) -> Result<T, Error> {
    let json = serde_json::to_value(value).map_err(ser::Error::custom)?;
    serde_json::from_value(json).map_err(ser::Error::custom)
}

fn number(n: impl Into<f64>) -> Result<Out, Error> {
    Ok(Out::Value(Value::Number(n.into())))
}

fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

impl<'s> ser::Serializer for Ser<'s> {
    type Ok = Out;
    type Error = Error;
    type SerializeSeq = List<'s>;
    type SerializeTuple = List<'s>;
    type SerializeTupleStruct = List<'s>;
    type SerializeTupleVariant = Impossible<Out, Error>;
    type SerializeMap = Fields<'s>;
    type SerializeStruct = Fields<'s>;
    type SerializeStructVariant = Impossible<Out, Error>;

    fn serialize_bool(self, v: bool) -> Result<Out, Error> {
        Ok(Out::Value(Value::Bool(v)))
    }
    fn serialize_i8(self, v: i8) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_i16(self, v: i16) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_i32(self, v: i32) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_i64(self, v: i64) -> Result<Out, Error> {
        number(v as f64)
    }
    fn serialize_u8(self, v: u8) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_u16(self, v: u16) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_u32(self, v: u32) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_u64(self, v: u64) -> Result<Out, Error> {
        number(v as f64)
    }
    fn serialize_f32(self, v: f32) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_f64(self, v: f64) -> Result<Out, Error> {
        number(v)
    }
    fn serialize_char(self, v: char) -> Result<Out, Error> {
        Ok(Out::Value(Value::String(SharedString::from(v.to_string()))))
    }
    fn serialize_str(self, v: &str) -> Result<Out, Error> {
        Ok(Out::Value(Value::String(SharedString::from(v))))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Out, Error> {
        Err(ser::Error::custom(format!("{}: bytes have no Slint form", self.field)))
    }
    fn serialize_none(self) -> Result<Out, Error> {
        Ok(Out::Absent)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Out, Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Out, Error> {
        Ok(Out::Absent)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Out, Error> {
        Ok(Out::Absent)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<Out, Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, name: &'static str, value: &T) -> Result<Out, Error> {
        let values = self.values;
        let crop = cover_crop(self.platform.as_deref().unwrap_or_default());
        Ok(match name {
            mark::THUMB => Out::Value(values.image(read(value)?, ImageSize::Thumb, Crop::None, true)),
            mark::PICTURE => Out::Value(values.image(read(value)?, ImageSize::Full, Crop::None, true)),
            mark::COVER_PICTURE => Out::Value(values.image(read(value)?, ImageSize::Full, crop, true)),
            mark::COVER => {
                let image: Option<ImageRef> = read(value)?;
                let shown = values.image(image.clone(), ImageSize::Thumb, crop, values.want_covers);
                let state = cover_state(values.images, &image, crop, &shown);
                Out::Cover(shown, state)
            }
            mark::COLOR => {
                let [a, r, g, b] = read::<u32, _>(value)?.to_be_bytes();
                Out::Value(Value::Brush(Brush::SolidColor(Color::from_argb_u8(a, r, g, b))))
            }
            mark::HAS => match value.serialize(self)? {
                Out::Absent => Out::Has(None),
                out => Out::Has(Some(out.value())),
            },
            mark::CARDS => match values.cards {
                Some((cards, view)) => Out::Value(cards.list(&self.owner, values.images, view)),
                None => value.serialize(self)?,
            },
            _ => value.serialize(self)?,
        })
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: &T,
    ) -> Result<Out, Error> {
        Err(ser::Error::custom(format!("{}: the enum variant {variant} has no Slint form", self.field)))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<List<'s>, Error> {
        Ok(List { ser: self, rows: Vec::with_capacity(len.unwrap_or_default()) })
    }
    fn serialize_tuple(self, len: usize) -> Result<List<'s>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<List<'s>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(ser::Error::custom(format!("{}: the enum variant {variant} has no Slint form", self.field)))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Fields<'s>, Error> {
        Ok(Fields::new(self.values, &self.field, None))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Fields<'s>, Error> {
        self.serialize_map(None)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(ser::Error::custom(format!("{}: the enum variant {variant} has no Slint form", self.field)))
    }
}

/// A list's rows, as a model.
struct List<'s> {
    ser: Ser<'s>,
    rows: Vec<Value>,
}

impl List<'_> {
    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let row = value.serialize(self.ser.values.at(&self.ser.field, &self.ser.owner, None))?;
        self.rows.push(row.value());
        Ok(())
    }

    fn done(self) -> Result<Out, Error> {
        Ok(Out::Value(Value::Model(ModelRc::new(VecModel::from(self.rows)))))
    }
}

impl ser::SerializeSeq for List<'_> {
    type Ok = Out;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Out, Error> {
        self.done()
    }
}

impl ser::SerializeTuple for List<'_> {
    type Ok = Out;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Out, Error> {
        self.done()
    }
}

impl ser::SerializeTupleStruct for List<'_> {
    type Ok = Out;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.push(value)
    }
    fn end(self) -> Result<Out, Error> {
        self.done()
    }
}

/// A struct's fields, or a map's entries, as a Slint struct.
struct Fields<'s> {
    values: &'s Values<'s>,
    /// The field this struct is in: it names a list of cards.
    owner: String,
    /// The struct's `platform`, once it has come.
    platform: Option<String>,
    fields: Vec<(String, Value)>,
    /// A map's key, until its value comes.
    key: Option<String>,
}

impl<'s> Fields<'s> {
    fn new(values: &'s Values<'s>, owner: &str, platform: Option<String>) -> Self {
        Fields { values, owner: owner.to_owned(), platform, fields: Vec::new(), key: None }
    }

    fn done(self) -> Result<Out, Error> {
        Ok(Out::Fields(self.fields))
    }

    fn field<T: Serialize + ?Sized>(&mut self, name: &str, value: &T) -> Result<(), Error> {
        let out = value.serialize(self.values.at(name, &self.owner, self.platform.as_deref()))?;
        let name = kebab(name);
        match out {
            Out::Fields(_) => self.fields.push((name, out.value())),
            Out::Value(value) => {
                if let ("platform", Value::String(platform)) = (name.as_str(), &value) {
                    self.platform = Some(platform.to_string());
                }
                self.fields.push((name, value));
            }
            Out::Absent => {}
            Out::Has(value) => {
                self.fields.push((format!("has-{name}"), Value::Bool(value.is_some())));
                if let Some(value) = value {
                    self.fields.push((name, value));
                }
            }
            Out::Cover(image, state) => {
                self.fields.push((format!("{name}-state"), state));
                self.fields.push((name, image));
            }
        }
        Ok(())
    }
}

impl ser::SerializeStruct for Fields<'_> {
    type Ok = Out;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, name: &'static str, value: &T) -> Result<(), Error> {
        self.field(name, value)
    }
    fn end(self) -> Result<Out, Error> {
        self.done()
    }
}

impl ser::SerializeMap for Fields<'_> {
    type Ok = Out;
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        match serde_json::to_value(key).map_err(ser::Error::custom)? {
            serde_json::Value::String(key) => self.key = Some(key),
            other => return Err(ser::Error::custom(format!("{}: the map key {other} isn't text", self.owner))),
        }
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let key = self.key.take().unwrap_or_default();
        self.field(&key, value)
    }
    fn end(self) -> Result<Out, Error> {
        self.done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Images that are never in.
    struct NoImages;

    impl ImageSource for NoImages {
        fn image(&self, _: &ImageRef, _: ImageSize, _: Crop, _: bool) -> slint_interpreter::Image {
            slint_interpreter::Image::default()
        }
        fn failed(&self, _: &ImageRef, _: ImageSize, _: Crop) -> bool {
            false
        }
        fn near(&self, _: &str, _: usize) -> bool {
            true
        }
    }

    fn of<T: Serialize>(value: &T) -> Value {
        Values::new(&NoImages).of(value).unwrap()
    }

    fn field<'v>(value: &'v Value, name: &str) -> &'v Value {
        let Value::Struct(fields) = value else { panic!("not a struct: {value:?}") };
        fields.get_field(name).unwrap_or_else(|| panic!("no field {name} in {value:?}"))
    }

    fn text(s: &str) -> Value {
        Value::String(s.into())
    }

    #[derive(Serialize)]
    struct Inner {
        long_name: String,
        count: u32,
    }

    #[derive(Serialize)]
    struct Outer {
        inner: Inner,
        on: bool,
        missing: Option<String>,
    }

    #[test]
    fn a_nested_struct_becomes_structs_with_kebab_cased_fields() {
        let value = of(&Outer { inner: Inner { long_name: "a".into(), count: 3 }, on: true, missing: None });
        assert_eq!(field(field(&value, "inner"), "long-name"), &text("a"));
        assert_eq!(field(field(&value, "inner"), "count"), &Value::Number(3.0));
        assert_eq!(field(&value, "on"), &Value::Bool(true));
        let Value::Struct(fields) = &value else { unreachable!() };
        assert!(fields.get_field("missing").is_none(), "None is left out: Slint gives the default");
    }

    #[test]
    fn a_list_becomes_a_model_of_its_rows() {
        let value = of(&vec!["a", "b"]);
        let Value::Model(rows) = value else { panic!("not a model") };
        use slint::Model;
        assert_eq!(rows.iter().collect::<Vec<_>>(), [text("a"), text("b")]);
    }

    #[test]
    fn an_enum_s_variant_is_its_serde_name() {
        assert_eq!(of(&nexus_api::Tone::Success), text("success"));
        assert_eq!(of(&nexus_ui_api::SlotKind::SettingsPanel), text("settings_panel"));
    }

    #[test]
    fn an_optional_part_comes_with_its_has_flag() {
        let some = nexus_ui_api::RunningVm {
            session: 1,
            key: "a:b".into(),
            title: "B".into(),
            emulator: String::new(),
            page: String::new(),
            state: "running".into(),
            elapsed_secs: 0,
            elapsed: "12 min".into(),
            in_front: false,
            keymap: Vec::new(),
        };
        #[derive(Serialize)]
        struct View {
            #[serde(serialize_with = "has")]
            running: Option<nexus_ui_api::RunningVm>,
        }
        fn has<S: serde::Serializer>(v: &Option<nexus_ui_api::RunningVm>, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_newtype_struct(mark::HAS, v)
        }
        let shown = of(&View { running: Some(some) });
        assert_eq!(field(&shown, "has-running"), &Value::Bool(true));
        assert_eq!(field(field(&shown, "running"), "elapsed"), &text("12 min"));
        let none = of(&View { running: None });
        assert_eq!(field(&none, "has-running"), &Value::Bool(false));
        let Value::Struct(fields) = &none else { unreachable!() };
        assert!(fields.get_field("running").is_none());
    }

    #[test]
    fn a_card_s_cover_comes_with_its_state_and_a_system_with_its_color() {
        let card = nexus_ui_api::GameCardVm {
            key: "a:b".into(),
            title: "B".into(),
            platform: "PS4".into(),
            system: nexus_ui_api::GameSystemVm { name: "PlayStation 4".into(), color: Some([0, 0x70, 0xd1]) },
            cover: None,
            selected: false,
            badges: Vec::new(),
            release_year: None,
            rating: None,
            size_bytes: None,
            size: String::new(),
            last_played_days: None,
            last_played: String::new(),
            releases: 0,
            availability: String::new(),
            availability_text: String::new(),
            hidden: false,
            can: Default::default(),
        };
        let value = of(&card);
        assert_eq!(field(&value, "cover-state"), &text("none"), "no cover");
        assert_eq!(field(&value, "system-name"), &text("PlayStation 4"));
        assert_eq!(
            field(&value, "system-color"),
            &Value::Brush(Brush::SolidColor(Color::from_argb_u8(255, 0, 0x70, 0xd1)))
        );
        let Value::Struct(fields) = &value else { unreachable!() };
        assert!(fields.get_field("release-year").is_none(), "absent: the theme reads 0");
    }
}
