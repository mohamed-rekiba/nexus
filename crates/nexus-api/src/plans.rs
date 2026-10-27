//! Downloads, installs and launches. Plugins describe the work as data; the host does it.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{HostPath, PluginResult, RelPath};

/// What the host asks a [`Downloader`] to plan. One kind for now; others come when a plugin
/// needs them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DownloadRequest {
    /// An emulator build. `None` means the newest one.
    EmulatorBuild { version: Option<String> },
}

/// The files a download needs. The host fetches them, resumes them, checks them and writes
/// them inside the plugin's data folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadPlan {
    /// Shown in the Downloads screen.
    pub label: String,
    pub artifacts: Vec<Artifact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Lowercase hex SHA-256 of the file. The host refuses the file if it doesn't match.
    pub sha256: String,
    #[serde(default)]
    pub signature: Option<Signature>,
    /// Where the file goes, inside the plugin's data folder.
    pub dest: RelPath,
    /// Unpack the file after the checks, then delete it.
    #[serde(default)]
    pub unpack: Option<Unpack>,
}

/// An archive the host unpacks into a folder of the plugin's data folder, after the file
/// passed its checks. Paths that would leave the folder are refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unpack {
    pub format: ArchiveFormat,
    /// The folder it goes into. Whatever was there before is replaced.
    pub into: RelPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    TarGz,
    Zip,
    /// A zip that holds one AppImage. The host takes it out and runs its
    /// `--appimage-extract` (Linux only), so `into` gets its `squashfs-root` folder.
    AppImageInZip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignatureAlgorithm {
    Ed25519,
}

/// A detached signature over the file's bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub algorithm: SignatureAlgorithm,
    /// Hex.
    pub public_key: String,
    /// Hex.
    pub signature: String,
}

/// A file the host downloaded and checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadedArtifact {
    pub dest: RelPath,
    pub sha256: String,
    pub size: u64,
    /// The folder it was unpacked into, when the artifact asked for that. The file itself is
    /// then gone.
    #[serde(default)]
    pub unpacked: Option<RelPath>,
}

/// Plans downloads of emulator builds.
#[async_trait]
pub trait Downloader: Send + Sync {
    async fn plan(&self, request: &DownloadRequest) -> PluginResult<DownloadPlan>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Sorts versions newest first, so "0.10.0" comes before "0.9.0". Versions that parse as
/// semver come first, in semver order; the others follow, by text.
pub fn newest_first(versions: &mut [String]) {
    use std::cmp::Ordering;
    versions.sort_by(|a, b| match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => b.cmp(&a),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => b.cmp(a),
    });
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledVersion {
    pub version: String,
    /// The install folder, inside the plugin's data folder.
    pub path: RelPath,
    /// A build the player brought (their own build), not one the plugin installed. The host
    /// never updates it by itself; the player switches to an official build.
    #[serde(default)]
    pub own: bool,
}

/// Installs, updates and removes emulator versions, using the plugin's scoped storage.
///
/// The host runs the workflow: it asks the [`Downloader`] for a plan, downloads and checks
/// every file, then calls `install` or `update` with the files.
#[async_trait]
pub trait Installer: Send + Sync {
    /// Versions the plugin can install.
    async fn available(&self) -> PluginResult<Vec<VersionInfo>>;

    async fn installed(&self) -> PluginResult<Vec<InstalledVersion>>;

    async fn install(&self, version: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion>;

    async fn uninstall(&self, version: &str) -> PluginResult<()>;

    /// Replaces `from` with `to`. By default: install `to`, then uninstall `from`, so a failed
    /// install leaves `from` in place.
    async fn update(&self, from: &str, to: &str, files: &[DownloadedArtifact]) -> PluginResult<InstalledVersion> {
        let installed = self.install(to, files).await?;
        self.uninstall(from).await?;
        Ok(installed)
    }
}

/// The process to start for a game. The host spawns it only if the manifest's
/// `permissions.process` allows `program`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub program: Location,
    #[serde(default)]
    pub args: Vec<LaunchArg>,
    /// Values are text or paths, like arguments.
    #[serde(default)]
    pub env: BTreeMap<String, LaunchArg>,
    /// `None` means the program's folder. A system program needs one.
    #[serde(default)]
    pub cwd: Option<Location>,
    /// For a program that hands the game to another one and may exit at once (a store's
    /// client, as `steam steam://rungameid/…`): text in the command line of the game's own
    /// process. The game is then that process and every process below it, from when it shows
    /// until it ends; the program started here is neither waited for nor stopped. `None`: the
    /// game is the program started here.
    #[serde(default)]
    pub game_process: Option<String>,
}

/// Where a program or folder is. As JSON, a bare string is a path in the data folder, and
/// the others are `{"folder": {...}}` and `{"system": "wine"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "LocationRepr", into = "LocationRepr")]
pub enum Location {
    /// In the plugin's data folder.
    Data(RelPath),
    /// In a host folder the manifest asks for.
    Folder(HostPath),
    /// A program the host finds on the system's `PATH`, by file name.
    System(String),
}

impl From<RelPath> for Location {
    fn from(path: RelPath) -> Self {
        Self::Data(path)
    }
}

impl From<HostPath> for Location {
    fn from(path: HostPath) -> Self {
        Self::Folder(path)
    }
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(path) => write!(f, "{path}"),
            Self::Folder(p) => write!(f, "{}[{}]/{}", p.folder, p.root, p.path),
            Self::System(name) => write!(f, "{name} (system)"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum LocationRepr {
    Bare(RelPath),
    Tagged(TaggedLocation),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaggedLocation {
    Data(RelPath),
    Folder(HostPath),
    System(String),
}

impl From<LocationRepr> for Location {
    fn from(repr: LocationRepr) -> Self {
        match repr {
            LocationRepr::Bare(path) | LocationRepr::Tagged(TaggedLocation::Data(path)) => Self::Data(path),
            LocationRepr::Tagged(TaggedLocation::Folder(path)) => Self::Folder(path),
            LocationRepr::Tagged(TaggedLocation::System(name)) => Self::System(name),
        }
    }
}

impl From<Location> for LocationRepr {
    fn from(location: Location) -> Self {
        match location {
            Location::Data(path) => Self::Bare(path),
            Location::Folder(path) => Self::Tagged(TaggedLocation::Folder(path)),
            Location::System(name) => Self::Tagged(TaggedLocation::System(name)),
        }
    }
}

/// One argument: text, or a path the host turns into a real path. As JSON, text is a bare
/// string and a path is `{"path": <location>}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LaunchArg {
    Text(String),
    Path { path: Location },
}

impl LaunchArg {
    pub fn path(path: impl Into<Location>) -> Self {
        Self::Path { path: path.into() }
    }
}

impl From<&str> for LaunchArg {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for LaunchArg {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&String> for LaunchArg {
    fn from(text: &String) -> Self {
        Self::Text(text.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub success: bool,
    pub duration_ms: u64,
}

/// Builds launch commands. The host starts, watches and stops the process.
#[async_trait]
pub trait Launcher: Send + Sync {
    /// `config` is the game's merged settings (emulator settings, then game settings).
    async fn prepare(&self, game: &str, config: &serde_json::Value) -> PluginResult<LaunchSpec>;

    async fn on_exit(&self, game: &str, exit: &ExitInfo) -> PluginResult<()> {
        let _ = (game, exit);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn newest_first_compares_versions_not_text() {
        let mut v = vec!["0.9.0".to_owned(), "0.10.0".to_owned(), "nightly".to_owned()];
        super::newest_first(&mut v);
        assert_eq!(v, ["0.10.0", "0.9.0", "nightly"]);
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;

    #[test]
    fn a_bare_string_is_a_data_path_and_other_locations_are_tagged() {
        let old: LaunchSpec = serde_json::from_str(r#"{"program": "builds/1/emu", "args": ["G0"]}"#).unwrap();
        assert_eq!(old.program, Location::Data(RelPath::new("builds/1/emu").unwrap()));
        assert_eq!(old.args, [LaunchArg::Text("G0".into())]);

        let game = HostPath::new("pc-games", 1, RelPath::new("Game/game.exe").unwrap());
        let spec = LaunchSpec {
            program: Location::System("wine".into()),
            args: vec![LaunchArg::path(game.clone()), "--fullscreen".into()],
            env: BTreeMap::new(),
            cwd: Some(Location::Folder(game.clone())),
            game_process: None,
        };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["program"], serde_json::json!({"system": "wine"}));
        assert_eq!(json["args"][0]["path"]["folder"]["folder"], "pc-games");
        assert_eq!(json["args"][1], "--fullscreen");
        let data = serde_json::to_value(Location::Data(RelPath::new("a/b").unwrap())).unwrap();
        assert_eq!(data, "a/b", "data paths stay bare strings on the wire");
        assert_eq!(serde_json::from_value::<LaunchSpec>(json).unwrap(), spec);
    }
}
