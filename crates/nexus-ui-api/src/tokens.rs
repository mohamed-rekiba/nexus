//! The design token schema. Every theme sets values for these names; the default theme sets all.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    /// `"#rrggbb"` or `"#rrggbbaa"`.
    Color,
    /// Logical pixels.
    Length,
    /// Milliseconds.
    Duration,
    Int,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenDef {
    /// The Slint property name on the `Tokens` global, and the key in `[theme.tokens]`.
    pub name: &'static str,
    pub kind: TokenKind,
    pub group: &'static str,
}

const fn t(name: &'static str, kind: TokenKind, group: &'static str) -> TokenDef {
    TokenDef { name, kind, group }
}

use TokenKind::*;

pub const TOKENS: &[TokenDef] = &[
    t("color-bg", Color, "color"),
    t("color-surface", Color, "color"),
    t("color-surface-raised", Color, "color"),
    t("color-text", Color, "color"),
    t("color-text-muted", Color, "color"),
    t("color-accent", Color, "color"),
    t("color-on-accent", Color, "color"),
    t("color-focus", Color, "color"),
    t("color-success", Color, "color"),
    t("color-warning", Color, "color"),
    t("color-danger", Color, "color"),
    t("color-info", Color, "color"),
    t("color-overlay", Color, "color"),
    t("font-family", Text, "typography"),
    t("font-size-s", Length, "typography"),
    t("font-size-m", Length, "typography"),
    t("font-size-l", Length, "typography"),
    t("font-size-xl", Length, "typography"),
    t("font-weight-regular", Int, "typography"),
    t("font-weight-bold", Int, "typography"),
    t("space-xs", Length, "spacing"),
    t("space-s", Length, "spacing"),
    t("space-m", Length, "spacing"),
    t("space-l", Length, "spacing"),
    t("space-xl", Length, "spacing"),
    t("radius-s", Length, "radii"),
    t("radius-m", Length, "radii"),
    t("radius-l", Length, "radii"),
    t("shadow-color", Color, "elevation"),
    t("shadow-blur-1", Length, "elevation"),
    t("shadow-blur-2", Length, "elevation"),
    t("motion-fast", Duration, "motion"),
    t("motion-normal", Duration, "motion"),
    t("motion-slow", Duration, "motion"),
    // "ltr" or "rtl": which side a theme puts its leading edge on.
    t("layout-direction", Text, "layout"),
];

pub fn token(name: &str) -> Option<&'static TokenDef> {
    TOKENS.iter().find(|t| t.name == name)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TokenValue {
    /// `0xRRGGBBAA`.
    Color(u32),
    Length(f32),
    Duration(f32),
    Int(i32),
    Text(String),
}

impl TokenValue {
    /// Reads a value of `kind` from TOML.
    pub fn parse(kind: TokenKind, value: &toml::Value) -> Result<Self, String> {
        let number = || value.as_float().or_else(|| value.as_integer().map(|i| i as f64));
        match kind {
            Color => value
                .as_str()
                .and_then(parse_color)
                .map(Self::Color)
                .ok_or_else(|| format!("expected a color like \"#rrggbb\" or \"#rrggbbaa\", got {value}")),
            Length => number().map(|n| Self::Length(n as f32)).ok_or_else(|| format!("expected a number, got {value}")),
            Duration => {
                number().map(|n| Self::Duration(n as f32)).ok_or_else(|| format!("expected milliseconds, got {value}"))
            }
            Int => value
                .as_integer()
                .and_then(|i| i32::try_from(i).ok())
                .map(Self::Int)
                .ok_or_else(|| format!("expected a whole number, got {value}")),
            Text => {
                value.as_str().map(|s| Self::Text(s.to_owned())).ok_or_else(|| format!("expected text, got {value}"))
            }
        }
    }
}

/// `#rrggbb` or `#rrggbbaa` as `0xRRGGBBAA`.
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(value << 8 | 0xff),
        8 => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_with_and_without_alpha() {
        assert_eq!(parse_color("#102030"), Some(0x102030ff));
        assert_eq!(parse_color("#10203040"), Some(0x10203040));
        assert_eq!(parse_color("102030"), None);
        assert_eq!(parse_color("#1020"), None);
    }

    #[test]
    fn token_names_are_unique() {
        let mut names: Vec<_> = TOKENS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOKENS.len());
    }
}
