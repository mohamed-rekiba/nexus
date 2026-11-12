# Writing a plugin

This guide is for developers who add an emulator, a game source or a service to Nexus. It
covers the plugin contract (`crates/nexus-api`), how the host loads and checks a plugin, the
manifest, each capability, folders and permissions, settings, the shared `emulator-kit`
library, registration, and tests.

Themes are plugins too, but they have no code. Read [themes.md](themes.md) to write one. The
layers around plugins are in [architecture.md](architecture.md). The build and check commands
are in [development.md](development.md). Terms are in [GLOSSARY.md](../GLOSSARY.md).

## Contents

- [What a plugin is](#what-a-plugin-is)
- [Built in or on disk](#built-in-or-on-disk)
- [The crate](#the-crate)
- [The manifest](#the-manifest)
- [Lifecycle](#lifecycle)
- [The plugin context](#the-plugin-context)
- [Permissions and folders](#permissions-and-folders)
- [Capabilities](#capabilities)
- [Backup](#backup)
- [Settings and the plugin page](#settings-and-the-plugin-page)
- [Errors](#errors)
- [Shared code: emulator-kit](#shared-code-emulator-kit)
- [Register a built-in plugin](#register-a-built-in-plugin)
- [Testing](#testing)
- [The built-in plugins](#the-built-in-plugins)

## What a plugin is

A plugin is a Rust type that gives the host its manifest. The host reads the manifest, checks
it, and then calls the plugin only through the traits in `nexus-api`. A plugin depends on
`nexus-api` only. That crate re-exports `async_trait`, `serde`, `serde_json`, `schemars`,
`semver` and `inventory`, so a plugin needs no other crate for them.

The traits, in `crates/nexus-api/src/plugin.rs`:

- `Plugin`: every plugin has `manifest()` and an optional `shutdown()`.
- `CodePlugin`: a plugin with code. Its `init(ctx)` returns the capabilities it provides, as a
  `Provided` value.
- `ThemePlugin`: a theme. Its `files()` returns its files. It has no `init`.
- `PluginBox`: a plugin as the host registers it: `PluginBox::Code` or `PluginBox::Theme`.

### Kinds

The manifest's `kind` is one of four values (`PluginKind`):

| `kind` | What it is |
| --- | --- |
| `emulator` | Runs games: catalog, builds, settings, launch. |
| `service` | Adds to the games other plugins find, and runs none itself, for example a store's art and details. |
| `theme` | Files only: no code and no `init`. See [themes.md](themes.md). |
| `system` | The machine's own settings and devices: network, VPNs, Bluetooth, sound, power, and on Nexus OS its updates and installer. |

The kind decides only privileges. Any plugin with code may provide any data capability
(`catalog`, `releases`, `metadata`, `slots`, `configurator`, `backup` and the rest). Only a
`system` plugin may:

- provide the system capabilities (`Capability::SYSTEM`: `network`, `vpn`, `bluetooth`,
  `audio`, `power`, `os_updates`, `os_install`);
- run programs with no time limit (`permissions.long_running`);
- run programs as the administrator through polkit (`permissions.polkit`).

## Built in or on disk

A plugin with code is always built into the Nexus binary. A plugin found on disk can only be a
theme. The host has no runtime for third-party code.

The app builds its kernel in `crates/nexus-app/src/main.rs`:

```rust
Kernel::builder(host)
    // ...
    .built_in_plugins()
    .discover(&extensions)
    .discover(&args::bundled_extensions())
    .boot()
```

- `built_in_plugins()` adds every plugin that registers a `PluginFactory` in a crate linked into
  the binary. These have the origin `BuiltIn`.
- `discover(dir)` treats each subfolder of `dir` with a `plugin.toml` or a `theme.toml` as a
  third-party plugin. The folders are `--extensions <path>` (default
  `<data-dir>/extensions`) and the `extensions` folder next to the binary (in a macOS app
  bundle, `../Resources/extensions`).

Three rules in `crates/nexus-core/src/kernel.rs` close the door to code on disk:

1. `validate()` refuses a third-party manifest with `runtime` `native`: "third-party plugins
   must be data-only".
2. `validate()` refuses a non-theme manifest with `runtime` `data`: "… plugin needs code".
3. The only loader for a folder is `DataThemeLoader` (`crates/nexus-core/src/themes.rs`). It
   accepts only `kind = "theme"` with `runtime` `data`.

So a plugin on disk is a theme whose manifest says `[runtime]` `type = "data"`. In safe mode
(`--safe-mode`), the host refuses every third-party plugin.

## The crate

A built-in code plugin is one crate under `plugins/<group>/<name>/`, with its `plugin.toml`
beside its `Cargo.toml`. This sketch follows `plugins/emulators/pc/src/lib.rs`:

```rust
use std::sync::Arc;

use nexus_api::*;

const MANIFEST: &str = include_str!("../plugin.toml");

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}

#[async_trait]
impl CodePlugin for MyPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let me = Arc::new(My { ctx });
        Ok(Provided {
            catalog: Some(me.clone()),
            launcher: Some(me),
            ..Provided::default()
        })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(MyPlugin)) }
```

`Provided` must hold exactly the capabilities the manifest's `capabilities` list declares. If
not, the host calls `shutdown`, then rejects the plugin with `CapabilityMismatch`.

`Provided` also has an `assets` field: read-only files (`Arc<dyn Files>`) that slot items and
catalog entries point at as `ImageSource::Asset`. The macro `nexus_api::system_art!()` serves
the crate's `assets/icon.png` and `assets/cover.jpg` as `icon.png` and `cover.jpg`, the names a
manifest's `[system]` uses. `scripts/make-system-art.sh` makes those files.

## The manifest

The manifest is `plugin.toml` for a plugin with code and `theme.toml` for a theme. Both use one
format: `PluginManifest` in `crates/nexus-api/src/manifest.rs`. The host parses it with
`PluginManifest::from_toml`.

A top-level key the parser doesn't know is not an error: it goes into `extra`, the table of
sections for one kind only (a theme's `[theme]`, parsed by `nexus-ui-api`). So a misspelt
top-level key is ignored. The sections below refuse unknown keys: `[permissions]`,
`[[permissions.filesystem]]`, `[[dependencies]]`, `[runtime]`, `[requires]`,
`[requires.recommended]`, `[[keymap]]` and `[system]`.

The PS5 plugin's manifest, shortened (`plugins/emulators/kytyps5/plugin.toml`):

```toml
id = "emu-kyty"
kind = "emulator"
name = "PS5"
emulator = "KytyPS5"
version = "0.1.0"
host_api = "^0.2"
platforms = ["linux", "macos", "windows"]
console = "PS5"
capabilities = ["catalog", "downloader", "installer", "configurator", "launcher", "compatibility", "slots", "game_installer", "backup"]
install_note = "Installs a PS5 game from its folder, an exFAT image or an unsigned PS5 package (.pkg). ..."

[[keymap]]
button = "Cross"
keys = "J"

[requires]
vulkan = "1.3"

[requires.recommended]
cpu_threads = 12
memory_gb = 14.5
gpu_memory_gb = 7.5
discrete_gpu = true

[permissions]
network = ["api.github.com", "github.com", "*.githubusercontent.com", "kytyps5.github.io"]
process = ["builds/*/kyty_emulator", "folder:kyty-build", "system:kyty_emulator"]

[[permissions.filesystem]]
folder = "games"
write = true

[[permissions.filesystem]]
folder = "kyty-data"
default = "~/.local/share/KytyPS5"
default_on = { macos = "~/Library/Application Support/KytyPS5", windows = "~/AppData/Roaming/KytyPS5" }
title = "KytyPS5 saves"
help = "Saves of every KytyPS5 build that has --data-folder."
write = true

[system]
id = "PS5"
name = "PlayStation 5"
icon = "icon.png"
cover = "cover.jpg"
```

### Top-level keys

| Key | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The plugin's id: 1 to 64 lowercase ASCII letters, digits or `-`, starting with a letter. |
| `kind` | yes | `emulator`, `service`, `theme` or `system`. |
| `name` | yes | What the player sees: in the plugin list, on the plugin page, and where a sentence means the console. An emulator names its console: `PS5`. |
| `emulator` | no | For an emulator, the name of the program it installs and runs, such as `KytyPS5`. Sentences about the program use it. Empty when the program has no name of its own; then `name` is used. |
| `version` | yes | The plugin's semver version. |
| `host_api` | yes | A semver range of the API versions the plugin works with, such as `^0.2`. The host refuses the plugin when the range doesn't include `nexus_api::API_VERSION` (now `0.2.0`). |
| `platforms` | no | `linux`, `macos`, `windows`. Empty means every system. |
| `console` | no | For an emulator, the console it plays: ASCII letters, digits and `-`, kept in uppercase (`ps5` and `PS5` are one console). A `releases` plugin gives the emulator that console's catalog. Not the systems the plugin runs on. |
| `capabilities` | no | The capabilities `init` provides. See [Capabilities](#capabilities). |
| `permissions` | no | What the plugin may do outside its data folder. See [Permissions and folders](#permissions-and-folders). |
| `dependencies` | no | `[[dependencies]]` entries. See below. |
| `activation` | no | `lazy` (default): `init` runs the first time the host needs a capability. `eager`: `init` runs at boot. |
| `runtime` | no | `[runtime]` with `type = "native"` (default: compiled into the binary) or `type = "data"` (a theme with no code). |
| `assets` | no | A folder of images, relative to the plugin's folder on disk. The host reads it only for a plugin loaded from a folder. A built-in plugin gives `Provided.assets` instead. |
| `config_schema` | no | A JSON value. The parser keeps it, but the host does not read it now: settings come from the `configurator` capability. |
| `requires` | no | What the computer needs for the plugin's games. See below. |
| `keymap` | no | `[[keymap]]` entries: the console's buttons and the keyboard keys that press them, in the order the launch splash lists them. |
| `install_note` | no | For an emulator with a game installer: what it installs from, in one or two sentences. The install sheet shows it. |
| `system` | no | `[system]`: the system the plugin's games show under. See below. |

### `[[dependencies]]`

| Key | Meaning |
| --- | --- |
| `id` | The plugin it needs. |
| `version` | A semver range. Default: any version. |
| `optional` | `true`: loaded first when present, ignored when missing. Default `false`. |

The kernel orders plugins so dependencies load first. It rejects a plugin whose required dependency is
missing, has the wrong version, failed to load, or is part of a cycle. An optional dependency
that is there but fails to start is ignored. No built-in plugin has
dependencies: plugins share code through `emulator-kit`, a library, not through each other.

### `[requires]` and `[requires.recommended]`

The host compares these with this computer (`crates/nexus-core/src/hardware.rs`). Games won't
start without `vulkan` or `avx2`. Below `recommended`, they may run slowly.

| Key | Meaning |
| --- | --- |
| `vulkan` | The Vulkan version the graphics card must support, such as `"1.3"`. |
| `avx2` | The processor must have AVX2. Not checked where the system can't say. |
| `recommended.cpu_threads` | Processor threads. |
| `recommended.memory_gb` | Memory in GB. |
| `recommended.gpu_memory_gb` | Graphics memory in GB. |
| `recommended.discrete_gpu` | A graphics card of its own, not one built into the processor. |

### `[[keymap]]`

Each entry has `button` (such as `"Cross"`) and `keys` (such as `"J"`). Both keys are required.
The contract suite checks that each button is listed once and that neither value is empty.

### `[system]`

| Key | Meaning |
| --- | --- |
| `id` | The code a game's `platform` has: `PS5`, `PC`, `Steam`. |
| `name` | The system's full name: `PlayStation 5`. |
| `color` | `#rrggbb`. Without it, a theme draws the system in its neutral look. |
| `icon` | A square picture in the plugin's assets. |
| `cover` | A wide picture in the plugin's assets, which a theme can show behind the system. |

A theme can put its own pictures in place of `icon` and `cover`: see [themes.md](themes.md).

### What the kernel checks

`validate()` in `crates/nexus-core/src/kernel.rs` rejects a plugin at boot when:

- the host is in safe mode and the plugin is third-party;
- `host_api` doesn't match the host's API version;
- it is a theme, and `platforms` doesn't include this system. An emulator made for other
  systems is not rejected: it loads turned off, so the player can turn it on with an own build;
- `[system]` `color` isn't `#rrggbb`;
- a folder name isn't lowercase letters, digits and `-`;
- a folder grant has both `write = true` and `listed_in`;
- the plugin is third-party with `runtime` `native`;
- a non-system plugin declares a system capability, or lists `long_running` or `polkit`;
- an entry of `long_running` or `polkit` isn't also in `process`;
- a theme declares any capability but `theme`, or doesn't declare `theme`;
- a non-theme declares `theme`, or has `runtime` `data`.

Later, at `init`, the host rejects a plugin whose `Provided` doesn't match `capabilities`. Each
rejection is in the boot report and the log ("plugin not loaded").

## Lifecycle

The host drives each plugin through three steps:

1. **Load.** The host builds the plugin and reads `manifest()`. No work happens yet.
2. **Init.** For a plugin with code, the host calls `init(ctx)` once: at boot for
   `activation = "eager"`, else the first time it needs one of the plugin's capabilities. The
   host activates dependencies first. A theme has no `init`: the theme manager reads its files
   when it needs them.
3. **Shutdown.** The host calls `shutdown()` at most once per activation, after the plugins
   that depend on it. The host makes repeated deactivation safe, so the plugin doesn't have to.

Every call into a plugin runs under a timeout and a panic guard (`crates/nexus-core/src/guard.rs`).
A panic becomes `PluginError::Panicked`, and a call that runs too long becomes
`PluginError::Timeout`. The defaults are in `HostConfig::new`: 30 seconds for a call
(`call_timeout`) and 5 seconds for `shutdown` (`shutdown_timeout`). Long work must therefore
start in the background and return at once. The game installer and the system capabilities
report that work through a job object the host passes in.

A failed `init` marks the plugin failed. The host publishes `Event::PluginFailed`, and every
later call to the plugin gets an error.

## The plugin context

`init` gets a `PluginContext` (`crates/nexus-api/src/context.rs`). It is the only way a plugin
reaches the host. Keep it, or the parts you need.

| Method | Type | What it gives |
| --- | --- | --- |
| `plugin()` | `PluginId` | The plugin's own id. |
| `config()` | `ConfigStore` | The plugin's settings, per `ConfigScope`. `get` returns `Null` when nothing is stored. |
| `storage()` | `Storage` | Files in the plugin's data folder. |
| `folders()` | `Folders` | The host folders the manifest asks for. |
| `programs()` | `Programs` | Find and run the programs `permissions.process` allows. |
| `http()` | `HttpClient` | HTTP to the hosts in `permissions.network`. |
| `log()` | `Logger` | The host log, tagged with the plugin's id: `debug`, `info`, `warn`, `error`. |
| `events()` | `EventSink` | Publish `Event::Custom` and subscribe to every event. |
| `services()` | `ServiceRegistry` | Host services, looked up by type. |

### Storage: the data folder

Each plugin has a data folder: `<data-dir>/plugins/<id>`. `Storage` reads and writes in it:
`read`, `write` (makes parent folders), `write_at`, `exists`, `list`, `remove`, `rename` and
`set_executable`. Every path is a `RelPath`, so no path can leave the folder.

### Programs

`Programs::find` returns a program's real path, or `None`. `Programs::output` runs a program
with arguments and no input, and returns its standard output: at most `MAX_PROGRAM_OUTPUT`
(64 KiB). The caller gives a timeout, which `MAX_PROGRAM_TIME` (10 seconds) caps. A program
that runs past it fails with `PluginError::Timeout`. Use it to see whether a build is there
and to read its version. `Programs::stream` runs a program with no time limit and passes each
line to a callback. It is for system plugins only, and needs the program in
`permissions.long_running` (and in `permissions.polkit` to run it as the administrator).

A program is a `Location`: `Location::Data(RelPath)` in the data folder,
`Location::Folder(HostPath)` in a host folder, or `Location::System(name)` on the system's
`PATH`.

### HTTP

`HttpClient::send` takes an `HttpRequest` (`Get`, `Head` or `Post`, with headers and a body)
and returns an `HttpResponse`. `HttpRequest::get(url)` is the short form. `HttpResponse::json`
fails with `PluginError::Network` on a status outside 200 to 299. The host refuses any host not
in `permissions.network`.

### Events

`EventSink::publish(topic, payload)` sends an `Event::Custom` from the plugin. A plugin can
publish nothing else. `EventSink::subscribe()` returns a stream of every `Event`
(`crates/nexus-api/src/event.rs`): plugin, download, install, process, library, state and
theme events.

A `releases` plugin publishes the topic `RELEASES_CHANGED` (`"releases.changed"`) when its
catalogs change between the host's asks. The host then asks for the catalogs again.

A plugin with `slots` publishes the topic `SLOTS_CHANGED` (`"slots.changed"`) when its
emulator panel (`emulator.panel`) changes between the host's asks, for example when work in
the background ends. The host then asks for the panel again. A slot action has the plugin's
call time limit, so long work runs in the background and its panel says how it goes.

### Services

`ServiceRegistry::get::<T>()` returns a host service. The host puts three in the registry of
every plugin:

- `HostInfo`: the API version, the platform, whether safe mode is on, and `runs_32bit`
  (the system has the 32-bit Linux loader).
- `VpnGate`: "Download only through a VPN", as the host sees it. Call `check()` before you
  fetch what the player downloads in bulk. `Ok(VpnPass)` lets you fetch.
  `Err(VpnBlock::Waiting)` means wait and try again. `Err(VpnBlock::NoVpn)` means skip the
  fetch. The gate only tells you the state: the plugin's HTTP doesn't go through the VPN.
- `Controllers`: the game controllers connected now, as SDL3 sees them. Each probe may take a
  second.

Plugins never reach each other through the registry.

## Permissions and folders

`[permissions]` says what the plugin may do outside its data folder. The host enforces every
entry (`Permissions` in `crates/nexus-api/src/manifest.rs`).

| Key | Meaning |
| --- | --- |
| `network` | Hosts for HTTP: an exact name (`github.com`), or a suffix pattern (`*.github.com`, which also matches `github.com`). Case doesn't matter. |
| `filesystem` | `[[permissions.filesystem]]` grants for named host folders. See below. |
| `process` | Programs the plugin may start. Empty means none. See below. |
| `long_running` | System plugins only: `system:` entries of `process` that may run with no time limit through `Programs::stream`, such as `system:bluetoothctl`. |
| `polkit` | System plugins only: `system:` entries of `process` that may run as the administrator through `pkexec`. |
| `links` | Links the host may open with the system's handler, by how they start: `steam://install/`. A link must be one line of plain characters. Empty means none. |

### Process rules

A `process` entry has one of three forms (`Permissions::allows_program`):

- A path in the data folder, split by `/`. A part may be `*` (any name) or `*.<ending>` (any
  name with that ending): `builds/*/kyty_emulator`, `builds/*/*.AppImage`. The rule and the
  path must have the same number of parts.
- `folder:<name>`: any file in the host folder `<name>`. The manifest must also grant that
  folder.
- `system:<name>`: a program on the system's `PATH`, by file name: `system:wine`. A name with
  `/`, `\` or `:` never matches.

`LaunchSpec`, `Programs::find` and `Programs::output` all go through these rules.

### Host folders

A host folder is a named folder outside the data folder, such as `games` or `kyty-build`. The
plugin asks for it with a grant. The player maps each name to one or more real folders. The
host saves the mapping under `app.folders.<name>` as a list of paths.

| Grant key | Meaning |
| --- | --- |
| `folder` | The name: lowercase letters, digits and `-`. Plugins that ask for the same name share the player's mapping. |
| `write` | `true` to write. Default `false`: read only. |
| `default` | Where the folder is until the player maps it. One path (`"~/Games"`), used even when it doesn't exist yet. Or a list (`["~/KytyPS5", "~/src/KytyPS5"]`): each folder in it that exists, in order. `~/` is the home folder. |
| `default_on` | A default for some systems: `{ macos = "~/Library/Application Support/X" }`. It wins over `default` on that system. |
| `title` | What Settings calls the folder. The name when none. |
| `help` | A line under the title. |
| `after` | On the plugin page, the key of the setting that this folder's row follows. At the end of the settings when none. |
| `was` | The folder's name before a rename. At the first start with it, the host copies the player's folders saved under it to the new name, when the new name has none. The old name keeps them. |
| `listed_in` | More folders, read from a file in a default folder: `{ file, key, holds }`. Each value of `key` in `file` (Valve's text format) that is an absolute path to a folder holding `holds`. Read only. |

Two host folders are shared (`SHARED_FOLDERS`). Settings shows them once, for every plugin
that asks for them, with the host's default, title and help, not the plugin's.

| Name | Default | Title |
| --- | --- | --- |
| `games` (`GAMES_FOLDER`) | `~/Games` | Game folders |
| `packages` (`PACKAGES_FOLDER`) | `~/Downloads` | Packages to install |

Installs go into the console's folder (`PS5`, `PS4`) inside the first `games` folder. A plugin
looks for packages in its console's folder inside each `packages` folder.

### `RelPath` and `HostPath`

`RelPath` (`crates/nexus-api/src/path.rs`) is a path inside a folder the host owns. It is
relative, uses `/`, and has no `.`, `..` or empty parts, and no `\` or `:`. Joining it to a base
folder can never leave that folder. The empty path (`RelPath::root()`) is the base folder.
`RelPath::new` returns `InvalidPath` for anything else; `?` turns that into
`PluginError::InvalidInput`.

`HostPath` (`crates/nexus-api/src/context.rs`) is a path in a host folder:

```rust
pub struct HostPath {
    pub folder: String, // the grant's name
    pub root: u32,      // which of the player's folders for that name, from 0
    pub path: RelPath,  // a path inside it
}
```

`Folders::roots(name)` lists the player's folders for a name, with each `root` index and its
real path to show. The other `Folders` methods take a `HostPath`: `list`, `size`, `exists`,
`read_range` (at most `MAX_FOLDER_READ`, 16 MiB, per call), `write`, `write_at`, `rename`,
`remove` and `set_executable`. The host refuses a name the manifest doesn't grant, a write
without `write = true`, and a symlink out of the folder.

## Capabilities

The manifest's `capabilities` list names them in snake case. `init` returns an implementation
for each in `Provided`. Each one is a trait in `crates/nexus-api/src`.

| Capability | `Provided` field | Trait | File |
| --- | --- | --- | --- |
| `catalog` | `catalog` | `Catalog` | `catalog.rs` |
| `downloader` | `downloader` | `Downloader` | `plans.rs` |
| `installer` | `installer` | `Installer` | `plans.rs` |
| `configurator` | `configurator` | `Configurator` | `config.rs` |
| `launcher` | `launcher` | `Launcher` | `plans.rs` |
| `compatibility` | `compatibility` | `Compatibility` | `extras.rs` |
| `slots` | `slots` | `Slots` | `slots.rs` |
| `game_installer` | `game_installer` | `GameInstaller` | `games.rs` |
| `metadata` | `metadata` | `Metadata` | `catalog.rs` |
| `releases` | `releases` | `Releases` | `releases.rs` |
| `backup` | `backup` | `Backup` | `backup.rs` |
| `theme` | none | `ThemePlugin` | `plugin.rs` |
| `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` | `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` | `NetworkSettings`, `VpnSettings`, `BluetoothSettings`, `AudioSettings`, `PowerSettings`, `OsUpdates`, `OsInstall` | `system.rs` |

### Catalog

`Catalog` lists, searches and describes the games one emulator can run:

- `list(PageRequest)` returns a `Page<GameSummary>`. `PageRequest::ALL` asks for every game.
- `search(&SearchQuery)` returns matching games.
- `details(game)` returns `GameDetails`, or `NotFound`.
- `artwork(game, ArtworkKind)` returns an `ImageSource` (`Cover`, `Background`, `Logo` or
  `Icon`), or `None`.

`GameSummary.id` is the plugin's own id for the game. The host makes it global as a `GameKey`,
written `plugin:local`. Ids must be unique: the contract suite checks it.

`GameDetails` has the game's folder (`folder`), other folders that belong to it (`also`, which
uninstall deletes with it), and `uninstall_link` for a game its store removes. Its `facts` are
`GameFact` values. Themes know the ids `region`, `age`, `version`, `title-id`, `content-id` and
`folder`, and show others by their label.

An `ImageSource` is a URL, an asset (`Asset(RelPath)`), or a file in a host folder the plugin
may read (`Folder(HostPath)`).

### Downloader and Installer

These two install emulator builds. The host runs the flow:

1. It asks `Downloader::plan(&DownloadRequest::EmulatorBuild { version })` for a
   `DownloadPlan`: a label and a list of `Artifact` values.
2. It downloads every artifact, checks its `sha256` (and an Ed25519 `signature` when given),
   writes it to `dest` in the data folder, and unpacks it when `unpack` asks. Formats:
   `TarGz`, `Zip`, `AppImageInZip`, `AppImage`. The last two run the AppImage's
   `--appimage-extract` (Linux only).
3. It calls `Installer::install(version, files)` or `Installer::update(from, to, files)` with
   the `DownloadedArtifact` values.

`Installer` also has `available()`, `installed()` and `uninstall(version)`. The default
`update` installs `to`, then uninstalls `from`, so a failed install leaves `from` in place.
`InstalledVersion.own` marks an own build: the host never updates it by itself.

### Launcher

`Launcher::prepare(game, config)` returns a `LaunchSpec`. `config` is the game's merged
settings: the emulator's, then the game's. The host starts the process only if
`permissions.process` allows `program`. It then watches the process and stops it.

`LaunchSpec` fields:

- `program`: a `Location`.
- `args`, `env`: each a `LaunchArg`, text or a path (`LaunchArg::path(...)`) the host turns
  into a real path.
- `cwd`: `None` means the program's folder. A system program needs one.
- `game_process`: for a program that hands the game to another one and may exit at once
  (Steam's client). Text in the command line of the game's own process. The host then
  follows that process.
- `relaunch`: an exit `code` and an `env`. When the program exits with that code, the host
  starts it again in the same session, with `env` added.
- `sandbox`: a `Sandbox`, with the `Location`s the game may `read` and the ones it may
  `write`. On Linux the host runs the whole command through bubblewrap, and the game sees only
  the system, the display, the sound, an empty home folder and those folders. Each folder goes
  through the same permission rules as a path argument. The host makes the `write` folders
  first. Without `bwrap` the launch fails. Ask for a sandbox on Linux only: elsewhere the
  launch fails.

`open_emulator(config)` returns the emulator's own window without a game, or `None` (the
default). `on_exit(game, &ExitInfo)` runs after the game ends.

### Configurator

`Configurator` describes and checks settings for a `ConfigScope`: `Emulator` or
`Game(<local id>)`.

- `schema(scope)`: a JSON Schema, usually from `schemars::schema_for!`.
- `defaults(scope)`: the default values.
- `validate(scope, value)`: `InvalidInput` with a message the player can read.

See [Settings and the plugin page](#settings-and-the-plugin-page).

### Compatibility

`Compatibility::status(game)` returns `CompatInfo`: a `CompatLevel` (`unknown`, `nothing`,
`boots`, `menus`, `ingame`, `playable`), and optional `notes`, `source` (the system it was
tested on), `page` and `tested` (`YYYY-MM-DD`). An unknown game must fail: the contract suite
checks it.

### Slots

A slot is a named place in the UI that plugins fill with data. The theme decides where and how
to draw it. `Slots::items(&SlotRequest)` returns `SlotItem` values, and
`Slots::run_action(action, game, args)` runs a button the player picked.

| Slot (`SlotId`) | Name | Where |
| --- | --- | --- |
| `GameBadge` | `game.badge` | Small labels on a game's card and Game Hub. |
| `GameAction` | `game.action` | Buttons in a game's Game Hub. |
| `GameDetailSection` | `game.detail-section` | Blocks of information in a game's Game Hub. |
| `SettingsPanel` | `settings.panel` | A settings form. |
| `EmulatorPanel` | `emulator.panel` | Blocks on the plugin page. |
| `CatalogAction` | `catalog.action` | Buttons and badges on the Catalog screen, from a `releases` plugin. |

The three `game.*` slots are per game. The host asks only the plugin that owns the game, with
`SlotRequest.game` set.

A `SlotItem` has an `id` (unique in the plugin), an `Order` (`priority`, `before`, `after`) and
`SlotContent`: `Badge`, `Action`, `Section`, `Block` or `SettingsPanel`.

A `Block` is a titled list of `BlockRow`s, on the plugin page only, after the build. A row has a
label, a value and a hint under the label, in a `Tone`. `progress` (0 to 1) adds a bar under it.
A row with an `action` is a button: picking it calls `run_action`, as an `Action` does. With
`switch` set, the button shows a switch, on or off, and left and right turn it off and on. The
controller moves through a page's block buttons after its build and its actions, and passes over
the rows that only show. Lichii's Firmware block is an example.

Three action names make the host do the work itself instead of calling `run_action`:

- `HOST_OPEN_SETTINGS` (`host:open-settings`): opens the Settings screen.
- `HOST_OPEN_EMULATOR` (`host:open-emulator`): starts `Launcher::open_emulator` as a session.
  No game starts until it closes, and its time is not play time.
- `HOST_INSTALL_GAME` (`host:install-game`): installs the package in `args` (a `HostPath`) with
  the plugin's game installer, as a job in Downloads.

### GameInstaller

`GameInstaller` installs games from packages and disc images the player has:

- `candidates()` lists the `Installable` sources in the folders the plugin may read.
- `begin(source, job)` checks `source`, starts the install in the background, and returns the
  work folder at once. A source that isn't a package fails here.

The work writes into the work folder and moves it into place at the end. It reports through
the `InstallJob`: `progress(done, total)`, `cancelled()`, and `finished(result)` exactly once.
On an error, the work folder must be gone. `games_folder()` gives the folder the player picked.
`take(from, to)` moves a folder instead of copying it, when the host agrees. The host removes
the work folder if the app stops before the end.

### Metadata

`Metadata::describe(&GameRef)` returns `GameMetadata` for a game, by its console ids, or `None`
when the store doesn't have it. `GameMetadata` has words (title, description, tagline,
publisher, genres, release date, age rating), a `Rating`, art (`icon`, `cover`, `background`,
`logo`, `screenshots`), `store_url` and `trailer`. Every field may be empty.

### Releases

`Releases::releases(&Console)` returns the `ReleaseList` for one console, or `None` when the
plugin has no catalog for it. The host gives each console's list to the emulator whose manifest
names that console. A `Release` is one copy of a game: a torrent, or a store's install (its
`store` link, which `permissions.links` must allow). What the catalog only shows goes in
`facts`.

`store_installs()` returns the installs a store is doing now, for a catalog whose store
installs its releases. The host shows them in Downloads. Default: none.

### System capabilities

`network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates` and `os_install` are for system
plugins only. The host uses the first active system plugin that provides each one, and shows a
capability that no plugin provides as unavailable. Quick calls answer within the call timeout.
Long work (joining a network, a Bluetooth scan, pairing, installing the OS) starts in the
background and reports through the `SystemJob` the host passes in: `progress`, `done`, `failed`
and `cancelled`. The traits are in `crates/nexus-api/src/system.rs`.

## Backup

The `backup` capability names what of a plugin's data a backup holds. The player backs up and
restores in Settings → Backup. The plugin only names its items. The host reads and writes the
files (`crates/nexus-core/src/backups.rs` and `crates/nexus-core/src/backup/`).

```rust
#[async_trait]
pub trait Backup: Send + Sync {
    async fn items(&self) -> PluginResult<Vec<BackupItem>>;
}
```

### `BackupItem`

| Field | Meaning |
| --- | --- |
| `name` | Its name in the backup, the same on every computer: 1 to 64 lowercase letters, digits and `-` (`BackupItem::valid_name`), such as `saves`. |
| `title` | What Settings calls it, such as "PS5 saves". |
| `place` | A `BackupPlace`: where the item is on this computer. A file, or a folder and everything in it. |
| `skip` | Paths below `place` to leave out, from the item's top: `cache`, or `GAME/log`. Each entry names one path; a folder of that name deeper down stays. |
| `large` | The item is large, such as Wine prefixes or DLC. It goes into a backup only when the player turns on "Include large items". |

Build an item with `BackupItem::new(name, title, place)`, then `.skip(&[...])` and `.large()`.

`BackupPlace` has two forms:

- `BackupPlace::Data(RelPath)`: in the plugin's data folder.
- `BackupPlace::Folder(HostPath)`: in a host folder the manifest grants.

### How the host backs up

For each turned-on plugin with the `backup` capability (a plugin that hasn't started yet
starts first), the host calls `items()` and finds each
item's real path through the plugin's data folder or its grants. It leaves out, with a line in
the log, an item with an invalid name or a place the grants refuse. It leaves out the large
items unless the player asked for them. Linked files and folders never go into a backup.

The backup file holds each item under `items/<plugin>/<item>`, beside `backup.json` and
`config.json`. The format is in `crates/nexus-core/src/backup/archive.rs`.

### How the host restores

1. The host refuses while a game runs, a game installs, or another backup or restore runs.
2. It calls `items()` again on this computer. It matches each item in the backup to the item
   of the same plugin and name here, and writes it to that item's place now. So a backup made
   on one computer restores on another whose folders are elsewhere.
3. Before it writes, it backs up the items here that the restore will replace.
4. It checks the whole file first and writes nothing on a problem: each entry stays inside its
   item, each item fits the free space, and each is a file or a folder here as in the backup.
5. It writes each file through a temporary file `.<name>.restoring` beside it. The temporary
   file takes the old file's place only once it is whole. A file there is replaced; files the
   backup doesn't have stay. Nothing is written through a link below the item's place.
6. An item in the backup that the plugin no longer names (or a plugin that isn't here) is left
   out. The message after the restore names it.
7. The settings go back at the next start ("Restart Nexus to finish"). The plugin's settings
   (`plugins.<id>`) go back; the player's folder mappings stay as they are on this computer.

The host writes a restored item even into a read-only grant. The host does the writing, and the
plugin named the place as its own data. So a plugin needs no `write = true` for a folder it only
reads, such as one with keys and firmware.

### Example

Most plugins give the same items every time. `emulator_kit::backup::Items` answers `items()`
with a fixed list. From `plugins/emulators/shadps4/src/lib.rs`:

```rust
backup: Some(emulator_kit::backup::Items::provided(vec![
    // Its settings, keys, saves and trophies; not its caches, logs and downloads.
    BackupItem::new(
        "data",
        "PS4 settings and saves",
        BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::root())),
    )
    .skip(&["cache", "log", "shader", "temp", "download", "addcont"]),
    // DLC can take many GB: only when asked for.
    BackupItem::new(
        "dlc",
        "PS4 DLC",
        BackupPlace::Folder(HostPath::new(DATA, 0, RelPath::new("addcont")?)),
    )
    .large(),
])),
```

Back up the plugin's own data only. Leave out what Nexus keeps itself (the plugin's settings
are in `config.json` already). Use `skip` for what the emulator makes again, as shadPS4 leaves
out its caches, logs and downloads.

## Settings and the plugin page

The host stores a plugin's settings in the app's `config.json`, under
`plugins.<id>.emulator` for `ConfigScope::Emulator` and `plugins.<id>.game:<local id>` for
`ConfigScope::Game`. The plugin reads them with `ctx.config().get(scope)`.

A settings form needs two capabilities:

- `configurator`, for the schema and the check of a new value.
- `slots`, with a `settings.panel` item whose content is
  `SlotContent::SettingsPanel { title, scope }`.

The host reads the schema as the fields of the panel (`crates/nexus-core/src/settings_schema.rs`).
It shows booleans, numbers, strings and string enums, and leaves other properties out. Fields
come in the order of the schema's `x-order` list, then the rest by name. With `schemars`, set
the order on the settings type, as `plugins/emulators/shadps4/src/lib.rs` does:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars", extend("x-order" = ["own_build", "fullscreen"]))]
pub struct Settings { /* ... */ }
```

An optional field left empty is unset, so the emulator's own value applies. Before it saves a
value, the host calls `validate`.

A game's own settings (the Game Hub) need only `configurator`: the host asks for the schema of
`ConfigScope::Game(<local id>)` and shows it when it has properties.

### The plugin page

Each plugin has a page in the plugin lists. For a plugin that is turned on, the page holds
(`PluginPageVm` in `crates/nexus-ui-api/src/vm.rs`):

- its builds, for an emulator;
- its settings panels from `settings.panel`;
- its `emulator.panel` items: sections, and actions as one row each;
- the host folders it asks for, except the shared `games` and `packages` (those are in
  Settings). With settings, the folder rows come after the field that a grant's `after` names,
  or after the last field.

A plugin that is turned off shows only its row. A theme can't be turned off.

## Errors

A plugin returns `PluginError` (`crates/nexus-api/src/error.rs`):

| Variant | Use |
| --- | --- |
| `NotFound` | No such game, version or file. |
| `InvalidInput` | Bad input. `InvalidPath`, `InvalidId` and `serde_json::Error` convert to it. |
| `PermissionDenied` | The host refused, or the plugin may not. |
| `Network` | HTTP failed. |
| `Storage` | A file operation failed. |
| `Unsupported` | Not on this system or for this game. |
| `Precondition` | Something must happen first, such as installing a build. |
| `OwnBuildMissing { has_downloaded, folder, program }` | The own build is turned on but isn't in the folder or on the `PATH`. The host words it for the player. |
| `Timeout`, `Panicked` | Set by the host's guard. |
| `Other` | Anything else. |

## Shared code: emulator-kit

`plugins/shared/emulator-kit` is a library, not a plugin. The PS5, PS4, Switch and PC
emulator plugins use it. It depends on `nexus-api` like a plugin, plus `tokio` and the vendored
Kraken decoder (`third_party/oozextract`) for PS5 packages.

A console emulator describes itself and the kit runs the rest. The plugin gives a `Flavor` for
its builds, which the `BuildManager` installs, updates and rolls back. It gives a `ConsoleSpec`
for its games, compatibility list and page, which the `ConsoleEmulator` serves. A plugin with a
block of its own on its emulator panel, such as Lichii's firmware, gives a `Panel` with
`with_panel`: the `ConsoleEmulator` shows its items after its own and offers it each action
first. The plugin keeps
its settings type, its launch arguments and its game installs. The kit reaches the host only
through the host facades in `host::Host`, never the whole `PluginContext`, so its tests run on
in-memory doubles.

| Module | What it holds |
| --- | --- |
| `archive_sets` | The archive sets of a download, by name, as the host unpacks them. |
| `args` | Extra arguments the player types, split like a shell would. |
| `backup` | `Items`: a fixed list of backup items. |
| `builds` | `Flavor`, `BuildManager`: official builds from GitHub releases, and the own build. |
| `cache` | JSON from the web, kept in the plugin's storage for a while. |
| `compat` | Community compatibility lists, by title id. |
| `console` | `ConsoleSpec`, `ConsoleEmulator`: a console emulator as a plugin. |
| `controls` | A console emulator's keyboard keys, for its panel. |
| `exfat` | Read-only exFAT disk images. |
| `fail` | A small error type for the package readers. |
| `games` | Installed console games in a host folder (PS5, PS4, Switch files). |
| `github` | GitHub releases, as the REST API returns them. |
| `host` | `Host`: the host facades the kit uses. |
| `installs` | Installing a game from a package or disc image into a game folder. |
| `pkg5` | A reader for PS5 debug packages. Retail packages are refused. |
| `read_at` | What the package readers read from: a file on disk or in a host folder. |
| `settings` | An emulator's settings type, stored by the host, read over this system's defaults. |
| `sfo` | PS4 `param.sfo` metadata. |
| `versions` | Emulator versions under `builds/<tag>`, with the current build and the one before it, in `versions.json`. |

The `fixtures` feature adds builders of synthetic packages and exFAT images for other crates'
tests.

## Register a built-in plugin

1. Make the crate under `plugins/<group>/<name>/`, with `plugin.toml` and a `Cargo.toml` that
   uses the workspace's `version`, `edition`, `rust-version`, `publish` and `license`, and
   `[lints] workspace = true`. Copy one of the plugins under `plugins/`.
2. Add the crate's path to `members` in the root `Cargo.toml`, and add it to
   `[workspace.dependencies]` (`my-plugin = { path = "plugins/<group>/<name>" }`).
3. Register the factory in the crate:
   `inventory::submit! { PluginFactory::code(|| Box::new(MyPlugin)) }`. A theme uses
   `PluginFactory::theme`. The `inventory` feature of `nexus-api` is on by default.
4. Add the crate to `[dependencies]` in `crates/nexus-app/Cargo.toml`, in the block under the comment
   `# Linked so their compile-time factories register. Don't remove as "unused".` Nothing calls
   the crate, so without this line the factory never links and the plugin never loads.
5. For seam tests in `tests/`, add the crate to `[dev-dependencies]` in `tests/Cargo.toml`.

## Testing

`nexus-testkit` (`crates/nexus-testkit`) runs a real core on a fake host. The main parts:

- `FakeHost`: a real core over a temporary data folder, with fake HTTP, fake processes and a
  `FakeClock`. Add plugins with `.plugin(StubPlugin)`, `.built_in(PluginBox)` or
  `.third_party(PluginBox)`, then `.boot_core().await` for a `TestCore` (or `.boot()` for the
  kernel only). `.http().serve(url, bytes)` answers a URL.
- `StubPlugin`: a plugin from a manifest and test-chosen behaviour.
  `StubPlugin::from_toml(...)` then `.provides(|ctx| Ok(Provided { ... }))`. It can also
  `panics_on_init`, `hangs_on_init`, `fails_on_init` and `hangs_on_shutdown`, and counts
  `init_calls` and `shutdown_calls`.
- `FakeBackup`: backup items a test sets. `StubPlugin::backs_up(id, permissions, backup)` is a
  plugin that provides only `backup`, with `permissions` as TOML for its `[permissions]`.
- `FakeCatalog`: an emulator with a catalog of games `G0`, `G1`, …, built with `.build()`.
  `FakeStore` describes games, and `FakeService` fills slots. `FakeEmulator` is an emulator a
  test scripts, from catalog to launch.
- `contract::emulator(&test, &plugin_id)`: runs every capability contract the plugin declares
  (catalog, downloader, configurator, compatibility, keymap, slots, installer, launcher). A
  broken rule panics with `contract <capability>: ...`.

Run the contract suite on a real plugin, as `tests/tests/seams/kyty.rs` does:

```rust
let test = FakeHost::new().built_in(nexus_api::PluginBox::code(KytyPlugin)).boot_core().await;
contract::emulator(&test, &PluginId::new("emu-kyty").unwrap()).await;
```

A backup test, as `tests/tests/seams/backup.rs` does:

```rust
let items = vec![
    BackupItem::new("saves", "Saves", BackupPlace::Folder(HostPath::new("emu-saves", 0, RelPath::root())))
        .skip(&["cache"]),
];
let plugin = StubPlugin::backs_up(
    "emu-b",
    "[[permissions.filesystem]]\nfolder = \"emu-saves\"\nwrite = true\n",
    FakeBackup::new(items),
);
let test = FakeHost::new().built_in(plugin.boxed()).boot_core().await;
```

Then map the folders with `Command::SetFolders`, set `Pref::BackupFolder`, and send
`Command::Backup(BackupCmd::BackUp { large: false })`.

`make test` runs every test (`cargo test --workspace`). `make check` runs the format check,
clippy and every test, as CI does (`scripts/check.sh`). See [development.md](development.md).

## The built-in plugins

Each is a working example. The capabilities below are copied from each `plugin.toml`.

**PS5: KytyPS5** (`plugins/emulators/kytyps5`, `emu-kyty`). Capabilities: `catalog`,
`downloader`, `installer`, `configurator`, `launcher`, `compatibility`, `slots`,
`game_installer`, `backup`. Builds come from KytyPS5's GitHub releases, through the kit's build
manager. Games are PS5 game folders in the `games` folders. A build with `--data-folder` keeps
its saves in the shared `kyty-data` folder, which is the plugin's one backup item. It installs
PS5 game folders, exFAT images and unsigned PS5 packages.

**PS4: shadPS4** (`plugins/emulators/shadps4`, `emu-shadps4`). The same capabilities as
KytyPS5. Builds come from shadPS4's official release; on Linux the host takes the AppImage out
of the zip and extracts it. Games are PS4 game folders. Backup: its data folder without caches
and logs, and its DLC as a large item.

**Switch: Lichii** (`plugins/emulators/lichii`, `emu-lichii`, Linux only). Capabilities:
`catalog`, `downloader`, `installer`, `configurator`, `launcher`, `slots`, `game_installer`,
`backup`. Builds are Lichii's AppImage. Games are Switch game files. Its backup items are in
its folders `lichii-data` and `lichii-config` and in its own data folder. It grants
`lichii-config` read only: the restore writes there anyway.

**PC** (`plugins/emulators/pc`, `emu-pc`). Capabilities: `catalog`, `launcher`, `slots`,
`game_installer`, `configurator`, `backup`. No builds of its own. It installs PC games from a
download folder, finds what starts them, and runs a Windows game through umu-launcher or Wine
off Windows. Backup: each game's Wine prefix, as a large item.

**Steam** (`plugins/emulators/steam`, `emu-steam`, Linux only). Capabilities: `catalog`,
`launcher`, `slots`, `releases`, `metadata`. Its library is the games Steam installed, read
from the `steam` grant, which uses `listed_in` for Steam's libraries on other drives. Play
hands the game to Steam and follows it with `LaunchSpec::game_process`. Install and uninstall
open `steam://` links that `permissions.links` allows.

**Game Catalog** (`plugins/services/release-catalog`, `release-catalog`, a service).
Capabilities: `releases`, `configurator`, `slots`. One catalog file per console (`ps5.json`,
`ps4.json`, `pc.json`), from a catalog address, a copy fetched before, or the copy bundled from
`assets/catalog/`.

**PlayStation Store** (`plugins/services/playstation-store`, `playstation-store`, a service).
Capability: `metadata`. It describes PS4 and PS5 games by title id from Sony's public catalog
API.

**This PC** (`plugins/system/host`, `system`, a system plugin). Capabilities: `network`, `vpn`,
`bluetooth`, `audio`, `power`, `os_updates`, `os_install`. It runs the system's own tools:
most on Linux, VPNs and the battery on macOS.

The two themes, Default and Classic (`plugins/themes/default`, `plugins/themes/classic`), are
covered in [themes.md](themes.md).
