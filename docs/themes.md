# Writing a theme

A theme is a folder of Slint markup, a `theme.toml` manifest and optional images, fonts and
sounds. It has no code. The launcher compiles the markup at run time, fills the `Vm` global
with the view, and carries out what the theme asks for through the `Host` global.

This page is for people who write a theme. It uses the words in [the glossary](../GLOSSARY.md).
The built-in theme is described in [reference/default-theme.md](reference/default-theme.md).
How plugins load is in [plugins.md](plugins.md). How to build and test the repository is in
[development.md](development.md). How the crates fit together is in
[architecture.md](architecture.md). The project overview is in [../README.md](../README.md).

## Contents

- [Where themes live](#where-themes-live)
- [A minimal theme](#a-minimal-theme)
- [The manifest](#the-manifest)
- [Inheritance](#inheritance)
- [The contract](#the-contract)
- [Host: what to ask for](#host-what-to-ask-for)
- [Torrent actions](#torrent-actions)
- [Slots](#slots)
- [Tokens and options](#tokens-and-options)
- [Responsive layout](#responsive-layout)
- [Input modes and pad keys](#input-modes-and-pad-keys)
- [Images, fonts and sounds](#images-fonts-and-sounds)
- [Rules the host checks](#rules-the-host-checks)
- [Dev mode](#dev-mode)
- [Switching and rollback](#switching-and-rollback)
- [Testing a theme](#testing-a-theme)
- [Limitations](#limitations)

## Where themes live

The default theme is built into the binary. Its files are in `plugins/themes/default/theme/`,
and `plugins/themes/default/build.rs` embeds them. Its id is `default` (`BASE_THEME` in
`crates/nexus-ui-host/src/host.rs`). Every other theme is a folder on disk.

The launcher looks in two places for theme folders. Both are set in
`crates/nexus-app/src/args.rs` and used in `crates/nexus-app/src/main.rs`.

- The extensions folder. It is the `--extensions <path>` argument, or `<data-dir>/extensions`
  when you give none. `--data-dir <path>` sets the data folder.
- The bundled folder, `extensions` next to the binary (`bundled_extensions`). In a macOS app
  bundle it is `../Resources/extensions` when that exists.

Each subfolder that holds a `theme.toml` (or a `plugin.toml`) is read as a plugin
(`Kernel::discover` in `crates/nexus-core/src/kernel.rs`). Folders are read in sorted order. When
two plugins have the same id, the first one found stays and the other is rejected as a
duplicate. The built-in plugins are registered before the folders, so no folder can replace
`default`.

The host rejects a theme at boot, with a warning in the log, when any of these hold
(`validate` in `kernel.rs`):

- The manifest does not read.
- `host_api` does not match the host's API version.
- `platforms` is set and does not include this system. Themes cannot be turned off, so the
  host leaves them out.
- The theme is third-party and its `[runtime] type` is not `"data"`.
- `capabilities` is anything other than `["theme"]`.
- `extends` names a theme that is not installed, or the chain of `extends` loops
  (`crates/nexus-core/src/resolve.rs`: `extends` counts as a load dependency).

`ThemeSet::from_core` (`crates/nexus-ui-host/src/themes.rs`) then collects the themes that
loaded. A theme whose `[theme]` section does not read is skipped with a warning.

`--safe-mode` loads built-in plugins and the default theme only. It rejects every folder theme.

`crates/nexus-core/src/themes.rs` has the loader. `DirFiles` reads a folder and refuses any
path that leaves it, symlinks included. `DataTheme` is a theme with no code.

## A minimal theme

The smallest real theme in the repository is `tests/fixtures/themes/fixture-grid/`. It extends
`default`, so it ships only the files it changes: a shell, one screen, one component and two
fonts.

```text
fixture-grid/
  theme.toml
  shell.slint
  screens/home.slint
  components/card.slint
  fonts/Inter-Regular.ttf
  fonts/Inter-Bold.ttf
```

Its manifest, `tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
id = "fixture-grid"
kind = "theme"
name = "Fixture grid"
version = "0.1.0"
host_api = "^0.2"
capabilities = ["theme"]

[runtime]
type = "data"

[theme]
ui_api = "^0.2"
extends = "default"
screens = ["home"]
components = ["card"]
input = ["pointer"]
navigation = "pointer"

[[theme.breakpoints]]
name = "compact"
max_width = 1000

[[theme.breakpoints]]
name = "wide"

[theme.assets]
fonts = ["fonts/Inter-Regular.ttf", "fonts/Inter-Bold.ttf"]

[theme.tokens]
color-bg = "#eef0f4"
color-surface = "#ffffff"
color-surface-raised = "#e3e7ef"
color-text = "#151821"
color-text-muted = "#5b6475"
color-accent = "#0f9d8a"
color-on-accent = "#ffffff"
color-focus = "#0f9d8a"
color-overlay = "#151821b3"
shadow-color = "#1518212e"
font-family = "Inter"
radius-m = 10
radius-l = 14

[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"
```

Its shell, `tests/fixtures/themes/fixture-grid/shell.slint`, shortened. The shell exports
`Shell`, which inherits `Window`. It picks the screen to show from `Vm.screen`.

```slint
import { Vm, Host, Json } from "types.slint";
import { Tokens } from "tokens.slint";
import { Button } from "components/button.slint";
import { ListItem } from "components/list-item.slint";
import { HomeScreen } from "screens/home.slint";
// ... one import for each other screen the shell shows

component NavItem inherits ListItem {
    in property <string> screen;
    // The screen's route, the name the command Navigate takes.
    in property <string> route;
    selected: Vm.screen == self.screen;
    clicked => { Host.command("Navigate", Json.text(self.route)); }
}

// A desktop layout: a sidebar of sections next to the current screen.
export component Shell inherits Window {
    title: "Launcher";
    background: Tokens.color-bg;
    preferred-width: 1280px;
    preferred-height: 760px;
    default-font-family: Tokens.font-family;
    default-font-size: Tokens.font-size-m;

    // ... a sidebar of NavItem, then:
    if Vm.screen == "home" || Vm.screen == "running" : HomeScreen { width: parent.width; height: parent.height; }
    // ... one line for each other screen
}
```

Its Home screen, `tests/fixtures/themes/fixture-grid/screens/home.slint`, shortened:

```slint
import { Vm, Host, Json } from "../types.slint";
import { Tokens } from "../tokens.slint";
import { Card } from "../components/card.slint";

// The library as a grid. Narrow windows get two columns.
export component HomeScreen inherits Rectangle {
    property <length> gap: Tokens.space-m;
    property <length> cell: 168px;
    property <length> row-height: 268px;
    property <int> columns: Vm.viewport.breakpoint == "compact"
        ? 2
        : Math.max(3, Math.floor((self.width - 2 * Tokens.space-l + root.gap) / (root.cell + root.gap)));

    Flickable {
        viewport-height: Math.ceil(Vm.home.games.length / root.columns) * (root.row-height + root.gap);
        for game[i] in Vm.home.games : Card {
            x: Math.mod(i, root.columns) * (root.cell + root.gap);
            y: Math.floor(i / root.columns) * (root.row-height + root.gap);
            width: root.cell;
            game: game;
            activated => { Host.command("OpenGame", Json.text(game.key)); }
        }
    }
}
```

Its card, `tests/fixtures/themes/fixture-grid/components/card.slint`, first lines:

```slint
import { GameCard } from "../types.slint";
import { Tokens } from "../tokens.slint";
import { SlotBadges } from "slot-badges.slint";

// A tile: the cover fills it, the title sits on a strip at the bottom.
export component Card inherits Rectangle {
    in property <GameCard> game;
    in property <length> cover-height: 200px;
    callback activated();
    // ...
}
```

The card imports `slot-badges.slint`, which this theme does not ship. The import resolves in
the parent theme (see [Inheritance](#inheritance)).

Run it with the `--extensions` argument. Warning: `--extensions` replaces the default
extensions folder for that run. Themes in `<data-dir>/extensions` do not load.

```sh
cargo run -p nexus-app -- --extensions tests/fixtures/themes --theme fixture-grid --dev --windowed
```

`--theme <id>` shows that theme for this run and does not save it. `--dev` reloads the theme
when a file changes. `--windowed` opens a normal window. Without `--theme`, the player picks the
theme on the Themes screen.

## The manifest

`theme.toml` holds the plugin fields and a `[theme]` section. The plugin fields are read as a
plugin manifest (`PluginManifest` in `crates/nexus-api/src/manifest.rs`). A folder theme needs
these:

| Key | Value |
|---|---|
| `id` | The theme's id. |
| `kind` | `"theme"`. |
| `name` | The name the player sees. |
| `version` | A semver version. |
| `host_api` | A semver range of the host API. |
| `capabilities` | `["theme"]`, and nothing else. |
| `[runtime] type` | `"data"`. |

The `[theme]` section is `ThemeSection` in `crates/nexus-ui-api/src/theme.rs`. An unknown key
in it is an error (`deny_unknown_fields`).

| Key | Meaning |
|---|---|
| `ui_api` | Required. A semver range that must include `UI_API_VERSION` (`0.2.0` in `crates/nexus-ui-api/src/lib.rs`). |
| `extends` | The parent theme's id. See [Inheritance](#inheritance). |
| `screens` | The screens this theme defines itself, by id (see below). |
| `components` | The components this theme defines itself, by id (see below). |
| `input` | Input modes the theme supports, the main one first: `pointer`, `gamepad` or `touch`. |
| `navigation` | `pointer` or `focus`. See [Limitations](#limitations). |
| `tokens` | Design token values. See [Tokens and options](#tokens-and-options). |
| `breakpoints` | Named window classes. See [Responsive layout](#responsive-layout). |
| `assets` | `images`, `fonts` and `sounds`. See [Images, fonts and sounds](#images-fonts-and-sounds). |
| `options` | Settings the player can change. See [Tokens and options](#tokens-and-options). |

A theme can define these screens (`ScreenId` in `crates/nexus-ui-api/src/vm.rs`). Each screen
file is `screens/<id>.slint` and exports the component in the second column
(`ScreenId::component`).

| Id | Component |
|---|---|
| `home` | `HomeScreen` |
| `game-detail` | `GameDetailScreen` |
| `catalog` | `CatalogScreen` |
| `downloads` | `DownloadsScreen` |
| `settings` | `SettingsScreen` |
| `emulators` | `EmulatorsScreen` |
| `themes` | `ThemesScreen` |
| `plugin` | `PluginScreen` |
| `game-settings` | `GameSettingsScreen` |
| `running` | `RunningScreen` |

A theme can define these components (`ComponentId` in `crates/nexus-ui-api/src/contract.rs`).
Each file is `components/<id>.slint` and exports the component in the second column.

| Id | Component |
|---|---|
| `card` | `Card` |
| `button` | `Button` |
| `list-item` | `ListItem` |
| `progress-bar` | `ProgressBar` |
| `dialog` | `Dialog` |
| `slot-badges` | `SlotBadges` |
| `slot-actions` | `SlotActions` |
| `slot-sections` | `SlotSections` |

`ThemeManifest::from_plugin` also refuses an image name that is not lowercase letters, digits
and `-`, starting with a letter.

`ui_api` is checked when the theme is resolved (when it starts or when the player switches to
it), for the theme and every parent. A mismatch fails with `ThemeError::UiApi`.

## Inheritance

`extends = "<id>"` makes the theme a child of another theme. A child writes only what differs.
Every theme chain should end in `default`, but nothing forces it: a theme with no `extends`
must supply every token and every file its shell imports.

`ThemeSet::chain` walks the `extends` links, the theme first. `merge_chain` in
`crates/nexus-ui-api/src/theme.rs` merges the chain from the root down, so the child wins:

| Part | Merge rule |
|---|---|
| Tokens | Per name. The child's value replaces the parent's. |
| Images and sounds | Per name. The child's file replaces the parent's. |
| Fonts | Added up. Every theme in the chain registers its fonts. |
| Breakpoints | A child with any breakpoint replaces the parent's whole list. |
| `input` | The first non-empty list in the chain, the theme first. |
| `navigation` | The first one set in the chain. |
| Options | Per name. The child's replaces the parent's. |

Slint files merge by path (`ResolvedTheme::slint_sources`). A file in the child replaces the
file with the same path in a parent. An import resolves in the child first, then in each
parent. This is how `components/card.slint` in the fixture imports `slot-badges.slint` and gets
the default theme's.

`ResolvedTheme::check` runs before compiling:

- No theme in the chain may ship a reserved file (see [The contract](#the-contract)).
- Every screen and component a theme lists must exist in that theme.
- Some theme in the chain must have a `shell.slint`.

A screen that a theme does not list still comes from the nearest parent that has the file.
Every token in `TOKENS` must have a value somewhere in the chain. A missing one fails with
`ThemeError::Token`, which names it.

## The contract

A theme sees four globals. The host serves them as virtual files. A theme imports them and
never ships its own copy.

| File | Global | Holds |
|---|---|---|
| `types.slint` | `Vm`, `Host`, `Json` | The data, the callbacks, and a helper for JSON text. |
| `tokens.slint` | `Tokens` | One property per design token. |
| `assets.slint` | `Assets` | One `image` per name in `[theme.assets.images]`, and a `loaded` bool. |
| `options.slint` | `Options` | One property per theme option. |

These four names are reserved (`RESERVED_FILES` in `crates/nexus-ui-api/src/contract.rs`). A
theme that ships a file with one of these names fails with `is served by the host; remove it`.

`types.slint` is `crates/nexus-ui-api/contract/types.slint`. It is the source of truth for
every field. It follows the core's view models (`AppView` in
`crates/nexus-ui-api/src/vm.rs`) field for field, with names in kebab case. A test in
`crates/nexus-ui-host/src/drift.rs` fails when the two disagree. Read the comments in
`types.slint` for what each field means. This page does not repeat them.

`Vm` is read-only for a theme. The host sets it after every change. The main groups are:

| Property | Holds |
|---|---|
| `screen`, `game`, `can-go-back` | Where the player is. `screen` is one of the screen ids. |
| `home`, `catalog` | The Library and the Catalog grids (`Library`, `GameCard`). |
| `detail`, `has-detail` | The Game Hub of the selected game (`GameDetail`). |
| `downloads`, `torrents` | Emulator downloads, and the catalogs' downloads (`Torrent`). |
| `settings`, `folders`, `vpn`, `system` | The Settings screen's data. |
| `themes`, `theme-plugins`, `theme-options` | The Themes screen's data. |
| `emulators`, `plugins`, `plugin-page` | The plugin screens' data. |
| `boot`, `art-ready`, `task` | The Welcome and boot screen. |
| `running`, `has-running` | The running game. |
| `notices`, `notice-cards` | Notices for the player. |
| `viewport`, `input-mode`, `pad-key`, `pad-name`, `power-menu`, `clock`, `monitors`, `monitor` | What the host knows about the window and the input. |

`Assets` is one `image` property for each image you declare, such as `Assets.logo` in
`plugins/themes/default/theme/components/boot.slint`:

```slint
Image { source: Assets.logo; width: parent.width; height: parent.height; image-fit: contain; }
```

`Options` is one property for each option, named as the option. A choice option is a `string`
that holds the chosen choice. A color option is a `color`. `options_slint` in `contract.rs`
builds it. The default theme declares options and does not read the `Options` global, so this
page has no sample for it.

The host compiles the theme from a virtual folder. `VFS_ROOT` in
`crates/nexus-ui-host/src/compile.rs` is `/launcher-theme`. No file is read from disk under it.
An import that leaves the folder is refused.

## Host: what to ask for

A theme changes nothing itself. It asks the host through `Host`. The `Host` global is in
`types.slint`.

`Host.command(name, args)` is the Host command. It sends a core `Command`
(`crates/nexus-core/src/app.rs`). `name` is the variant's name. `args` is its payload as JSON
text, or `""` for a variant with no payload. The host builds a `serde` value from the two
(`command` in `crates/nexus-ui-host/src/host.rs`):

- With `args` empty, the value is the bare name: `Host.command("Back", "")` reads as `"Back"`.
- With `args`, the value is `{ name: args }`: `Host.command("Select", "\"emu:astro\"")` reads as
  `{"Select": "emu:astro"}`.

If the args are not JSON, or do not fit the variant, the host drops the command and writes a
warning to the log (`the theme sent a command that doesn't read`). The theme gets no error.

Build the args with the `Json` global. `Json.text(s)` makes a JSON string with the escapes.
`Json.text-or-null(s)` gives `null` for `""`. `Json.flag(b)` gives `true` or `false`. Examples
from the comment in `types.slint`:

```slint
Host.command("Back", "");
Host.command("Select", Json.text(game.key));
Host.command("Session", "{\"Launch\":" + Json.text(game.key) + "}");
```

`plugins/themes/default/theme/commands.slint` wraps the commands, one function each. Read it
for working args of most commands.

### The commands

`Command` has these variants. A variant with a sub-enum takes that enum, written as JSON the
same way.

| Variant | Payload |
|---|---|
| `Navigate` | A `Route`: `"Home"`, `"Catalog"`, `"Downloads"`, `"Settings"`, `"Emulators"`, `"Themes"`, `{"GameDetail": key}`, `{"Plugin": id}`, `{"GameSettings": key}`, `{"Running": session}`. |
| `Back` | None. |
| `Select`, `OpenGame` | A game key. |
| `Library` | `LibraryCmd`: `"Refresh"`, `{"Hide": {"game": key, "hidden": bool}}`. |
| `Search` | Text. |
| `LibrarySearch`, `CatalogSearch` | Text. |
| `LibrarySystem`, `CatalogSystem` | A system name or a grid filter (`ingame`, `installed`, `hidden`), or `null` for all. |
| `LibraryGenre`, `CatalogGenre` | A genre, or `null`. |
| `LibrarySort`, `CatalogSort` | `title`, `played`, `size`, `smallest`, `rating`, `release`, `compat` or `added`. |
| `LibraryMore`, `CatalogMore` | None. Shows the next page. |
| `GridColumns` | `{"grid": "Library" or "Catalog", "columns": n}`. |
| `Session` | `SessionCmd`: `{"Launch": key}`, `{"Stop": session}`, `{"ShowGame": session}`, `"ToggleFocus"`. |
| `AskResult`, `ReportResult` | A game key, or an answer (`ingame`, `menus`, `boots`, `nothing`, or `null`). |
| `ConfirmInstall`, `CancelInstall` | None. |
| `Torrent` | `TorrentCmd`: `Download`, `Act`, `SeedDownloads`. See [Torrent actions](#torrent-actions). |
| `GameInstall` | `GameInstallCmd`: `Download`, `Package`, `RemoveUnpacked`, `Cancel`. |
| `Vpn` | `VpnCmd`: `SetOnly`, `Choose`, `Connect`, `Disconnect`, `Import`. |
| `Notice` | `NoticeCmd`: `"DismissAll"`, `{"Dismiss": id}`, `{"Open": id}`. |
| `LauncherUpdate` | `LauncherUpdateCmd`: `"Check"`, `"Download"`. |
| `System` | `SystemCommand` (`crates/nexus-core/src/system.rs`). |
| `Pref` | A pair: `["sounds", false]`. The key is a `Pref` name in snake case (`crates/nexus-core/src/preferences.rs`). |
| `SetThemeOption` | `{"key": option, "value": text}`. |
| `SetConfig`, `SetConfigField`, `RunAction`, `SetPluginEnabled`, `SetFolders` | Plugin settings and plugin actions. See `app.rs`. |
| `Download`, `PauseDownload`, `ResumeDownload`, `CancelDownload` | Emulator downloads. See `app.rs`. |
| `InstallEmulator`, `UpdateEmulator`, `UninstallEmulator`, `UninstallGame` | Emulators and installed games. See `app.rs`. |
| `CheckUpdates` | None. |
| `ThemeSwitched` | The host sends it. A theme asks for a switch with `Host.switch-theme`. |

The doc comments on `Command` in `app.rs` describe the args of each variant.

### The other callbacks

The other `Host` callbacks never reach the core. They go to the app shell
(`UiRequest` in `host.rs`).

| Callback | Does |
|---|---|
| `switch-theme(id)` | Switches theme. See [Switching and rollback](#switching-and-rollback). |
| `play-sound(name)` | Plays a sound from `[theme.assets.sounds]`. |
| `open-url(url)` | Opens an `https://` page in the browser. Other addresses are dropped. |
| `open-game-folder(key)`, `open-game-log(key)` | Opens the game's folder or its emulator log. The theme names the game, never a path. |
| `open-download-folder(id)` | Opens a download's folder. |
| `play-trailer(url)`, `pause-trailer()`, `seek-trailer(seconds)`, `close-trailer()` | The trailer player. `play-trailer` takes `https://` only. |
| `grid-window(grid, first, last)` | Tells the host which cards of a grid are on screen. |
| `quit()`, `restart()`, `log-out()` | Closes the launcher, restarts it, or ends the Nexus session. |

Only covers near the cards `grid-window` names are loaded. A theme with a grid calls it when the
visible cards change. From `plugins/themes/default/theme/components/library-view.slint`:

```slint
Host.grid-window(Nav.catalog ? "catalog" : "library", grid.first-row * grid.columns,
    (grid.first-row + grid.screen-rows) * grid.columns - 1);
```

## Torrent actions

Each `Torrent` in `Vm.torrents` has an `actions` list of text names, in the order to show them.
The core builds it (`torrent_vm` in `crates/nexus-core/src/view.rs`). The names are:
`start`, `pause`, `cancel`, `remove`, `stop-seeding`, `install`, `play`, `remove-unpacked`,
`cancel-install`, and `open-store` (for an install a store does).

The theme turns an action into a command. The table shows where each goes.

| Action | Command |
|---|---|
| `start`, `pause`, `cancel`, `remove`, `stop-seeding`, `open-store` | `Torrent`, `{"Act": {"id": id, "action": Name}}`. `Name` is the action in Pascal case: `StopSeeding`, `OpenStore`. |
| `install` | `GameInstall`, `{"Download": {"id": id}}`. A `folder` is optional. |
| `play` | `Session`, `{"Launch": game}`. `game` is the `Torrent`'s `game` field. |
| `remove-unpacked` | `GameInstall`, `{"RemoveUnpacked": {"id": id}}`. |
| `cancel-install` | `GameInstall`, `{"Cancel": {"id": id}}`. |

`TorrentAction` also has `DeleteFiles`. The core never lists it in `actions`. A theme sends it
when the player chooses to remove a download and its files.

The default theme does this in `plugins/themes/default/theme/commands.slint`:

```slint
// A download's action as its command's name: stop-seeding is StopSeeding.
pure function pascal(action: string) -> string {
    action == "start" ? "Start" : action == "pause" ? "Pause" : action == "cancel" ? "Cancel"
        : action == "remove" ? "Remove" : action == "delete-files" ? "DeleteFiles"
        : action == "stop-seeding" ? "StopSeeding" : action == "open-store" ? "OpenStore" : action
}
```

```slint
public function torrent-action(id: string, action: string) {
    if (action == "install") {
        Host.command("GameInstall", "{\"Download\":{\"id\":" + Json.text(id) + "}}");
    } else if (action == "remove-unpacked") {
        Host.command("GameInstall", "{\"RemoveUnpacked\":{\"id\":" + Json.text(id) + "}}");
    } else if (action == "cancel-install") {
        Host.command("GameInstall", "{\"Cancel\":{\"id\":" + Json.text(id) + "}}");
    } else {
        Host.command("Torrent", "{\"Act\":{\"id\":" + Json.text(id) + ",\"action\":" + Json.text(Commands.pascal(action)) + "}}");
    }
}
```

## Slots

A slot item is something a plugin adds to a screen. It reaches a theme as a `Slot` (see
`types.slint`). A slot has a kind, and the theme draws it:

| Where the theme gets it | Kind | Draw it as |
|---|---|---|
| `GameCard.badges`, `GameDetail.badges` | `badge` | A small tag. `short` is the text for small places. |
| `GameDetail.actions` | `action` | A button. |
| `GameDetail.sections` | `section` | A block with `rows`. |
| `SettingsPanel.slot` | `settings_panel` | A form from `fields`. |

`Slot.tone` is `neutral`, `info`, `success`, `warning` or `danger`. `Slot.style` is `primary` or
`secondary`.

Set `accessible-role: text` and `accessible-label: <slot>.label` on each slot item you draw.
Tests find an element by that label. The host does not check this rule. See
[Limitations](#limitations).

To run an action, send `RunAction` with the slot's `plugin`, `action` and `args`, and the game
key. `args` is JSON text that the theme passes back unchanged. From
`plugins/themes/default/theme/commands.slint`:

```slint
public function run-action(plugin: string, action: string, game: string, args: string) {
    Host.command("RunAction", "{\"plugin\":" + Json.text(plugin) + ",\"action\":" + Json.text(action)
        + ",\"game\":" + Json.text-or-null(game) + ",\"args\":" + (args == "" ? "null" : args) + "}");
}
```

The default theme draws badges in `plugins/themes/default/theme/components/slot-badges.slint`:

```slint
for item in root.items : Rectangle {
    accessible-role: text;
    accessible-label: item.label;
    accessible-value: item.text;
    // ...
    Text {
        text: root.short && item.short != "" ? item.short : item.text;
        // ...
    }
}
```

The slot names a plugin can fill are in `SlotId` (`crates/nexus-api/src/slots.rs`). A theme
does not name them. It reads the lists in `Vm`.

## Tokens and options

A token is a named design value. `Tokens.<name>` holds it in Slint. The list is `TOKENS` in
`crates/nexus-ui-api/src/tokens.rs`. Every token must have a value somewhere in the chain.
A token name that is not in the list fails with `no such token`.

| Group | Tokens | Kind |
|---|---|---|
| color | `color-bg`, `color-surface`, `color-surface-raised`, `color-text`, `color-text-muted`, `color-accent`, `color-on-accent`, `color-focus`, `color-success`, `color-warning`, `color-danger`, `color-info`, `color-overlay` | Color |
| typography | `font-family` | Text |
| typography | `font-size-s`, `font-size-m`, `font-size-l`, `font-size-xl` | Length |
| typography | `font-weight-regular`, `font-weight-bold` | Int |
| spacing | `space-xs`, `space-s`, `space-m`, `space-l`, `space-xl` | Length |
| radii | `radius-s`, `radius-m`, `radius-l` | Length |
| elevation | `shadow-color` | Color |
| elevation | `shadow-blur-1`, `shadow-blur-2` | Length |
| motion | `motion-fast`, `motion-normal`, `motion-slow` | Duration |
| layout | `layout-direction` | Text, `"ltr"` or `"rtl"` |

In `theme.toml`, write each kind as follows (`TokenValue::parse`):

- Color: `"#rrggbb"` or `"#rrggbbaa"`.
- Length: a number, in logical pixels.
- Duration: a number, in milliseconds.
- Int: a whole number.
- Text: a string.

An option is a setting the player changes on the theme's page. It sets tokens. `Vm.theme-options`
lists them as fields, and `SetThemeOption` sets one (`{"key": name, "value": text}`). Option
names use lowercase letters, digits and `-`, starting with a letter.

A color option sets one color token. From
`tests/fixtures/themes/fixture-grid/theme.toml`:

```toml
[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"
```

A choice option sets a group of tokens for each choice. From
`plugins/themes/default/theme/theme.toml`:

```toml
[theme.options.direction]
label = "Layout direction"
type = "choice"
default = "ltr"

[theme.options.direction.choices.ltr]
layout-direction = "ltr"

[theme.options.direction.choices.rtl]
layout-direction = "rtl"
```

Until the player sets an option, the theme's own `[theme.tokens]` stay in force. The option's
`default` only names the value `Options` shows. Once the player sets an option, `merge_chain`
applies it on top of the tokens.

A change to an option does not recompile the theme. The host sets `Tokens` and `Options`
again on the running instance and renders.

A bad value falls back in two ways. A color option with a value that does not read as a color
leaves the token as the chain set it (the host writes a warning to the log). The `Options`
global shows the option's default for the same value. A choice option with an unknown choice
shows the default in `Options` and sets no tokens.

## Responsive layout

A breakpoint is a named window class. `[[theme.breakpoints]]` lists them in order. The first
one whose limits all hold is the current one (`Breakpoint::matches`). A breakpoint with no
limits always holds, so put it last. If none holds, the name is `""`.

| Key | Limit |
|---|---|
| `min_width`, `max_width` | The window width, in logical pixels. |
| `min_aspect`, `max_aspect` | Width divided by height. |
| `min_scale` | Physical pixels per logical pixel. |

The host sets `Vm.viewport`:

| Field | Holds |
|---|---|
| `width`, `height` | The window size, in logical pixels. |
| `scale` | The screen's pixel density. |
| `aspect` | Width over height. |
| `size` | How big the player wants everything: from `MIN_SCALE` to `MAX_SCALE` (`crates/nexus-core/src/preferences.rs`), where 1 is the theme's own size. The `NEXUS_SCALE` environment variable sets it for one run. |
| `breakpoint` | The current breakpoint's name. |

The fixture's Home screen reads the breakpoint (see
[A minimal theme](#a-minimal-theme)):

```slint
property <int> columns: Vm.viewport.breakpoint == "compact"
    ? 2
    : Math.max(3, Math.floor((self.width - 2 * Tokens.space-l + root.gap) / (root.cell + root.gap)));
```

A theme that does not multiply its sizes by `Vm.viewport.size` ignores the player's
"Size of everything". The fixture does not use it.

## Input modes and pad keys

`input` in the manifest lists the devices the theme supports, the main one first. The host
puts the first entry in `Vm.input-mode` (`pointer`, `gamepad` or `touch`). When a gamepad button
arrives, `Vm.input-mode` becomes `gamepad`.

A theme needs no gamepad code. The host turns each button into a key press on the window
(`PadButton::action` in `crates/nexus-ui-host/src/input.rs`):

| Button | Key sent |
|---|---|
| Up, Down, Left, Right | The arrow keys |
| South (A, Cross) | Return |
| East (B, Circle) | Escape |
| Start | Menu |
| Select | Delete |
| North (Triangle) | `/` |
| West (Square) | Insert |
| Left shoulder | Backtab |
| Right shoulder | Tab |
| Left trigger, right trigger | PageUp, PageDown |

Two buttons are not keys. The Guide button (the PS or Xbox button) sends `Navigate` to `Home`,
and does nothing while a game runs. The Guide button held closes the running game, or, with
no game, raises `Vm.power-menu` by one. A theme opens its power menu when that number changes.

While a game's window is in front, or the launcher's window has no focus, the other buttons do
nothing. The host still reads the Guide button, tapped or held.

While a pad key is delivered, `Vm.pad-key` is `true`. It is `false` for a keyboard key. A
theme uses it to tell the two apart. A held button repeats as a repeated key press
(`event.repeat`), so a theme can ignore it. From
`plugins/themes/default/theme/shell.slint`:

```slint
capture-key-pressed(event) => {
    Osk.pad = Vm.pad-key;
    // ...
}
```

`Vm.pad-name` is the controller's name, empty for none. A theme can show different hints from
`Vm.input-mode`. From `plugins/themes/default/theme/components/boot.slint`:

```slint
property <bool> pad: Vm.input-mode == "gamepad";
```

## Images, fonts and sounds

Declare each file in `[theme.assets]`. Paths are relative to the theme folder, use `/`, and have
no `.`, `..` or empty parts (`RelPath` in `crates/nexus-api/src/path.rs`).

```toml
[theme.assets]
fonts = ["fonts/Inter-Regular.ttf", "fonts/Inter-Bold.ttf"]

[theme.assets.images]
logo = "images/logo.png"

[theme.assets.sounds]
move = "sounds/move.wav"
```

The second and third blocks are from `plugins/themes/default/theme/theme.toml`.

**Images.** An image name becomes `Assets.<name>`. The host decodes PNG and JPEG
(`crates/nexus-ui-host/src/images.rs`) and shrinks an image that is larger than `MAX_WIDTH` by
`MAX_HEIGHT`. A missing or undecodable image fails the load with `ThemeError::Asset`. Images
for games come from the `Vm` (`GameCard.cover` and the like), not from `Assets`.

**Fonts.** The host registers each font file. It imports them in the generated `tokens.slint`.
Set `font-family` to the family name inside the files, as the fixture does with `"Inter"`. A
font path must resolve inside the theme folder (`ResolvedTheme::font_paths`). A built-in
theme has no folder, so its fonts are written once to a `launcher-fonts` folder in the system's
temp folder.

**Sounds.** A sound name is what `Host.play-sound(name)` takes. The app plays WAV and Ogg Vorbis
(`rodio` features in the workspace `Cargo.toml`). The host gives no sound when the player has
turned sounds off or the name is not declared. The theme gets no error in either case.

## Rules the host checks

`contract::lint` (`crates/nexus-ui-api/src/contract.rs`) checks each `.slint` file before
compiling. A file that breaks a rule fails the load with `ThemeError::File`, which names the
file.

- No `@image-url`. Show images from `Assets`, from the `Vm`, or from slot icons.
- Every `import` and `export ... from` names a `.slint` file. A font import is refused: list fonts
  in `theme.toml`.
- A screen file exports its component under the name in the screen table, and a component file
  under the name in the component table. The shell exports `Shell`.

Comments are ignored when the rules are checked. Strings are not.

The compiler adds one rule (`crates/nexus-ui-host/src/compile.rs`): an import that leaves
`VFS_ROOT` fails the compile with `outside the theme`.

A compile error fails the load with `ThemeError::Compile`. Its text lists each error as
`file:line:column: message`.

## Dev mode

`--dev` makes the app shell check the theme's files on every `TICK`
(`crates/nexus-ui-host/src/shell.rs`). When the newest change time in the theme's folders
differs from the last load (`ResolvedTheme::stamp`), the host loads the theme again
(`ThemeHost::reload_if_changed`). It reads every folder in the chain that is on disk.

If the new files do not load, the last good version stays on screen. The error goes to the log,
and the host does not try again until a file changes.

Dev mode reloads the markup, images, fonts and sounds. It does not read `theme.toml` again.
Restart the app after you edit the manifest. See [Limitations](#limitations).

## Switching and rollback

The player switches theme on the Themes screen. A theme can do the same with
`Host.switch-theme(id)`, as the default theme does in
`plugins/themes/default/theme/screens/plugin.slint`:

```slint
if (!Vm.plugin-page.current) { Host.switch-theme(Vm.plugin-page.plugin.id); }
```

The host queues the request and carries it out after the callback returns, because a switch drops
the instance that runs the callback.

`ThemeHost::switch` does these steps in order (`load` in `host.rs`):

1. Resolve the theme and its chain, with the player's option values.
2. Decode the images and read the sounds.
3. Compile the markup.
4. Create the new instance in the same window, set the tokens, the options and the images,
   and bind the current view.
5. Save the choice and announce `ThemeChanged` (`Core::theme_switched`).
6. Show the new instance, then drop the old one.

A failure in steps 1 to 5 leaves the old theme on screen. A failure to save the choice does the
same. The core does not announce `ThemeChanged` in either case. The core keeps the navigation, selection, downloads and
running games, because they are not in the theme.

At start, the host loads `--theme`, else the saved theme, else `default`. If that theme fails, it
logs the error and loads `default`. It does not overwrite the saved choice. In safe mode it
loads `default` and refuses every other theme.

A failed switch from `Host.switch-theme` is written to the log
(`the theme's request failed`). I found no message to the player. See
[Limitations](#limitations).

## Testing a theme

Tests drive a theme the way a player does. `ThemeDriver` in
`crates/nexus-testkit/src/driver.rs` is the theme driver. It uses the accessible labels and
values of elements, keys, clicks and Slint's test clock. It never reads the `Vm` globals or calls
the theme's callbacks. That is why slot items need `accessible-label`.

The main steps:

| Step | Does |
|---|---|
| `count(label)`, `first(label)` | Count the elements with a label, or take the first. |
| `value(label)`, `values(label)` | Read the accessible value. |
| `focused(label)`, `lit(label)` | Which elements show as selected. |
| `press(label, value)`, `press_first(label)` | Run an element's default action. |
| `click(label)` | A mouse click in the middle of the element. |
| `navigate(key)`, `tap(key)` | Send a key. `navigate` carries out what the theme asked for. |
| `type_text(text)` | Type characters on a keyboard. |
| `settle()`, `wait(duration)` | Move Slint's clock on. |

The seam tests are in `tests/tests/seams/` (package `nexus-tests`, test binary `seams`). The
tests that matter for a theme:

- `themes.rs`: loading, inheritance and the rules. `write_theme` and `composed` in
  `tests/tests/seams/common/mod.rs` write a theme into a temporary folder and boot the app on
  it. `fixture_themes()` points at `tests/fixtures/themes`.
- `switching.rs`: switching, rollback, safe mode, live options and dev reload.
- `slots.rs`: slot items in the fixture and in the default theme, and the compact breakpoint.
- `gamepad.rs` and `sounds.rs`: the pad keys and `play-sound`.
- `system_theme.rs`: the default theme's machine settings over a fake system plugin.
- `default_theme/`: the default theme, one module for each screen. `theme_files.rs` checks
  that every font the theme names ships with it and that every animation time goes through
  `Look.timed`.

Run the theme tests:

```sh
cargo test -p nexus-tests --test seams themes::
```

`make check` runs formatting, clippy and every test, as CI does ([development.md](development.md)).

## Limitations

- `navigation` is read and merged into `MergedTheme`, but no host code reads it. Setting it to
  `pointer` or `focus` changes nothing.
- Only the first `input` entry is read. The host never sets `touch`. After the first pad button,
  `Vm.input-mode` stays `gamepad` until the app restarts.
- The host does not check that slot items set `accessible-label`. `contract.rs` lists it among
  the rules, but `lint` does not test for it.
- Dev mode does not read `theme.toml` again. The manifest, which includes tokens, breakpoints
  and the asset list, is read once at boot. Restart to see a manifest edit.
- Dev mode never reloads the default theme. It is built in and has no folder on disk.
- A failed `Host.switch-theme` is logged and is not shown to the player.
- A theme cannot add screens. The set of screens is `ScreenId`, and `screens` can only name
  those.
- A theme that extends a theme that is not installed never reaches `ThemeSet`. The core
  rejects it at boot, so `ThemeError::MissingParent` does not appear through the normal path.
- The `Options` global has no sample here, because no file allowed as a sample reads it.
