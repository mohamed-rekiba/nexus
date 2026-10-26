//! `plugin.toml` and `theme.toml`: one manifest format for every kind of plugin.

use std::collections::BTreeSet;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::{PluginId, RelPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Runs games: catalog, builds, settings, launching.
    Emulator,
    /// Code that adds to the games other plugins find, and runs none itself: a store's art
    /// and details, for example.
    Service,
    Theme,
    /// The machine's own settings and devices: network, VPNs, Bluetooth, sound, power, and on
    /// Nexus OS its updates and installer. See [`crate::SystemPlugin`].
    System,
}

/// A console an emulator plays, as its manifest names it: `PS5`, `PS4`, `PC`. It is kept in
/// uppercase, so `ps5` and `PS5` are one console. A name has ASCII letters, digits and `-`
/// only, so it is safe in a file name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Console(String);

impl Console {
    /// The console `name`, in any case. An error for an empty name or one with other
    /// characters.
    pub fn new(name: &str) -> Result<Self, String> {
        let name = name.trim();
        let plain = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !plain {
            return Err(format!("a console is ASCII letters, digits and -, not {name:?}"));
        }
        Ok(Self(name.to_ascii_uppercase()))
    }

    /// The name in uppercase: `PS5`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name of the console's catalog file, without `.json`: the name in lowercase, `ps5`.
    pub fn file_name(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl std::fmt::Display for Console {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Console {
    type Error = String;
    fn try_from(name: String) -> Result<Self, String> {
        Self::new(&name)
    }
}

impl From<Console> for String {
    fn from(console: Console) -> String {
        console.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// The platform this binary was built for, if the launcher supports it.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else {
            None
        }
    }
}

/// An extension point a plugin can provide. The manifest declares them; the host checks that
/// `init` provides exactly the declared set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Catalog,
    Downloader,
    Installer,
    Configurator,
    Launcher,
    Compatibility,
    Saves,
    Controllers,
    Shaders,
    Slots,
    GameInstaller,
    LaunchMiddleware,
    /// Describes games from a store's catalog: art, rating, genres, description. See
    /// [`crate::Metadata`].
    Metadata,
    /// Lists the games an emulator's catalog offers to download. See [`crate::Releases`].
    Releases,
    /// A theme manifest declares only this. The theme's files come from [`crate::ThemePlugin`],
    /// not from `init`, so no emulator can provide it.
    Theme,
    /// System plugins only, like the rest below: network and Wi-Fi. See [`crate::NetworkSettings`].
    Network,
    /// See [`crate::VpnSettings`].
    Vpn,
    /// See [`crate::BluetoothSettings`].
    Bluetooth,
    /// See [`crate::AudioSettings`].
    Audio,
    /// See [`crate::PowerSettings`].
    Power,
    /// See [`crate::OsUpdates`].
    OsUpdates,
    /// See [`crate::OsInstall`].
    OsInstall,
}

impl Capability {
    /// The capabilities only a system plugin provides.
    pub const SYSTEM: [Capability; 7] = [
        Capability::Network,
        Capability::Vpn,
        Capability::Bluetooth,
        Capability::Audio,
        Capability::Power,
        Capability::OsUpdates,
        Capability::OsInstall,
    ];
}

/// What a plugin may do outside its own data folder. The host enforces every entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Hosts the plugin may reach over HTTP: an exact name (`github.com`) or a suffix
    /// pattern (`*.github.com`, which also matches `github.com`).
    #[serde(default)]
    pub network: Vec<String>,
    /// Named host folders the plugin may read, and write if `write = true`. The player maps
    /// each name to real folders; the plugin reaches them through [`crate::Folders`].
    #[serde(default)]
    pub filesystem: Vec<PathGrant>,
    /// Programs inside the data folder the plugin may launch. `*` as the last part matches
    /// any file name, for example `builds/*/kyty`. Empty means the plugin may not spawn.
    #[serde(default)]
    pub process: Vec<String>,
    /// System plugins only: entries of `process` that may run with no time limit, their output
    /// read line by line ([`crate::Programs::stream`]), for example `system:bluetoothctl`.
    #[serde(default)]
    pub long_running: Vec<String>,
    /// System plugins only: entries of `process` that may run as the administrator through
    /// polkit (`pkexec`), which asks the player when its rules say so.
    #[serde(default)]
    pub polkit: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathGrant {
    /// The folder's name: lowercase letters, digits and `-`, for example `pc-games`. Plugins
    /// that ask for the same name share the player's mapping once the player sets one; until
    /// then each plugin uses its own `default`.
    pub folder: String,
    #[serde(default)]
    pub write: bool,
    /// Where the folder is until the player maps it. `~/` means the home folder. The shared
    /// folders ([`SHARED_FOLDERS`]) have the host's default.
    #[serde(default)]
    pub default: Option<DefaultFolder>,
    /// Another default on some systems, for example `macos = "~/Library/Application Support/X"`.
    #[serde(default)]
    pub default_on: std::collections::BTreeMap<Platform, String>,
    /// What Settings calls the folder, for example "Packages to install". The name if none.
    #[serde(default)]
    pub title: Option<String>,
    /// A line under the title in Settings.
    #[serde(default)]
    pub help: Option<String>,
    /// More folders, from a list in a default folder, while the player hasn't set the folder:
    /// Steam's own folder lists its libraries on other drives.
    #[serde(default)]
    pub listed_in: Option<ListedFolders>,
}

/// Folders a file lists, read by the host: each value of `key` in `file` (a path in the
/// folder), in Valve's text format (`"key" "value"`), that is an absolute path to a folder
/// holding `holds`. `{ file = "steamapps/libraryfolders.vdf", key = "path", holds =
/// "steamapps" }` are the libraries Steam lists. A listed folder is read only, as the grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedFolders {
    pub file: String,
    pub key: String,
    pub holds: String,
}

/// A grant's `default`: one folder, or a list of places to look.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DefaultFolder {
    /// `default = "~/Games"`: this folder, also when it doesn't exist yet.
    One(String),
    /// `default = ["~/KytyPS5", "~/src/KytyPS5"]`: each folder in the list that exists, in
    /// order. None when none exists.
    Search(Vec<String>),
}

/// The player's game folders, shared by every plugin that asks for it: games of any console,
/// each install in its console's folder inside the first one (`PS5`, `PS4`).
pub const GAMES_FOLDER: &str = "games";
/// Where the games folder is until the player maps it.
pub const GAMES_DEFAULT: &str = "~/Games";
/// The player's packages to install, shared by every plugin that installs them: each plugin
/// looks in its console's folder inside each one (`PS5`, `PS4`), as installs go to the games
/// folder.
pub const PACKAGES_FOLDER: &str = "packages";
/// Where the packages folder is until the player maps it.
pub const PACKAGES_DEFAULT: &str = "~/Downloads";

/// A folder the host owns and shows once, in Settings, for every plugin that asks for it:
/// its name, default, title and help line. A plugin's own default, title and help for it
/// are left out.
pub struct SharedFolder {
    pub name: &'static str,
    pub default: &'static str,
    pub title: &'static str,
    pub help: &'static str,
}

pub const SHARED_FOLDERS: [SharedFolder; 2] = [
    SharedFolder {
        name: GAMES_FOLDER,
        default: GAMES_DEFAULT,
        title: "Game folders",
        help: "Games from any console. Installs go to PS5 or PS4 inside the first folder. Separate folders with ;",
    },
    SharedFolder {
        name: PACKAGES_FOLDER,
        default: PACKAGES_DEFAULT,
        title: "Packages to install",
        help: "Game packages (.pkg) and disc images in a console's folder, for example PS5 or PS4, can be installed. Separate folders with ;",
    },
];

/// The shared folder named `name`, if it is one.
pub fn shared_folder(name: &str) -> Option<&'static SharedFolder> {
    SHARED_FOLDERS.iter().find(|f| f.name == name)
}

impl PathGrant {
    /// The one folder the grant means until the player maps it on `platform`: the grant's
    /// default for that system, its default, or the host's. `None` when the default is a
    /// list to search: see [`PathGrant::search_paths`].
    pub fn default_path(&self, platform: Option<Platform>) -> Option<&str> {
        let (own, searched) = match &self.default {
            Some(DefaultFolder::One(path)) => (Some(path.as_str()), false),
            Some(DefaultFolder::Search(_)) => (None, true),
            None => (None, false),
        };
        platform
            .and_then(|p| self.default_on.get(&p))
            .map(String::as_str)
            .or(own)
            .or_else(|| if searched { None } else { shared_folder(&self.folder).map(|f| f.default) })
    }

    /// The places to look for the folder until the player maps it, when the default is a
    /// list and [`PathGrant::default_path`] has none. Empty otherwise.
    pub fn search_paths(&self, platform: Option<Platform>) -> &[String] {
        match &self.default {
            Some(DefaultFolder::Search(paths)) if self.default_path(platform).is_none() => paths,
            _ => &[],
        }
    }

    pub fn valid_name(name: &str) -> bool {
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }
}

/// One part of a process rule: `*` matches any name, `*.AppImage` any name with that ending.
fn part_matches(rule: &str, part: &str) -> bool {
    rule == "*" || rule == part || rule.strip_prefix('*').is_some_and(|end| !end.is_empty() && part.ends_with(end))
}

impl Permissions {
    /// The grant for a named folder, if the manifest asks for it.
    pub fn folder(&self, name: &str) -> Option<&PathGrant> {
        self.filesystem.iter().find(|g| g.folder == name)
    }

    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.network.iter().any(|rule| {
            let rule = rule.to_ascii_lowercase();
            match rule.strip_prefix("*.") {
                Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
                None => host == rule,
            }
        })
    }

    /// Whether `permissions.process` lets the plugin start `program`:
    ///
    /// - in the data folder, a path pattern such as `builds/*/kyty` or `builds/*/*.AppImage`;
    /// - in a host folder, `folder:<name>` (any file in it), for a folder the manifest asks for;
    /// - on the system `PATH`, `system:<name>`, such as `system:wine`.
    pub fn allows_program(&self, program: &crate::Location) -> bool {
        use crate::Location;
        match program {
            Location::Data(path) => {
                let parts: Vec<&str> = path.as_str().split('/').collect();
                self.process.iter().filter(|rule| !rule.contains(':')).any(|rule| {
                    let rule_parts: Vec<&str> = rule.split('/').collect();
                    rule_parts.len() == parts.len() && rule_parts.iter().zip(&parts).all(|(r, p)| part_matches(r, p))
                })
            }
            Location::Folder(path) => {
                self.folder(&path.folder).is_some()
                    && self.process.iter().any(|rule| rule.strip_prefix("folder:") == Some(path.folder.as_str()))
            }
            Location::System(name) => {
                let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..";
                plain && self.process.iter().any(|rule| rule.strip_prefix("system:") == Some(name.as_str()))
            }
        }
    }

    /// Whether `program` may run long ([`Permissions::long_running`]).
    pub fn allows_long_running(&self, program: &crate::Location) -> bool {
        self.allows_program(program) && Self::listed(&self.long_running, program)
    }

    /// Whether `program` may run through polkit ([`Permissions::polkit`]).
    pub fn allows_polkit(&self, program: &crate::Location) -> bool {
        self.allows_program(program) && Self::listed(&self.polkit, program)
    }

    /// Whether a system program is named in `rules` (`system:<name>`).
    fn listed(rules: &[String], program: &crate::Location) -> bool {
        let crate::Location::System(name) = program else { return false };
        rules.iter().any(|rule| rule.strip_prefix("system:") == Some(name.as_str()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub id: PluginId,
    #[serde(default = "any_version")]
    pub version: VersionReq,
    /// An optional dependency is loaded first when present, and ignored when missing.
    #[serde(default)]
    pub optional: bool,
}

fn any_version() -> VersionReq {
    VersionReq::STAR
}

/// When the host calls `init`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    /// At boot.
    Eager,
    /// The first time the host needs one of the plugin's capabilities.
    #[default]
    Lazy,
}

/// How the plugin's code runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Runtime {
    /// Compiled into the host binary (first-party only).
    #[default]
    Native,
    /// No code: a data-only theme.
    Data,
}

/// What a plugin's games need from the computer (`[requires]`). Games won't start without
/// `vulkan` or `avx2`; below `recommended` they may run slowly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Requirements {
    /// The Vulkan version the graphics card must support, for example `"1.3"`.
    pub vulkan: Option<String>,
    /// The processor must have AVX2 (x86-64-v3). Not checked where the system can't say.
    pub avx2: bool,
    pub recommended: Recommended,
}

/// What a plugin's games need to run well (`[requires.recommended]`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Recommended {
    pub cpu_threads: Option<u32>,
    pub memory_gb: Option<f32>,
    pub gpu_memory_gb: Option<f32>,
    /// A graphics card of its own, not one built into the processor.
    pub discrete_gpu: bool,
}

/// One button of the console and the keyboard keys that press it: "Cross" and "J".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    pub button: String,
    pub keys: String,
}

/// The manifest shared by emulator and theme plugins.
///
/// Sections for one kind only (such as `[theme]`) are kept in `extra` and parsed by the crate
/// that owns them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub kind: PluginKind,
    /// What the player sees for the plugin: in the plugin list, on its page, and where a
    /// sentence means the console it plays. An emulator names its console: `PS5`.
    pub name: String,
    /// For an emulator, the name of the program it installs and runs, for example `KytyPS5`.
    /// Sentences about the program use it: its install, its builds, its first-play prompt.
    /// Empty when the program has no name of its own (PC). See [`PluginManifest::emulator_name`].
    #[serde(default)]
    pub emulator: String,
    pub version: Version,
    /// The `nexus-api` versions this plugin works with.
    pub host_api: VersionReq,
    /// The systems the plugin runs on. Empty means every system.
    #[serde(default)]
    pub platforms: Vec<Platform>,
    /// For an emulator, the console it plays, for example `PS5`, `PS4` or `PC`. A catalog
    /// plugin gives the emulator that console's catalog. `None` gives it no catalog. Not the
    /// systems the plugin runs on: see `platforms`.
    #[serde(default)]
    pub console: Option<Console>,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub runtime: Runtime,
    /// Folder of images and other files the plugin's slot data may refer to.
    #[serde(default)]
    pub assets: Option<RelPath>,
    /// JSON Schema of the plugin-wide settings, usually generated with `schemars`.
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    /// What the computer needs for the plugin's games: the host compares it with this PC.
    #[serde(default)]
    pub requires: Requirements,
    /// An emulator's keyboard keys for the console's buttons (`[[keymap]]`), in the order the
    /// launch splash lists them. Empty when the plugin lists none.
    #[serde(default)]
    pub keymap: Vec<KeyBinding>,
    /// For an emulator with a game installer: what it installs from, in one or two sentences.
    /// The install sheet shows it. Empty when the plugin says nothing.
    #[serde(default)]
    pub install_note: String,
    /// The system the plugin's games show under (`[system]`): what the Library's chips and a
    /// card's badge call it, and its color. Themes draw it from this, not from the system's code.
    #[serde(default)]
    pub system: Option<SystemLook>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// A system's look: `id` is the code a game's `platform` has (`PS5`, `PC`, `Steam`), `name` its
/// full name ("PlayStation 5"), and `color` its color as `#rrggbb`. Without a color a theme
/// draws the system in its neutral look.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemLook {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

impl SystemLook {
    /// `color` as red, green and blue; `None` without one, or when it isn't `#rrggbb`.
    pub fn rgb(&self) -> Option<[u8; 3]> {
        let hex = self.color.as_deref()?.strip_prefix('#').filter(|h| h.len() == 6)?;
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        Some([byte(0)?, byte(2)?, byte(4)?])
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("invalid manifest: {0}")]
pub struct ManifestError(pub String);

impl PluginManifest {
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|e| ManifestError(e.message().to_owned()))
    }

    pub fn supports(&self, platform: Platform) -> bool {
        self.platforms.is_empty() || self.platforms.contains(&platform)
    }

    /// The name of the program the plugin runs: `emulator`, or `name` when it has none.
    pub fn emulator_name(&self) -> &str {
        if self.emulator.is_empty() { &self.name } else { &self.emulator }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KYTY: &str = r#"
        id = "emu-kyty"
        kind = "emulator"
        name = "Native example"
        version = "0.1.0"
        host_api = "^0.1"
        platforms = ["linux", "macos"]
        capabilities = ["catalog", "launcher"]

        [permissions]
        network = ["*.github.com"]
        process = ["builds/*/kyty"]

        [[dependencies]]
        id = "firmware"
        version = ">=1.0"
        optional = true

        [theme]
        extends = "base"
    "#;

    #[test]
    fn parses_every_section_and_keeps_kind_specific_ones() {
        let m = PluginManifest::from_toml(KYTY).unwrap();
        assert_eq!(m.id.as_str(), "emu-kyty");
        assert_eq!(m.kind, PluginKind::Emulator);
        assert!(m.capabilities.contains(&Capability::Launcher));
        assert_eq!(m.dependencies[0].id.as_str(), "firmware");
        assert!(m.dependencies[0].optional);
        assert_eq!(m.activation, Activation::Lazy);
        assert_eq!(m.runtime, Runtime::Native);
        assert!(m.extra.contains_key("theme"));
        assert!(m.supports(Platform::Linux));
        assert!(!m.supports(Platform::Windows));
    }

    #[test]
    fn an_emulator_may_list_its_keyboard_keys_in_order() {
        let m = PluginManifest::from_toml(&format!(
            "{KYTY}\n[[keymap]]\nbutton = \"Cross\"\nkeys = \"J\"\n[[keymap]]\nbutton = \"Circle\"\nkeys = \"L\"\n"
        ))
        .unwrap();
        let keys: Vec<_> = m.keymap.iter().map(|k| (k.button.as_str(), k.keys.as_str())).collect();
        assert_eq!(keys, [("Cross", "J"), ("Circle", "L")]);
        assert!(!m.extra.contains_key("keymap"), "a field of its own");
        assert!(PluginManifest::from_toml(KYTY).unwrap().keymap.is_empty(), "none by default");
        let wrong = format!("{KYTY}\n[[keymap]]\nbutton = \"Cross\"\nkey = \"J\"\n");
        assert!(PluginManifest::from_toml(&wrong).is_err(), "a misspelt field is an error");
    }

    #[test]
    fn an_emulator_may_say_what_it_installs_from() {
        let m = PluginManifest::from_toml(&format!("install_note = \"Installs from a folder.\"\n{KYTY}")).unwrap();
        assert_eq!(m.install_note, "Installs from a folder.");
        assert!(!m.extra.contains_key("install_note"), "a field of its own");
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().install_note, "", "none by default");
    }

    #[test]
    fn an_emulator_may_name_its_software_apart_from_the_plugin() {
        let m = PluginManifest::from_toml(&format!("emulator = \"KytyPS5\"\n{KYTY}")).unwrap();
        assert_eq!(m.emulator, "KytyPS5");
        assert_eq!(m.emulator_name(), "KytyPS5");
        assert_eq!(m.name, "Native example", "the plugin keeps its own name");
        assert!(!m.extra.contains_key("emulator"), "a field of its own");
        let plain = PluginManifest::from_toml(KYTY).unwrap();
        assert_eq!(plain.emulator, "", "none by default");
        assert_eq!(plain.emulator_name(), "Native example", "without one, the plugin's name");
    }

    #[test]
    fn an_emulator_names_its_console_in_any_case() {
        let with = |console: &str| {
            PluginManifest::from_toml(&format!(
                "id = \"emu\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
                 console = \"{console}\"\n"
            ))
        };
        let ps5 = with("ps5").unwrap().console.unwrap();
        assert_eq!(ps5, Console::new("PS5").unwrap(), "one console in either case");
        assert_eq!((ps5.as_str(), ps5.file_name().as_str()), ("PS5", "ps5"));
        assert!(with("../ps5").is_err(), "letters, digits and dashes only");
        assert!(Console::new(" ").is_err());
        assert_eq!(PluginManifest::from_toml(KYTY).unwrap().console, None);
    }

    #[test]
    fn folder_and_system_rules_need_their_own_pattern() {
        use crate::{HostPath, Location};
        let m = PluginManifest::from_toml(
            "id = \"pc\"\nkind = \"emulator\"\nname = \"PC\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
             [permissions]\nprocess = [\"folder:pc-games\", \"folder:not-granted\", \"system:wine\"]\n\
             [[permissions.filesystem]]\nfolder = \"pc-games\"",
        )
        .unwrap();
        let p = &m.permissions;
        let in_folder = |name: &str| Location::Folder(HostPath::new(name, 0, RelPath::new("Game/start.sh").unwrap()));
        assert!(p.allows_program(&in_folder("pc-games")));
        assert!(!p.allows_program(&in_folder("not-granted")), "the folder itself must be granted");
        assert!(!p.allows_program(&in_folder("other")));
        assert!(p.allows_program(&Location::System("wine".into())));
        assert!(!p.allows_program(&Location::System("umu-run".into())));
        assert!(!p.allows_program(&Location::System("/usr/bin/wine".into())), "a name, never a path");
        assert!(
            !p.allows_program(&Location::Data(RelPath::new("pc-games/Game/start.sh").unwrap())),
            "folder rules match no data path"
        );
    }

    #[test]
    fn network_and_process_rules_match_only_what_they_name() {
        let m = PluginManifest::from_toml(KYTY).unwrap();
        let p = &m.permissions;
        assert!(p.allows_host("github.com"));
        assert!(p.allows_host("api.GitHub.com"));
        assert!(!p.allows_host("evilgithub.com"));
        assert!(!p.allows_host("github.com.evil.net"));
        let data = |p: &str| crate::Location::Data(RelPath::new(p).unwrap());
        assert!(p.allows_program(&data("builds/1.2/kyty")));
        assert!(!p.allows_program(&data("builds/1.2/other")));
        assert!(!p.allows_program(&data("builds/1.2/x/kyty")));
        assert!(part_matches("*.AppImage", "Shadps4-sdl.AppImage"));
        assert!(!part_matches("*.AppImage", "Shadps4-sdl.AppImage.sh"));
        assert!(!part_matches("*.AppImage", ".AppImage.x"));
    }

    #[test]
    fn a_folder_default_is_one_path_or_a_list_to_search() {
        let grant = |default: &str| {
            let m = PluginManifest::from_toml(&format!(
                "id = \"emu\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
                 [[permissions.filesystem]]\nfolder = \"emu-build\"\n{default}\n"
            ))
            .unwrap();
            m.permissions.filesystem[0].clone()
        };
        let one = grant("default = \"~/Emu\"");
        assert_eq!(one.default_path(None), Some("~/Emu"));
        assert!(one.search_paths(None).is_empty());

        let list = grant("default = [\"~/Emu\", \"~/src/Emu\"]\ndefault_on = { macos = \"~/Applications/Emu\" }");
        assert_eq!(list.default_path(None), None);
        assert_eq!(list.search_paths(Some(Platform::Linux)), ["~/Emu", "~/src/Emu"]);
        assert_eq!(list.default_path(Some(Platform::Macos)), Some("~/Applications/Emu"), "a system's own default wins");
        assert!(list.search_paths(Some(Platform::Macos)).is_empty());

        assert_eq!(grant("").default_path(None), None);
    }

    #[test]
    fn rejects_bad_ids_and_bad_versions() {
        let bad_id = KYTY.replace("\"emu-kyty\"", "\"Emu Native\"");
        assert!(PluginManifest::from_toml(&bad_id).is_err());
        let bad_req = KYTY.replace("\"^0.1\"", "\"not a range\"");
        assert!(PluginManifest::from_toml(&bad_req).is_err());
    }
}
