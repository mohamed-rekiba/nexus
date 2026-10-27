# Architecture

How Nexus is built today: the crates, how data moves between them, what the plugins can and
can't do, and what the app keeps on disk. It is for contributors, and for plugin and theme
authors who want the mental model first.

- Writing a plugin: [plugins.md](plugins.md). Writing a theme: [themes.md](themes.md).
- Archives and PC games in detail: [reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).
- Windows, focus and the desktop on each system: [reference/desktop.md](reference/desktop.md).
- Building, running and packaging: [development.md](development.md) and the [README](../README.md).

## Shape

Nexus is one binary made of crates in one Cargo workspace. The crate graph is the boundary: a
crate can't use what it doesn't depend on. Inside the binary a small kernel loads plugins of
four kinds: emulators, services, themes and system plugins.

```
plugins (emulators, services)  ──▶ nexus-api
themes (theme.toml + .slint)   ──▶ nexus-ui-api ──▶ nexus-api
nexus-core                     ──▶ nexus-api, nexus-ui-api
nexus-ui-host (Slint)          ──▶ nexus-core, nexus-ui-api
nexus-app (the binary)         ──▶ nexus-ui-host, nexus-core, every built-in plugin
```

- `nexus-core` has no UI toolkit and no emulator code. Its view models are plain data.
- Only `nexus-ui-host` and `nexus-app` know Slint.
- Only `nexus-app` touches the system's own facilities (windows, BitTorrent, archives, Vulkan,
  VPNs, libmpv, gamepads, audio). The core reaches them through ports (see [Ports](#ports)).
- The workspace denies `unsafe_code`. A few modules of `nexus-app` allow it, each with its
  reason on the same line: `clock.rs`, `desktop.rs` (Win32), `disk.rs`, `gamepad.rs`,
  `gpu_probe.rs`, `libarchive.rs`, `processes.rs` (Windows), `trailer.rs` and `vpn.rs`.

### The crates

| Crate | Owns |
| --- | --- |
| `crates/nexus-api` | The plugin API. The plugin traits, `PluginManifest`, `Capability`, `Provided`, `PluginContext` and its facades (`ConfigStore`, `Storage`, `Folders`, `Programs`, `HttpClient`, `Logger`, `EventSink`, `ServiceRegistry`), `RelPath` and `HostPath`, plans (`DownloadPlan`, `LaunchSpec`), slots, `Event`, `VpnGate`. Semver-versioned: a manifest's `host_api` must match `API_VERSION`, which is set by hand with the API, not from Nexus's version. |
| `crates/nexus-core` | The kernel and the app. Plugin lifecycle and registry (`Kernel`), the event bus (`EventBus`), settings (`ConfigService`), the app state and commands (`Core`, `AppState`, `Command`), view building (`view.rs`), HTTP downloads, torrents, unpacking, game installs, launches, VPN rules, catalogs, "This PC", the launcher update, and the port traits. |
| `crates/nexus-ui-api` | The theme contract. View models (`vm.rs`: `AppView` and the rest), the theme manifest (`ThemeManifest`), design tokens, and `contract/types.slint` with the `Vm` and `Host` globals. No toolkit code. |
| `crates/nexus-ui-host` | `ThemeSet` (finds themes, resolves `extends`), `ThemeHost`: compiles a theme with `slint-interpreter`, binds the view models, turns gamepad buttons into keys, and swaps themes. `AppShell` (`shell.rs`) owns the `ThemeHost` on the UI thread: it routes the theme's requests, coalesces renders, loads images (`loader.rs`) and runs the tick, through its `Platform` port. |
| `crates/nexus-app` | The binary. The wiring (`main.rs`), the command line (`args.rs`), the launcher's update at startup (`update.rs`), the shell's platform (`platform.rs`), and the system adapters: `desktop.rs`, `processes.rs` (a running game's processes), `torrents.rs` (librqbit), `archives.rs` and `libarchive.rs`, `hardware.rs` and `gpu_probe.rs` (Vulkan), `trailer.rs` (libmpv), `gamepad.rs` (gilrs), `audio.rs` (rodio), `http.rs` (reqwest), `disk.rs`, `clock.rs`. |
| `crates/nexus-testkit` | `FakeHost` (a real core over a temporary data folder), `FakeClock`, `StubPlugin`, fakes for every port, `FakePlatform` (the app shell's platform), `SpanRecorder`, and `contract::emulator`, the capability contract checks. |
| `plugins/emulators/*` | `emu-kyty` (KytyPS5, PS5), `emu-shadps4` (shadPS4, PS4), `emu-pc` (PC games), `emu-steam` (the games Steam installed, Linux). |
| `plugins/services/*` | `playstation-store` (store art and details) and `release-catalog` (the Catalog's releases). |
| `plugins/themes/default` | The built-in theme, id `default`. A crate only so its files are embedded in the binary. |
| `plugins/shared/emulator-kit` | Code the emulators share: GitHub releases, build folders, game scans, package readers, install jobs. A library, not a plugin. |
| `tests/` | The seam tests (see [Tests](#tests)). |

A plugin's id, not its folder or crate, names its data folder and its saved settings. Moving
a crate changes nothing for the player.

## Threads

The core runs on a multi-threaded tokio runtime. The UI runs on the main thread in Slint's
event loop, where the `AppShell` owns the `ThemeHost`. They meet in two places:

- Theme callbacks become `UiRequest`s. The shell queues them and carries them out on its next
  `poll`, so a request never runs inside the callback that made it. The shell hands core
  commands and image loads to `Platform::spawn`, which the app runs on the runtime.
- Core events, finished work, gamepad presses and trailer frames come back to the UI thread
  with `slint_interpreter::invoke_from_event_loop` (in `main.rs` and `platform.rs`).

The shell takes its time from the core's `Clock` port and keeps no timer of its own. It asks the
platform to call `poll` again (`Platform::wake_in`); the app does that with a Slint timer, and a
poll does what the clock says is due. The app also calls `on_tick` every `TICK` (250 ms).

The UI thread's state (`App`, which holds the `AppShell`) lives in a thread-local in
`crates/nexus-app/src/main.rs`.

## Data flows one way

```
theme callback ─▶ UiRequest::Core(Command) ─▶ Core::dispatch ─▶ AppState changes
      ▲                                                               │
      │                                                   Core::changed()
      │                                                               ▼
  Slint `Vm` ◀─ bind::vm_values ◀─ view::build ◀─ ThemeHost::render ◀─ Event::StateChanged
```

1. A theme calls a `Host` callback. `ThemeHost::wire` turns it into a `UiRequest`: a core
   `Command`, or something only the app can do (`SwitchTheme`, `PlaySound`, `OpenUrl`,
   `OpenPath`, `Quit`, `Restart`, `LogOut`, `Trailer`, `Render`). `AppShell::on_request`
   carries it out: a command goes to the runtime, the rest to the `Platform`.
2. `Core::dispatch` runs the command. All state lives in one `AppState` behind a mutex in
   `Core`: navigation (`nav`, a stack of `Route`s), selection, the library, details, store
   metadata, catalogs and offers, downloads, sessions, notices and more.
3. A change calls `Core::changed(area)`. It sends a `Change { area }` (the torrents, the
   catalogs, the installs, …) to `Core::changes()`, a broadcast for the core and its tests, and
   publishes `Event::StateChanged` on the `EventBus` (a `tokio::sync::broadcast` channel of
   1024; a reader that falls behind skips the oldest). Plugins see neither the change nor its
   area.
4. The app reads every event from `core.events()` and hands it to `AppShell::on_core_event`,
   which asks for a render. An atomic flag keeps at most one event waiting on the UI thread.
5. The shell renders at most once per `RENDER_GAP` (8 ms, a frame at 120 Hz). A burst of events
   or image loads renders once. Each render starts loading the images it asks for.
6. `ThemeHost::render` calls `Core::view`, which runs `view::build` over the state and returns
   an `AppView`. `bind::vm_values` turns it into Slint values, set on the theme's `Vm` global.
   Lists are changed in place where the rows allow it, so row animations don't restart.

Themes hold no state of their own. That is why a theme switch loses nothing: the new view tree
binds to the same `AppView`.

The shell's tick (`AppShell::on_tick`, every 250 ms) also renders when the window size changes,
the clock's minute turns, a trailer is open, the boot screen waits for Home's art, or every
15 s by the clock while a game runs. The same tick moves the window to its monitor, writes
`window shown` to the standard output once, makes the window show under Gamescope, and in dev
mode (`--dev`) reloads edited theme files.

## Plugins

### Kinds

`crates/nexus-api/src/plugin.rs` has one root trait and four kinds:

- `Plugin`: `manifest()` and `shutdown()`. Every plugin has these.
- `EmulatorPlugin: Plugin` adds `init(ctx) -> Provided`: emulator capabilities.
- `ServicePlugin: Plugin` adds the same `init`, for code that adds to other plugins' games and
  runs none. The kernel rejects a service that declares anything but `metadata`, `releases`,
  `slots` and `configurator`.
- `ThemePlugin: Plugin` adds `files()`, and has no other code. A theme has no `init`.
- `SystemPlugin: Plugin` adds the same `init`, for the machine's own settings: the system
  capabilities below, and `configurator`. Only a system plugin may declare those, or run
  programs long (`Programs::stream`) or through polkit.

The host holds a plugin as a `PluginByKind`. Built-in plugins register at compile time with
`inventory`: `PluginFactory::emulator`, `::service`, `::theme` or `::system`. A crate registers only if
it is linked, which is why `nexus-app` names each plugin crate (`use emu_kyty as _;`).

### Capabilities

Each extension point is one trait. A plugin implements only the ones it has and returns them in
`Provided`. `init` must provide exactly the capabilities the manifest declares, or the plugin
fails with `LoadError::CapabilityMismatch`.

| Capability | Trait | Provided by |
| --- | --- | --- |
| `catalog` | `Catalog` (the installed games) | all three emulators |
| `downloader` | `Downloader` (plans emulator downloads) | KytyPS5, shadPS4 |
| `installer` | `Installer` (installs builds) | KytyPS5, shadPS4 |
| `configurator` | `Configurator` (settings schemas) | KytyPS5, shadPS4, release-catalog |
| `launcher` | `Launcher` (`prepare` a `LaunchSpec`) | all three emulators |
| `compatibility` | `Compatibility` | KytyPS5, shadPS4 |
| `controllers` | `Controllers` | KytyPS5, shadPS4 |
| `game_installer` | `GameInstaller` | all three emulators |
| `slots` | `Slots` (badges, actions, sections as data) | all three emulators, release-catalog |
| `metadata` | `Metadata` (store art and details) | playstation-store |
| `releases` | `Releases` (what the Catalog offers) | release-catalog |
| `saves`, `shaders`, `launch_middleware` | `Saves`, `Shaders`, `LaunchMiddleware` | none yet |
| `theme` | (`ThemePlugin::files`) | themes only |
| `network`, `vpn`, `bluetooth`, `audio`, `power` | `NetworkSettings`, `VpnSettings`, `BluetoothSettings`, `AudioSettings`, `PowerSettings` | the `system` plugin |
| `os_updates`, `os_install` | `OsUpdates`, `OsInstall` (Nexus OS) | the `system` plugin (Linux) |

The core aggregates across plugins through these traits only: one library from every
`Catalog`, one Catalog from every `Releases`.

### Manifest

Every plugin has a manifest: `plugin.toml`, or `theme.toml` for a theme. `PluginManifest`
(`crates/nexus-api/src/manifest.rs`) has the same base fields for every kind: `id`, `kind`,
`name`, `version`, `host_api`, `platforms`, `capabilities`, `permissions`, `dependencies`,
`activation` (`lazy` by default, or `eager`) and `runtime` (`native` by default, or
`{ type = "data" }`). An emulator also has `console` (`PS5`, `PS4`, `PC`: it picks the
emulator's catalog), `emulator` (the program's name), `[requires]` (what this PC needs),
`[[keymap]]` and `install_note`. Sections for one kind, such as `[theme]`, stay in `extra` and
the crate that owns them parses them.

### Built-in and discovered plugins

`KernelBuilder` collects candidates from two places:

- `built_in_plugins()`: every `PluginFactory` linked into the binary (`Origin::BuiltIn`).
- `discover(dir)`: each subfolder of `dir` with a `plugin.toml` or `theme.toml`
  (`Origin::ThirdParty`). The app discovers `<data-dir>/extensions` (or `--extensions <path>`)
  and the `extensions` folder that ships beside the binary (`Resources/extensions` in a macOS
  app bundle).

There is no runtime for third-party code. A plugin found on disk must have `runtime = data`,
and the only `Loader` is `DataThemeLoader`, so a third-party plugin is a data-only theme.

### Lifecycle

`Kernel::boot` (`crates/nexus-core/src/kernel.rs`):

1. **Validate** each candidate: the manifest reads, `host_api` matches, safe mode (third-party
   plugins are rejected), folder grant names, kind rules (an emulator or service needs code, a
   theme declares `theme`, a third-party plugin is data-only), and no duplicate id.
   A theme for other systems is rejected. An emulator or service for other systems loads, but
   turned off, with a warning.
2. **Resolve** load order with `petgraph` (`resolve.rs`): `tarjan_scc` finds cycles and names
   each member; the rest is sorted with `toposort`. A theme's `[theme] extends` counts as a
   dependency. A plugin whose dependency is missing, the wrong version or rejected is rejected.
3. **Load**: built-in plugins are already code; folders go to a `Loader`. Code whose kind
   differs from its manifest's `kind` is rejected.
4. **Init** the `eager` plugins (today only the default theme, which has no `init`). Others
   init the first time the core asks `Kernel::provided` for them. Dependencies init first.
5. **Deactivate**: `Kernel::deactivate` deactivates dependents first, then calls `shutdown`
   under `shutdown_timeout`, at most once per activation. `Kernel::shutdown` does this for every
   plugin in reverse load order.

Each step's failures go into the `BootReport` and the log. Boot carries on without them. A
plugin whose `init` fails is `PluginStatus::Failed` for the rest of the run and is not retried.

### Turned on and off

`Kernel::enabled` reads two lists in `config.json`. A plugin is on unless its id is in
`app.disabled_plugins`. A plugin made for other systems is off unless its id is in
`app.enabled_plugins` (the player may have an own build for this system).
`Command::SetPluginEnabled` edits the right list. Themes can't be turned off, and a busy
emulator can't be turned off. A plugin that is off is left out of `Kernel::providers` and
`Kernel::of_kind`, and `Kernel::provided` refuses it, so it gives no games, catalogs, slots,
settings or updates. It stays listed, and turning it on needs no restart. When no `Releases`
plugin is on, there is no Catalog: `Route::Catalog` is refused and leaves the navigation stack.

### The context and permissions

`init` gets a `PluginContext` built in `Kernel::context_for`. Each piece is scoped to the plugin:

- `config`: the plugin's settings, stored under `plugins.<id>.<scope>` in `config.json`. The
  scope is `emulator` or `game:<game key>`.
- `storage`: the plugin's own folder, `plugins/<id>/` in the data folder (`DirStorage`). A
  `RelPath` is always relative, with no `.` or `..`, and a symlink that leads out is refused.
- `folders`: named host folders (`HostFolders`). A manifest asks for names in
  `permissions.filesystem` (`folder`, `write`, `default`, `default_on`, `title`, `help`). The
  player maps each name to real folders, saved under `app.folders.<name>`; until then the
  grant's default applies. A plugin addresses a file with a `HostPath` (folder name, root
  index, `RelPath`). The host refuses names the manifest doesn't ask for, writes without
  `write = true`, and symlinks out. Two names are shared by every plugin and shown once in
  Settings: `games` (`~/Games`) and `packages` (`~/Downloads`).
- `programs`: find and briefly run the programs `permissions.process` allows: a path pattern in
  the data folder (`builds/*/kyty`), `folder:<name>`, or `system:<name>` on `PATH`. At most
  `MAX_PROGRAM_TIME` (10 s) and `MAX_PROGRAM_OUTPUT` (64 KiB).
- `http`: HTTP limited to `permissions.network` (exact hosts or `*.suffix`). The transport never
  follows redirects; the core follows them and checks every hop (`facades::send_checked`).
- `log`: tracing, tagged with `plugin_id`.
- `events`: subscribe to every `Event`; publish only `Event::Custom`.
- `services`: a `ServiceRegistry` with `HostInfo` (API version, platform, safe mode) and the
  `VpnGate` (see [VPN](#downloads-only-through-a-vpn)).

Plugins never reach each other.

### The guard

Every call into a plugin goes through `Kernel::guard` (`guard.rs`): a `plugin_call` tracing span
with `plugin_id` and `method`, a timeout (`call_timeout`, 30 s, on the kernel's clock), and
`catch_unwind`. A panic
becomes `PluginError::Panicked`, a timeout `PluginError::Timeout`, and the caller gets a
`CoreError::Plugin` (usually shown as a notice). Only a failed `init` marks a plugin failed; a
later call that fails leaves it active. The timeout fires at the plugin's next `.await`: code
that blocks its thread without awaiting holds that worker thread.

## Ports

The core reaches the system through traits in `crates/nexus-core/src/ports.rs`. `KernelBuilder`
starts with a stand-in for each; the app plugs in the real ones; the test kit has fakes.

| Port | Stand-in | App adapter | Test fake |
| --- | --- | --- | --- |
| `HttpTransport` | `NoNetwork` | `http::ReqwestTransport` | `FakeHttp` |
| `ProcessSpawner` | `TokioSpawner` (real; the app keeps it) | `TokioSpawner` | `FakeSpawner` |
| `ProcessTree` | `GroupTree` (the process group alone) | `processes::SystemProcesses` | `FakeProcessTree` |
| `Windows` | `NoWindows` | `desktop::SystemWindows` | `FakeWindows` |
| `LoginSession` | `NoLoginSession` | `desktop::SystemLoginSession` | `FakeLoginSession` |
| `Opener` | `NoOpener` | `desktop::SystemOpener` | `FakeOpener` |
| `Hardware` | `NoHardware` | `hardware::SystemHardware` | `FakeHost::hardware` / `free_space` |
| `Torrents` | `NoTorrents` | `torrents::RqbitTorrents` | `FakeTorrents` |
| `Network` (VPNs) | `NoVpn` | none: a system plugin's `vpn` replaces it once one provides it | `FakeNetwork` |
| `Archives` | `NoArchives` | `archives::HostArchives` | `FakeArchives` |
| `Clock` | `TokioClock` (real; the app keeps it) | `TokioClock` | `FakeClock` |

- `TokioSpawner` starts each game as the leader of its own process group on Unix; on Windows
  the "group" is the process and its children. `terminate` asks it to close (SIGTERM, or
  `taskkill` without `/F`); `kill` ends the group.
- `ProcessTree` lists a running game's processes (`members`): the one the launcher started, its
  process group, and what a sandbox moved out of the group below them (Proton's pressure-vessel
  does). `end` asks them to close or kills them. `find(text)` finds the processes with that text
  in their command line, for a game another program started (Unix only: `ps -A -ww -o args=`). It
  is the one place that knows which processes are the game; the core and the desktop both work
  from its list.
- `Windows` says which window is in front (`front`), brings one of the game's processes'
  windows up (`show_game`), brings the launcher back (`show_launcher`), and answers
  `has_window`, `window_close_ends_game` (macOS only) and `problem`.
- `LoginSession` starts the launcher at login (`can_start_at_login`, `start_at_login`) and
  installs the launcher's own package with the administrator prompt (`install_package`, `pkexec`
  on Linux). It is the player's login session, not a play session.
- `Opener` opens a link (`open_link`, such as `steam://install/440`) or a file or folder
  (`open_path`) with the system's opener.
- `Hardware::specs` is slow (it runs system tools); the core calls it once a run, off the async
  threads. `free_bytes` gives a disk's free space.
- `Archives` has one call, `unpack`, which takes the free space the core read from `Hardware`.
  Each adapter lists an archive with its format readers, then hands the listing to the core's
  archive plan (`plan_unpack`, `archive_plan.rs`). The plan refuses passwords, paths and links
  out of the folder, and archives bigger than the free space; the adapter writes only what the
  plan holds. Failures are a typed `ArchiveError`, and `HostArchives` picks its `unar` fallback
  by the error's kind. `contract::archives` in the test kit checks the same refusals against
  `HostArchives` and `FakeArchives`.
- Calls into `Windows`, `LoginSession`, `Opener`, `ProcessTree` and `Network` run in
  `spawn_blocking`.

The app shell has a port of its own, `Platform` (`nexus-ui-host/src/shell.rs`): it runs work
off the UI thread, wakes the shell, opens links and paths (the app's adapter uses
`SystemOpener`), quits and restarts, writes the lines the Nexus session reads, plays sounds and
trailers, reads the local time, and asks the window system about monitors and focus. The app
plugs in `platform::SystemPlatform`; tests plug in `FakePlatform`, which records each call and
runs the work only when the test says.
- `Clock` is the core's time: `now`, `sleep` and `interval`, and on `dyn Clock`, `sleep_until`
  and `timeout`. Every timer of the core runs on it: the pollers, the game watcher, the
  plugin calls' timeouts (`Kernel::guard`), the select settle, the prefetch, the update checks.
  No other code in `nexus-core` uses `tokio::time`. `TokioSpawner` runs real programs, so it
  keeps real time (`TokioClock`) whatever clock the kernel has.

## State that persists

Everything lives in one data folder: `$XDG_DATA_HOME/nexus` (or `~/.local/share/nexus`) on
Linux, `~/Library/Application Support/nexus` on macOS, `%APPDATA%\nexus` on Windows.
`--data-dir` picks another.

| Path | What |
| --- | --- |
| `config.json` | All settings: the app's under `app`, each plugin's under `plugins.<id>.<scope>`. Written on every change, through `config.json.tmp` and a rename. |
| `plugins/<id>/` | Each plugin's `Storage`: emulator builds, the store's `catalog.json` cache, the release catalog's `fetched/`, `refreshed/` and `sources/`. |
| `logs/<plugin>/<game>.log` | A running game's output. The run before stays as `<game>.previous.log`. |
| `cache/net/` | Images fetched by URL, named by the SHA-256 of the URL. |
| `cache/thumbs/` | Smaller JPEG copies of art: `<hash>-card.jpg` (enough to cover a 512 × 768 card), `<hash>-full.jpg` (fits 1920 × 1080), and `-cut-` variants. Other files there are removed at start. |
| `torrents/<info hash>.torrent` | A torrent's metadata, so a restart doesn't ask the swarm again. |
| `torrents/resume/` | librqbit's fastresume data: which pieces each torrent was checked to have. Its `session.json` (the list of torrents) is removed when a session starts; the core starts the ones it wants. |
| `updates/` | A downloaded launcher update and its `ready.json`. |
| `extensions/` | Third-party themes, unless `--extensions` names another folder. |

Some `app` keys the core keeps: `theme`, `theme-options`, `disabled_plugins`,
`enabled_plugins`, `folders`, `downloads` and `installs` (work in progress, resumed at start),
`game_installs` (work folders to clean up), `torrents`, `playtime`, `results`, `seed_downloads`,
`vpn_only`, `vpn`, `update_repo`, `auto_update`, `pc_noticed`.

Downloaded games are not in the data folder: they go to the player's `packages` folders and are
installed into the `games` folders.

## Downloads and torrents

There are two download paths.

**Plans (emulator builds).** A `Downloader` returns a `DownloadPlan`: artifacts with URL,
SHA-256, an optional ed25519 signature, a destination in the plugin's folder, and an optional
`Unpack` (`TarGz`, `Zip`, or `AppImageInZip`, Linux only). `Downloads`
(`crates/nexus-core/src/downloads.rs`) is one long-lived task fed by a channel (submit, pause,
resume, cancel). It runs `max_downloads` jobs at once (2) and queues the rest. Each file goes to
`<dest>.part`, resumes with Range requests, and moves into place only after its hash and
signature match. Unpacking (`unpack.rs`) refuses paths and links that leave the folder. Running
downloads are saved under `app.downloads`, and installs under `app.installs.<plugin>`, so both
resume after a crash.

**Torrents (Catalog releases).** `crates/nexus-core/src/torrents.rs` drives the `Torrents`
port. A download starts only from the player (`Command::DownloadRelease`). Its states
(`TorrentState`) are Resolving (reading the torrent's files), Ready (waiting for the player),
Checking, Downloading, Paused, Complete, Failed and Cancelled. Files go to
`<folder>/torrent-<info hash>`, where the folder is the one picked (inside one of the
emulator's `packages` folders) or `<console>` in the first one. A start checks free space
first. Each download is saved under `app.torrents.<info hash>`; after a restart unfinished ones
come back paused. A finished download seeds while `app.seed_downloads` is on, at most
`SEED_UPLOAD_LIMIT` (128 KiB/s); after a restart it seeds only once its files check out
(`TorrentStart::SeedOnly`). One task (`watch_torrents`) polls
the running torrents every `torrent_poll` (1 s) while any run.

The app's adapter (`crates/nexus-app/src/torrents.rs`) is librqbit 9.0.1: one session, made at
first use, no local peer discovery, at most 50 peers, the upload cap above, and JSON
persistence with fastresume in `torrents/resume/`.

**Unpack.** When a download completes, the core finds its archive sets
(`crates/nexus-core/src/archives.rs`) and unpacks them through the `Archives` port into
`unpacked/<folder>/<name>/` in the download, through a `.part` folder. The unpack state is
saved with the download. The app's `HostArchives` lists each archive first, and the core's
archive plan refuses passwords, paths out of the folder and archives bigger than the free
space, then the adapter writes. Its readers are `zip`, `tar` with `flate2`, `sevenz-rust2`, and libarchive loaded at run time for
RAR and the rest. Details: [reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).

## Game installs

A `GameInstaller` decodes a package; the core owns the job (`game_installs.rs`).
`GameInstaller::begin(source, job)` checks the source and returns a work folder at once; the
plugin then writes in the background through its `Folders` and reports through the
`InstallJob` the core gave it:

- `progress`, `cancelled`, and `finished` (exactly once; on an error the work folder must be
  gone).
- `games_folder`: the game folder the player picked, or `None` (the emulator kit then uses
  `<console>` in the first `games` folder).
- `take(from, to)`: move a folder instead of copying it. The core agrees only for a copy it made
  itself (a download's `unpacked/` folder) on the same disk, and cleans up the folders the move
  emptied.

The job shows in Downloads. If the plugin's work drops the job without finishing (a panic), the
job fails. Work folders are saved under `app.game_installs`, and one a crash left is removed at
the next start (`clean_up_game_installs`). `Command::InstallDownload` installs a finished
torrent: it checks free space, unpacks first when needed, then calls the installer. A slot
action `host:install-game` (`HOST_INSTALL_GAME`) installs a package the plugin listed.

## Launches and the game watcher

`launch::launch` (`crates/nexus-core/src/launch.rs`):

1. `Launching::claim`: one game at a time; a second press of Play is refused. No game starts
   while its emulator installs or updates.
2. A first play whose emulator isn't installed asks the player first (`ask_to_install`).
3. `Launcher::prepare` returns a `LaunchSpec`; launch middleware (Tower-style, ordered by
   `before`/`after`, priority and id) may change or stop it. No built-in plugin adds any yet.
4. The program must pass `permissions.process`. Its output goes to the game's log.
5. The process is spawned, a `Session` is added, `Event::ProcessStarted` is published, and the
   Running screen opens.

The running-game module (`crates/nexus-core/src/running.rs`) then owns the session until it
ends. A game is its processes, as `ProcessTree::members` lists them, and it ends when none is
left: a start script that starts the game in the background and exits keeps its session. On
the way the module tries `show_game` every 250 ms until the game has a window or `show_timeout`
(20 s) passes, then reads `front` and `has_window` every `watch_every` (1 s). It keeps
`AppState.game_in_front` up to date; a window of any of the game's processes counts. When the
desktop can't say what is in front (`Front::Unknown`), the last window the launcher showed
counts. Only where `window_close_ends_game` (macOS) does a window that was seen and is gone for
three reads in a row stop the game: elsewhere a launcher's own menu closes before the game's
window opens.

A launch can hand the game to another program: `LaunchSpec.game_process` holds words of the
game's own command line, as Steam's `SteamLaunch AppId=<id> ` (the Steam plugin). The program
started here (`steam steam://rungameid/<id>`) is then left alone: reaped when it exits, never
stopped. The module looks for the game with `ProcessTree::find` until `hand_over_timeout`
(2 min: the client may first start and update itself); the first process found that wasn't
there before the launch is the game's first, and the rest works as above. A game that never
shows ends the session with a notice, and no play time; Stop before it shows ends it with
nothing played.

Stop asks the game's processes to close, waits up to `stop_grace` (4 s) for them, and kills what
is left. When the game exits,
`finish` saves play time under `app.playtime` (sessions of 3 s or more), posts a notice with the
log for a crash the player didn't cause, may ask how far the game got, brings the launcher back
when "Back to the launcher" is on, calls `Launcher::on_exit`, publishes `Event::ProcessExited`,
and runs an emulator update that waited for the game.

While the game's window is in front, or the launcher's window doesn't have focus, `ThemeHost`
drops every pad button but a hold of the PS (Guide) button, which sends `Command::Stop` for
each running game, at once; a tap does nothing while a game runs. With no game running, a tap
opens Home and a hold opens the power menu. Per-system details: [reference/desktop.md](reference/desktop.md).

## Library, metadata and catalogs

**Library.** `Command::RefreshLibrary` lists every `Catalog` plugin's games, sorts them by
title, reads their `game.badge` slots and selects the first game. The first load is the boot:
it reads the emulators' builds and asks the `Metadata` plugins about the games, 6 at a time,
waiting at most 10 s. Then `AppState.booted` turns true. Later loads do this in the background.

**Metadata.** The core keeps what a `Metadata` plugin says in `AppState.meta`, by game, and
asks about each game at most once a run (`AppState.asked`). The `playstation-store` service
keeps its answers in its own `catalog.json` (14 days for a hit, 3 for a miss).

**Catalogs.** A `Releases` plugin answers `releases(console)` for the console each turned-on
emulator names; only the first emulator for a console gets its catalog. `refresh_releases`
asks every `Releases` plugin, groups the releases into offers (`releases::offers`: one game per
title id, or per title without its edition words), reads the `catalog.action` slot, and posts a
notice when releases were added. The core reads the catalogs once a run after the boot, when
the Catalog first opens, and again whenever a catalog plugin publishes `RELEASES_CHANGED`
(`watch_catalogs`). One read runs at a time, so a read that began before a catalog changed
can't put its old copies over a newer read's.

The `release-catalog` service (`plugins/services/release-catalog`) answers at once with the
best copy it has: one fetched from its catalog address (`fetched/`), one refreshed from the
forums (`refreshed/`), else the copy bundled into the binary from `assets/catalog/` (`ps5.json`,
`ps4.json`, `pc.json`). Fetches and refreshes run in the plugin's own task, which publishes
`RELEASES_CHANGED` when it has something new. The catalog sources come from
`assets/catalog-sources.json` the same way.

Inside the plugin, each part has its own module:

- `topic.rs` is one typed model of a catalog file. The refresh (`refresh.rs`) changes it, and
  `format.rs` turns it into releases in one function, `format::release`.
- `store.rs` (`CopyStore`) keeps the fetched and refreshed copies and their meta files.
- `schedule.rs` holds the due-time and back-off rules. It takes the time as an input.
- `sources.rs` reads the catalog sources. Only the bundled sources say what a forum's topics
  are for, so a catalog file reads the same whichever sources are in use.
- `lib.rs` (`Catalogs`) tracks what the core asked for, runs the task and builds the slot
  items over these modules.

## The machine's settings

`crates/nexus-core/src/system.rs`. For each system capability the core uses the first active
system plugin that provides it (`provider`), calling it under the guard. `SystemState` holds the
network and Wi-Fi list, Bluetooth and its devices, the sound outputs and level, the battery,
Nexus OS's update and the stick's disks, each capability's unavailability reason, and the
jobs. `refresh` reads it all every `REFRESH_EVERY` (5 s) while a system plugin is on, and after
each `Command::System` action. A job (`HostJob`) changes the state as its plugin reports, reads
the machine again when it ends, and stays listed `JOB_KEPT` (8 s) so the theme can say how it
ended. `Place` says whether Nexus is the whole screen (`NEXUS_SESSION=1`, Nexus OS's marker), on
Nexus OS, or on its USB stick (`rd.live.image` and the installer); the theme shows the network,
sound and Bluetooth controls only when it's the whole screen. The view model is `SystemVm`.

## Downloads only through a VPN

`crates/nexus-core/src/vpn.rs`, with the `Network` port. While `app.vpn_only` is on, the core
binds the torrent port to the VPN's interface (`Torrents::route`) before a download talks to
the swarm. Without that VPN a download waits (`waiting_vpn`).

The kill switch: while a download runs or waits, `watch_torrents` checks the VPN every
`vpn_poll` (2 s). When it drops or changes, every running download pauses and is marked
waiting, and `Torrents::stop_all` closes the connections. When a VPN is back, the port is
rebound and only the waiting downloads start again. A restart forgets the waiting.

Plugins see the rule through `VpnGate::check`: `Ok(VpnPass::Open)`, `Ok(VpnPass::Unbound)` (the
setting is on but this system can't bind traffic), or `Err(VpnBlock::Waiting | NoVpn)`. The gate
only says whether the VPN is up; a plugin's HTTP doesn't go through it. When a plugin waited and
the VPN comes back, the core reads the catalogs again.

The app adapter binds librqbit with `SessionOptions::bind_device_name`. Linux and macOS are
supported; on Windows the setting is unavailable.

## The launcher's update

`crates/nexus-core/src/launcher_update.rs`. On `Command::CheckLauncherUpdate` the core reads
`latest.json` from the latest release of `app.update_repo` (default `mohamed-rekiba/nexus`),
following redirects only to GitHub's hosts. A higher version is an update.
`Command::DownloadLauncherUpdate` downloads this system's package, checks its SHA-256 and keeps
it in `updates/` with `ready.json`. At the next start, before anything else, `main` calls
`apply_launcher_update`: for an installed launcher only (an AppImage, `Nexus.app`, or a package
folder with `LICENSE` and `extensions/`), `apply_staged` swaps it in, puts the old one back on
failure, and starts the new binary. The Restart button (`UiRequest::Restart`) quits and starts
again so the update goes in.

Emulator updates are separate: `schedule_update_checks` runs `check_updates` 60 s after start
and every 6 hours while automatic updates are on. An emulator whose game runs updates after the
game closes. An own build never updates by itself.

## Themes

A theme owns presentation: layout, navigation, components, tokens, assets, motion and sound.
The contract lives in `crates/nexus-ui-api`:

- `contract/types.slint`: the `Vm` global (everything to show) and the `Host` global (every
  callback). The host serves it; a theme never ships a copy. It also generates `tokens.slint`
  (`Tokens`), `assets.slint` (`Assets`) and `options.slint` (`Options`).
- Screens (`ScreenId`): one file and component each, such as `screens/home.slint` exporting
  `HomeScreen`. The root is `shell.slint` exporting `Shell`.
- Components (`ComponentId`): card, button, list item, progress bar, dialog, and the three slot
  renderers.
- `contract::lint` refuses `@image-url` and font imports before compiling: images and fonts are
  declared in `theme.toml` and reach the theme through `Assets` and `Tokens`.

`ThemeSet` builds the `extends` chain up to `default`, which has every screen, component and
token. Imports resolve in the theme first, then each parent. Tokens, images, fonts, sounds and
options merge, the child winning.

`ThemeHost` compiles with `slint-interpreter` (`compile.rs`). The compiler's file loader serves
only a virtual folder (`VFS_ROOT`); a path outside it is refused, so markup can't read the disk.
Slint markup has no file, network or process access, so a data-only theme needs no sandbox.

`ThemeHost::switch` loads the theme, decodes its assets, compiles it, creates the new tree in the
same window (`create_with_existing_window`), binds tokens, options, assets and the view, saves
the choice (`Core::theme_switched`, which publishes `ThemeChanged`), shows it, and only then
drops the old one. Any failure keeps the old theme. A switch requested from a theme callback
runs on a zero-delay timer, because the switch drops the tree running that callback. At start,
`start_with` falls back to `default` if the chosen theme fails, without changing the saved
choice; safe mode shows only `default`. A changed theme option resets tokens without a
recompile.

Gamepads: `gamepad.rs` reads controllers with `gilrs` on its own thread and repeats a held
button after 380 ms, then every 85 ms. `PadButton::action` (`input.rs`) maps each button to a
key the theme already handles (d-pad to arrows, South to Return, East to Escape, and so on).
`Vm.pad-key` is true while that key is delivered, so a theme tells a controller's keys from a
keyboard's (the default theme's on-screen keyboard opens for a controller only).

## Startup and shutdown

`crates/nexus-app/src/main.rs`, in order:

1. Tracing starts. `--supports <feature>` and `--probe-gpu` (the Vulkan probe's child process)
   answer and exit.
2. Arguments parse (`args.rs`). `apply_launcher_update` (`update.rs`) swaps in a kept update
   and restarts if there is one.
3. A tokio runtime starts. `Kernel::builder` gets the ports, `built_in_plugins()`, the two
   `discover` folders, and `boot()`s.
4. `Core::start` builds the `AppState`, spawns the download manager, removes work folders a
   crash left (`clean_up_game_installs`), restores saved torrents, resumes installs and plan
   downloads, schedules emulator update checks, reads the hardware once in the background, and
   starts `watch_catalogs`.
5. `--folder` mappings apply as `Command::SetFolders`. `quit_on_signal` is spawned.
6. `AppShell::new` makes the `ThemeHost`, which collects the themes; `start_with` shows the
   chosen theme or `default`.
7. Old thumbnails are swept, and the gamepad thread, the core event task and the 250 ms tick
   start.
8. `Command::RefreshLibrary` is dispatched, the first render runs, and Slint's event loop runs.

The event loop ends on Quit, Restart, or a signal: Ctrl-C, SIGTERM or SIGHUP quit the same way
as the Quit button, and a second signal exits at once (code 130). Then the timer and the UI state
are dropped and `Core::shutdown` runs, at most `SHUTDOWN_LIMIT` (10 s), while the runtime still
runs:

1. `Torrents::stop_all`, so each torrent writes down the pieces it has. Downloads stay as they
   were saved, and the next start resumes them.
2. `Kernel::shutdown`: every plugin's `shutdown`, in reverse load order.

After a Restart, the launcher starts itself again with the same arguments.

## Performance notes

These shape the code; keep them in mind when changing it.

- **One render per frame.** Each render rebuilds the whole `AppView` and rebinds it, so renders
  are coalesced: one pending flag for core events, and the shell's `RENDER_GAP` between renders.
- **The Catalog grid is built only while the Catalog is on screen.** `view::build` keeps the
  last grid in `AppState.catalog_grid`. On other screens `catalog_vm` reuses it instead of
  searching and sorting every offer again. It is cleared when the last catalog plugin turns off.
- **Genre matching is cached.** `releases::genres_in` keeps each genre text it matched (up to
  16,384 texts), since every view asks for every offer's genres and a catalog has few
  distinct texts.
- **Images.** Only cards on or near the screen ask for art. The shell's `ImageLoader` fetches 8
  and decodes 4 at once, off the UI thread, drops loads that left the screen (`StillWanted`),
  and keeps smaller JPEGs in `cache/thumbs`, so a warm start decodes small files.
  `prefetch.rs` fetches the art beside the selection; each select cancels the last prefetch.
- **Select settles.** `Command::Select` waits 150 ms before loading details, so scrolling past
  games loads nothing.
- **Images show in batches.** A cover that comes in doesn't change the core: the shell shows
  the covers that came in at most every `IMAGES_GAP` (100 ms), with `ThemeHost::render_images`,
  which binds the last view again instead of asking the core for a new one.
- **Long card lists change by row.** `bind::CardModels` keeps the Catalog's, the Library's and
  Home's cards as last shown, with their covers' state, and the theme's list for each. A render
  converts and replaces only the cards that changed, so its cost hardly grows with the pages
  loaded (5 ms with 1,860 Catalog cards in a release build, from 41). Each card still asks for its
  cover, so the image cache keeps and loads what shows.
- **Grids build only what's near the screen.** A grid's slot keeps its place for every shown
  game, but builds its card only within a screen of the visible rows.
- **Grids page.** Library and Catalog grids grow `GRID_PAGE` (60) games at a time.
- **Dev profile.** `[profile.dev]` builds the workspace at `opt-level = 1` and every dependency at
  `opt-level = 2`, so image decoding, JSON and Slint run near release speed under `cargo run`
  and in tests.

## Tests

Four seams, from the highest:

1. **The core's API** through `FakeHost` (`nexus-testkit`): commands in, view models, events and
   errors out. Boot rules, lifecycle and isolation, spans, library, catalogs, downloads,
   torrents, installs, launches, VPN, settings. The core runs on a `FakeClock`: tokio's time,
   paused at boot (the `test-util` feature, in the test kit only). Its time moves when a test
   calls `advance`, or when every task waits for a timer; a slow machine doesn't move it, and
   work in `spawn_blocking` holds it. A test waits for work with `TestCore::until(area, done)`,
   which checks `done` after each change in that area, or with `FakeClock::run_until_idle`,
   and for a timer with `advance`. No seam test sleeps; the screenshot tests move Slint's
   time themselves.
2. **Capability contracts**: `nexus_testkit::contract::emulator`, run on KytyPS5, shadPS4 and
   PC.
3. **The composed host**: the core and the `AppShell` the app ships, on Slint's testing backend
   (`i-slint-backend-testing`, pinned with Slint), with `FakePlatform` and the core's
   `FakeClock`: theme completeness, inheritance, switching and rollback, slots, gamepad,
   screenshots, and the shell's own rules (`shell.rs`: render coalescing, the tick, the requests
   only the app carries out). `Composed::pump` hands the theme's requests to the shell and runs
   the work it started; the renders a test asks for load no images until it calls
   `load_images`.
4. **The desktop adapter against real windows** (`crates/nexus-app/tests/desktop.rs`). It opens
   windows, so it runs only with `NEXUS_DESKTOP_TESTS=1`.

`NEXUS_REAL_RELEASES=1` runs `crates/nexus-app/src/real_releases.rs`, which installs the real
emulator builds over the network. `scripts/check.sh` runs `cargo fmt --check`, clippy with
warnings as errors, and the whole test suite.
