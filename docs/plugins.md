# Writing a plugin

This guide covers emulator and service plugins: Rust crates that the launcher links in and calls
through the plugin contract in `crates/nexus-api`. Themes are plugins too, but they hold no code;
see [docs/themes.md](themes.md). The design behind all of this is in
[docs/architecture.md](architecture.md), and the build setup is in
[docs/development.md](development.md).

The real plugins are the best examples, and this guide quotes them:

| Plugin | Kind | What to read it for |
|---|---|---|
| `plugins/emulators/kytyps5` (`emu-kyty`) | emulator | Every capability: builds from GitHub, settings, slots, a game installer |
| `plugins/emulators/shadps4` (`emu-shadps4`) | emulator | The same, with an AppImage build and DLC into another program's folder |
| `plugins/emulators/pc` (`emu-pc`) | emulator | The smallest emulator: no builds, games in host folders, programs on `PATH` |
| `plugins/emulators/steam` (`emu-steam`) | emulator | No console: another program's library, read-only folders with a search default, and a launch that hands the game over (`game_process`) |
| `plugins/services/playstation-store` | service | The smallest plugin: one `Metadata` capability and a cache in storage |
| `plugins/services/release-catalog` | service | `Releases`, settings, slots, a background task, events, `shutdown` |
| `plugins/shared/emulator-kit` | not a plugin | Shared code the emulators use |

## Contents

- [Kinds of plugin](#kinds-of-plugin)
- [Built in, or from an extensions folder](#built-in-or-from-an-extensions-folder)
- [The crate](#the-crate)
- [The manifest](#the-manifest)
- [Lifecycle](#lifecycle)
- [PluginContext](#plugincontext)
- [Capabilities](#capabilities)
- [Settings](#settings)
- [Slots](#slots)
- [Errors](#errors)
- [Shared code: emulator-kit](#shared-code-emulator-kit)
- [Testing](#testing)
- [Checklist for a new plugin](#checklist-for-a-new-plugin)

## Kinds of plugin

`nexus_api::PluginKind` has four values. A theme implements `ThemePlugin` and has no code
beyond its files. The other three kinds implement one trait, `CodePlugin`, and differ only in
their privileges (ADR 0001, `docs/adr/0001-capabilities-follow-the-capability-not-the-kind.md`).

| `kind` | Trait | What it does |
|---|---|---|
| `emulator` | `CodePlugin` | Runs games: lists them, installs builds, launches them |
| `service` | `CodePlugin` | Adds to the games other plugins find, and runs none |
| `theme` | `ThemePlugin` | Files only: `theme.toml`, Slint markup and assets |
| `system` | `CodePlugin` | The machine's own settings: network and Wi-Fi, VPNs, Bluetooth, sound, power and battery, Nexus OS's updates and installer |

All four implement `Plugin`, which has `manifest()` and an optional `shutdown()`. A
`CodePlugin` also implements `init(ctx: PluginContext) -> PluginResult<Provided>`. The host
holds a plugin as a `PluginBox`: `PluginBox::code(..)` or `PluginBox::theme(..)`. A theme
manifest with code, or a code manifest with a theme, doesn't load.

Any plugin with code may provide the data capabilities: `catalog`, `releases`, `metadata`,
`slots`, `configurator`, `launcher` and the rest. Steam's emulator plugin, for example, lists
releases and describes games. The kind governs privileges only, in `validate` in
`crates/nexus-core/src/kernel.rs`:

- Only a system plugin may provide the system capabilities (`Capability::SYSTEM`: `network`,
  `vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install`).
- Only a system plugin may run programs long (`permissions.long_running`, `Programs::stream`)
  or through polkit (`permissions.polkit`).
- A theme declares only `theme`, and no other kind may declare it.

For each system capability the core uses the first active system plugin that provides it; one
nobody provides shows as unavailable. The built-in one is `plugins/system/host` (`id =
"system"`).

## Built in, or from an extensions folder

There are two ways a plugin reaches the host, and they differ in what they may hold.

**Built in.** The crate is linked into the `nexus` binary and registers itself at compile time
with `inventory`:

```rust
inventory::submit! { PluginFactory::code(|| Box::new(PcPlugin)) }
inventory::submit! { PluginFactory::code(|| Box::new(PlayStationStore)) }
```

`KernelBuilder::built_in_plugins()` creates every registered factory. A factory only registers
if its crate is linked, so `crates/nexus-app` lists each plugin crate in its `Cargo.toml` and
keeps a `use emu_pc as _;` line for it in `src/main.rs`. Built-in plugins have
`Origin::BuiltIn` and run in-process with `runtime` left at its default, `native`.

**From an extensions folder.** `KernelBuilder::discover(dir)` treats each subfolder of `dir`
that holds a `plugin.toml` or `theme.toml` as a third-party plugin (`Origin::ThirdParty`). The
app discovers two folders: the one given with `--extensions <folder>` (by default
`<data-dir>/extensions`), and `extensions/` next to the binary (`Contents/Resources/extensions`
in the macOS app).

A third-party plugin must be data-only: `validate` in `crates/nexus-core/src/kernel.rs` refuses
`runtime = native` from a folder with "third-party plugins must be data-only", and refuses an
emulator or service with `runtime = data` ("needs code"). The only loader is
`DataThemeLoader`, so today a folder in `extensions/` can hold a theme and nothing else. An
emulator or service plugin is always a built-in crate. In `--safe-mode`, third-party plugins
don't load at all.

In tests, `FakeHost::third_party(plugin)` registers in-process code as third-party, to check
those rules.

## The crate

A plugin crate depends on `nexus-api` only (plus `emulator-kit` for emulators, if it helps).
`nexus-api` re-exports what plugins need: `async_trait`, `inventory`, `serde`, `serde_json`,
`schemars` and `semver`. Point derives at the re-exports:

```rust
use nexus_api::serde::{Deserialize, Serialize};
use nexus_api::schemars::{self, JsonSchema};

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde")]
#[schemars(crate = "nexus_api::schemars")]
pub struct Settings { /* ... */ }
```

The `Cargo.toml` of `plugins/services/playstation-store` is the whole shape:

```toml
[package]
name = "playstation-store"
description = "The PlayStation Store's descriptions of PS4 and PS5 games ..."
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

Every plugin embeds its own manifest and parses it in `manifest()`:

```rust
const MANIFEST: &str = include_str!("../plugin.toml");

#[async_trait]
impl Plugin for PcPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::from_toml(MANIFEST).expect("the bundled manifest is valid")
    }
}
```

Files the plugin ships with go into the binary too. `release-catalog/build.rs` embeds the
catalog files from `assets/catalog/`, and the default theme's `build.rs` embeds every file
under `theme/`.

## The manifest

`plugin.toml` parses into `nexus_api::PluginManifest` (`crates/nexus-api/src/manifest.rs`).
This is the manifest of `plugins/emulators/pc`, with its comments trimmed:

```toml
id = "emu-pc"
kind = "emulator"
name = "PC"
version = "0.1.0"
host_api = "^0.2"
platforms = ["linux", "macos", "windows"]
console = "PC"
capabilities = ["catalog", "launcher", "slots", "game_installer"]
install_note = "Installs a PC game from its folder, a GOG installer or an AppImage, ..."

[permissions]
process = ["folder:games", "system:umu-run", "system:wine", "system:wine64", "system:sh"]

[[permissions.filesystem]]
folder = "games"
write = true

[[permissions.filesystem]]
folder = "packages"
```

And a service, `plugins/services/playstation-store/plugin.toml`:

```toml
id = "playstation-store"
kind = "service"
name = "PlayStation Store"
version = "0.1.0"
host_api = "^0.2"
platforms = ["linux", "macos", "windows"]
capabilities = ["metadata"]

[permissions]
network = ["m.np.playstation.com", "image.api.playstation.com"]
```

### Fields

| Field | Meaning |
|---|---|
| `id` | `PluginId`: 1 to 64 lowercase letters, digits or `-`, starting with a letter. Two plugins with one id: the second is rejected (`LoadError::Duplicate`). |
| `kind` | `emulator`, `service`, `theme` or `system`. The kind sets the privileges, not the data capabilities: see "Kinds of plugin". |
| `name` | What the player sees. An emulator names its console: `"PS5"`. |
| `emulator` | The program the emulator runs, such as `"KytyPS5"`. Sentences about the program use it. Empty for PC. `emulator_name()` falls back to `name`. |
| `version` | The plugin's semver version. |
| `host_api` | The `nexus-api` versions it works with, as a semver range. The host refuses a plugin whose range doesn't include `API_VERSION` (`LoadError::HostApi`). `API_VERSION` changes by hand with the API, not with Nexus's releases. |
| `platforms` | `linux`, `macos`, `windows`. Empty means all. An emulator made for other systems still loads, turned off, so the player can use their own build; a theme for another system is refused. |
| `console` | `Console`: the console an emulator plays (`PS5`, `PS4`, `PC`), stored in uppercase. A `releases` service gives the emulator that console's catalog. |
| `capabilities` | The set of `Capability` values the plugin provides (snake case: `game_installer`, `os_updates`). `init` must provide exactly this set. |
| `permissions` | `network`, `filesystem` and `process`; see below. |
| `dependencies` | `[[dependencies]]` with `id`, `version` (a range, default any) and `optional`. Dependencies activate first; a cycle or a missing required one rejects the plugin. |
| `activation` | `lazy` (default): `init` runs the first time the host needs a capability. `eager`: at boot. |
| `runtime` | `[runtime] type = "native"` (default, built-in only) or `"data"` (themes only). |
| `assets` | A folder of images that slot data may name as `ImageSource::Asset`. A built-in plugin gives `Provided::assets` instead. |
| `config_schema` | JSON Schema of the plugin-wide settings. The built-in plugins return theirs from `Configurator::schema` instead. |
| `requires` | `[requires]` with `vulkan` (`"1.3"`), `avx2`, and `[requires.recommended]` `cpu_threads`, `memory_gb`, `gpu_memory_gb`, `discrete_gpu`. The "This PC" check compares them with the computer. |
| `keymap` | `[[keymap]]` entries of `button` and `keys`, in order. The launch splash and the emulator's page show them. |
| `install_note` | One or two sentences the install sheet shows for an emulator with a game installer. |
| `system` | `[system]`: the system the plugin's games show under. `id` is the code in their `platform` (`PS5`, `PC`, `Steam`), `name` its full name ("PlayStation 5"), and optional `color` as `#rrggbb` (another form doesn't load). Themes draw a game's system from these, not from its code: a plugin without one gets the theme's neutral look and its code as its name. |

Sections for one kind only, such as `[theme]`, land in `extra` and are parsed by the crate that
owns them. Unknown fields inside `permissions`, `requires`, `keymap` and `system` are errors.

### Permissions

The host enforces every entry. Nothing outside them works.

**`network`**: hosts the plugin's `HttpClient` may reach. An exact name (`github.com`) or a
suffix pattern (`*.githubusercontent.com`, which also matches the bare domain). Redirects are
checked too, so list the CDN a download redirects to. shadPS4 lists
`["api.github.com", "github.com", "*.githubusercontent.com"]`.

**`filesystem`**: named host folders, each a `[[permissions.filesystem]]` `PathGrant`:

| Key | Meaning |
|---|---|
| `folder` | The name: lowercase letters, digits and `-`. |
| `write` | `true` to write, rename, remove or mark programs. Default read-only. |
| `default` | Where it is until the player maps it: one path (`"~/Games"`), or a list to search, each one that exists in order (`["~/KytyPS5", "~/src/KytyPS5"]`). `~/` is the home folder. |
| `default_on` | A per-system default: `default_on = { macos = "~/Library/Application Support/shadPS4" }`. |
| `title`, `help` | What Settings and the plugin's page call the folder, and a line under it. |
| `listed_in` | More folders that a file in a default folder lists, while the player hasn't mapped the folder: `{ file = "steamapps/libraryfolders.vdf", key = "path", holds = "steamapps" }` (Steam's libraries). The host reads the file (Valve's text: `"key" "value"`), and keeps each value that is an absolute path to a folder holding `holds`, once. Read-only: a grant with it and `write = true` doesn't load. |

Plugins that ask for the same name share the player's mapping. Two names are the host's own,
in `SHARED_FOLDERS`: `games` (`GAMES_FOLDER`, default `~/Games`) and `packages`
(`PACKAGES_FOLDER`, default `~/Downloads`). Settings shows each once, with the host's title and
help; a plugin's own `default`, `title` and `help` for them are ignored. Each console's games
install into a folder named for the console inside the first `games` folder (`PS5/`, `PS4/`,
`PC/`), and its packages are found in that console's folder inside each `packages` folder.

**`process`**: programs the host may start for the plugin, through `Programs` or a
`LaunchSpec`. Three forms, matched by `Permissions::allows_program`:

| Rule | Matches |
|---|---|
| `builds/*/emu`, `builds/*/*.AppImage` | A path in the plugin's data folder. `*` as a whole part matches any name; `*.ext` any name with that ending. The parts must line up one to one. |
| `folder:<name>` | Any file in a host folder the manifest asks for. |
| `system:<name>` | A program on the system's `PATH`, by file name, never a path. |

An empty list means the plugin may not start anything.

**`links`**: links the host may open for the plugin with the system's handler, by how they
start: `["steam://install/"]`. A release's `store` link opens only when one of them starts it,
and only when it is one line of plain characters.

**`long_running`** and **`polkit`**, system plugins only: entries of `process` (`system:<name>`)
that may run with no time limit, their output read line by line (`Programs::stream`), and that
may run as the administrator through `pkexec`. Each must also be in `process`; the host refuses
them from any other kind.

## Lifecycle

`crates/nexus-core/src/kernel.rs` drives it:

1. **Discover.** Built-in factories and extension folders become candidates.
2. **Validate.** Bad manifests, a wrong `host_api`, bad folder names, safe mode, kind and runtime
   rules. Each rejection lands in `BootReport::rejected` with a `LoadError`.
3. **Resolve.** Dependencies are ordered; cycles and missing ones are rejected.
4. **Load.** `manifest()` is read. No work happens yet.
5. **Init.** For `activation = "eager"` at boot, otherwise the first time the host needs a
   capability. The host checks that `Provided::capabilities()` equals the manifest's
   `capabilities`; if not, it calls `shutdown` and fails the plugin with
   `LoadError::CapabilityMismatch`.
6. **Shutdown.** At most once per activation, under a timeout (5 s by default), dependents first.
   It runs through `Kernel::deactivate`, and for every plugin when the app quits
   (`Kernel::shutdown`). Turning a plugin off in the app makes the core stop using it; it
   doesn't call `shutdown`.

Every call into a plugin runs under a timeout (`HostConfig::call_timeout`, 30 s) and a panic
guard. A panic or a timeout becomes `PluginError::Panicked` or `PluginError::Timeout` for that
call; the app goes on.

A plugin that starts its own task must stop it in `shutdown`. `release-catalog` keeps the
`JoinHandle` of its refresh task and aborts it there, and also aborts a task left from an
activation before.

## PluginContext

`init` gets a `PluginContext` (`crates/nexus-api/src/context.rs`). It is the only way a plugin
reaches the host, and it is `Clone`, so capability objects keep their own copy:

```rust
#[async_trait]
impl CodePlugin for PcPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        let platform = ctx
            .services()
            .get::<HostInfo>()
            .and_then(|h| h.platform)
            .or_else(Platform::current)
            .unwrap_or(Platform::Linux);
        let pc = Arc::new(Pc { ctx, platform });
        Ok(Provided {
            catalog: Some(pc.clone()),
            launcher: Some(pc.clone()),
            game_installer: Some(pc.clone()),
            slots: Some(pc),
            ..Provided::default()
        })
    }
}
```

| Method | Type | What it gives |
|---|---|---|
| `plugin()` | `&PluginId` | The plugin's own id. |
| `config()` | `ConfigStore` | `get(&scope)` and `set(&scope, value)`: the plugin's settings per `ConfigScope`, stored by the host in `config.json` under `plugins.<id>`. `get` returns `Null` when nothing is stored. |
| `storage()` | `Storage` | Files in the plugin's data folder, `<data-dir>/plugins/<id>`: `read`, `write`, `write_at`, `exists`, `list`, `remove`, `rename`, `set_executable`. Paths are `RelPath`s. |
| `folders()` | `Folders` | The named host folders from `permissions.filesystem`, through `HostPath`s. |
| `programs()` | `Programs` | `find(&Location)` and `output(&Location, args, timeout)` for the programs `permissions.process` allows. |
| `http()` | `HttpClient` | `send(HttpRequest)` to the hosts in `permissions.network`. `HttpResponse::json()` fails on a non-2xx status. |
| `log()` | `Logger` | `debug`, `info`, `warn`, `error` into the host log, tagged with the plugin id. |
| `events()` | `EventSink` | `publish(topic, payload)` sends an `Event::Custom`; `subscribe()` is a stream of every `Event`. |
| `services()` | `ServiceRegistry` | Host services by type: `get::<HostInfo>()` and `get::<VpnGate>()` are always there. |

### Paths: RelPath and HostPath

`RelPath` is a path inside a folder the host owns. It is relative, uses `/`, and has no `.`,
`..`, empty part, `\` or `:`, so joining it to its base can never leave the base.
`RelPath::new` checks that; `RelPath::root()` is the base itself; `join(name)` checks the
part it adds.

`HostPath` is a path in a host folder: `folder` (the grant's name), `root` (which of the
player's folders for that name, an index into `Folders::roots`) and `path` (a `RelPath`
inside it). A plugin finds its games by walking the roots, as `Pc::games` does:

```rust
for root in folders.roots(GAMES).await? {
    let dir = HostPath::new(GAMES, root.root, path(CONSOLE_FOLDER)?);
    for entry in folders.list(&dir).await.unwrap_or_default() {
        // ...
    }
}
```

`Folders` refuses a name the manifest doesn't ask for, a write without `write = true`, and a
symlink that leads out of the folder. `read_range` reads at most `MAX_FOLDER_READ` (16 MiB) per
call; `write_at` writes big files in pieces of that size; `size` counts at most
`MAX_SIZE_FILES` entries. `rename` moves within one folder and root only, and fails if the
target exists.

### Programs

`Programs::output` runs an allowed program in its own folder with no input, for at most
`MAX_PROGRAM_TIME` (10 s), and returns at most `MAX_PROGRAM_OUTPUT` (64 KiB) of its standard
output. Use it to check a build: KytyPS5 runs each new build with `--help` and reads which
options it knows. `find` returns the real path, or `None`; for `Location::System` it looks on
`PATH`, which is how the PC plugin picks `umu-run` or `wine`.

`Programs::stream` (system plugins, `long_running`) runs a program until it ends, hands each
line of its output and errors to a callback, stops it when a cancel check says so, and returns
its exit code. With `elevated` it runs through `pkexec` (`polkit`); pkexec's 126 or 127 means the
player dismissed the password prompt. An action that must report a failure (forgetting a network,
switching the sound output) runs this way, for its exit code.

A plugin never starts a game itself. It returns a `LaunchSpec`, and the host starts, watches and
stops the process.

### Host services

`HostInfo` has `api_version`, `platform` (an `Option<Platform>`) and `safe_mode`. Prefer its
`platform` over `Platform::current()`: tests set it with `FakeHost::platform`.

`VpnGate::check()` tells whether the player's "Download only through a VPN" lets the plugin
fetch now: `Ok(VpnPass::Open)`, `Ok(VpnPass::Unbound { reason })` where the system can't bind
traffic to a VPN, or `Err(VpnBlock::Waiting | NoVpn)`. Ask it before fetching what the player
downloads in bulk; `release-catalog` asks it before a forum refresh. The plugin's own HTTP does
not go through the VPN.

### Events

The bus carries `Event` (`crates/nexus-api/src/event.rs`): plugin activation, downloads,
installs, processes, `LibraryChanged`, `StateChanged`, `ThemeChanged`, and `Custom`. A plugin
publishes only `Custom`, with its id attached by the host. `release-catalog` publishes
`RELEASES_CHANGED` (`"releases.changed"`) when its catalogs change between the core's asks, and
the core asks for them again.

## Capabilities

`Provided` (`crates/nexus-api/src/plugin.rs`) holds one optional `Arc` per capability. Its
`capabilities()` set must equal the manifest's. The traits are all `async_trait`, `Send + Sync`,
and return `PluginResult`.

| `Capability` | `Provided` field | Trait and methods | File |
|---|---|---|---|
| `catalog` | `catalog` | `Catalog`: `list(PageRequest)`, `search(&SearchQuery)`, `details(game)`, `artwork(game, ArtworkKind)` | `catalog.rs` |
| `downloader` | `downloader` | `Downloader`: `plan(&DownloadRequest) -> DownloadPlan` | `plans.rs` |
| `installer` | `installer` | `Installer`: `available`, `installed`, `install(version, files)`, `uninstall(version)`, `update(from, to, files)` | `plans.rs` |
| `configurator` | `configurator` | `Configurator`: `schema(&scope)`, `defaults(&scope)`, `validate(&scope, value)` | `config.rs` |
| `launcher` | `launcher` | `Launcher`: `prepare(game, config) -> LaunchSpec`, `on_exit(game, &ExitInfo)` | `plans.rs` |
| `compatibility` | `compatibility` | `Compatibility`: `status(game) -> CompatInfo` | `extras.rs` |
| `slots` | `slots` | `Slots`: `items(&SlotRequest)`, `run_action(action, game, args)` | `slots.rs` |
| `game_installer` | `game_installer` | `GameInstaller`: `candidates()`, `begin(&HostPath, Arc<dyn InstallJob>) -> HostPath` | `games.rs` |
| `metadata` | `metadata` | `Metadata`: `describe(&GameRef) -> Option<GameMetadata>` | `catalog.rs` |
| `releases` | `releases` | `Releases`: `releases(&Console) -> Option<ReleaseList>` | `releases.rs` |

`theme` is a capability too, but only a theme's manifest declares it, and no `init` provides it.
The system capabilities are in "The machine (system plugins)" below.

The files are in `crates/nexus-api/src/`. What the built-in plugins provide:

| Plugin | Capabilities |
|---|---|
| `emu-kyty`, `emu-shadps4` | catalog, downloader, installer, configurator, launcher, compatibility, slots, game_installer |
| `emu-pc` | catalog, launcher, slots, game_installer |
| `emu-steam` | catalog, launcher, slots, releases, metadata |
| `release-catalog` | releases, configurator, slots |
| `playstation-store` | metadata |

The API has no seam that no plugin uses: saves, shader caches, launch middleware, and firmware
or game-file downloads came out in API 0.2.0, and come back when a plugin needs one. An
emulator's controllers come from its manifest's `[[keymap]]`: with keys listed, its page says
it plays with a controller or the keyboard.

### Catalog

The games an emulator can run, by the plugin's own id for each game. The host makes the id
global as a `GameKey` (`<plugin-id>:<game-id>`). `details` fails with `NotFound` for an
unknown game. Ids must be unique; the contract suite checks it.

`GameSummary` carries `title_id` and `content_id` when the plugin knows them: a `Metadata`
service describes games by those. `GameDetails::facts` are `GameFact`s; themes know the ids
`region`, `age`, `version`, `title-id`, `content-id` and `folder`, and show others by label.
`GameDetails::folder` is the game's `HostPath`: it gives the Game Hub "Open game folder", and
"Uninstall" when the folder is not a root of a folder grant with `write = true`.

Images are `ImageSource`: `Url`, `Asset` (a `RelPath` in the plugin's assets) or `Folder` (a
`HostPath`, such as a game's own icon).

### Downloader and Installer: plans, not side effects

A plugin never fetches files itself. `Downloader::plan` returns a `DownloadPlan`: a `label` and
`Artifact`s, each with a `url`, an optional `size`, a lowercase hex `sha256`, an optional
Ed25519 `signature`, a `dest` in the data folder and an optional `Unpack` (`TarGz`, `Zip` or
`AppImageInZip`, into a folder that is replaced). The host downloads, resumes, checks the hash
and unpacks, then calls `Installer::install` (or `update`) with the `DownloadedArtifact`s.
KytyPS5's `plan` is a good example: it refuses a build GitHub gives no checksum for.

`InstalledVersion::own` marks a build the player brought; the host never updates it by itself.
`newest_first` sorts version strings, semver first.

### Launcher

`prepare(game, config)` returns a `LaunchSpec`: a `program` (`Location`), `args` and `env` of
`LaunchArg`s, and an optional `cwd`. `config` is the game's merged settings: the emulator's,
then the game's on top.

`Location` is `Data(RelPath)` in the data folder, `Folder(HostPath)` in a host folder, or
`System(name)` on `PATH`. A `LaunchArg` is text or `LaunchArg::path(location)`, which the host
turns into a real path. A system program needs a `cwd`. The host spawns only what
`permissions.process` allows.

A program that hands the game to another one, as a store's client does, sets `game_process`:
words of the game's own command line. The game is then that process and the ones below it, and
the program started here is neither waited for nor stopped. The Steam plugin sets
`SteamLaunch AppId=<id> `. See "A game another program starts" in
[the desktop reference](reference/desktop.md).

Fail with `PluginError::Precondition` and a sentence for the player when something must happen
first ("install KytyPS5 first"); the Game Hub shows it. A precondition the host knows has a
type of its own, and the host words it: `OwnBuildMissing { has_downloaded, folder, program }`
says the player's own build is on but isn't there, and the host names the emulator's page.

### GameInstaller

Installs games from packages, disc images or finished catalog downloads into the `games`
folders. `candidates()` lists what the plugin can install from the folders it may read. `begin`
checks the source, starts the work in the background, and returns at once with the work folder.
The work reports through `InstallJob`: `progress`, `cancelled`, and `finished` exactly once. On
an error the work folder must be gone; the host removes it if the app stops first.
`InstallJob::games_folder` is the folder the player picked, and `take` lets the host move a
folder it made itself (such as unpacked archives) in place of a copy.

A slot action named `HOST_INSTALL_GAME` (`"host:install-game"`) with a `HostPath` as `args`
asks the host to run the install as a job in Downloads. KytyPS5 offers one such button per
candidate on its page. `plugins/shared/emulator-kit/src/installs.rs` holds the shared work:
read a tree through `Folders`, write a `.part` folder, move it into place.

For the Catalog's downloads to install, the emulator names its `console`, asks for `packages`
(where downloads go) and `games` with `write = true`, and provides `game_installer`.

### Releases

`releases(&Console)` returns one console's `ReleaseList`, or `None`. The core asks each
`releases` plugin for the console of each emulator, and gives the list to that emulator. A
`Release` is one torrent: its `id` is the info hash, `torrent` a magnet link. `ReleaseSource`
says whether the list was `Fetched`, `Cached` or `Bundled`, and `problem` why a fetch didn't
give it. Publish `RELEASES_CHANGED` when the lists change on their own.

A `Release` has no one catalog's schema. Its fields are what the host sorts, groups, searches
and downloads by: title ids, platform and system, version, region, size, the day the catalog
added it, the torrent and its counts, a store link, and the pictures. The rest is data the
plugin makes:

- `facts`: `Fact { kind, label, value, url }`, in the order the Game Hub lists them. The
  `FactKind` says what a fact is about: `publisher`, `developer`, `release-date` (a date or a
  year, which the host writes in words and sorts by), `players`, `update`, `format`,
  `firmware`, `interface-languages`, `audio-languages`, `genre` (the genre chips' words, not
  listed) and `compatibility` (a community tier with its page, shown on its own: the release
  catalog gives ProtonDB's tier of a Windows game packed with Wine). An unknown kind reads as
  `other` and is listed by its label.
- `metadata`: a `GameMetadata` about the release's game, such as Steam's words and art. The
  host fills in from it what the `metadata` plugins leave out.
- `install_warning`: why the release may not install, shown beside it. The release catalog
  sets it for a PS5 package.

A catalog whose store installs its releases lists the installs the store is doing in
`store_installs()`: each a `StoreInstall` with its release, title, phase (`Queued`,
`Downloading`, `Paused`, `Installing`), bytes done of the total, and `open`, a link to the
store's own Downloads page (allowed in `permissions.links`). The host reads them every
`store_poll` (2 s) while one runs or for a minute after an Install, ten times as long
otherwise; it shows each in Downloads, and when one ends it reads the Library and the catalogs
again. The Steam plugin reads them from its app files: every app not fully installed.

An emulator may describe its own games with `metadata`: the Steam plugin gives a Steam game
(`platform` `Steam`, its app id as the title id) Steam's store's details, from
`store.steampowered.com/api/appdetails`, kept a week in its storage.

A game its store removes (Steam's) gives `GameDetails.uninstall_link`, `steam://uninstall/440`:
Uninstall opens it, when `permissions.links` allows it, instead of deleting the game's folder.

A release a store installs has an empty `torrent` and a `store` link instead
(`steam://install/440`): the Game Hub's Install opens that link, which the plugin's
`permissions.links` must allow, and downloads nothing. `hidden` puts the game under the
Catalog's "Hidden" filter, as if the player hid it in Nexus (Steam's Hidden collection);
`favorite` gives its card a "Favorite" badge. An emulator may provide `releases` for its own
console: the Steam plugin's console is `Steam`, and its list is the player's Steam library.

`Vm.catalog-available` in the theme contract is true only while a plugin with `releases` is
turned on. A `releases` plugin may have a boolean setting `show_hidden_filter`: when one sets it
to `false`, the Catalog has no "Hidden" filter.

An emulator that provides `releases` itself is its console's only catalog: the core asks no
other plugin for that console (the Steam plugin's console, `Steam`). The Game Catalog gives no
list for a console whose `show_<console>` setting (`show_ps5`) is `false`. Its settings schema names one such toggle for each console an emulator turned on
plays. The library is the emulator's, so that console's installed games stay.

### Metadata

`describe(&GameRef)` returns a store's `GameMetadata` for a game, by `platform`, `title_id`,
`content_id`, `title` and `region`, or `None`. Every field may be empty. `playstation-store`
caches answers in `catalog.json` in its storage.

### The machine (system plugins)

One trait per capability, in `crates/nexus-api/src/system.rs`: `NetworkSettings`,
`VpnSettings`, `BluetoothSettings`, `AudioSettings`, `PowerSettings`, `OsUpdates` and
`OsInstall`. Each async trait can say why it's unavailable here (`unavailable()`, for example
"This PC has no Bluetooth adapter"), and the core shows that reason. `VpnSettings` is blocking,
like the port it replaced: the host calls it off its async threads.

Quick calls answer within the call timeout. Long work (joining a network, a Bluetooth scan,
pairing, installing Nexus OS) starts in the background and returns at once; it reports through
the `SystemJob` the host passes in: `progress(percent, text)`, then `done(text)` or
`failed(reason)`, and it checks `cancelled()`. An install's `done` text is `Installed` as JSON.

The core keeps "Download only through a VPN" itself: once a system plugin provides `vpn`, it
reads and switches the VPNs through it, and binds torrents to the VPN's interface as before.

## Settings

A plugin with `configurator` describes its settings as JSON Schema, usually generated from a
Rust struct with `schemars::schema_for!`. The host stores the values; the theme draws the form.
From `plugins/services/release-catalog/src/lib.rs`:

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "nexus_api::serde", deny_unknown_fields)]
#[schemars(crate = "nexus_api::schemars")]
pub struct Settings {
    /// Where the catalog files are: `https://xyz.dev/catalog` gives
    /// `https://xyz.dev/catalog/ps5.json`. Empty for the catalog that ships with Nexus.
    #[serde(default)]
    #[schemars(title = "Catalog address")]
    pub catalog_url: String,
    // ...
    #[serde(default)]
    #[schemars(title = "Refresh from the forums")]
    pub refresh_from_forums: bool,
}

#[async_trait]
impl Configurator for Catalogs {
    async fn schema(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        Ok(serde_json::to_value(schemars::schema_for!(Settings))?)
    }
    async fn defaults(&self, _scope: &ConfigScope) -> PluginResult<Value> {
        Ok(serde_json::to_value(Settings::default())?)
    }
    async fn validate(&self, _scope: &ConfigScope, value: &Value) -> PluginResult<()> {
        // InvalidInput with a sentence the player can read.
    }
}
```

How the core turns a schema into fields (`fields` in `crates/nexus-core/src/view.rs`):

- `boolean` is a toggle, `integer` and `number` a number, `string` a text field, and an `enum`
  or `oneOf`/`anyOf` of `const`s a choice. Other properties are left out.
- The label is the property's `title` (`#[schemars(title = "...")]`), else its key. The help
  line is its `description`, which schemars takes from the doc comment.
- A Rust enum variant's doc comment labels that choice.
- An `Option<_>` choice gets a first entry, "As the emulator", which leaves the value unset.
- Fields come in the order of the schema's `x-order` list, then the rest by name:
  `#[schemars(extend("x-order" = ["own_build", "release_repo", ...]))]` in KytyPS5.

`ConfigScope` is `Emulator` (the plugin's own settings) or `Game(id)` (one game's). KytyPS5
returns a different schema per scope: `Settings` for the emulator, `GameSettings`, with
optional fields, for a game. Every change goes through `validate` before the host stores it.
Read the values with `ctx.config().get(&ConfigScope::Emulator)`; a missing value is `Null`, so
deserialise into a struct with `#[serde(default)]` fields.

The form shows where a `settings_panel` slot item points it; see the next section.

## Slots

Slots are UI a plugin adds as data. The theme decides where and how each one shows, so every
theme shows them. `Slots::items(&SlotRequest)` returns `SlotItem`s for one `SlotId`; the
request has a `game` for the per-game slots.

| `SlotId` | Name | Where |
|---|---|---|
| `GameBadge` | `game.badge` | Small labels on a game's card and Game Hub |
| `GameAction` | `game.action` | Buttons on a game's Game Hub |
| `GameDetailSection` | `game.detail-section` | Blocks of facts on a game's Game Hub |
| `SettingsPanel` | `settings.panel` | A settings form |
| `EmulatorPanel` | `emulator.panel` | Blocks on the plugin's page, for an emulator or a service |
| `CatalogAction` | `catalog.action` | Buttons and status badges on the Catalog, from a `releases` service |

The per-game slots are asked only of the plugin that owns the game.

A `SlotItem` has an `id` (unique in the plugin), an `Order`, and `SlotContent`:

| `SlotContent` | Fields |
|---|---|
| `Badge` | `label`, `short` (for small places; empty for the same), `tone` (`Tone`: neutral, info, success, warning, danger), `icon` |
| `Action` | `label`, `action`, `args` (JSON), `style` (`ActionStyle`: primary, secondary), `icon` |
| `Section` | `title`, `rows` of `SectionRow { label, value }` |
| `SettingsPanel` | `title`, `scope` (a `ConfigScope`): the theme draws the `Configurator` form for that scope |

When the player picks an action, the host calls `run_action(action, game, args)` with the
item's `action` and `args`. After it, the host reads the plugin's slots again. Two action names
are the host's own: `HOST_OPEN_SETTINGS` (`"host:open-settings"`) opens Settings, and
`HOST_INSTALL_GAME` installs a package.

KytyPS5's `items` shows each kind:

```rust
Ok(match (request.slot, request.game.as_deref()) {
    (SlotId::GameBadge, Some(game)) => {
        vec![item("compat", games::compat_badge(&self.status(game).await?, self.platform))]
    }
    (SlotId::EmulatorPanel, None) => vec![item(
        "check",
        SlotContent::Action {
            label: "Check for new builds".into(),
            action: "check".into(),
            args: Value::Null,
            style: ActionStyle::Secondary,
            icon: None,
        },
    )],
    (SlotId::SettingsPanel, None) => vec![item(
        "settings",
        SlotContent::SettingsPanel { title: "KytyPS5".into(), scope: ConfigScope::Emulator },
    )],
    _ => Vec::new(),
})
```

Themes render each item with the accessible label `<plugin-id>:<item-id>`, such as
`emu-kyty:compat`. Tests find slot items by that label.

## Errors

`PluginError` (`crates/nexus-api/src/error.rs`) carries text only:

| Variant | Use it for |
|---|---|
| `NotFound` | No such game, build or file |
| `InvalidInput` | Bad settings or arguments; the message is for the player |
| `PermissionDenied` | The host's refusals |
| `Network` | HTTP failures |
| `Storage` | File failures |
| `Unsupported` | A request this plugin doesn't handle, such as a `DownloadRequest` kind |
| `Precondition` | Something must happen first: install a build, map a folder |
| `OwnBuildMissing` | Own build is on, but it isn't there: `has_downloaded`, `folder`, `program`. The host words it |
| `Timeout`, `Panicked` | Set by the host's guard |
| `Other` | Anything else |

`InvalidPath`, `InvalidId` and `serde_json::Error` convert into `InvalidInput`, so `?` works on
`RelPath::new` and on JSON.

## Shared code: emulator-kit

`plugins/shared/emulator-kit` is a library, not a plugin. A console emulator describes itself,
and the kit runs the rest:

- A `Flavor` describes the emulator's builds: its release source, which asset fits this system,
  the archive format, where the program is in a build, the test run, where your own build may
  be, and two hooks. `on_switch` runs when the current build changes, and `on_leave_own` runs
  when your own build gives way to an official one. KytyPS5 moves its saves and caches in
  them; shadPS4 needs neither.
- The `BuildManager` runs the flow over a flavor. It is the plugin's `Downloader` and
  `Installer`, gives the build panel's buttons (use your own build, switch to official builds,
  check for new builds, go back) and runs their actions. It also picks the program a game
  starts, and checks your own build is there first: if not, it fails with
  `PluginError::OwnBuildMissing`.
- A `ConsoleSpec` describes the console: its games, package formats, compatibility list (the
  parser is the real plug point), keyboard keys and game page section. The `ConsoleEmulator`
  over a spec and a flavor is the plugin's `Catalog`, `Compatibility`, `Configurator` and
  `Slots`, and lists its packages to install (`candidates`).
- The plugin's settings type implements `Settings`. The kit reads the stored settings over
  this system's defaults, for both emulators. `SettingsStore<S>` is the default
  `Configurator` of a settings type: the schema, defaults and checks of `S` and its per-game
  settings. `ConsoleEmulator` hands its settings capability to it.

The plugin keeps its settings type, its launch arguments and its game installs. Its `lib.rs`
holds the flavor, the settings and the launch arguments.

The kit never takes the whole `PluginContext`. It takes the narrow facades in `Host`
(`Host::of(&ctx)`): storage, folders, programs, HTTP, settings and the log. So its unit tests
run on in-memory doubles of those alone.

| Module | What it does |
|---|---|
| `builds` | `Flavor` and `BuildManager`: official builds, your own build, and the own-build precondition |
| `console` | `ConsoleSpec` and `ConsoleEmulator`: games, compatibility, settings and the emulator's page |
| `settings` | The `Settings` trait, the stored settings over this system's defaults, and the default `Configurator` |
| `host` | `Host`, the narrow facades the kit takes |
| `github` | GitHub releases as the REST API returns them |
| `cache` | JSON from the web, kept in storage for a time, and the last copy when the web is down. Only GitHub's API gets GitHub's `Accept` header |
| `versions` | Builds in `builds/<tag>`, the current one and the one before it, in `versions.json`, changed under one lock |
| `games` | Installed PS5 and PS4 games in a host folder, up to three folders down; `GameIndex`, `compat_badge` |
| `compat` | The community compatibility lists, by title id |
| `controls` | A console's keyboard keys: `Controls { keys }` gives `keyboard_panel()` |
| `args` | Extra arguments the player types, split as a shell would |
| `installs` | The background work of a `GameInstaller` |
| `archive_sets` | A download's archive sets, as the host unpacks them |
| `pkg5`, `exfat`, `sfo`, `read_at` | The PS5 package, exFAT and `param.sfo` readers |
| `fail` | A small error type for the readers |

Its `fixtures` feature builds synthetic packages and images for tests.

## Testing

### Unit tests

Test pure logic inside the plugin crate, as `playstation-store` tests `regions` and
`release-catalog` tests that every bundled catalog reads:

```sh
cargo test -p playstation-store
cargo test -p release-catalog
```

### The fake host

`crates/nexus-testkit` runs a real core over a temporary data folder with fake HTTP, processes,
desktop, torrents, VPNs and archives.

- `FakeHost::new()` builds one. Add plugins with `built_in(PluginBox)`,
  `third_party(PluginBox)`, `plugin(StubPlugin)` or `discover(dir)`. Set the system with
  `platform(Some(Platform::Linux))`, the home folder with `home(path)`, and more
  (`safe_mode`, `hardware`, `free_space`, `call_timeout`, `max_downloads`).
- `host.http()` is a `FakeHttp`: `serve(url, bytes)` returns the body's SHA-256, `redirect`,
  `hold_next`, `cut_next_after`, and `sent()` lists the requests.
- `boot().await` gives a `TestHost` (kernel, `BootReport`, fakes); `boot_core().await` gives a
  `TestCore`, with the app started on it as `test.core`.
- `test.spawner` is a `FakeSpawner`: `prints(program, output)` sets what a program prints,
  `on_path(name, path)` puts a system program on `PATH`, `wait_for(n)` returns the n-th
  `SpawnedProcess`, which can `exit(code)`.
- `test.core.dispatch(Command::...)` drives the app as the UI does; `test.core.view()` reads
  the view models a theme would get.

`StubPlugin::from_toml(manifest)` is a plugin made from a manifest, with `provides(|ctx| ...)`
for its `init`, `console(name)`, and `panics_on_init`, `hangs_on_init`, `fails_on_init`,
`hangs_on_shutdown` to test the host's guards. It counts `init_calls` and `shutdown_calls`.

### The contract suite

`nexus_testkit::contract::emulator(&test, &plugin_id)` checks a loaded plugin against the
rules the host relies on, for every capability it provides: unique game ids, an unknown game
failing, a schema that is a JSON object and defaults that pass `validate`, slot ids unique in
each slot, each `[[keymap]]` button once, and a full install, uninstall and launch through the
host. Each broken rule panics
with `contract <capability>: ...`. From `tests/tests/shadps4.rs`:

```rust
#[tokio::test]
async fn shadps4_meets_every_contract() {
    let (test, _games) = boot().await;
    contract::emulator(&test, &shad()).await;
}
```

where `boot` serves a release and a compatibility list on `FakeHttp`, writes a game folder, and
boots the plugin:

```rust
let test = FakeHost::new()
    .platform(Some(Platform::Linux))
    .built_in(PluginBox::code(ShadPlugin))
    .boot_core()
    .await;
```

`tests/tests/seams/contracts.rs` shows the suite catching a stub catalog that lists one game twice.

### The tests crate

`tests/` (package `nexus-tests`) holds the seam tests: one module per area in `tests/tests/seams/` (one test binary), with
shared helpers in `tests/tests/seams/common/mod.rs`. Each plugin has its own file: `kyty.rs`,
`shadps4.rs`, `pc.rs`, `releases.rs`, `metadata.rs`, `catalog_refresh.rs`, `slots.rs` and
more. A new plugin adds its crate to `[dev-dependencies]` in `tests/Cargo.toml`, a file
here, and its `mod` line in `main.rs`.

```sh
cargo test -p nexus-tests --test seams shadps4::  # one area
cargo test -p nexus-tests                   # every seam test
scripts/check.sh                            # format, clippy, every test (as CI)
```

`tests/tests/seams/real_archives.rs` installs real release archives when `NEXUS_REAL_ARCHIVES` names
a folder of them, and `NEXUS_REAL_RELEASES=1 cargo test -p nexus-app real` downloads real
emulator releases; see [docs/development.md](development.md).

## Checklist for a new plugin

1. Make the crate under `plugins/emulators/<name>` or `plugins/services/<name>`, depending on
   `nexus-api` (and `emulator-kit` if it helps).
2. Write `plugin.toml`: `id`, `kind`, `name`, `version`, `host_api = "^0.2"`, `capabilities`,
   and only the permissions it needs.
3. Implement `Plugin` and `CodePlugin`. Return exactly the declared capabilities from `init`.
4. Register it: `inventory::submit! { PluginFactory::code(|| Box::new(MyPlugin)) }`.
5. Add it to the workspace `members` and `[workspace.dependencies]` in the root `Cargo.toml`,
   to `crates/nexus-app/Cargo.toml`, and as `use my_plugin as _;` in
   `crates/nexus-app/src/main.rs`.
6. Test it: unit tests in the crate, and a module in `tests/tests/seams/` (and its `mod`
   line in `main.rs`) that runs `contract::emulator` on it.
7. Run `scripts/check.sh`.

To see it in the app, `cargo run -p nexus-app -- --windowed`; the plugin shows under Emulators.
The README's [What is where](../README.md#what-is-where) lists where each part lives.
