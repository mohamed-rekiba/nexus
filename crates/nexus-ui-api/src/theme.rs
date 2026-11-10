//! `theme.toml`: the `[theme]` section, and a theme's values with its option values applied.

use std::collections::BTreeMap;

use nexus_api::{PluginId, PluginManifest, RelPath};
use semver::VersionReq;
use serde::{Deserialize, Serialize};

use crate::tokens::{TOKENS, TokenValue, token};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Pointer,
    Gamepad,
    Touch,
}

/// How the player moves around.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Navigation {
    /// Point and click.
    #[default]
    Pointer,
    /// One focused element at a time, moved with arrows or a gamepad.
    Focus,
}

/// A named window class. The first breakpoint whose limits all hold is the current one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Breakpoint {
    pub name: String,
    /// Logical pixels.
    #[serde(default)]
    pub min_width: Option<f32>,
    #[serde(default)]
    pub max_width: Option<f32>,
    /// Width divided by height.
    #[serde(default)]
    pub min_aspect: Option<f32>,
    #[serde(default)]
    pub max_aspect: Option<f32>,
    /// Physical pixels per logical pixel (DPI scale).
    #[serde(default)]
    pub min_scale: Option<f32>,
}

impl Breakpoint {
    pub fn matches(&self, width: f32, height: f32, scale: f32) -> bool {
        let aspect = if height > 0.0 { width / height } else { 0.0 };
        let at_least = |limit: Option<f32>, v: f32| limit.is_none_or(|l| v >= l);
        let at_most = |limit: Option<f32>, v: f32| limit.is_none_or(|l| v <= l);
        at_least(self.min_width, width)
            && at_most(self.max_width, width)
            && at_least(self.min_aspect, aspect)
            && at_most(self.max_aspect, aspect)
            && at_least(self.min_scale, scale)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeAssets {
    /// Name (a Slint identifier, used as `Assets.<name>`) to a PNG in the theme folder.
    #[serde(default)]
    pub images: BTreeMap<String, RelPath>,
    /// TrueType or OpenType files to register.
    #[serde(default)]
    pub fonts: Vec<RelPath>,
    /// Name (passed to `Host.play-sound`) to a sound file.
    #[serde(default)]
    pub sounds: BTreeMap<String, RelPath>,
}

/// A setting the player can change, which sets design tokens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeOption {
    pub label: String,
    /// Kept out of the Settings list. The option still works, for a build that sets it itself.
    #[serde(default)]
    pub hidden: bool,
    #[serde(flatten)]
    pub kind: OptionKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OptionKind {
    /// Sets one color token.
    Color { token: String, default: String },
    /// Each choice sets a group of tokens.
    Choice { default: String, choices: BTreeMap<String, BTreeMap<String, toml::Value>> },
}

/// The `[theme]` section of `theme.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSection {
    /// The `nexus-ui-api` versions this theme works with.
    pub ui_api: VersionReq,
    /// The theme the app starts with, falls back to when another fails, and uses in safe mode.
    /// The Themes list shows it first. Exactly one installed theme sets it.
    #[serde(default)]
    pub fallback: bool,
    /// Input devices the theme supports, the main one first.
    #[serde(default)]
    pub input: Vec<InputMode>,
    #[serde(default)]
    pub navigation: Option<Navigation>,
    #[serde(default)]
    pub tokens: BTreeMap<String, toml::Value>,
    /// Named window sizes, the first match wins.
    #[serde(default)]
    pub breakpoints: Vec<Breakpoint>,
    #[serde(default)]
    pub assets: ThemeAssets,
    #[serde(default)]
    pub options: BTreeMap<String, ThemeOption>,
    /// Pictures for a system, by its id (`[theme.systems.PS5]`), in place of the ones its
    /// emulator plugin ships: PNG or JPEG files in the theme folder.
    #[serde(default)]
    pub systems: BTreeMap<String, Art<RelPath>>,
}

/// A system's two pictures: a square icon, and a wide cover. A theme names files
/// (`Art<RelPath>`), and the UI host decodes them.
/// A picture left out stays the emulator plugin's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Art<T> {
    pub icon: Option<T>,
    pub cover: Option<T>,
}

impl<T> Default for Art<T> {
    fn default() -> Self {
        Self { icon: None, cover: None }
    }
}

impl<T> Art<T> {
    pub fn get(&self, picture: crate::SystemPicture) -> Option<&T> {
        match picture {
            crate::SystemPicture::Icon => self.icon.as_ref(),
            crate::SystemPicture::Cover => self.cover.as_ref(),
        }
    }

    /// Each picture there is, through `f`; the first error stops it.
    pub fn try_map<U, E>(self, mut f: impl FnMut(crate::SystemPicture, T) -> Result<U, E>) -> Result<Art<U>, E> {
        Ok(Art {
            icon: self.icon.map(|t| f(crate::SystemPicture::Icon, t)).transpose()?,
            cover: self.cover.map(|t| f(crate::SystemPicture::Cover, t)).transpose()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeManifest {
    pub plugin: PluginManifest,
    pub theme: ThemeSection,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ThemeError {
    #[error("no theme {0}")]
    Unknown(PluginId),
    #[error("theme {theme}: {message}")]
    Manifest { theme: PluginId, message: String },
    #[error("theme {theme} needs UI API {required}, this host has {actual}")]
    UiApi { theme: PluginId, required: String, actual: String },
    #[error("{0}")]
    Fallback(String),
    #[error("theme {theme}: token {token}: {message}")]
    Token { theme: PluginId, token: String, message: String },
    #[error("theme {theme}: {file}: {message}")]
    File { theme: PluginId, file: String, message: String },
    #[error("theme {theme} does not compile:\n{diagnostics}")]
    Compile { theme: PluginId, diagnostics: String },
    #[error("theme {theme}: {message}")]
    Binding { theme: PluginId, message: String },
    #[error("theme {theme}: asset {asset}: {message}")]
    Asset { theme: PluginId, asset: String, message: String },
    #[error("{0}")]
    Platform(String),
}

impl ThemeManifest {
    pub fn from_plugin(plugin: &PluginManifest) -> Result<Self, ThemeError> {
        let err = |message: String| ThemeError::Manifest { theme: plugin.id.clone(), message };
        let section = plugin.extra.get("theme").ok_or_else(|| err("no [theme] section".into()))?;
        // Keys a theme manifest may not use, refused with what to do instead.
        for (key, instead) in [
            ("extends", "a theme stands alone; import the shared parts from kit/ instead of extending a theme"),
            ("screens", "a theme ships every screen, so it doesn't list them; remove `screens`"),
            ("components", "a theme replaces a kit file by shipping kit/<name>.slint; remove `components`"),
        ] {
            if section.get(key).is_some() {
                return Err(err(format!("`{key}` is not supported: {instead}")));
            }
        }
        let theme: ThemeSection =
            section.clone().try_into().map_err(|e: toml::de::Error| err(e.message().to_owned()))?;
        for name in theme.assets.images.keys() {
            if !is_identifier(name) {
                return Err(err(format!("image name {name:?} must be lowercase letters, digits and '-'")));
            }
        }
        Ok(Self { plugin: plugin.clone(), theme })
    }

    pub fn id(&self) -> &PluginId {
        &self.plugin.id
    }

    pub fn check_ui_api(&self, actual: &semver::Version) -> Result<(), ThemeError> {
        if self.theme.ui_api.matches(actual) {
            Ok(())
        } else {
            Err(ThemeError::UiApi {
                theme: self.id().clone(),
                required: self.theme.ui_api.to_string(),
                actual: actual.to_string(),
            })
        }
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An option's value, as the theme's `Options` global holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionValue {
    /// A choice's name.
    Choice(String),
    /// `0xRRGGBBAA`.
    Color(u32),
}

/// A theme's values with the token defaults and the player's options applied.
#[derive(Clone, Debug, PartialEq)]
pub struct MergedTheme {
    pub id: PluginId,
    pub tokens: BTreeMap<String, TokenValue>,
    pub images: BTreeMap<String, RelPath>,
    pub fonts: Vec<RelPath>,
    pub sounds: BTreeMap<String, RelPath>,
    /// Each system's pictures the theme sets in place of its emulator plugin's.
    pub system_art: BTreeMap<String, Art<RelPath>>,
    pub breakpoints: Vec<Breakpoint>,
    pub input: Vec<InputMode>,
    pub navigation: Navigation,
    pub options: BTreeMap<String, ThemeOption>,
    /// Every option's value: the player's, or the default. Themes read them from `Options`.
    pub option_values: BTreeMap<String, OptionValue>,
}

impl MergedTheme {
    pub fn breakpoint(&self, width: f32, height: f32, scale: f32) -> String {
        self.breakpoints.iter().find(|b| b.matches(width, height, scale)).map(|b| b.name.clone()).unwrap_or_default()
    }
}

/// `manifest`'s values with the player's option values applied. A token the theme leaves out
/// keeps its default.
pub fn merge(manifest: &ThemeManifest, option_values: &serde_json::Value) -> Result<MergedTheme, ThemeError> {
    let theme = manifest.id().clone();
    let section = &manifest.theme;
    let token_error =
        |token: &str, message: String| ThemeError::Token { theme: theme.clone(), token: token.to_owned(), message };

    let mut tokens: BTreeMap<String, TokenValue> =
        TOKENS.iter().map(|t| (t.name.to_owned(), t.default_value())).collect();
    for (name, value) in &section.tokens {
        let def = token(name).ok_or_else(|| token_error(name, "no such token".into()))?;
        tokens.insert(name.clone(), TokenValue::parse(def.kind, value).map_err(|m| token_error(name, m))?);
    }

    // Options the player set change tokens last. Unset options leave the theme's tokens, which
    // are its default look.
    let mut values = BTreeMap::new();
    for (name, option) in &section.options {
        if !is_slint_name(name) {
            return Err(ThemeError::Manifest {
                theme: theme.clone(),
                message: format!("option {name:?}: use lowercase letters, digits and dashes, starting with a letter"),
            });
        }
        values.insert(name.clone(), option_value(option, option_values.get(name).and_then(serde_json::Value::as_str)));
        let Some(chosen) = option_values.get(name).and_then(serde_json::Value::as_str) else { continue };
        match &option.kind {
            OptionKind::Color { token: name_of_token, .. } => {
                let def = token(name_of_token).ok_or_else(|| token_error(name_of_token, "no such token".into()))?;
                match TokenValue::parse(def.kind, &toml::Value::String(chosen.to_owned())) {
                    Ok(value) => {
                        tokens.insert(name_of_token.clone(), value);
                    }
                    // A bad value the player typed is ignored; the theme keeps its own token.
                    Err(message) => tracing::warn!(option = %name, %message, "theme option ignored"),
                }
            }
            OptionKind::Choice { choices, .. } => {
                let Some(set) = choices.get(chosen) else { continue };
                for (token_name, value) in set {
                    let def = token(token_name).ok_or_else(|| token_error(token_name, "no such token".into()))?;
                    tokens.insert(
                        token_name.clone(),
                        TokenValue::parse(def.kind, value).map_err(|m| token_error(token_name, m))?,
                    );
                }
            }
        }
    }

    Ok(MergedTheme {
        id: theme.clone(),
        tokens,
        images: section.assets.images.clone(),
        fonts: section.assets.fonts.clone(),
        sounds: section.assets.sounds.clone(),
        system_art: section.systems.clone(),
        breakpoints: section.breakpoints.clone(),
        input: if section.input.is_empty() { vec![InputMode::Pointer] } else { section.input.clone() },
        navigation: section.navigation.unwrap_or_default(),
        options: section.options.clone(),
        option_values: values,
    })
}

/// An option's value: `chosen` when it is valid, else the default.
fn option_value(option: &ThemeOption, chosen: Option<&str>) -> OptionValue {
    match &option.kind {
        OptionKind::Choice { default, choices } => {
            OptionValue::Choice(chosen.filter(|c| choices.contains_key(*c)).unwrap_or(default).to_owned())
        }
        OptionKind::Color { default, .. } => OptionValue::Color(
            chosen.and_then(crate::parse_color).or_else(|| crate::parse_color(default)).unwrap_or(0x000000ff),
        ),
    }
}

/// A Slint property name: a lowercase letter, then lowercase letters, digits and dashes.
fn is_slint_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The options as a JSON Schema, for the Settings screen.
pub fn options_schema(options: &BTreeMap<String, ThemeOption>) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> = options
        .iter()
        .filter(|(_, option)| !option.hidden)
        .map(|(name, option)| {
            let schema = match &option.kind {
                OptionKind::Color { default, .. } => {
                    serde_json::json!({"type": "string", "title": option.label, "format": "color", "default": default})
                }
                OptionKind::Choice { default, choices } => serde_json::json!({
                    "type": "string", "title": option.label, "default": default,
                    "enum": choices.keys().collect::<Vec<_>>(),
                }),
            };
            (name.clone(), schema)
        })
        .collect();
    serde_json::json!({"type": "object", "properties": properties})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, theme: &str) -> ThemeManifest {
        let text = format!(
            "id = \"{id}\"\nkind = \"theme\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
             capabilities = [\"theme\"]\n[theme]\nui_api = \"^0.2\"\n{theme}"
        );
        ThemeManifest::from_plugin(&PluginManifest::from_toml(&text).unwrap()).unwrap()
    }

    fn all_tokens() -> String {
        let mut s = String::from("[theme.tokens]\n");
        for t in TOKENS {
            let value = match t.kind {
                crate::TokenKind::Color => "\"#000000\"".to_owned(),
                crate::TokenKind::Text => "\"Sans\"".to_owned(),
                _ => "1".to_owned(),
            };
            s.push_str(&format!("{} = {value}\n", t.name));
        }
        s
    }

    #[test]
    fn the_old_inheritance_keys_are_refused_with_what_to_do() {
        for (key, line) in [
            ("extends", "extends = \"base\""),
            ("screens", "screens = [\"home\"]"),
            ("components", "components = [\"card\"]"),
        ] {
            let text = format!(
                "id = \"old\"\nkind = \"theme\"\nname = \"old\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
                 capabilities = [\"theme\"]\n[theme]\nui_api = \"^0.2\"\n{line}"
            );
            let err = ThemeManifest::from_plugin(&PluginManifest::from_toml(&text).unwrap()).unwrap_err();
            assert!(matches!(&err, ThemeError::Manifest { message, .. } if message.contains(key)), "{err}");
        }
    }

    #[test]
    fn a_token_the_theme_leaves_out_keeps_its_default() {
        let lonely = manifest("lonely", "[theme.tokens]\ncolor-bg = \"#000000\"");
        let merged = merge(&lonely, &serde_json::Value::Null).unwrap();
        assert_eq!(merged.tokens["color-bg"], TokenValue::Color(0x000000ff));
        assert_eq!(merged.tokens["color-surface"], TokenValue::Color(0x121a31ff), "Classic's surface");
        assert_eq!(merged.tokens.len(), TOKENS.len());
    }

    #[test]
    fn a_wrong_token_value_is_an_error_naming_it() {
        let wrong = manifest("wrong", "[theme.tokens]\ncolor-bg = \"blue\"");
        let err = merge(&wrong, &serde_json::Value::Null).unwrap_err();
        assert!(matches!(err, ThemeError::Token { ref token, .. } if token == "color-bg"), "{err}");
    }

    #[test]
    fn options_change_tokens_only_once_the_player_picks_a_value() {
        let base = manifest(
            "base",
            &format!(
                "{}\n[theme.options.accent]\nlabel = \"Accent\"\ntype = \"color\"\ntoken = \"color-accent\"\ndefault = \"#00ff00\"\n\
                 [theme.options.density]\nlabel = \"Density\"\ntype = \"choice\"\ndefault = \"cozy\"\n\
                 [theme.options.density.choices.cozy]\nspace-m = 16\n[theme.options.density.choices.compact]\nspace-m = 8\n",
                all_tokens()
            ),
        );
        // Unset: the theme's own tokens (all_tokens sets colors to black and lengths to 1).
        let defaults = merge(&base, &serde_json::Value::Null).unwrap();
        assert_eq!(defaults.tokens["color-accent"], TokenValue::Color(0x000000ff));
        assert_eq!(defaults.tokens["space-m"], TokenValue::Length(1.0));

        let picked = merge(&base, &serde_json::json!({"accent": "#0000ff", "density": "compact"})).unwrap();
        assert_eq!(picked.tokens["color-accent"], TokenValue::Color(0x0000ffff));
        assert_eq!(picked.tokens["space-m"], TokenValue::Length(8.0));

        let schema = options_schema(&picked.options);
        assert_eq!(schema["properties"]["density"]["enum"], serde_json::json!(["compact", "cozy"]));
    }

    #[test]
    fn a_hidden_option_is_not_in_the_settings_list_and_still_sets_its_tokens() {
        let base = manifest(
            "base",
            &format!(
                "{}\n[theme.options.density]\nlabel = \"Density\"\ntype = \"choice\"\ndefault = \"cozy\"\nhidden = true\n\
                 [theme.options.density.choices.cozy]\nspace-m = 16\n[theme.options.density.choices.compact]\nspace-m = 8\n",
                all_tokens()
            ),
        );
        let picked = merge(&base, &serde_json::json!({"density": "compact"})).unwrap();
        assert_eq!(picked.tokens["space-m"], TokenValue::Length(8.0), "it still works");
        assert!(options_schema(&picked.options)["properties"].as_object().unwrap().is_empty());
    }

    #[test]
    fn breakpoints_pick_the_first_match() {
        let base = manifest(
            "base",
            &format!(
                "{}\n[[theme.breakpoints]]\nname = \"tv\"\nmin_width = 1600\nmin_aspect = 1.5\n\
                 [[theme.breakpoints]]\nname = \"wide\"\nmin_width = 1000\n[[theme.breakpoints]]\nname = \"compact\"\n",
                all_tokens()
            ),
        );
        let merged = merge(&base, &serde_json::Value::Null).unwrap();
        assert_eq!(merged.breakpoint(1920.0, 1080.0, 1.0), "tv");
        assert_eq!(merged.breakpoint(1600.0, 1600.0, 1.0), "wide");
        assert_eq!(merged.breakpoint(800.0, 600.0, 2.0), "compact");
    }
}
