//! `theme.toml`: the `[theme]` section, and merging a theme with the themes it extends.

use std::collections::BTreeMap;

use nexus_api::{PluginId, PluginManifest, RelPath};
use semver::VersionReq;
use serde::{Deserialize, Serialize};

use crate::tokens::{TOKENS, TokenValue, token};
use crate::{ComponentId, ScreenId};

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
    /// The parent theme. Missing screens, components, tokens and assets come from it.
    #[serde(default)]
    pub extends: Option<PluginId>,
    /// Screens this theme defines itself.
    #[serde(default)]
    pub screens: Vec<ScreenId>,
    /// Components this theme defines itself.
    #[serde(default)]
    pub components: Vec<ComponentId>,
    /// Input devices the theme supports, the main one first.
    #[serde(default)]
    pub input: Vec<InputMode>,
    #[serde(default)]
    pub navigation: Option<Navigation>,
    #[serde(default)]
    pub tokens: BTreeMap<String, toml::Value>,
    /// Replaces the parent's breakpoints when not empty.
    #[serde(default)]
    pub breakpoints: Vec<Breakpoint>,
    #[serde(default)]
    pub assets: ThemeAssets,
    #[serde(default)]
    pub options: BTreeMap<String, ThemeOption>,
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
    #[error("theme {theme} extends {parent}, which isn't installed")]
    MissingParent { theme: PluginId, parent: PluginId },
    #[error("theme {theme} extends itself through {chain:?}")]
    Cycle { theme: PluginId, chain: Vec<PluginId> },
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

/// A file a merged theme refers to, with the chain layer it comes from (0 = the theme itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerPath {
    pub layer: usize,
    pub path: RelPath,
}

/// An option's value, as the theme's `Options` global holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionValue {
    /// A choice's name.
    Choice(String),
    /// `0xRRGGBBAA`.
    Color(u32),
}

/// A theme with everything it inherits filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct MergedTheme {
    pub id: PluginId,
    /// The theme first, then its parent, up to the root theme.
    pub chain: Vec<PluginId>,
    pub tokens: BTreeMap<String, TokenValue>,
    pub images: BTreeMap<String, LayerPath>,
    pub fonts: Vec<LayerPath>,
    pub sounds: BTreeMap<String, LayerPath>,
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

/// Merges `chain` (the theme first, then its parents) and applies the player's option values.
/// Every token must end up with a value.
pub fn merge_chain(chain: &[&ThemeManifest], option_values: &serde_json::Value) -> Result<MergedTheme, ThemeError> {
    let theme = chain.first().expect("a chain has at least the theme itself").id().clone();
    let token_error =
        |token: &str, message: String| ThemeError::Token { theme: theme.clone(), token: token.to_owned(), message };

    let mut tokens = BTreeMap::new();
    let mut images = BTreeMap::new();
    let mut sounds = BTreeMap::new();
    let mut fonts = Vec::new();
    let mut breakpoints = Vec::new();
    // Root theme first, so children overwrite.
    for (layer, manifest) in chain.iter().enumerate().rev() {
        let section = &manifest.theme;
        for (name, value) in &section.tokens {
            let def = token(name).ok_or_else(|| token_error(name, "no such token".into()))?;
            tokens.insert(name.clone(), TokenValue::parse(def.kind, value).map_err(|m| token_error(name, m))?);
        }
        for (name, path) in &section.assets.images {
            images.insert(name.clone(), LayerPath { layer, path: path.clone() });
        }
        for (name, path) in &section.assets.sounds {
            sounds.insert(name.clone(), LayerPath { layer, path: path.clone() });
        }
        fonts.extend(section.assets.fonts.iter().map(|path| LayerPath { layer, path: path.clone() }));
        if !section.breakpoints.is_empty() {
            breakpoints = section.breakpoints.clone();
        }
    }

    // Options the player set change tokens last. Unset options leave the chain's tokens,
    // which are the theme's default look.
    let options = merge_options(chain);
    let mut values = BTreeMap::new();
    for (name, option) in &options {
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

    if let Some(missing) = TOKENS.iter().find(|t| !tokens.contains_key(t.name)) {
        return Err(token_error(missing.name, "no theme in the chain sets it".into()));
    }

    let first = |f: fn(&ThemeSection) -> Option<Navigation>| chain.iter().find_map(|m| f(&m.theme));
    Ok(MergedTheme {
        id: theme.clone(),
        chain: chain.iter().map(|m| m.id().clone()).collect(),
        tokens,
        images,
        fonts,
        sounds,
        breakpoints,
        input: chain
            .iter()
            .map(|m| &m.theme.input)
            .find(|i| !i.is_empty())
            .cloned()
            .unwrap_or(vec![InputMode::Pointer]),
        navigation: first(|t| t.navigation).unwrap_or_default(),
        options,
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

/// The options of a chain (the theme first), the theme's own winning.
pub fn merge_options(chain: &[&ThemeManifest]) -> BTreeMap<String, ThemeOption> {
    let mut options = BTreeMap::new();
    for manifest in chain.iter().rev() {
        options.extend(manifest.theme.options.clone());
    }
    options
}

/// The options as a JSON Schema, for the Settings screen.
pub fn options_schema(options: &BTreeMap<String, ThemeOption>) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> = options
        .iter()
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
             capabilities = [\"theme\"]\n[theme]\nui_api = \"^0.1\"\n{theme}"
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
    fn a_child_overrides_tokens_and_inherits_the_rest() {
        let base = manifest("base", &all_tokens());
        let child = manifest("child", "extends = \"base\"\n[theme.tokens]\ncolor-accent = \"#ff0000\"");
        let merged = merge_chain(&[&child, &base], &serde_json::Value::Null).unwrap();
        assert_eq!(merged.tokens["color-accent"], TokenValue::Color(0xff0000ff));
        assert_eq!(merged.tokens["color-bg"], TokenValue::Color(0x000000ff));
        assert_eq!(merged.chain, [PluginId::new("child").unwrap(), PluginId::new("base").unwrap()]);
    }

    #[test]
    fn a_chain_missing_a_token_is_an_error_naming_it() {
        let lonely = manifest("lonely", "[theme.tokens]\ncolor-bg = \"#000000\"");
        let err = merge_chain(&[&lonely], &serde_json::Value::Null).unwrap_err();
        assert!(matches!(err, ThemeError::Token { ref token, .. } if token == "color-surface"), "{err}");
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
        let defaults = merge_chain(&[&base], &serde_json::Value::Null).unwrap();
        assert_eq!(defaults.tokens["color-accent"], TokenValue::Color(0x000000ff));
        assert_eq!(defaults.tokens["space-m"], TokenValue::Length(1.0));

        let picked = merge_chain(&[&base], &serde_json::json!({"accent": "#0000ff", "density": "compact"})).unwrap();
        assert_eq!(picked.tokens["color-accent"], TokenValue::Color(0x0000ffff));
        assert_eq!(picked.tokens["space-m"], TokenValue::Length(8.0));

        let schema = options_schema(&picked.options);
        assert_eq!(schema["properties"]["density"]["enum"], serde_json::json!(["compact", "cozy"]));
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
        let merged = merge_chain(&[&base], &serde_json::Value::Null).unwrap();
        assert_eq!(merged.breakpoint(1920.0, 1080.0, 1.0), "tv");
        assert_eq!(merged.breakpoint(1600.0, 1600.0, 1.0), "wide");
        assert_eq!(merged.breakpoint(800.0, 600.0, 2.0), "compact");
    }
}
