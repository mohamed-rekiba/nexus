//! The design token schema. A theme sets the tokens it changes; the others keep their defaults.

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
    /// The value a theme that doesn't set it gets: the Classic look's. Read with
    /// [`TokenDef::default_value`].
    pub default: &'static str,
}

const fn t(name: &'static str, kind: TokenKind, group: &'static str, default: &'static str) -> TokenDef {
    TokenDef { name, kind, group, default }
}

impl TokenDef {
    /// [`Self::default`] as a value of the token's kind.
    pub fn default_value(&self) -> TokenValue {
        let value = match self.kind {
            Color | Text => toml::Value::String(self.default.to_owned()),
            Length | Duration => toml::Value::Float(self.default.parse().expect("a token's default is a number")),
            Int => toml::Value::Integer(self.default.parse().expect("a token's default is a whole number")),
        };
        TokenValue::parse(self.kind, &value).expect("a token's default has the token's kind")
    }
}

use TokenKind::*;

pub const TOKENS: &[TokenDef] = &[
    t("color-bg", Color, "color", "#05070f"),
    t("color-surface", Color, "color", "#121a31"),
    t("color-surface-raised", Color, "color", "#1a2440"),
    t("color-text", Color, "color", "#f5f7fb"),
    t("color-text-muted", Color, "color", "#b9c3da"),
    t("color-accent", Color, "color", "#0a6cff"),
    t("color-on-accent", Color, "color", "#ffffff"),
    t("color-focus", Color, "color", "#ffffff"),
    t("color-success", Color, "color", "#3ddc97"),
    t("color-warning", Color, "color", "#ffb547"),
    t("color-danger", Color, "color", "#ff5a6a"),
    t("color-info", Color, "color", "#7ab4ff"),
    t("color-overlay", Color, "color", "#03050ccc"),
    // Deeper than the surface: sheets, hints, the backs of panels.
    t("color-panel", Color, "color", "#0b1020"),
    // Less than muted: hints and the third line of a row.
    t("color-text-faint", Color, "color", "#7d88a3"),
    // Text and marks drawn on `color-focus`.
    t("color-on-focus", Color, "color", "#0a0d18"),
    // The accent, light: links, and the glow of a focused field.
    t("color-accent-soft", Color, "color", "#8dbaff"),
    // Between success and warning: a game that reaches its menus.
    t("color-caution", Color, "color", "#ffd24d"),
    // Light and dark marks laid over anything, at an alpha the kit picks: lines, fills, shades.
    t("color-tint", Color, "color", "#ffffff"),
    t("color-shade", Color, "color", "#000000"),
    t("font-family", Text, "typography", "Inter"),
    t("font-size-s", Length, "typography", "15"),
    t("font-size-m", Length, "typography", "19"),
    t("font-size-l", Length, "typography", "26"),
    t("font-size-xl", Length, "typography", "44"),
    t("font-weight-regular", Int, "typography", "400"),
    t("font-weight-bold", Int, "typography", "700"),
    t("space-xs", Length, "spacing", "4"),
    t("space-s", Length, "spacing", "10"),
    t("space-m", Length, "spacing", "18"),
    t("space-l", Length, "spacing", "28"),
    t("space-xl", Length, "spacing", "48"),
    t("radius-s", Length, "radii", "8"),
    t("radius-m", Length, "radii", "14"),
    t("radius-l", Length, "radii", "20"),
    t("shadow-color", Color, "elevation", "#00000099"),
    t("shadow-blur-1", Length, "elevation", "18"),
    t("shadow-blur-2", Length, "elevation", "36"),
    t("motion-fast", Duration, "motion", "160"),
    t("motion-normal", Duration, "motion", "260"),
    t("motion-slow", Duration, "motion", "420"),
    // "ltr" or "rtl": which side a theme puts its leading edge on.
    t("layout-direction", Text, "layout", "ltr"),
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
    fn every_default_has_its_tokens_kind() {
        for t in TOKENS {
            // Panics with the token's name in the backtrace when a default is wrong.
            let _ = t.default_value();
        }
        assert_eq!(token("color-accent").map(TokenDef::default_value), Some(TokenValue::Color(0x0a6cffff)));
    }

    #[test]
    fn token_names_are_unique() {
        let mut names: Vec<_> = TOKENS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOKENS.len());
    }
}
