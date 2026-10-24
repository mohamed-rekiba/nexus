//! What a theme ships and what it can rely on.
//!
//! A theme folder holds `theme.toml`, `shell.slint` (optional when the parent has one),
//! `screens/*.slint`, `components/*.slint` and asset files. The host serves these virtual
//! files to every theme; themes import them and must not ship files with these names:
//!
//! - `types.slint`: the `Vm` global (data) and `Host` global (callbacks).
//! - `tokens.slint`: the `Tokens` global, one property per design token.
//! - `assets.slint`: the `Assets` global, one `image` per name in `[theme.assets.images]`.
//!
//! Every `.slint` import resolves in the theme first, then in each parent up to the root
//! theme. That gives screen and component overrides with fallback.
//!
//! Rules the host checks before compiling:
//! - No `@image-url`: images come from `Assets`, view models, or slot icons.
//! - No font imports: declare fonts in `[theme.assets.fonts]`.
//! - Each screen file exports its component under the name in [`ScreenId::component`], and
//!   each component file under [`ComponentId::component`].
//! - Rendered slot items set `accessible-role: text; accessible-label: <slot>.label;`.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::ScreenId;
use crate::tokens::{TOKENS, TokenKind};

/// The root of every theme. It exports `Shell`, which inherits `Window`.
pub const SHELL_FILE: &str = "shell.slint";
pub const SHELL_COMPONENT: &str = "Shell";

pub const TYPES_FILE: &str = "types.slint";
pub const TOKENS_FILE: &str = "tokens.slint";
pub const ASSETS_FILE: &str = "assets.slint";
/// The `Options` global: one property per theme option, holding its value.
pub const OPTIONS_FILE: &str = "options.slint";

/// Files the host serves. A theme that ships one of these is refused.
pub const RESERVED_FILES: [&str; 4] = [TYPES_FILE, TOKENS_FILE, ASSETS_FILE, OPTIONS_FILE];

/// The fixed contract file. See `contract/types.slint`.
pub const TYPES_SLINT: &str = include_str!("../contract/types.slint");

impl ScreenId {
    pub fn file(self) -> String {
        format!("screens/{}.slint", self.as_str())
    }

    pub fn component(self) -> &'static str {
        match self {
            Self::Home => "HomeScreen",
            Self::GameDetail => "GameDetailScreen",
            Self::Catalog => "CatalogScreen",
            Self::Downloads => "DownloadsScreen",
            Self::Settings => "SettingsScreen",
            Self::Emulators => "EmulatorsScreen",
            Self::Themes => "ThemesScreen",
            Self::Plugin => "PluginScreen",
            Self::GameSettings => "GameSettingsScreen",
            Self::Running => "RunningScreen",
        }
    }
}

/// Building blocks a theme can override one by one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentId {
    Card,
    Button,
    ListItem,
    ProgressBar,
    Dialog,
    /// A row of badge slot items.
    SlotBadges,
    /// The action slot items of a game.
    SlotActions,
    /// The section slot items of a game.
    SlotSections,
}

impl ComponentId {
    pub const ALL: [ComponentId; 8] = [
        Self::Card,
        Self::Button,
        Self::ListItem,
        Self::ProgressBar,
        Self::Dialog,
        Self::SlotBadges,
        Self::SlotActions,
        Self::SlotSections,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Button => "button",
            Self::ListItem => "list-item",
            Self::ProgressBar => "progress-bar",
            Self::Dialog => "dialog",
            Self::SlotBadges => "slot-badges",
            Self::SlotActions => "slot-actions",
            Self::SlotSections => "slot-sections",
        }
    }

    pub fn file(self) -> String {
        format!("components/{}.slint", self.as_str())
    }

    pub fn component(self) -> &'static str {
        match self {
            Self::Card => "Card",
            Self::Button => "Button",
            Self::ListItem => "ListItem",
            Self::ProgressBar => "ProgressBar",
            Self::Dialog => "Dialog",
            Self::SlotBadges => "SlotBadges",
            Self::SlotActions => "SlotActions",
            Self::SlotSections => "SlotSections",
        }
    }
}

/// `tokens.slint`. `fonts` are absolute paths of font files to register; they are imported
/// here because every theme imports this file.
pub fn tokens_slint(fonts: &[String]) -> String {
    let mut out = String::from("// Served by the host. Values come from the theme's tokens and options.\n");
    for font in fonts {
        let _ = writeln!(out, "import {font:?};");
    }
    out.push_str("export global Tokens {\n");
    for t in TOKENS {
        let kind = match t.kind {
            TokenKind::Color => "color",
            TokenKind::Length => "length",
            TokenKind::Duration => "duration",
            TokenKind::Int => "int",
            TokenKind::Text => "string",
        };
        let _ = writeln!(out, "    in-out property <{kind}> {};", t.name);
    }
    out.push_str("}\n");
    out
}

/// `options.slint` for the merged theme's options: a choice is its name as a `string`, a color
/// option a `color`.
pub fn options_slint(options: &std::collections::BTreeMap<String, crate::ThemeOption>) -> String {
    let mut out = String::from("// Served by the host: the theme's option values.\nexport global Options {\n");
    for (name, option) in options {
        let kind = match option.kind {
            crate::OptionKind::Choice { .. } => "string",
            crate::OptionKind::Color { .. } => "color",
        };
        let _ = writeln!(out, "    in-out property <{kind}> {name};");
    }
    out.push_str("}\n");
    out
}

/// `assets.slint` for the merged theme's image names.
pub fn assets_slint<'a>(images: impl IntoIterator<Item = &'a String>) -> String {
    let mut out = String::from("// Served by the host: the theme's images.\nexport global Assets {\n");
    for name in images {
        let _ = writeln!(out, "    in-out property <image> {name};");
    }
    out.push_str("    in-out property <bool> loaded;\n}\n");
    out
}

/// Checks one theme `.slint` source against the rules above. `expected` is the component the
/// file must export, for screen and component files.
pub fn lint(source: &str, expected: Option<&str>) -> Result<(), String> {
    let code = strip_comments(source);
    if code.contains("@image-url") {
        return Err("uses @image-url; declare the image in [theme.assets.images] and use Assets.<name>".into());
    }
    // Statements end with `;`, so an import split over several lines is still one piece.
    for statement in code.split(';') {
        let statement = statement.trim_start();
        let is_import =
            statement.strip_prefix("import").is_some_and(|rest| rest.starts_with([' ', '\t', '\n', '\r', '{', '"']));
        let is_reexport = statement.starts_with("export {") || statement.starts_with("export *");
        if !(is_import || is_reexport) {
            continue;
        }
        let target = statement.rsplit('"').nth(1).unwrap_or_default();
        if !target.ends_with(".slint") {
            return Err(format!("imports {target:?}; only .slint files can be imported (declare fonts in theme.toml)"));
        }
    }
    if let Some(name) = expected {
        let exported = code.lines().any(|l| {
            let l = l.trim();
            l.starts_with(&format!("export component {name} ")) || l.starts_with(&format!("export component {name}{{"))
        });
        if !exported {
            return Err(format!("must export component {name}"));
        }
    }
    Ok(())
}

/// The source without `//` and `/* */` comments. String literals are kept as they are.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_refuses_disk_access_and_wrong_exports() {
        assert!(lint("Image { source: @image-url(\"x.png\"); }", None).is_err());
        assert!(lint("import \"fonts/a.ttf\";", None).is_err());
        assert!(lint("import\n    \"/abs/a.ttf\";", None).is_err(), "an import split over lines");
        assert!(lint("import /* hidden */ \"/abs/a.ttf\";", None).is_err());
        assert!(lint("export * from \"/abs/a.ttf\";", None).is_err());
        assert!(lint("// import \"x.ttf\";\nexport component A inherits Rectangle {}", None).is_ok(), "a comment");
        assert!(
            lint("export component A inherits Rectangle { Text { text: \"import \\\"x.ttf\\\"\"; } }", None).is_ok()
        );
        assert!(lint("import { Card } from \"../components/card.slint\";", None).is_ok());
        assert!(lint("export component HomeScreen inherits Rectangle {}", Some("HomeScreen")).is_ok());
        assert!(lint("export component Home inherits Rectangle {}", Some("HomeScreen")).is_err());
    }

    #[test]
    fn generated_globals_list_every_token_and_image() {
        let tokens = tokens_slint(&["/t/font.ttf".into()]);
        assert!(tokens.starts_with("//") && tokens.contains("import \"/t/font.ttf\";"));
        assert!(tokens.contains("in-out property <color> color-accent;"));
        assert!(tokens.contains("in-out property <duration> motion-fast;"));
        let assets = assets_slint(&["logo".to_string()]);
        assert!(assets.contains("in-out property <image> logo;"));
    }
}
