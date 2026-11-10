# Writing a theme

A theme is a folder of Slint markup with a `theme.toml` manifest, and images, fonts and sounds
if it wants them. It has no code. Nexus compiles the markup when it shows the theme. The host
fills the `Vm` global with the view, and carries out what the theme asks through the `Host`
global.

Every theme stands alone. It ships its own shell and every screen. The host serves a kit of
shared Slint files under `kit/`, and a theme imports what it wants from it. The reasons are in
[ADR 0003](adr/0003-themes-stand-alone-with-a-shared-kit.md).

This page is for designers and developers who write a theme. Terms follow
[the glossary](../GLOSSARY.md). The built-in Classic theme is described file by file in
[reference/classic-theme.md](reference/classic-theme.md). How plugins load is in
[plugins.md](plugins.md). How to build and test the repository is in
[development.md](development.md).

## Contents

- [The built-in themes](#the-built-in-themes)
- [Where a theme is installed](#where-a-theme-is-installed)
- [The folder](#the-folder)
- [theme.toml](#themetoml)
- [The files the host serves](#the-files-the-host-serves)
- [The view model: Vm](#the-view-model-vm)
- [Host: what to ask for](#host-what-to-ask-for)
- [Torrent actions](#torrent-actions)
- [Slots](#slots)
- [The kit](#the-kit)
- [Tokens](#tokens)
- [Options](#options)
- [Breakpoints and the size of everything](#breakpoints-and-the-size-of-everything)
- [Keys and the gamepad](#keys-and-the-gamepad)
- [Images, fonts, sounds and system art](#images-fonts-sounds-and-system-art)
- [Rules the host checks](#rules-the-host-checks)
- [Dev mode](#dev-mode)
- [Switching, the fallback theme and safe mode](#switching-the-fallback-theme-and-safe-mode)
- [Testing a theme](#testing-a-theme)
- [Limitations](#limitations)

## The built-in themes

Two themes are compiled into the binary. Each crate's `build.rs` embeds every file under its
`theme/` folder.

- **Default** (`default`, `plugins/themes/default/theme/`). Its `theme.toml` says
  `fallback = true`, so the app starts with it. Home is a list of systems on the far side of
  the reading direction: "All games" first, then each entry of `Vm.emulators`. The focused
  system's cover fills the screen behind the list. Enter or a click opens the Library on that
  system's games (`LibrarySystem`). Menu opens the emulator's plugin page. The Library shows
  the systems as chips in the top bar (`components/chip-row.slint`). Default sets no tokens:
  the defaults are its look.
- **Classic** (`classic`, `plugins/themes/classic/theme/`): the old launcher's look, with a
  row of game tiles and a hero on Home. It sets every token, each to its default value.

The two share most files. Only `theme.toml`, `shell.slint` (one comment), `screens/home.slint`,
`components/nav.slint`, `components/top-bar.slint` and `components/library-view.slint` differ.
Default adds `components/chip-row.slint`, `images/all-games-icon.png` and
`images/all-games-cover.jpg`. Both draw Downloads, the Game Hub, plugin pages
and the Emulators and Themes lists with kit pages.

## Where a theme is installed

Put the theme's folder in the extensions folder. Nexus reads it at start.

- The extensions folder is `--extensions <path>`, or `<data-dir>/extensions` without it.
- The data folder is `--data-dir <path>`, or by default (`default_data_dir` in
  `crates/nexus-app/src/args.rs`):
  - Linux: `$XDG_DATA_HOME/nexus`, else `~/.local/share/nexus`.
  - macOS: `~/Library/Application Support/nexus`.
  - Windows: `%APPDATA%\nexus`.
- Nexus also reads `extensions` next to the binary, or `../Resources/extensions` in a macOS app
  bundle when that folder exists (`bundled_extensions`).

`Kernel::discover` (`crates/nexus-core/src/kernel.rs`) reads each subfolder that holds a
`plugin.toml` or a `theme.toml`, in sorted order. The built-in plugins are registered first.
When two plugins have the same id, the first one stays and the other is rejected as a
duplicate. So a folder can't replace `default` or `classic`.

The kernel rejects a theme folder when (`validate` in `kernel.rs`):

- the manifest doesn't read;
- `host_api` doesn't match the host's API version (`API_VERSION` in `crates/nexus-api/src/lib.rs`
  is `0.2.0`);
- `platforms` is set and doesn't include this system;
- `[runtime] type` is not `"data"` (`"third-party plugins must be data-only"`);
- `capabilities` is anything other than `["theme"]`.

A rejected plugin is in the log as `plugin not loaded`, with the reason. A theme whose `[theme]`
section doesn't read is left out of the Themes list, and the log says `theme skipped`
(`ThemeSet::from_core` in `crates/nexus-ui-host/src/themes.rs`).

`--safe-mode` loads the built-in plugins only, and shows the fallback theme.

## The folder

`tests/fixtures/themes/fixture-grid/` is the smallest complete theme in the repository: a light
grid theme with a sidebar. Its layout is the layout every theme has:

```text
fixture-grid/
  theme.toml
  shell.slint
  screens/home.slint
  screens/game-detail.slint
  screens/catalog.slint
  screens/downloads.slint
  screens/settings.slint
  screens/emulators.slint
  screens/themes.slint
  screens/plugin.slint
  screens/game-settings.slint
  screens/running.slint
  components/card.slint
  fonts/Inter-Regular.ttf
  fonts/Inter-Bold.ttf
  fonts/OFL.txt
```

A theme must ship `shell.slint` and the ten screen files. Other `.slint` files (such as
`components/`) and asset files are the theme's choice. The screen list is `ScreenId::ALL` in
`crates/nexus-ui-api/src/vm.rs`. Each screen file exports one component, under this name
(`ScreenId::component` in `crates/nexus-ui-api/src/contract.rs`):

| `Vm.screen` | File | Exported component |
| --- | --- | --- |
| `home` | `screens/home.slint` | `HomeScreen` |
| `game-detail` | `screens/game-detail.slint` | `GameDetailScreen` |
| `catalog` | `screens/catalog.slint` | `CatalogScreen` |
| `downloads` | `screens/downloads.slint` | `DownloadsScreen` |
| `settings` | `screens/settings.slint` | `SettingsScreen` |
| `emulators` | `screens/emulators.slint` | `EmulatorsScreen` |
| `themes` | `screens/themes.slint` | `ThemesScreen` |
| `plugin` | `screens/plugin.slint` | `PluginScreen` |
| `game-settings` | `screens/game-settings.slint` | `GameSettingsScreen` |
| `running` | `screens/running.slint` | `RunningScreen` |

`shell.slint` exports `Shell`, which inherits `Window`. The shell decides which screen shows for
`Vm.screen`, and where. The host checks only that each file exists and exports its name. A screen
can be a one-line kit page:

```slint
import { DownloadsPage } from "../kit/downloads.slint";

// Downloads: the kit's page, drawn in this theme's tokens.
export component DownloadsScreen inherits DownloadsPage {}
```

A missing file stops the load with `ThemeError::File`, which names the file:
`screens/downloads.slint: is missing: a theme ships its shell and every screen`.

## theme.toml

The top of the file is the plugin manifest. A theme folder needs these keys. From
`tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
id = "fixture-grid"
kind = "theme"
name = "Fixture grid"
version = "0.1.0"
host_api = "^0.2"
capabilities = ["theme"]

[runtime]
type = "data"
```

The `[theme]` section is `ThemeSection` in `crates/nexus-ui-api/src/theme.rs`. It refuses keys it
doesn't know.

| Key | Type | Default | What it does |
| --- | --- | --- | --- |
| `ui_api` | version range | required | The `nexus-ui-api` contract versions the theme works with. The host's is `UI_API_VERSION`, `0.2.0` (`crates/nexus-ui-api/src/lib.rs`). A theme that doesn't match fails with `ThemeError::UiApi`. |
| `fallback` | bool | `false` | This theme is the one the app starts with, falls back to, and shows in safe mode. Exactly one installed theme sets it. Default does. See [Switching](#switching-the-fallback-theme-and-safe-mode). |
| `input` | list | `["pointer"]` | The input devices the theme supports, the main one first: `pointer`, `gamepad`, `touch`. The first one is `Vm.input-mode` at start. |
| `navigation` | `pointer` or `focus` | `pointer` | How the player moves around. No host code reads it yet. |
| `tokens` | table | none | Values for [tokens](#tokens). A token the theme leaves out keeps its default. |
| `breakpoints` | array of tables | none | Named window sizes. See [Breakpoints](#breakpoints-and-the-size-of-everything). |
| `assets` | table | none | `images`, `fonts`, `sounds`. See [Images, fonts, sounds](#images-fonts-sounds-and-system-art). |
| `options` | table | none | Settings the player changes, which set tokens. See [Options](#options). |
| `systems` | table | none | A system's own icon and cover, by system id. See [System art](#images-fonts-sounds-and-system-art). |

The host refuses three keys of the old inheritance model, each with what to do instead
(`ThemeManifest::from_plugin`):

| Key | Message |
| --- | --- |
| `extends` | `` `extends` is not supported: a theme stands alone; import the shared parts from kit/ instead of extending a theme `` |
| `screens` | `` `screens` is not supported: a theme ships every screen, so it doesn't list them; remove `screens` `` |
| `components` | `` `components` is not supported: a theme replaces a kit file by shipping kit/<name>.slint; remove `components` `` |

A theme with one of them is left out of the Themes list. The log has the message under
`theme skipped`.

## The files the host serves

The host serves four files at the root of every theme. A theme imports them and never ships
them: a theme that ships one fails with `is served by the host; remove it`
(`RESERVED_FILES` in `contract.rs`).

| File | Global | Holds |
| --- | --- | --- |
| `types.slint` | `Vm`, `Host`, `Json` and the structs | The view model, the callbacks, and helpers to build command args. The source is `crates/nexus-ui-api/contract/types.slint`. |
| `tokens.slint` | `Tokens` | One property per token. It also imports the theme's fonts. |
| `assets.slint` | `Assets` | One `image` per name in `[theme.assets.images]`, and `loaded`. |
| `options.slint` | `Options` | One property per option: a `string` for a choice option, a `color` for a color option. |

Import paths are relative to the importing file. The shell imports `"types.slint"`. A screen
imports `"../types.slint"`.

## The view model: Vm

`Vm` holds everything the theme shows. The host sets it after every change, and a theme only
reads it. It is the core's `AppView` (`crates/nexus-ui-api/src/vm.rs`) field for field, with
kebab-case names. The comments in `crates/nexus-ui-api/contract/types.slint` say what each field
and struct holds. Read that file as the reference.

| Topic | `Vm` properties |
| --- | --- |
| Where the player is | `screen`, `game`, `can-go-back` |
| Home and the Library | `home` (with `home.library`), `has-detail`, `detail` |
| The Catalog | `catalog`, `catalog-sources`, `catalog-releases`, `catalog-updated`, `catalog-loading`, `catalog-available`, `catalog-actions`, `catalog-status` |
| Downloads | `downloads`, `torrents`, `seed-downloads`, `torrents-downloading`, `torrents-sharing` |
| Settings | `vpn`, `backup`, `settings`, `theme-options`, `themes`, `folders`, `sounds`, `fullscreen`, `return-on-exit`, `auto-update`, `scale`, `can-start-at-login`, `start-at-login`, `start-at-login-error`, `hardware`, `launcher-update`, `system`, `monitors`, `monitor` |
| Plugins | `emulators`, `plugins`, `emulator-plugins`, `theme-plugins`, `has-plugin-page`, `plugin-page` |
| Trailers | `can-play-trailers`, `has-trailer`, `trailer`, `trailer-frame` |
| Prompts | `has-install-prompt`, `install-prompt`, `has-result-prompt`, `result-prompt` |
| Start | `boot`, `art-ready` |
| Status | `task`, `clock`, `has-running`, `running`, `notices`, `notice-cards`, `notice-art`, `notice-opens` |
| Window and input | `viewport`, `input-mode`, `pad-key`, `power-menu`, `pad-name` |

An optional part comes with a `has-` flag: read `detail` only when `has-detail` is true. Text
for the player, such as sizes and dates, comes ready to show (display text, built in the
core's `display.rs`).

## Host: what to ask for

A theme changes nothing itself. It asks the host through the `Host` global in `types.slint`.

`Host.command(name, args)` sends a core `Command` (`crates/nexus-core/src/app.rs`). `name` is
the variant's name. `args` is its payload as JSON text, or `""` for a variant with no payload.
The host builds the value from the two (`command` in `crates/nexus-ui-host/src/host.rs`):

- With empty `args`, the value is the bare name: `Host.command("Back", "")` reads as `"Back"`.
- With `args`, the value is `{ name: args }`: `Host.command("Select", "\"emu:astro\"")` reads as
  `{"Select": "emu:astro"}`.

A command whose args aren't JSON, or don't fit the variant, is dropped. The log says
`the theme sent a command that doesn't read`. The theme gets no error.

Build the args with the `Json` global. `Json.text(s)` makes a JSON string with its escapes.
`Json.text-or-null(s)` gives `null` for `""`. `Json.flag(b)` gives `true` or `false`.

```slint
Host.command("Back", "");
Host.command("Select", Json.text(game.key));
Host.command("Session", "{\"Launch\":" + Json.text(game.key) + "}");
```

Use `kit/commands.slint` instead of writing the JSON. Its `Commands` global has one function
for each thing the player asks: `Commands.back()`, `Commands.launch(key)`,
`Commands.set-sounds(on)` and so on. Read it for working args of most commands.

### The commands

A variant with a sub-enum takes that enum, written as JSON the same way.

| Variant | Payload |
| --- | --- |
| `Navigate` | A `Route`: `"Home"`, `"Catalog"`, `"Downloads"`, `"Settings"`, `"Emulators"`, `"Themes"`, `{"GameDetail": key}`, `{"Plugin": id}`, `{"GameSettings": key}`, `{"Running": session}`. |
| `Back` | None. |
| `Select`, `OpenGame`, `UninstallGame`, `AskResult` | A game key. |
| `Library` | `"Refresh"`, or `{"Hide": {"game": key, "hidden": bool}}`. |
| `Search`, `LibrarySearch`, `CatalogSearch` | Text. |
| `LibrarySystem`, `CatalogSystem` | A system id (`PS5`) or a grid filter (`ingame`, `installed`, `hidden`), or `null` to clear them. |
| `LibraryGenre`, `CatalogGenre` | A genre, or `null` for every genre. |
| `LibrarySort`, `CatalogSort` | `title`, `played`, `size`, `smallest`, `rating`, `release`, `compat` or `added`. |
| `LibraryMore`, `CatalogMore` | None. Shows the next page. |
| `GridColumns` | `{"grid": "Library" or "Catalog", "columns": n}`. |
| `Session` | `{"Launch": key}`, `{"Stop": session}`, `{"ShowGame": session}`, `"ToggleFocus"`. |
| `ReportResult` | `"ingame"`, `"menus"`, `"boots"`, `"nothing"`, or `null` for Not now. |
| `ConfirmInstall`, `CancelInstall` | None. |
| `Torrent` | `{"Download": {"offer": key, "release": id, "folder": path}}` (`folder` optional), `{"Act": {"id": id, "action": action}}`, `{"SeedDownloads": bool}`. See [Torrent actions](#torrent-actions). |
| `GameInstall` | `{"Download": {"id": id, "folder": path}}` (`folder` optional), `{"Package": {"plugin": id, "source": {"folder": name, "root": n, "path": path}}}` (a `HostPath`), `{"RemoveUnpacked": {"id": id}}`, `{"Cancel": {"id": id}}`. |
| `Vpn` | `{"SetOnly": bool}`, `{"Choose": name}`, `{"Connect": name}`, `{"Disconnect": name}`, `{"Import": path}`. |
| `Backup` | `"Refresh"`, `{"BackUp": {"large": bool}}`, `{"Restore": file}`, `{"RestoreFile": path}`, `{"Delete": file}`, `{"Browse": path or null}`, `"CloseBrowser"`, `{"ShowHidden": bool}`, `{"Filter": text}`. |
| `Notice` | `"DismissAll"`, `{"Dismiss": id}`, `{"Open": id}`. |
| `LauncherUpdate` | `"Check"`, `"Download"`. |
| `System` | `SystemCommand` (`crates/nexus-core/src/system.rs`): `"Refresh"`, `"ScanWifi"`, `{"JoinWifi": {"ssid": name, "password": text or null}}`, `{"ForgetWifi": name}`, `{"SetBluetooth": bool}`, `"ScanBluetooth"`, `{"Pair": address}`, `{"ConnectDevice": address}`, `{"DisconnectDevice": address}`, `{"RemoveDevice": address}`, `{"SetOutput": name}`, `{"SetVolume": 0-100}`, `{"SetMuted": bool}`, `"Restart"`, `"ShutDown"`, `"CheckOsUpdate"`, `{"InstallOs": path}`, `"ChoosePartitions"`, `{"CancelJob": id}`. |
| `Pref` | A pair, such as `["sounds", false]`. The key is a `Pref` in snake case (`crates/nexus-core/src/preferences.rs`). The player sets `sounds`, `fullscreen`, `return_on_exit`, `scale`, `start_at_login`, `auto_update`, `monitor`, `welcomed` and `backup_folder`. |
| `SetThemeOption` | `{"key": option, "value": text}`. |
| `SetConfig` | `{"plugin": id, "scope": scope, "value": json}`. A scope is `{"type": "emulator"}` or `{"type": "game", "game": text}`. Pass a settings panel's `scope` back unchanged. |
| `SetConfigField` | `{"plugin": id, "scope": scope, "key": field, "value": text}`. |
| `RunAction` | `{"plugin": id, "action": name, "game": key or null, "args": json}`. |
| `SetPluginEnabled` | `{"plugin": id, "enabled": bool}`. A theme stays on. |
| `SetFolders` | `{"name": folder, "paths": text}`, the folders separated by `;`. |
| `Download` | `{"plugin": id, "request": {"type": "emulator_build", "version": text or null}}`. |
| `PauseDownload`, `ResumeDownload`, `CancelDownload` | A download's id (a number). |
| `InstallEmulator` | `{"plugin": id, "version": text or null}`. |
| `UpdateEmulator` | `{"plugin": id}`. |
| `UninstallEmulator` | `{"plugin": id, "version": text}`. |
| `CheckUpdates` | None. |
| `ThemeSwitched` | The host sends it. A theme asks for a switch with `Host.switch-theme`. |

The doc comments on `Command` in `app.rs` say more about each variant.

### The other callbacks

The other `Host` callbacks never reach the core. They go to the app shell (`UiRequest` in
`crates/nexus-ui-host/src/host.rs`).

| Callback | Does |
| --- | --- |
| `switch-theme(id)` | Switches to the theme `id`. |
| `play-sound(name)` | Plays a sound from `[theme.assets.sounds]`. |
| `open-url(url)` | Opens a page in the system's browser. Only `https://` addresses; others are dropped. |
| `open-game-folder(key)`, `open-game-log(key)` | Opens a game's folder, or its emulator log. The theme names the game, never a path. |
| `open-download-folder(id)` | Opens a download's folder, once it is on disk. |
| `play-trailer(url)`, `pause-trailer()`, `seek-trailer(seconds)`, `close-trailer()` | The trailer player. `play-trailer` takes `https://` only. `seek-trailer` goes back with a negative number. |
| `grid-window(grid, first, last)` | Says which cards of a grid (`"library"` or `"catalog"`) are on screen. Only covers near them load. |
| `quit()`, `restart()`, `log-out()` | Closes Nexus, starts it again, or ends the Nexus session. |

A theme with a grid calls `grid-window` when the visible cards change. Until it calls, the host
loads covers near cards 0 to 29 (`Window::START` in `crates/nexus-ui-host/src/images.rs`).

## Torrent actions

Each `Torrent` in `Vm.torrents` has `actions`, a list of names in the order to show them. The
names are `start`, `pause`, `cancel`, `remove`, `stop-seeding`, `install`, `play`,
`remove-unpacked`, `cancel-install`, and `open-store` for an install a store does.

The theme turns an action into a command:

| Action | Command |
| --- | --- |
| `start`, `pause`, `cancel`, `remove`, `stop-seeding`, `open-store` | `Torrent`, `{"Act": {"id": id, "action": Name}}`. `Name` is the action in Pascal case: `StopSeeding`, `OpenStore`. |
| `install` | `GameInstall`, `{"Download": {"id": id}}`. |
| `play` | `Session`, `{"Launch": game}`, with the torrent's `game`. |
| `remove-unpacked` | `GameInstall`, `{"RemoveUnpacked": {"id": id}}`. |
| `cancel-install` | `GameInstall`, `{"Cancel": {"id": id}}`. |

The core never lists `delete-files`. A theme sends it (`"DeleteFiles"`) when the player removes a
download with its files. `Commands.torrent-action(id, action)` in `kit/commands.slint` does all of
this, and `play` is `Commands.launch(game)`.

## Slots

A slot item is something a plugin adds to a screen. It reaches a theme as a `Slot` struct.

| Where | `kind` | Draw it as |
| --- | --- | --- |
| `GameCard.badges`, `GameDetail.badges` | `badge` | A small tag. `short` is its text for small places. |
| `GameDetail.actions` | `action` | A button. |
| `GameDetail.sections` | `section` | A block of `rows`. |
| `SettingsPanel.slot` | `settings_panel` | A form of `fields`. |

`tone` is `neutral`, `info`, `success`, `warning` or `danger`. `style` is `primary` or
`secondary`. To run an action, send `RunAction` with the slot's `plugin`, `action` and `args`
and the game's key: `Commands.run-action(plugin, action, game, args)`. `args` and `scope` are
JSON text: pass them back unchanged.

A plugin's page has its own lists: `PluginPage.sections` holds the plugin's blocks (the
`emulator.panel` slot), and `PluginPage.actions` the action slots among them.

Set `accessible-role: text` and `accessible-label: <slot>.label` on each slot item you draw. The
label is `<plugin-id>:<item-id>`. Tests find items by it. The kit's `SlotBadges`, `SlotActions`
and `SlotSections` do it.

## The kit

The kit is shared Slint files the host serves under `kit/`, beneath the theme's own files. The
list is `KIT` in `crates/nexus-ui-api/src/contract.rs`. The files are in
`crates/nexus-ui-api/kit/`. Import a kit file by its path: `"kit/button.slint"` from the
shell, `"../kit/button.slint"` from a screen or a component.

Three rules hold for every kit file. Tests in `contract.rs` check them:

- It draws only from tokens. No hex color is in a kit file.
- It imports only the host's files (`../types.slint`, `../tokens.slint`) and other kit files,
  by bare name.
- It passes the same rules as a theme's files (see [Rules the host checks](#rules-the-host-checks)).

| File | Exports | What it gives |
| --- | --- | --- |
| `backup-browser.slint` | `Browser`, `BackupBrowser` | "Restore from another folder": a sheet with places, folders and backups, details, its own keys and one dialog before a restore. One click opens a place or a folder, or asks to restore a backup. |
| `button.slint` | `Button` | A small round button. Primary and the current choice are white; others light up on hover and focus. |
| `commands.slint` | `Commands` | One function for each thing the player asks of the core. |
| `dialog.slint` | `Dialog` | A titled card: a small-caps header over a card. |
| `downloads.slint` | `Dl`, `DownloadsPage` | The Downloads screen, its keys, and its install and remove sheets. |
| `game-detail.slint` | `Hub`, `GameDetailPage` | The Game Hub: buttons, releases, fact cards, technical details, screenshots, the download sheet. |
| `game-menu.slint` | `GameMenu`, `MenuRow`, `GameMenuSheet` | A game's options menu, with Uninstall asking first. |
| `icon.slint` | `Icon` | The launcher's icons, drawn as paths. |
| `install-os.slint` | `Setup`, `InstallOs` | Nexus OS's USB stick: welcome, the disks, the question before erasing, progress. |
| `install-prompt.slint` | `Prompt`, `InstallPrompt` | The prompt to install an emulator before a first play. |
| `keyboard.slint` | `Osk`, `TextField`, `OnScreenKeyboard` | A text field, and the on-screen keyboard it opens for a controller. |
| `lingering.slint` | `Lingering` | Keeps a sheet on screen while it slides out. |
| `list-item.slint` | `ListItem` | A row: a label, a line under it, a value at the end. |
| `look.slint` | `Look` | Shared sizes (`k`, `px`, `tpx`), `rtl`, colors made from tokens, `timed`, and words and colors for ratings, results, sorts and states. |
| `notices.slint` | `Notices`, `NoticeStack` | The notices on screen; Delete closes the newest. |
| `origin.slint` | `Origin` | The card the Game Hub opens from, for its cover to grow out of it. |
| `plugin-list.slint` | `PluginList`, `PluginListSheet` | The Emulators or Themes list as a side sheet, and its keys. |
| `plugin.slint` | `PluginPage` | One plugin's sheet: on or off, builds, actions, settings, folders; for a theme, "Use this theme" and its options. |
| `power-menu.slint` | `PowerMenu`, `PowerMenuSheet` | Log out, Restart Nexus, Restart, Shut down, or Quit Nexus. |
| `progress-bar.slint` | `ProgressBar` | A thin progress bar. |
| `result-prompt.slint` | `Ask`, `ResultPrompt` | "How far did it get?" after a game closes. |
| `settings-layout.slint` | `LauncherRows`, `DownloadsRows`, `SystemRows`, `BackupRows`, `PowerRows` | Where each row of a Settings tab is, so the keys know where to stop. |
| `settings-rows.slint` | `Prefs`, `Tab`, `Header`, `Row`, `Value`, `Editor`, `FieldRow`, `FolderRow` | The rows of a settings sheet, and where the keys are in it. |
| `settings-sections.slint` | `GamesSection`, `UpdatesSection`, `LauncherSection`, `DownloadsSection`, `PcSection`, `SystemSection`, `BackupSection`, `PowerSection` | The rows of each Settings tab. |
| `sheet-frame.slint` | `SheetFrame` | A side sheet: the wash behind and the panel that slides in. A click on the wash goes back. |
| `shot-viewer.slint` | `Viewer`, `ShotViewer` | A game's screenshots, full screen. |
| `slot-actions.slint` | `SlotActions` | Plugin actions as round buttons. |
| `slot-badges.slint` | `SlotBadges` | Plugin badges as small tags with a dot in their tone. |
| `slot-sections.slint` | `SlotSections` | Plugin sections as fact cards. |
| `system-panel.slint` | `Sys`, `NetIcon`, `SoundIcon`, `BluetoothIcon`, `BatteryIcon`, `SystemSheet` | Network, sound and Bluetooth icons and panels, for when Nexus is the whole screen, and the battery. |
| `trailer-player.slint` | `Trailers`, `TrailerPlayer` | A trailer in the window, with its keys. |
| `widgets.slint` | `FocusRing`, `KeyCap`, `BackButton`, `Stars`, `Chip`, `Pill`, `Kicker`, `Switch`, `Shimmer`, `Cover` | Small widgets. |

### What a shell does for the kit

The kit's pages and sheets have no keys of their own. Each has a global with a `key` function,
and the shell sends it the keys. The built-in shells do this in `capture-key-pressed` and
`key-pressed`. In short, from `plugins/themes/classic/theme/shell.slint`:

- `Browser.key(event.text, event.repeat, event.modifiers.control)` first while `Browser.open`,
  then `Osk.key(...)` while `Osk.open`;
- `PowerMenu.key`, `Setup.key`, `Sys.key`, `Trailers.key`, `Ask.key`, `GameMenu.key`,
  `Viewer.key`, `Prompt.key` while each is open;
- `Prefs.key` on a settings sheet, `PluginList.key` on Emulators and Themes, `Hub.key` on the
  Game Hub, `Dl.key` on Downloads.

The shell also shows each sheet while it is open (for example
`if Browser.open : BackupBrowser { ... }`), and sets `Look.window-height` from the window's
height so the first frame has the right size. Copy the order from a built-in shell.

### Replacing a kit file

Ship a file at the same path, for example `kit/commands.slint`. The host serves the kit first,
then the theme's files over it, so the theme's copy wins (`ResolvedTheme::slint_sources`). Every
file that imports it, the kit's own included, gets the theme's copy. The theme's copy is checked
as a theme file, not as a kit file, so it may use hex colors. Keep its exports: a kit file that
imports it still needs them, or the theme doesn't compile.

## Tokens

A token is a named design value. Slint reads it as `Tokens.<name>`. The list and the defaults
are `TOKENS` in `crates/nexus-ui-api/src/tokens.rs`. A theme sets only the tokens it changes in
`[theme.tokens]`. The defaults are the Classic look. A name that isn't in the list fails the load
with `no such token`.

| Token | Kind | Default |
| --- | --- | --- |
| `color-bg` | color | `#05070f` |
| `color-surface` | color | `#121a31` |
| `color-surface-raised` | color | `#1a2440` |
| `color-text` | color | `#f5f7fb` |
| `color-text-muted` | color | `#b9c3da` |
| `color-accent` | color | `#0a6cff` |
| `color-on-accent` | color | `#ffffff` |
| `color-focus` | color | `#ffffff` |
| `color-success` | color | `#3ddc97` |
| `color-warning` | color | `#ffb547` |
| `color-danger` | color | `#ff5a6a` |
| `color-info` | color | `#7ab4ff` |
| `color-overlay` | color | `#03050ccc` |
| `color-panel` | color | `#0b1020` |
| `color-text-faint` | color | `#7d88a3` |
| `color-on-focus` | color | `#0a0d18` |
| `color-accent-soft` | color | `#8dbaff` |
| `color-caution` | color | `#ffd24d` |
| `color-tint` | color | `#ffffff` |
| `color-shade` | color | `#000000` |
| `font-family` | text | `Inter` |
| `font-size-s` | length | `15` |
| `font-size-m` | length | `19` |
| `font-size-l` | length | `26` |
| `font-size-xl` | length | `44` |
| `font-weight-regular` | int | `400` |
| `font-weight-bold` | int | `700` |
| `space-xs` | length | `4` |
| `space-s` | length | `10` |
| `space-m` | length | `18` |
| `space-l` | length | `28` |
| `space-xl` | length | `48` |
| `radius-s` | length | `8` |
| `radius-m` | length | `14` |
| `radius-l` | length | `20` |
| `shadow-color` | color | `#00000099` |
| `shadow-blur-1` | length | `18` |
| `shadow-blur-2` | length | `36` |
| `motion-fast` | duration | `160` |
| `motion-normal` | duration | `260` |
| `motion-slow` | duration | `420` |
| `layout-direction` | text | `ltr` |

What the newer color tokens are for, from the comments in `tokens.rs`:

- `color-panel`: deeper than the surface: sheets, hints, the backs of panels.
- `color-text-faint`: less than muted: hints and the third line of a row.
- `color-on-focus`: text and marks drawn on `color-focus`.
- `color-accent-soft`: the accent, light: links, and the glow of a focused field.
- `color-caution`: between success and warning: a game that reaches its menus.
- `color-tint`, `color-shade`: light and dark marks laid over anything, at an alpha the kit
  picks.
- `layout-direction`: `"ltr"` or `"rtl"`. `Look.rtl` reads it.

Write each kind in `theme.toml` as `TokenValue::parse` reads it:

- color: `"#rrggbb"` or `"#rrggbbaa"`;
- length: a number, in logical pixels;
- duration: a number, in milliseconds;
- int: a whole number;
- text: a string.

A value of the wrong kind fails the load with `ThemeError::Token`, which names the token.

The kit's animations use the motion tokens or `Look.timed(...)`. With the motion tokens at 0,
`Look.timed` gives 0 too, so "Animations: Off" cuts every move.

## Options

An option is a setting the player changes on the theme's page. It sets tokens. Option names
use lowercase letters, digits and `-`, and start with a letter.

A choice option sets a group of tokens for each choice. From
`plugins/themes/classic/theme/theme.toml`:

```toml
# "off" turns the transitions into cuts.
[theme.options.motion]
label = "Animations"
type = "choice"
default = "on"

[theme.options.motion.choices.on]
motion-fast = 160
motion-normal = 260
motion-slow = 420

[theme.options.motion.choices.off]
motion-fast = 0
motion-normal = 0
motion-slow = 0
```

A color option sets one color token. From `tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"
```

`hidden = true` keeps an option out of the Settings list. It still works when something else
sets it. Both built-in themes hide "Layout direction", because the right-to-left layout is not
finished.

How the values apply (`merge` in `theme.rs`):

- Until the player sets an option, the theme's own `[theme.tokens]` (or the defaults) stay.
  `default` only says what `Options` holds.
- Once the player sets an option, its tokens go over the theme's.
- A color value that doesn't read leaves the token as it was, with a warning in the log.
- An unknown choice sets no tokens. `Options` then holds the default.

`Vm.theme-options` lists the shown theme's options as fields. `Commands.set-theme-option(key,
value)` sets one. A change sets `Tokens` and `Options` again on the running theme, with no
recompile.

A theme reads its option values from the `Options` global. Cut down from the test
`a_theme_reads_its_option_values_from_the_options_global` in `tests/tests/seams/themes.rs`:

```slint
import { Options } from "../options.slint";
export component HomeScreen inherits Rectangle {
    Text {
        text: Options.motion;
        color: Options.tint;
    }
}
```

## Breakpoints and the size of everything

A breakpoint is a named window size. `[[theme.breakpoints]]` lists them in order. The first one
whose limits all hold is the current one (`Breakpoint::matches`). A breakpoint with no limits
always holds, so put it last. When none holds, the name is `""`.

| Key | Limit |
| --- | --- |
| `name` | The name `Vm.viewport.breakpoint` holds. |
| `min_width`, `max_width` | The window's width, in logical pixels. |
| `min_aspect`, `max_aspect` | Width divided by height. |
| `min_scale` | Physical pixels per logical pixel. |

The built-in themes have `compact` (`max_width = 900`) and then `regular`.

`Vm.viewport` holds `width`, `height`, `scale` (the screen's pixel density), `aspect`,
`breakpoint`, and `size`. `size` is the player's "Size of everything", from 0.8 to 1.4, where 1
is the theme's own size (`MIN_SCALE` and `MAX_SCALE` in `crates/nexus-core/src/preferences.rs`).
The `NEXUS_SCALE` environment variable sets it for one run. A theme that doesn't multiply its
sizes by `size` ignores the setting. The kit's `Look.k` does it: `Look.k` maps a 1920×1080
canvas to the window, times `size`. Write a size as `N * Look.px` for N pixels at 1080p.

## Keys and the gamepad

A theme needs no gamepad code. The host turns each controller button into a key press on the
window (`PadButton::action` in `crates/nexus-ui-host/src/input.rs`). So a keyboard does what a
controller does.

| Button | Key the theme gets |
| --- | --- |
| D-pad and left stick | `Key.UpArrow`, `Key.DownArrow`, `Key.LeftArrow`, `Key.RightArrow` |
| South (A, ✕) | `Key.Return` |
| East (B, ○) | `Key.Escape` |
| North (Y, △) | `"/"` |
| West (X, □) | `Key.Insert` |
| Start (Options) | `Key.Menu` |
| Select (Create, Share, View) | `Key.Delete` |
| Left shoulder (L1) | `Key.Backtab` |
| Right shoulder (R1) | `Key.Tab` |
| Left trigger (L2) | `Key.PageUp` |
| Right trigger (R2) | `Key.PageDown` |

The Guide button (PS, Xbox or Logitech) is not a key:

- A tap sends `Navigate` to `Home`. While a game runs, a tap does nothing.
- Held for 600 ms (`GUIDE_HOLD` in `crates/nexus-app/src/gamepad.rs`), it stops the running
  games. With no game running, it adds one to `Vm.power-menu`. A theme opens its power menu when
  that number changes.

While a game's window is in front, or Nexus's window has no focus, the other buttons do
nothing.

A held button or stick repeats after 380 ms, then every 85 ms (`gamepad.rs`). A repeat comes as
a repeated key press, so `event.repeat` is true. Ignore repeats of Enter: a held ✕ would play,
download or install again.

`Vm.pad-key` is true while the host delivers a controller's key, and false for a keyboard's. The
kit's text fields open the on-screen keyboard only for a controller. `Vm.input-mode` is the first
entry of `input` at start, and `gamepad` once a controller button comes in. `Vm.pad-name` is the
controller's name, empty for none.

## Images, fonts, sounds and system art

Declare each file in `[theme.assets]`. Paths are relative to the theme folder and use `/`. From
`plugins/themes/default/theme/theme.toml`:

```toml
[theme.assets]
fonts = [
    "fonts/Inter-Regular.ttf",
    "fonts/Inter-Medium.ttf",
    "fonts/Inter-SemiBold.ttf",
    "fonts/Inter-Bold.ttf",
    "fonts/Tajawal-Regular.ttf",
    "fonts/Tajawal-Bold.ttf",
]

[theme.assets.images]
logo = "images/logo.png"
all-games-icon = "images/all-games-icon.png"
all-games-cover = "images/all-games-cover.jpg"

[theme.assets.sounds]
move = "sounds/move.wav"
select = "sounds/select.wav"
back = "sounds/back.wav"
error = "sounds/error.wav"
start = "sounds/start.wav"
```

**Images.** Each name becomes `Assets.<name>`. A name uses lowercase letters, digits and `-`,
and starts with a letter. The host decodes PNG and JPEG, and shrinks an image to fit 1920×1080
(`MAX_WIDTH`, `MAX_HEIGHT` in `crates/nexus-ui-host/src/images.rs`). A missing or broken image
stops the load with `ThemeError::Asset`. Game art comes from `Vm` (`GameCard.cover` and the
like), not from `Assets`.

**Fonts.** The host registers each font file, through imports in the served `tokens.slint`. Set
`font-family` to the family name inside the files, such as `"Inter"`. A font path must stay
inside the theme folder. A built-in theme has no folder, so the host writes its fonts once to
`launcher-fonts/<theme>/` in the system's temp folder.

**Sounds.** A sound's name is what `Host.play-sound(name)` takes. Nexus plays WAV and Ogg
Vorbis (the `rodio` features in the workspace `Cargo.toml`). The host plays nothing when the
player turned sounds off or the name isn't declared. The theme gets no error. The built-in
themes play `move`, `select`, `back`, `error` and `start`.

**System art.** Each entry of `Vm.emulators` has `system` (such as `PS5`), `icon` (square) and
`cover` (wide). The emulator plugin ships the two pictures. A theme can put its own in their
place, by system id, as PNG or JPEG files:

```toml
[theme.systems.PS5]
icon = "images/ps5-icon.png"
cover = "images/ps5-cover.jpg"
```

The host decodes them when it loads the theme. A picture left out stays the plugin's. A missing
or broken file stops the load with `ThemeError::Asset`. The key is the system, so the picture
stays when another emulator plays the system.

## Rules the host checks

Before compiling, the host checks each of the theme's `.slint` files with `contract::lint`
(`crates/nexus-ui-api/src/contract.rs`). A file that breaks a rule stops the load with
`ThemeError::File`, which names the file.

- No `@image-url`. Show images from `Assets`, from `Vm`, or from slot icons.
- Every `import` and `export ... from` names a `.slint` file. A font import is refused: list
  fonts in `theme.toml`.
- A screen file exports its component under the name in [the table](#the-folder). `shell.slint`
  exports `Shell`.

Comments don't count when the rules are checked.

The host also refuses a theme that ships a [served file](#the-files-the-host-serves), or misses
the shell or a screen.

The compiler serves the files from a virtual folder, `/launcher-theme` (`VFS_ROOT` in
`crates/nexus-ui-host/src/compile.rs`). It reads nothing from disk under it. An import that
leaves the folder fails with `outside the theme`. A compile error stops the load with
`ThemeError::Compile`, which lists each error as `file:line:column: message`.

## Dev mode

Run Nexus with `--dev` to see edits as you save them. Every 250 ms (`TICK` in
`crates/nexus-ui-host/src/shell.rs`), the host compares the newest change time of the theme's
files with the last load. When it changed, the host loads the theme again.

- An edit that doesn't load keeps the last good version on screen. The log says
  `theme edit doesn't load; keeping the last good version`. The host doesn't try again until a
  file changes.
- Markup, images, fonts and sounds reload. `theme.toml` doesn't: the host read it at start.
  Restart Nexus after you change it.
- The built-in themes don't reload: they have no folder on disk.

`--theme <id>` shows a theme for this run without saving it.

## Switching, the fallback theme and safe mode

The player switches theme on the Themes screen. A theme can do the same with
`Host.switch-theme(id)`. The kit's `PluginPage` calls it for "Use this theme".

A switch (`ThemeHost::switch` in `crates/nexus-ui-host/src/host.rs`) reads the theme and its
option values, decodes its images, sounds and system art, compiles it, makes the new instance
in the same window and binds the view. Then the core saves the choice, and the new theme shows.
A failure at any step leaves the old theme on screen. The log says
`the theme's request failed`. The player sees no message. Navigation, downloads and running
games stay: they are in the core, not in the theme.

The fallback theme is the one installed theme with `fallback = true`. Default is that theme.
The host starts it when the player chose none, and when the chosen theme fails to load. The
Themes list shows it first. No theme, or more than one, with `fallback = true` stops the start
with `ThemeError::Fallback`. When more than one sets it, the error names them. So a folder theme must not set it.

`--safe-mode` loads no theme folder and shows the fallback theme only. A switch to another theme
fails with `safe mode shows the fallback theme only`.

## Testing a theme

Tests drive a theme the way a player does, through `ThemeDriver` in
`crates/nexus-testkit/src/driver.rs`. It finds elements by their accessible label and value,
sends keys and clicks, and moves Slint's clock. It never reads `Vm` or calls the theme's
callbacks. So give each element a test needs an `accessible-label`.

| Step | Does |
| --- | --- |
| `count(label)`, `first(label)` | How many elements have the label, or the first. |
| `value(label)`, `values(label)`, `description(label)` | Read accessible values. |
| `focused(label)`, `lit(label)` | Which elements show as focused. |
| `press(label, value)`, `press_first(label)` | Run an element's default action. |
| `click(label)` | A mouse click in the middle of the element. |
| `tap(key)`, `navigate(key)` | Send a key. `navigate` also carries out what the theme asked. |
| `type_text(text)` | Type characters. |
| `settings_tab(name)` | Go to a Settings tab. |
| `settle()`, `wait(duration)` | Move Slint's clock on. |

The seam tests are in `tests/tests/seams/` (package `nexus-tests`, test binary `seams`):

- `every_theme.rs` runs one suite against every built-in theme (`THEMES`): every screen renders,
  the Settings tabs and rows, the Backup tab, the on-screen keyboard, and the backup browser's
  keys, filter, restore dialog and mouse. `THEMES` lists the two built-in themes.
- `themes.rs` covers loading and the rules: the fallback theme, a theme's own card and token,
  refused `extends`, a missing screen, a wrong `ui_api`, imports out of the folder,
  `@image-url` and font imports, served files, a wrong export, the `Options` global, kit
  replacement, and exactly one fallback.
- `classic_theme/` and `default_theme/` test each built-in theme's own screens and files.
  `theme_files.rs` in each checks that every animation time goes through `Look.timed`.

`write_theme` in `tests/tests/seams/common/mod.rs` writes a test theme: a copy of Default's
files under a new id, with your `[theme]` lines and files over it.

Run the theme tests:

```sh
cargo test -p nexus-tests --test seams themes::
cargo test -p nexus-tests --test seams every_theme::
```

Render every screen offscreen to PNG files in `target/tmp/screenshots/`:

```sh
cargo test -p nexus-tests --test seams screenshots::
```

`make check` runs the format check, clippy and every test, as CI does.

## Limitations

- `navigation` is read into the merged theme, but no host code reads it.
- Only the first `input` entry is used, at start. The host never sets `touch`. After the first
  controller button, `Vm.input-mode` stays `gamepad` until Nexus restarts.
- The host doesn't check that slot items set `accessible-label`.
- Dev mode doesn't read `theme.toml` again, and never reloads a built-in theme.
- A failed `Host.switch-theme` is in the log only. The player sees no message.
- A theme can't add screens. The screens are `ScreenId::ALL`.
- Classic's own files still use some fixed hex colors, so a token change doesn't reach every
  part of it. See [reference/classic-theme.md](reference/classic-theme.md).
