//! A plugin's settings schema, as schemars writes it (JSON Schema), read two ways: as the fields
//! a settings panel shows, and as the type of the text the player types into one.

use nexus_ui_api::{FieldKind, FieldVm};
use serde_json::Value;

use crate::{CoreError, CoreResult};

/// Settings fields from a JSON Schema object: booleans, numbers, strings and string enums.
/// Other properties are left out. They come in the order the schema's `x-order` list names,
/// then the rest by name: JSON objects don't keep their order.
pub(crate) fn fields(schema: &Value, values: &Value) -> Vec<FieldVm> {
    let order: Vec<&str> = schema
        .get("x-order")
        .and_then(Value::as_array)
        .map(|o| o.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = unordered_fields(schema, values);
    out.sort_by_key(|f| order.iter().position(|k| *k == f.key).unwrap_or(order.len()));
    out
}

/// The value the text `text` is for the field `key`, typed by the schema. An optional field
/// left empty is unset, so the emulator's own value applies.
pub(crate) fn typed(schema: &Value, key: &str, text: String) -> CoreResult<Value> {
    let prop = schema.pointer(&format!("/properties/{key}")).unwrap_or(&Value::Null);
    Ok(match type_of(prop) {
        _ if text.trim().is_empty() && nullable(prop) => Value::Null,
        Some("boolean") => Value::Bool(text == "true"),
        Some("integer") => text
            .trim()
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| CoreError::Invalid(format!("{key} must be a whole number")))?,
        Some("number") => text
            .trim()
            .parse::<f64>()
            .map(Value::from)
            .map_err(|_| CoreError::Invalid(format!("{key} must be a number")))?,
        _ => Value::String(text),
    })
}

/// A choice's values with their labels ("" when it has none): an `enum` list, or `oneOf`/`anyOf`
/// entries. schemars makes a `const` entry, labelled by its doc comment, for each variant with
/// one, and groups the rest into one `enum` entry.
fn choices(schema: &Value) -> Vec<(String, String)> {
    let listed = |o: &Value| -> Vec<(String, String)> {
        o.get("enum")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(|v| (v.to_owned(), String::new())).collect())
            .unwrap_or_default()
    };
    let Some(options) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) else {
        return listed(schema);
    };
    options
        .iter()
        .flat_map(|o| match o.get("const").and_then(Value::as_str) {
            Some(value) => {
                let label = o.get("description").or_else(|| o.get("title")).and_then(Value::as_str);
                vec![(value.to_owned(), label.unwrap_or_default().to_owned())]
            }
            None => listed(o),
        })
        .collect()
}

/// A property's schema with an `Option` taken off: schemars writes `Option<T>` as a type list
/// with "null", or as `anyOf` with a `null` entry.
fn nullable(prop: &Value) -> bool {
    let in_types = prop.get("type").and_then(Value::as_array).is_some_and(|t| t.iter().any(|v| v == "null"));
    let in_any = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|o| o.iter().any(|v| v.get("type") == Some(&Value::from("null"))));
    in_types || in_any
}

/// The property's own type: "string", "integer"…, without "null".
fn type_of(prop: &Value) -> Option<&str> {
    match prop.get("type")? {
        Value::String(t) => Some(t),
        Value::Array(types) => types.iter().filter_map(Value::as_str).find(|t| *t != "null"),
        _ => None,
    }
}

/// What a property is, past an `Option`'s `anyOf` and a `$ref` into the schema's `$defs`.
fn resolved<'a>(schema: &'a Value, prop: &'a Value) -> &'a Value {
    let inner = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .filter(|o| o.len() == 2 && nullable(prop))
        .and_then(|o| o.iter().find(|v| v.get("type") != Some(&Value::from("null"))))
        .unwrap_or(prop);
    inner
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/").or_else(|| r.strip_prefix("#/definitions/")))
        .and_then(|name| schema.get("$defs").or_else(|| schema.get("definitions"))?.get(name))
        .unwrap_or(inner)
}

fn unordered_fields(schema: &Value, values: &Value) -> Vec<FieldVm> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else { return Vec::new() };
    let text = |v: Option<&Value>| match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    properties
        .iter()
        .filter_map(|(key, prop)| {
            // A `$ref` points into the schema's `$defs`: Rust enums come out that way.
            let target = resolved(schema, prop);
            let mut choices = choices(target);
            // An optional choice can also be left to the emulator's own setting.
            if nullable(prop) && !choices.is_empty() {
                choices.insert(0, (String::new(), "As the emulator".into()));
            }
            let kind = match type_of(target).or_else(|| type_of(prop)) {
                _ if !choices.is_empty() => FieldKind::Choice,
                Some("boolean") => FieldKind::Toggle,
                Some("integer" | "number") => FieldKind::Number,
                Some("string") => FieldKind::Text,
                _ => return None,
            };
            Some(FieldVm {
                key: key.clone(),
                label: prop.get("title").and_then(Value::as_str).unwrap_or(key).to_owned(),
                help: prop.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
                kind,
                value: text(values.get(key).or_else(|| prop.get("default"))),
                choice_labels: choices.iter().map(|(_, label)| label.clone()).collect(),
                choices: choices.into_iter().map(|(value, _)| value).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_fields_follow_the_schema_s_order_when_it_names_one() {
        let schema = serde_json::json!({
            "x-order": ["fullscreen", "width", "amd_cpu"],
            "properties": {
                "amd_cpu": { "type": "boolean" },
                "extra": { "type": "string" },
                "fullscreen": { "type": "boolean" },
                "width": { "type": "integer" },
            },
        });
        let keys: Vec<_> = fields(&schema, &Value::Null).into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["fullscreen", "width", "amd_cpu", "extra"], "unnamed fields come last");

        let unordered = serde_json::json!({ "properties": { "b": { "type": "string" }, "a": { "type": "string" } } });
        let keys: Vec<_> = fields(&unordered, &Value::Null).into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["a", "b"]);
    }

    #[test]
    fn a_choice_can_be_a_reference_to_a_list_of_constants() {
        // What schemars makes of a Rust enum whose variants have doc comments.
        let schema = serde_json::json!({
            "properties": { "mode": { "title": "Present mode", "$ref": "#/$defs/Mode", "default": "Mailbox" } },
            "$defs": { "Mode": { "oneOf": [
                { "type": "string", "const": "Mailbox", "description": "Low latency." },
                { "type": "string", "const": "Fifo" },
            ] } },
        });
        let field = fields(&schema, &Value::Null).remove(0);
        assert_eq!(
            (field.label.as_str(), field.kind, field.value.as_str()),
            ("Present mode", FieldKind::Choice, "Mailbox")
        );
        assert_eq!(field.choices, ["Mailbox", "Fifo"]);
        assert_eq!(field.choice_labels, ["Low latency.", ""], "a doc comment names its choice");
    }

    #[test]
    fn variants_without_doc_comments_are_choices_too() {
        // schemars groups the variants with no doc comment into one `enum` entry.
        let schema = serde_json::json!({
            "properties": { "out": { "title": "Video out", "$ref": "#/$defs/Out" } },
            "$defs": { "Out": { "oneOf": [
                { "type": "string", "enum": ["FullHd", "Uhd"] },
                { "type": "string", "const": "Title", "description": "Game default" },
            ] } },
        });
        let field = fields(&schema, &Value::Null).remove(0);
        assert_eq!(field.choices, ["FullHd", "Uhd", "Title"]);
        assert_eq!(field.choice_labels, ["", "", "Game default"]);
    }

    #[test]
    fn an_optional_choice_can_be_left_to_the_emulator() {
        let schema = serde_json::json!({
            "properties": { "mode": { "anyOf": [{ "$ref": "#/$defs/Mode" }, { "type": "null" }] } },
            "$defs": { "Mode": { "type": "string", "enum": ["A", "B"] } },
        });
        let field = fields(&schema, &serde_json::json!({ "mode": "B" })).remove(0);
        assert_eq!((field.kind, field.value.as_str()), (FieldKind::Choice, "B"));
        assert_eq!(field.choices, ["", "A", "B"]);
        assert_eq!(field.choice_labels, ["As the emulator", "", ""]);
    }

    #[test]
    fn typed_text_takes_the_schema_s_type() {
        let schema = serde_json::json!({ "properties": {
            "on": { "type": "boolean" },
            "width": { "type": "integer" },
            "gamma": { "type": "number" },
            "name": { "type": "string" },
            "fps": { "type": ["integer", "null"] },
        } });
        let typed = |key: &str, text: &str| typed(&schema, key, text.into()).map_err(|e| e.to_string());
        assert_eq!(typed("on", "true"), Ok(Value::Bool(true)));
        assert_eq!(typed("on", "yes"), Ok(Value::Bool(false)), "only \"true\" is on");
        assert_eq!(typed("width", " 1280 "), Ok(Value::from(1280)));
        assert_eq!(typed("width", "wide"), Err("width must be a whole number".into()));
        assert_eq!(typed("gamma", "2.2"), Ok(Value::from(2.2)));
        assert_eq!(typed("gamma", "x"), Err("gamma must be a number".into()));
        assert_eq!(typed("name", " Kyty "), Ok(Value::from(" Kyty ")), "text stays as typed");
        assert_eq!(typed("fps", " "), Ok(Value::Null), "an optional field left empty is unset");
        assert_eq!(typed("fps", "60"), Ok(Value::from(60)));
        assert_eq!(typed("unknown", "5"), Ok(Value::from("5")), "a key the schema doesn't name is text");
    }
}
