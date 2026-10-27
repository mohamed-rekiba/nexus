# Writing a plugin

A plugin is a Rust crate that Nexus compiles into its binary. It tells the host what it can do
in a manifest (`plugin.toml`), and it returns the code for each thing from one function
(`init`). The host calls that code. The plugin never calls the host except through a
`PluginContext`.

This guide is for Rust developers who write or maintain plugins: emulators, services and system
plugins. Themes are files, not code: see [themes.md](themes.md). The shape of the whole app is in
[architecture.md](architecture.md). How to build, run and test the repo is in
[development.md](development.md). Terms (capability, kind, flavor, and so on) are in
[the glossary](../GLOSSARY.md).

Every Rust and TOML sample below comes from a real file in the repo. The line under each sample
names the file. Doc samples are not compiled, so read the file when you need the full code.

## What a plugin is

A plugin has three parts:

- A manifest, `plugin.toml`, read with `PluginManifest::from_toml` (`crates/nexus-api/src/manifest.rs`).
- A type that implements `Plugin` (its manifest and its `shutdown`) and `CodePlugin` (its `init`).
- One or more capabilities: traits that the plugin implements and `init` returns in a `Provided`.

A plugin depends on `nexus-api` only. That crate holds every type that crosses between the host and
a plugin, and it re-exports `serde`, `serde_json`, `schemars`, `semver`, `async_trait` and
`inventory`, so a plugin needs no other dependency for them (`crates/nexus-api/src/lib.rs`).

## Kinds

The manifest `kind` is `emulator`, `service`, `theme` or `system` (`PluginKind`).

| Kind | Code | What it is for |
| --- | --- | --- |
| `emulator` | yes | Runs games: catalog, builds, settings, launching. |
| `service` | yes | Adds to the games other plugins find, and runs none: a store's art and details. |
| `system` | yes | The machine's own settings and devices: network, VPNs, Bluetooth, sound, power. |
| `theme` | no | Files only. See [themes.md](themes.md). |

The kind governs privileges. It does not govern which data capabilities a plugin may provide
([ADR 0001](adr/0001-capabilities-follow-the-capability-not-the-kind.md)). Any plugin with code
may provide `catalog`, `releases`, `metadata`, `slots`, `configurator` and the rest. The Steam
emulator, for example, provides `releases` and `metadata` (`plugins/emulators/steam/plugin.toml`).

Only a `system` plugin may do these three things:

- provide a system capability (`Capability::SYSTEM`),
- run a program long (`permissions.long_running`, `Programs::stream`),
- run a program through polkit (`permissions.polkit`).

The kernel checks this at boot in `validate` (`crates/nexus-core/src/kernel.rs`). Beyond
privileges, `validate` uses the kind only to tell a theme from a plugin with code.

## Built-in and data-only plugins

Nexus has two kinds of origin (`Origin` in `kernel.rs`):

- **Built-in.** The crate sits in this workspace, `crates/nexus-app` links it, and it registers
  itself with `inventory::submit!`. It runs in the host process.
- **Third-party.** A folder with a `plugin.toml` or `theme.toml`, found by
  `KernelBuilder::discover`. It must declare `[runtime] type = "data"`.

Today a third-party plugin can only be a theme. No runtime for third-party code exists. The
evidence in the code:

- `validate` rejects a third-party manifest whose runtime is `native`: "third-party plugins must
  be data-only".
- `validate` also rejects a non-theme plugin whose runtime is `data`: "needs code".
- The only loader the kernel registers is `DataThemeLoader` (`crates/nexus-core/src/themes.rs`),
  and it accepts a theme with `Runtime::Data` only. `crates/nexus-app` adds no other loader.

`--safe-mode` loads built-in plugins only. A third-party plugin is then rejected with
`LoadError::SafeMode`.

Writing a new emulator, service or system plugin therefore means adding a crate to this repo. See
[the checklist](#checklist-for-a-new-plugin).

## The crate

A plugin crate sits under `plugins/emulators/`, `plugins/services/` or `plugins/system/`. It
depends on `nexus-api`, and an emulator may also depend on `emulator-kit`.

```toml
[package]
name = "playstation-store"
description = "The PlayStation Store's descriptions of PS4 and PS5 games: art, logo, rating, genres, release date and screenshots, by title id."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true
license.workspace = true

[dependencies]
nexus-api.workspace = true

[lints]
workspace = true
```

Source: `plugins/services/playstation-store/Cargo.toml`.

The crate has three jobs in `src/lib.rs`: embed the manifest, implement the traits, and register
the plugin. This is the whole entry point of the PlayStation Store service:

```rust
#[async_trait]
impl CodePlugin for PlayStationStore {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let cache = match ctx.storage().read(&RelPath::new(CACHE)?).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        Ok(Provided { metadata: Some(Arc::new(Store { ctx, cache: Mutex::new(cache) })), ..Provided::default() })
    }
}

inventory::submit! { PluginFactory::code(|| Box::new(PlayStationStore)) }
```

Source: `plugins/services/playstation-store/src/lib.rs`.

The `inventory::submit!` line registers the plugin at compile time. It works only if the host
binary links the crate, so `crates/nexus-app/src/main.rs` has one `use <crate> as _;` line for each
built-in plugin. `KernelBuilder::built_in_plugins` then creates every registered factory. A
theme registers with `PluginFactory::theme`.

The `Plugin` impl returns the manifest from the embedded file:

```rust
const MANIFEST: &str = include_str!("../plugin.toml");

pub struct PcPlugin;

#[async_trait]
impl Plugin for PcPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}
```

Source: `plugins/emulators/pc/src/lib.rs`.

`Plugin::shutdown` has a default that does nothing. Override it to stop work you started in
`init`: the Game Catalog service aborts its refresh task there
(`plugins/services/release-catalog/src/lib.rs`).

The host holds a plugin as a `PluginBox`: `PluginBox::code(..)` for a plugin with code and
`PluginBox::theme(..)` for a theme (`crates/nexus-api/src/plugin.rs`).

## The manifest

The manifest is TOML, parsed into `PluginManifest`. The sub-tables (`[permissions]`, a filesystem
grant, `[[keymap]]`, `[requires]`, `[system]`) reject keys they do not name. Top-level keys the
struct does not name go to `extra`, which the crate that owns them reads (a theme's `[theme]`
table, for example).

This is the head of the KytyPS5 manifest, trimmed:

```toml
id = "emu-kyty"
kind = "emulator"
# The plugin is shown by the console it plays.
name = "PS5"
# The program the plugin installs and runs. Sentences about the program name it.
emulator = "KytyPS5"
version = "0.1.0"
host_api = "^0.2"
# KytyPS5 ships Linux and Windows builds, and macOS x86-64 builds that run under Rosetta 2.
platforms = ["linux", "macos", "windows"]
# The console it plays: it gets that console's catalog.
console = "PS5"
capabilities = ["catalog", "downloader", "installer", "configurator", "launcher", "compatibility", "slots", "game_installer"]

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

# The system its games show under: what the Library and a card call it, and its color.
[system]
id = "PS5"
name = "PlayStation 5"
```

Source: `plugins/emulators/kytyps5/plugin.toml`.

Fields of `PluginManifest`:

| Field | Meaning |
| --- | --- |
| `id` | `PluginId`: 1 to 64 lowercase letters, digits or `-`, starting with a letter. |
| `kind` | `emulator`, `service`, `theme` or `system`. |
| `name` | What the player sees. An emulator names its console: `PS5`. |
| `emulator` | The program an emulator installs and runs (`KytyPS5`). Empty when it has no name of its own. `PluginManifest::emulator_name` returns it, or `name` when empty. |
| `version` | The plugin's version (semver). |
| `host_api` | The `nexus-api` versions the plugin works with. The host refuses it unless the range matches `API_VERSION`. |
| `platforms` | Systems the plugin runs on: `linux`, `macos`, `windows`. Empty means every system. |
| `console` | The console an emulator plays (`PS5`, `PS4`, `PC`). Kept in uppercase. The core gives the emulator that console's catalog. |
| `capabilities` | The set of capabilities `init` provides. |
| `permissions` | What the plugin may do outside its data folder. See below. |
| `dependencies` | Plugins that load first. |
| `activation` | `lazy` (default) or `eager`. |
| `runtime` | `native` (default) or `data`. |
| `assets` | A folder of images that slot data and catalog entries name. |
| `config_schema` | A JSON Schema field. No host code reads it today: see [Limitations](#limitations). |
| `requires` | What the plugin's games need from the computer: `vulkan`, `avx2`, and `[requires.recommended]`. |
| `keymap` | An emulator's keyboard keys for the console's buttons, in the order the launch splash lists them. |
| `install_note` | One or two sentences the install sheet shows. |
| `system` | `[system]`: the id, name and `#rrggbb` color that the Library's chips and a card's badge use for the plugin's games. |

`API_VERSION` is a constant in `crates/nexus-api/src/lib.rs`. A change to the plugin API raises it
by hand, together with every built-in plugin's `host_api`.

A dependency names another plugin:

```toml
[[dependencies]]
id = "firmware"
version = ">=1.0"
optional = true
```

Source: `crates/nexus-api/src/manifest.rs`, the `KYTY` constant in its tests.

An optional dependency loads first when it is present, and the host ignores it when it is missing
or has the wrong version. A required dependency that is missing, has the wrong version, or failed
to load rejects the plugin. A dependency cycle rejects its members (`LoadError`).

### What the kernel checks

`validate` rejects a manifest at boot when any of these holds:

- safe mode is on and the origin is third-party,
- `host_api` does not match the host's API version,
- a theme does not support this platform (other plugins load turned off instead: see
  [Lifecycle](#lifecycle)),
- `[system] color` is not `#rrggbb`,
- a filesystem folder name has characters other than lowercase letters, digits and `-`,
- a filesystem grant has both `write = true` and `listed_in`,
- a third-party plugin has the `native` runtime,
- a plugin that is not `system` declares a system capability, or `long_running` or `polkit`,
- an entry of `long_running` or `polkit` is not also in `process`,
- a theme declares anything but `theme`, or an emulator, service or system plugin declares `theme`,
- an emulator, service or system plugin has the `data` runtime.

### Permissions

`[permissions]` lists what the plugin may do outside its own data folder. The host enforces each
entry (`Permissions` in `manifest.rs`):

| Key | Rule |
| --- | --- |
| `network` | Hosts the plugin may reach over HTTP: an exact name (`github.com`) or a suffix pattern (`*.github.com`, which also matches `github.com`). |
| `filesystem` | Named host folders the plugin may read, and write with `write = true`. |
| `process` | Programs the plugin may start. See below. |
| `long_running` | System plugins only: entries of `process` that may run with no time limit. |
| `polkit` | System plugins only: entries of `process` that may run as the administrator through `pkexec`. |
| `links` | Link prefixes the host may open for the plugin, such as `steam://install/`. |

A `process` entry has one of three forms (`Permissions::allows_program`):

- A path pattern in the plugin's data folder, such as `builds/*/kyty_emulator`. A `*` part
  matches any name. `*.AppImage` matches any name with that ending.
- `folder:<name>`: any file in a granted host folder. The manifest must also grant that folder.
- `system:<name>`: a program on the system `PATH`, by file name only, such as `system:wine`.

A filesystem grant (`PathGrant`) has a `folder` name and these options:

- `write`: allow writes. Default is read only.
- `default`: where the folder is until the player maps it. A string is one folder. A list is a set of
  places to look, and each one that exists counts (`default = ["~/KytyPS5", "~/src/KytyPS5"]`).
  `~/` means the home folder.
- `default_on`: another default for one platform.
- `title` and `help`: the words Settings shows.
- `listed_in`: more folders, read from a file inside the default folder, while the player has not
  set the folder. The Steam plugin uses it to find libraries on other drives.

Two folder names are shared by every plugin that asks for them, and the host owns their default,
title and help (`SHARED_FOLDERS`): `games` (`GAMES_FOLDER`) and `packages` (`PACKAGES_FOLDER`). A
plugin leaves out its own default for them.

This is how KytyPS5 asks for them, and for the player's own build:

```toml
[[permissions.filesystem]]
folder = "games"
write = true

# PS5 packages (.pkg) and exFAT disc images to install. Read only.
[[permissions.filesystem]]
folder = "packages"

[[permissions.filesystem]]
folder = "kyty-build"
title = "Your own KytyPS5 build"
default = [
    "~/KytyPS5", "~/kyty", "~/Kyty",
]
```

Source: `plugins/emulators/kytyps5/plugin.toml`, trimmed (the real `default` list is longer).

This is the permission block of the one plugin that may run programs long. A rule in `long_running`
or `polkit` must also appear in `process`:

```toml
[permissions]
process = [
    "system:nmcli",
    "system:bluetoothctl",
    "system:pactl",
    "system:systemctl",
    "system:rpm-ostree",
    "system:nexus-os-install",
    "system:pmset",
]
long_running = [
    "system:nmcli",
    "system:bluetoothctl",
    "system:pactl",
    "system:systemctl",
    "system:nexus-os-install",
]
polkit = ["system:nexus-os-install"]
```

Source: `plugins/system/host/plugin.toml`, with its comments removed.

## Lifecycle

`Kernel` (`crates/nexus-core/src/kernel.rs`) runs every plugin through these steps:

1. **Discover.** Built-in plugins come from `built_in_plugins`. Third-party folders come from
   `discover`: each subfolder with a `plugin.toml` or `theme.toml`.
2. **Validate.** `validate` checks each manifest. A bad one goes into `BootReport::rejected` with a
   `LoadError`. Two plugins with one id: the second gets `LoadError::Duplicate`.
3. **Resolve.** `resolve` orders the plugins from their dependencies (a theme's `extends` counts as a
   dependency). `BootReport::order` lists the loaded plugins, dependencies first.
4. **Load.** The kernel builds each plugin. A plugin whose code does not match its `kind` (a theme
   that has an `init`, or the reverse) is rejected. A plugin made for other systems still loads,
   but turned off. The player may turn it on to use their own build.
5. **Init (activate).** `CodePlugin::init` runs once. A plugin with `activation = "eager"` runs at
   boot. A `lazy` plugin runs the first time the host asks for its capabilities
   (`Kernel::provided`). A theme has no `init`.
6. **Deactivate.** `Kernel::deactivate` first deactivates the plugins that depend on this one, then
   calls `shutdown`. `Kernel::shutdown` deactivates every plugin, dependents first. The state then
   returns to loaded, so a later call runs `init` again.

The host checks that `init` provides exactly the capabilities the manifest declares
(`Provided::capabilities`). A mismatch fails the plugin with `LoadError::CapabilityMismatch`, and
the host calls `shutdown` first.

A plugin whose `init` fails stays failed for the rest of the run. Later calls return the same reason
as `CoreError::PluginUnavailable`. The kernel publishes `Event::PluginActivated`,
`Event::PluginFailed` and `Event::PluginDeactivated` on the event bus.

The player can turn a plugin off. The host then refuses to activate it: `Kernel::provided` returns
`PluginUnavailable` ("it is turned off"). The host saves the choice under `app.disabled_plugins`,
and under `app.enabled_plugins` for a plugin made for other systems.

### Timeouts and panics

Every call into a plugin goes through `guarded` (`crates/nexus-core/src/guard.rs`), which `init`
and `shutdown` use too. It adds:

- a tracing span, `plugin_call`, tagged with `plugin_id` and the method name,
- a timeout: `HostConfig::call_timeout` for a call and for `init`, `HostConfig::shutdown_timeout`
  for `shutdown`. A timeout returns `PluginError::Timeout`,
- a panic guard. A panic returns `PluginError::Panicked`, and the other plugins keep working.

The timeout fires at the plugin's next `.await`. A plugin that blocks its thread without awaiting
holds that worker thread until it returns. Do not block in a plugin: use async calls, or
`tokio::task::spawn_blocking`.

## PluginContext

`init` gets a `PluginContext`. It is the only way a plugin reaches the host. Each plugin gets its
own, scoped to its id and its manifest (`Kernel::context_for`).

```rust
pub struct PluginContext {
    plugin: PluginId,
    config: Arc<dyn ConfigStore>,
    storage: Arc<dyn Storage>,
    folders: Arc<dyn Folders>,
    programs: Arc<dyn Programs>,
    http: Arc<dyn HttpClient>,
    log: Logger,
    events: Arc<dyn EventSink>,
    services: ServiceRegistry,
}
```

Source: `crates/nexus-api/src/context.rs`.

Each field has a method of the same name: `ctx.storage()`, `ctx.folders()`, and so on. The
context is `Clone`. A plugin keeps it, or keeps the parts it needs.

| Facade | What it gives | Host side |
| --- | --- | --- |
| `config()` | `ConfigStore`: the plugin's settings, by `ConfigScope`. | `ScopedConfig` in `config.rs`, saved under `plugins.<id>`. |
| `storage()` | `Storage`: files in the plugin's own data folder. | `DirStorage` over `HostConfig::plugin_dir`. |
| `folders()` | `Folders`: host folders the manifest asks for. | `HostFolders` in `folders.rs`. |
| `programs()` | `Programs`: find and run programs the manifest allows. | `HostPrograms` in `programs.rs`. |
| `http()` | `HttpClient`: HTTP to the hosts in `permissions.network`. | `AllowlistHttp` in `facades.rs`. |
| `log()` | `Logger`: `debug`, `info`, `warn`, `error`. | `TracingLog`, tagged with the plugin id. |
| `events()` | `EventSink`: publish `Event::Custom`, and subscribe to all events. | `PluginEvents` in `facades.rs`. |
| `services()` | `ServiceRegistry`: host facts, looked up by type. | Built in `Kernel::context_for`. |

### Paths

Two path types keep a plugin inside its folders:

- `RelPath`: a relative path with `/` and no `.`, `..`, empty, `\` or `:` parts. Joining it to a base
  folder never leaves that folder. The empty path is the base folder.
- `HostPath`: a `folder` name from the manifest, a `root` (which of the player's folders for that
  name), and a `RelPath` inside it.

### Storage and folders

`Storage` works on the plugin's data folder with `RelPath`: `read`, `write` (creates parent
folders), `write_at`, `exists`, `list`, `remove`, `rename` and `set_executable`.

`Folders` works on host folders with `HostPath`. The host refuses:

- a name the manifest does not ask for (`PermissionDenied`),
- a write, rename or `set_executable` without `write = true`,
- a symlink that leaves the folder.

`Folders::roots(name)` lists the player's folders for a name. It is empty until the player maps the
name and the grant has no default. `Folders::read_range` returns at most `MAX_FOLDER_READ` bytes
per call. `Folders::size` counts at most `MAX_SIZE_FILES` entries.

### Programs

`Programs` finds and runs programs that `permissions.process` allows. The host refuses any other.

- `find` returns a program's real path, or `None` when it is not there.
- `output` runs a program with no input and returns its standard output. The output is at most
  `MAX_PROGRAM_OUTPUT` bytes, and the run is at most `MAX_PROGRAM_TIME`. A longer run fails with
  `PluginError::Timeout`.
- `stream` runs a program that `long_running` allows, and hands each line to a callback. With
  `elevated = true` it runs through `pkexec`, and `polkit` must allow it. The default
  implementation returns `PermissionDenied`.

`Programs` is for short checks, such as reading a build's `--help`. Games start through
`Launcher::prepare`, which returns a `LaunchSpec`. The host starts that process.

### HTTP

`HttpClient::send` takes an `HttpRequest` (`HttpRequest::get(url)` builds a GET) and returns an
`HttpResponse`. `HttpResponse::json` reads a 2xx body as JSON. The host checks every request
against `permissions.network`. It follows redirects, up to `MAX_REDIRECTS`, and checks each hop. A
URL with user info, or with a scheme other than `http` or `https`, is refused.

### Events

`EventSink::publish(topic, payload)` sends an `Event::Custom` tagged with the plugin id. A plugin
can publish custom events only. `subscribe` returns a stream of every `Event`. The Game Catalog
publishes `RELEASES_CHANGED` when its catalogs change between the core's asks, and the core then
asks again.

### Services

`ServiceRegistry::get::<T>()` returns a host service by type. Two are always present
(`Kernel::context_for`):

- `HostInfo`: `api_version`, `platform`, `safe_mode`, `runs_32bit`.
- `VpnGate`: `check()` tells whether the plugin may fetch now under "Download only through a VPN".
  A plugin asks it before bulk fetches. It reports on the VPN only: the plugin's own HTTP does not
  go through the VPN's interface.

The PC plugin reads `HostInfo` in `init`:

```rust
let host = ctx.services().get::<HostInfo>();
let platform = host.as_ref().and_then(|h| h.platform).or_else(Platform::current).unwrap_or(Platform::Linux);
let runs_32bit = host.is_some_and(|h| h.runs_32bit);
```

Source: `plugins/emulators/pc/src/lib.rs`.

Plugins do not reach each other through the registry.

## Capabilities

A capability is one thing a plugin does for the host. The manifest names it in `capabilities`.
`init` returns it in a field of `Provided` with the matching name. The two must agree.

| Manifest name | Trait | `Provided` field |
| --- | --- | --- |
| `catalog` | `Catalog` | `catalog` |
| `downloader` | `Downloader` | `downloader` |
| `installer` | `Installer` | `installer` |
| `configurator` | `Configurator` | `configurator` |
| `launcher` | `Launcher` | `launcher` |
| `compatibility` | `Compatibility` | `compatibility` |
| `slots` | `Slots` | `slots` |
| `game_installer` | `GameInstaller` | `game_installer` |
| `metadata` | `Metadata` | `metadata` |
| `releases` | `Releases` | `releases` |
| `theme` | none (themes only) | none |
| `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` | `NetworkSettings`, `VpnSettings`, `BluetoothSettings`, `AudioSettings`, `PowerSettings`, `OsUpdates`, `OsInstall` | `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` |

`Provided` also has an `assets` field: a `Files` object that serves the images slot data and
catalog entries name as `ImageSource::Asset`. The manifest's `assets` folder is the alternative.

The traits are in `crates/nexus-api/src/`. All except `VpnSettings` are `async_trait` traits.

### Catalog

`Catalog` (`catalog.rs`) lists, searches and describes the games one emulator can run.

- `list(PageRequest)` returns a `Page<GameSummary>`. `PageRequest::ALL` asks for everything.
- `search(&SearchQuery)` returns `GameSummary` values, at most `limit`.
- `details(game)` returns `GameDetails`, or `PluginError::NotFound`.
- `artwork(game, ArtworkKind)` returns an `ImageSource` or `None`.

A `GameSummary.id` is the plugin's own game id. The host makes it global with a `GameKey`
(`plugin:local`).

### Downloader and Installer

These two traits describe work as data (`plans.rs`). The host does the side effects.

- `Downloader::plan(&DownloadRequest)` returns a `DownloadPlan`: a label and a list of `Artifact`
  values. An artifact has a `url`, a lowercase hex `sha256`, an optional `Signature`, a `dest` in
  the plugin's data folder, and an optional `Unpack`. The host fetches, resumes and checks each
  file, and refuses one whose SHA-256 does not match. It unpacks an archive into `Unpack.into`
  (`ArchiveFormat`: `TarGz`, `Zip` or `AppImageInZip`) and deletes the archive.
- `Installer` installs, updates and removes emulator versions with the files the host downloaded:
  `available`, `installed`, `install`, `uninstall`, and `update`. The default `update` installs the
  new version and then uninstalls the old, so a failed install leaves the old one.

### Launcher

`Launcher::prepare(game, config)` returns a `LaunchSpec`. `config` is the game's merged settings:
the emulator's settings with the game's on top. The host starts, watches and stops the process, and
starts it only if `permissions.process` allows `program`. `on_exit` has a default that does nothing.

```rust
#[async_trait]
impl Launcher for Kyty {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let found = self.emu.games().find(self.emu.host().folders.as_ref(), game).await?;
        let builds = self.emu.builds();
        // `config` is the emulator settings with the game's settings on top; the game's
        // fields are optional, so null ones fall back to the emulator's.
        let settings = builds.settings().for_launch(config).await;
        let run = builds.run().await?;
        let args = self.args_for(&run.program, run.tag.as_deref(), &settings, LaunchArg::path(found.dir)).await;
        Ok(LaunchSpec { program: run.program, args, env: Default::default(), cwd: run.cwd, game_process: None })
    }
}
```

Source: `plugins/emulators/kytyps5/src/lib.rs`.

A `Location` says where a program or folder is: `Data(RelPath)` in the data folder,
`Folder(HostPath)` in a host folder, or `System(String)` on the `PATH`. A `LaunchArg` is text or a
path (`LaunchArg::path(..)`) that the host turns into a real path. Set `game_process` for a program
that hands the game to another one and may exit at once, such as a store's client.

### Configurator

`Configurator` describes and checks settings. See [Settings](#settings).

### Compatibility

`Compatibility::status(game)` returns a `CompatInfo`: a `CompatLevel` (`Unknown`, `Nothing`,
`Boots`, `Menus`, `Ingame`, `Playable`) and optional notes, source system, page and test date
(`extras.rs`).

### GameInstaller

`GameInstaller` installs games from packages and disc images the player has (`games.rs`). The plugin
reads and decodes. The host owns the job: progress, cancel, and clean-up after a crash.

- `candidates()` lists the `Installable` packages in folders the plugin may read.
- `begin(source, job)` checks `source`, starts the install in the background, and returns the work
  folder at once. The work moves into place when done, and reports through the `InstallJob`:
  `progress`, `cancelled`, `finished` (exactly once), and the optional `games_folder` and `take`.

A slot action named `HOST_INSTALL_GAME` asks the host to run an install.

### Releases

`Releases::releases(&Console)` returns the catalog of games to download for one console, as a
`ReleaseList`, or `None` when the plugin has no catalog for that console (`releases.rs`). A service
reads the lists. The core gives each list to the emulator that plays its console.

A `Release` holds the fields the host sorts, groups, searches and downloads by: `id` (the torrent's
info hash), `title_id`, `platform`, `title`, `region`, `size_bytes`, `torrent` (a magnet link),
`facts`, and more. Set `store` for a release a store installs, such as `steam://install/440`. The
manifest must allow that link in `permissions.links`. `Releases::store_installs` reports the
installs a store is doing. It returns none by default.

A `Fact` has a `FactKind` (`Publisher`, `ReleaseDate`, `Firmware`, `Compatibility`, and others), a
label, a value and an optional page.

### Metadata

`Metadata::describe(&GameRef)` returns a game's words and art as `GameMetadata`, or `None` when the
catalog has no such game. The host fills in what the `metadata` plugins leave out. The PlayStation
Store service is the example in this repo.

### System capabilities

A system plugin provides the machine's settings (`system.rs`). The core uses the first active
system plugin that provides each capability, and shows a capability nobody provides as
unavailable.

Quick calls answer within the call timeout. Long work (joining a network, a scan, pairing,
installing the OS) starts in the background and returns at once. It reports through the
`SystemJob` the host passes in: `progress`, `done`, `failed`, `cancelled`.

`VpnSettings` is not async. The host runs its blocking calls off its async threads.

The built-in system plugin (`plugins/system/host`) builds its manifest from `plugin.toml`, and on
macOS and Windows it keeps only `Vpn` and `Power`:

```rust
impl Plugin for HostSystem {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid");
        if !cfg!(target_os = "linux") {
            manifest.capabilities.retain(|c| matches!(c, Capability::Vpn | Capability::Power));
        }
        manifest
    }
}
```

Source: `plugins/system/host/src/lib.rs`.

## Settings

A plugin with settings provides `Configurator`: `schema(scope)`, `defaults(scope)` and
`validate(scope, value)`. A `ConfigScope` is `Emulator` (the plugin's own settings) or
`Game(id)` (one game's). The host stores the values through `ConfigStore`, and a theme draws the form
from the schema.

Write the settings as Rust types and let `schemars` make the schema. Point the derives at the
re-exports in `nexus_api`:

```rust
/// Per-game settings: anything set here wins over the KytyPS5 settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars")]
pub struct GameSettings {
    #[serde(default)]
    #[schemars(title = "Video out")]
    pub video_out: Option<VideoOut>,
    #[serde(default)]
    #[schemars(title = "Extra arguments")]
    pub extra_args: Option<String>,
}
```

Source: `plugins/emulators/kytyps5/src/lib.rs`.

The host reads these parts of the schema (`crates/nexus-core/src/settings_schema.rs`):

- `properties` of type boolean (a toggle), integer or number (a number), and string (text). Other
  properties are left out of the form.
- `enum`, `oneOf` and `anyOf` give a choice. A `$ref` into `$defs` is followed. A doc comment on a
  variant is its label.
- `Option<T>` becomes a field that may be unset. An optional choice gets an "As the emulator"
  entry.
- `title` is the label, `description` is the help line, and `default` is the value shown until the
  player sets one.
- `x-order` is a list of keys that sets the order of fields. The rest follow, by name.

An emulator sets `x-order` in its derive, for example `#[schemars(crate = "nexus_api::schemars",
extend("x-order" = ["own_build", "release_repo", ...]))]` (`plugins/emulators/kytyps5/src/lib.rs`).

A console emulator built on the kit does not implement `Configurator` by hand. It implements the
kit's `Settings` trait on its settings type, and `SettingsStore<S>` provides `Configurator` for
it: see [Shared code](#shared-code-emulator-kit). A game scope uses the `Game` type of that trait.

To show a settings form in Settings, return a `SlotContent::SettingsPanel` in the
`settings.panel` slot.

## Slots

A slot is a named place in the UI that plugins fill with data. A theme decides where and how it
draws the data. A plugin provides `Slots`:

```rust
#[async_trait]
pub trait Slots: Send + Sync {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>>;

    async fn run_action(&self, action: &str, game: Option<&str>, args: &serde_json::Value) -> PluginResult<()>;
}
```

Source: `crates/nexus-api/src/slots.rs`.

The `SlotId` values (`SlotId::ALL`):

| Slot | Where it shows |
| --- | --- |
| `game.badge` | Small labels on a game's card and detail page. |
| `game.action` | Buttons on a game's detail page. |
| `game.detail-section` | Blocks of extra information on a game's detail page. |
| `settings.panel` | A settings form in the Settings screen. |
| `emulator.panel` | Blocks on a plugin's page, and in the Emulator Management screen. |
| `catalog.action` | Buttons on the Catalog screen, and badges that say what they did, from a service with `releases`. |

The first three are per game (`SlotId::is_per_game`): the host asks only the plugin that owns the
game, and `SlotRequest.game` carries the plugin's own game id.

An item is a `SlotItem` with an `id` (unique within the plugin), an `Order` and a `SlotContent`:
`Badge`, `Action`, `Section` or `SettingsPanel`. `Order` has a `priority`, and `before` and `after`
lists of peer ids. `before` and `after` win over `priority`, and load order never matters.

When the player picks an `Action` button, the host calls `Slots::run_action` with the item's
`action` and `args`. Two action names belong to the host: `HOST_OPEN_SETTINGS` and
`HOST_INSTALL_GAME`.

The console emulator in the kit builds its slots like this (trimmed):

```rust
#[async_trait]
impl<F: Flavor> Slots for ConsoleEmulator<F> {
    async fn items(&self, request: &SlotRequest) -> PluginResult<Vec<SlotItem>> {
        let item = |id: &str, content| SlotItem { id: id.into(), order: Order::default(), content };
        Ok(match (request.slot, request.game.as_deref()) {
            (SlotId::SettingsPanel, None) => vec![item(
                "settings",
                SlotContent::SettingsPanel { title: F::NAME.into(), scope: ConfigScope::Emulator },
            )],
            _ => Vec::new(),
        })
    }
```

Source: `plugins/shared/emulator-kit/src/console.rs`, with the other match arms removed.

## Errors

Plugin calls return `PluginResult<T>`, which is `Result<T, PluginError>` (`error.rs`).

| Variant | Use |
| --- | --- |
| `NotFound` | The thing does not exist. `Catalog::details` returns it for an unknown game. |
| `InvalidInput` | Bad input. `Configurator::validate` returns it with a message the player can read. |
| `PermissionDenied` | The manifest does not allow it. The host returns it for a facade call. |
| `Network`, `Storage` | A network or storage failure. |
| `Unsupported` | The plugin cannot do it here. |
| `Precondition` | Something must happen first, such as installing a build. |
| `OwnBuildMissing` | The player's own build is on but missing. The host words it for the player. |
| `Timeout`, `Panicked` | The host reports these about a plugin call. |
| `Other` | Anything else. |

Most variants carry text. A typed precondition (`OwnBuildMissing`) carries data, and the host words
it, so a plugin does not name the host's screens. `PluginError` converts from `InvalidPath`,
`InvalidId` and `serde_json::Error`, all as `InvalidInput`, so `?` works on them.

## Shared code: emulator-kit

`emulator-kit` (`plugins/shared/emulator-kit`) is a crate, not a plugin. Two emulators use it
as a whole (KytyPS5 and shadPS4), and the PC plugin uses parts of it. It depends on `nexus-api` like
a plugin, plus `tokio` and the Kraken decoder for PS5 packages.

A console emulator describes itself, and the kit runs the rest:

- A `Flavor` (`builds.rs`) describes the emulator's builds. The `BuildManager` plans downloads,
  installs, updates, rolls back and removes builds, and picks the program a game starts. A new
  build runs once with `--help` before it becomes current (`Flavor::runs`). The kit keeps the
  current build and the one before it, in the plugin's `versions.json`.
- A `ConsoleSpec` (`console.rs`) describes the console: its games, package formats, compatibility
  list, keyboard keys and game page section. `ConsoleEmulator` implements `Catalog`,
  `Compatibility`, `Configurator` and `Slots` from it.
- The plugin keeps its settings type (`Settings`), its launch arguments, and its game installs.

The kit reaches the host only through `Host` (`host.rs`): storage, folders, programs, HTTP, config,
the log and the platform. `Host::of(&ctx)` builds it from a `PluginContext`. It never takes the
whole context, so its tests run on small in-memory doubles.

A `Flavor` implementation, trimmed to its required items:

```rust
#[async_trait]
impl Flavor for KytyFlavor {
    type Settings = Settings;
    const NAME: &'static str = "KytyPS5";
    const CONSOLE: &'static str = CONSOLE;
    const OWN_VERSION: bool = true;
    const OFFICIAL_STYLE: ActionStyle = ActionStyle::Primary;

    fn release_source(&self, settings: &Settings) -> PluginResult<ReleaseSource> {
        let (repo, prereleases) = release_source(settings, self.platform)?;
        Ok(ReleaseSource { url: releases_url(&repo), cache: releases_cache(&repo)?, prereleases })
    }

    fn asset_matches(&self, name: &str) -> bool {
        asset_matches(name, self.platform)
    }

    fn archive_format(&self, asset: &str) -> ArchiveFormat {
        if asset.ends_with(".zip") { ArchiveFormat::Zip } else { ArchiveFormat::TarGz }
    }

    async fn program(&self, storage: &dyn Storage, tag: &str) -> PluginResult<RelPath> {
        data::binary(storage, tag, self.platform).await
    }

    /// The test run's `--help` must print KytyPS5's banner.
    fn runs(&self, help: &str) -> bool {
        banner_version(help).is_some()
    }

    fn own_build(&self) -> OwnBuild {
        let name = data::binary_name(self.platform);
        let files = OWN_PATHS.map(|dir| if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") });
        OwnBuild { folder: OWN_BUILD, itself: true, files: files.to_vec(), system: "kyty_emulator" }
    }
}
```

Source: `plugins/emulators/kytyps5/src/lib.rs`, with the optional methods removed (`pretty`,
`version`, `work_dir`, `on_switch`, `on_leave_own`).

The optional methods have defaults. `on_switch` and `on_leave_own` run when the current build
changes, so an emulator that keeps its data next to its program can move it.

The whole `init` of the KytyPS5 plugin wires the kit to `Provided`:

```rust
#[async_trait]
impl CodePlugin for KytyPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let host = Host::of(&ctx);
        let spec = ConsoleSpec {
            console: Console::Ps5,
            packages: &[Format::Ps5Package, Format::Exfat],
            compat: CompatList { url: COMPATIBILITY, page: LIST_PAGE, parse: compat::kyty },
            keys: KEYS,
            section: "kyty",
            version_row: false,
            own_row: true,
        };
        let emu = Arc::new(ConsoleEmulator::start(KytyFlavor { platform: host.platform }, spec, host).await?);
        let kyty = Arc::new(Kyty { emu: emu.clone(), options: Default::default() });
        Ok(Provided {
            catalog: Some(emu.clone()),
            downloader: Some(emu.builds().clone()),
            installer: Some(emu.builds().clone()),
            configurator: Some(emu.clone()),
            launcher: Some(kyty.clone()),
            compatibility: Some(emu.clone()),
            game_installer: Some(kyty),
            slots: Some(emu),
            ..Provided::default()
        })
    }
}
```

Source: `plugins/emulators/kytyps5/src/lib.rs`.

For the PC games flow that these folders and installs feed, see
[reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).

## Testing

`nexus-testkit` (`crates/nexus-testkit`) gives plugin tests a real core over fakes. The root
module comment in `crates/nexus-testkit/src/lib.rs` lists the pieces. The ones a plugin author
uses:

- `FakeHost`: builds a real core over a temporary data folder, with fake HTTP, fake processes and
  a `FakeClock`. `.plugin(StubPlugin)` adds a stub, `.built_in(PluginBox)` adds a real plugin such
  as `PluginBox::code(KytyPlugin)`, `.third_party(PluginBox)` adds one with the third-party origin.
  `.boot()` returns a `TestHost` with the `Kernel` and the `BootReport`. `.boot_core()` also starts
  the app and returns a `TestCore`.
- `StubPlugin`: a plugin from a manifest and test-chosen behavior. `from_toml(..)` builds it,
  `.provides(..)` sets what `init` returns, and `panics_on_init`, `hangs_on_init`, `fails_on_init`
  and `hangs_on_shutdown` script a failure. `init_calls` and `shutdown_calls` count calls.
- `FakeEmulator`: a small emulator a test scripts, from its catalog to the download, install,
  launch and exit of its build. It keeps every capability contract.
- `FakeCatalog`, `FakeStore` and `FakeService`: plugins built from a few choices.
- `FakeSystem`: a system plugin over a machine the test controls.
- `FakeHttp`: serves bodies by URL (`serve`, which returns the body's SHA-256) and redirects.
- `MemStorage`: a plugin's data folder in memory.

`StubPlugin` and `FakeEmulator` show how a plugin is built from a manifest string and a closure.
This is how `FakeEmulator` makes its plugin:

```rust
StubPlugin::from_toml(&format!(
    "id = \"{plugin_id}\"\nkind = \"emulator\"\n{names}\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
     capabilities = [\"catalog\", \"downloader\", \"installer\", \"launcher\", \"slots\"]\n[permissions]\n{permissions}\n"
))
.provides(move |ctx| {
    *emu.storage.lock().unwrap() = Some(ctx.storage().clone());
    let e = Arc::new(emu.clone());
    Ok(Provided {
        catalog: Some(e.clone()),
        downloader: Some(e.clone()),
        installer: Some(e.clone()),
        launcher: Some(e.clone()),
        slots: Some(e),
        ..Provided::default()
    })
})
```

Source: `crates/nexus-testkit/src/emulator.rs`, `FakeEmulator::plugin_named`.

### The contract suite

`contract::emulator(&test, &plugin_id)` checks any emulator plugin against the capability contracts
(`crates/nexus-testkit/src/contract.rs`). Run it on a started core with the plugin loaded. It
panics with a message that starts `contract <capability>:` when the plugin breaks a rule. It runs the
checks for the capabilities the plugin declares:

- `catalog`: the list is not empty, ids are unique, paging gives the same items, an unknown game is
  `NotFound`, and a search finds a game by its title.
- `downloader`: a plan has a label, files, a 64-digit hex SHA-256, and a destination for each file.
- `configurator`: the schema is a JSON object, the defaults validate, and garbage is `InvalidInput`.
- `compatibility`: a known game answers, and an unknown game fails.
- `slots`: item ids are unique within each slot.
- `installer`: an install through the host shows up as installed, and an uninstall removes it.
- `launcher`: a launch through the host spawns a program from the data folder, a granted host
  folder, or the system `PATH`, and the host reports the exit.
- `keymap`: each button appears once, and no button or key text is empty.

`contract::archives` does the same for an `Archives` adapter (host code, not plugins).

The tests live in the `tests` crate, one module per area, in `tests/tests/seams/`
(see [development.md](development.md)). `contracts.rs` there shows the suite catching a plugin that
lists one game twice, and passing `FakeEmulator`. `kyty.rs`, `shadps4.rs`, `pc.rs` and `steam.rs`
each boot the real plugin on a fake host and call `contract::emulator`. Unit tests of the plugin's
own functions sit in the plugin crate, in `#[cfg(test)]` modules.

Run the checks with the project's check command, in [development.md](development.md).

## Checklist for a new plugin

1. Pick the kind. Pick it by the privileges you need. Data capabilities work in any kind.
2. Create the crate under `plugins/emulators/`, `plugins/services/` or `plugins/system/`. Copy
   a small one: `plugins/services/playstation-store` for a service. Depend on `nexus-api` (and
   `emulator-kit` for a console emulator). Use `[lints] workspace = true`.
3. Add the crate to `[workspace] members` and to `[workspace.dependencies]` in the root
   `Cargo.toml`.
4. Write `plugin.toml`: `id`, `kind`, `name`, `version`, `host_api` (a range that matches
   `API_VERSION`), `platforms`, `capabilities`, and `console` for an emulator.
5. Ask for the least permissions: only the hosts in `network`, only the folders in `filesystem`
   (`write = true` only where needed), only the programs in `process`, and only the prefixes in `links`.
6. Write the `Plugin` and `CodePlugin` impls. Embed the manifest with `include_str!`. Return in
   `Provided` exactly the capabilities the manifest declares.
7. Implement the traits. Return `PluginError::NotFound`, `InvalidInput` and the other typed
   errors. Do not block a thread without awaiting.
8. Register the plugin with `inventory::submit! { PluginFactory::code(..) }`.
9. Link the crate from `crates/nexus-app`: add it to its `Cargo.toml` and add a `use <crate> as _;`
   line in `crates/nexus-app/src/main.rs`.
10. Write tests. Boot the plugin with `FakeHost::new().built_in(PluginBox::code(..)).boot_core()`
    and call `contract::emulator` for an emulator. Add a module in `tests/tests/seams/` and a
    dev-dependency in `tests/Cargo.toml`.
11. Run the repo's check command (format, clippy with warnings as errors, and every test): see
    [development.md](development.md).
12. If you raise `API_VERSION`, raise `host_api` in every built-in `plugin.toml` in the same change.

## Limitations

- **No runtime for third-party code.** A folder in `extensions/` can only be a data-only theme.
  Writing a plugin means adding a crate to this repo and building Nexus. The `Loader` trait
  exists so a runtime could be added, but the app registers none. The comment on `Origin` says
  built-in plugins run "without a sandbox", which suggests a sandbox for others. None exists.
- **Manifest `config_schema` is not read.** `PluginManifest` has the field, but no code outside
  `nexus-api` uses it. Settings schemas come from `Configurator::schema`.
- **Panic guard and timeout are cooperative.** A plugin that blocks a thread without awaiting is
  not stopped. See [Timeouts and panics](#timeouts-and-panics).
- **Not verified here.** The doc samples are not compiled. The checklist is derived from reading
  the code and the existing plugins, not from adding a new plugin.
