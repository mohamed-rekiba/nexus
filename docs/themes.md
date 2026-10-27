# Writing a theme

A theme is a folder of data: `theme.toml`, Slint markup and asset files. It owns the whole
presentation, from layout and navigation to components, colors and motion. It has no code: the
host compiles its markup with the Slint interpreter, binds the core's view models to it, and
carries out what it asks for. Switching themes keeps the screen, the selection, downloads and
running games.

The default theme, `plugins/themes/default`, has every screen, component and token, and every
other theme extends it. Its behaviour, screen by screen, is in
[docs/reference/default-theme.md](reference/default-theme.md). The design is in
[docs/architecture.md](architecture.md), and code plugins are in [docs/plugins.md](plugins.md).

## Contents

- [Where themes live](#where-themes-live)
- [A minimal theme](#a-minimal-theme)
- [The manifest](#the-manifest)
- [Inheritance](#inheritance)
- [The contract](#the-contract)
- [Tokens and options](#tokens-and-options)
- [Responsive layout](#responsive-layout)
- [Input modes and pad keys](#input-modes-and-pad-keys)
- [Images](#images)
- [Fonts and sounds](#fonts-and-sounds)
- [Rules the host checks](#rules-the-host-checks)
- [Dev mode](#dev-mode)
- [Testing a theme](#testing-a-theme)

## Where themes live

The host finds a theme in any folder it discovers: each subfolder with a `theme.toml` is one
theme (`KernelBuilder::discover` in `crates/nexus-core/src/kernel.rs`). The app discovers:

- the folder given with `--extensions <folder>`, by default `<data-dir>/extensions`;
- `extensions/` next to the binary, which the release package ships empty.

`<data-dir>` is `~/.local/share/nexus` on Linux, `~/Library/Application Support/nexus` on
macOS and `%APPDATA%\nexus` on Windows, or `--data-dir`. A theme from a folder is third-party
and must declare `[runtime] type = "data"`. `--safe-mode` loads none of them and shows the
default theme.

The default theme is different: it is a built-in crate whose `build.rs` embeds every file under
`plugins/themes/default/theme/` into the binary (`theme_default::DefaultTheme`).

## A minimal theme

The test theme `tests/fixtures/themes/fixture-grid` is a complete example. It replaces Home with
a grid, replaces the card, ships its own `shell.slint`, and takes everything else from
`default`. Its `theme.toml`, trimmed:

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
color-accent = "#0f9d8a"
font-family = "Inter"
radius-m = 10

[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"
```

Its folder:

```text
fixture-grid/
  theme.toml
  shell.slint
  screens/home.slint
  components/card.slint
  fonts/Inter-Regular.ttf
  fonts/Inter-Bold.ttf
```

Its Home screen (`screens/home.slint`), shortened:

```slint
import { Vm, Host, Json } from "../types.slint";
import { Tokens } from "../tokens.slint";
import { Card } from "../components/card.slint";

export component HomeScreen inherits Rectangle {
    property <int> columns: Vm.viewport.breakpoint == "compact" ? 2 : 4;
    Flickable {
        for game[i] in Vm.home.games : Card {
            x: Math.mod(i, root.columns) * 184px;
            y: Math.floor(i / root.columns) * 284px;
            game: game;
            activated => { Host.command("OpenGame", Json.text(game.key)); }
        }
    }
}
```

To try a theme in the app:

```sh
cargo run -p nexus-app -- --extensions path/to/folder --theme fixture-grid --dev --windowed
```

`--theme <id>` shows that theme for this run without saving it. Otherwise pick it under Themes.

## The manifest

`theme.toml` is a plugin manifest (`nexus_api::PluginManifest`, see
[docs/plugins.md](plugins.md#the-manifest)) with `kind = "theme"`, `capabilities = ["theme"]`
and a `[theme]` section. The section parses into `ThemeSection`
(`crates/nexus-ui-api/src/theme.rs`); unknown keys are errors.

| Key | Meaning |
|---|---|
| `ui_api` | The `nexus-ui-api` versions the theme works with, a semver range. Every theme in the chain must match `UI_API_VERSION`, which changes by hand with the contract, not with Nexus's releases. |
| `extends` | The parent theme's id. What the theme leaves out comes from the parent. |
| `screens` | The `ScreenId`s this theme defines itself, as kebab-case names. Each needs `screens/<name>.slint`. |
| `components` | The `ComponentId`s this theme defines itself. Each needs `components/<name>.slint`. |
| `input` | `pointer`, `gamepad`, `touch`: the input devices it supports, the main one first. |
| `navigation` | `pointer` (point and click) or `focus` (one focused element, moved with keys or a pad). It describes the theme; the theme's own markup implements it. |
| `tokens` | Values for design tokens, by name. |
| `breakpoints` | `[[theme.breakpoints]]`, see [Responsive layout](#responsive-layout). Replaces the parent's list when not empty. |
| `assets` | `images`, `fonts` and `sounds`, see below. |
| `options` | Settings the player can change, which set tokens. |

The screens (`ScreenId`, in `crates/nexus-ui-api/src/vm.rs`) and the components each file must
export (`crates/nexus-ui-api/src/contract.rs`):

| Screen | File | Component |
|---|---|---|
| `home` (Home and Library) | `screens/home.slint` | `HomeScreen` |
| `game-detail` | `screens/game-detail.slint` | `GameDetailScreen` |
| `catalog` (Catalog and Search) | `screens/catalog.slint` | `CatalogScreen` |
| `downloads` | `screens/downloads.slint` | `DownloadsScreen` |
| `settings` | `screens/settings.slint` | `SettingsScreen` |
| `emulators` | `screens/emulators.slint` | `EmulatorsScreen` |
| `themes` | `screens/themes.slint` | `ThemesScreen` |
| `plugin` | `screens/plugin.slint` | `PluginScreen` |
| `game-settings` | `screens/game-settings.slint` | `GameSettingsScreen` |
| `running` | `screens/running.slint` | `RunningScreen` |

| Component | File | Component name |
|---|---|---|
| `card` | `components/card.slint` | `Card` |
| `button` | `components/button.slint` | `Button` |
| `list-item` | `components/list-item.slint` | `ListItem` |
| `progress-bar` | `components/progress-bar.slint` | `ProgressBar` |
| `dialog` | `components/dialog.slint` | `Dialog` |
| `slot-badges` | `components/slot-badges.slint` | `SlotBadges` |
| `slot-actions` | `components/slot-actions.slint` | `SlotActions` |
| `slot-sections` | `components/slot-sections.slint` | `SlotSections` |

`shell.slint` exports `Shell`, which inherits `Window`: the window, and which screen shows for
`Vm.screen`. A theme may leave it out when a parent has one.

## Inheritance

`extends` builds a chain from the theme up to the root, usually `default`
(`ThemeSet::chain` in `crates/nexus-ui-host/src/themes.rs`). A missing parent or a cycle refuses
the theme. The chain is resolved once, at load:

- **Files.** Every `.slint` import goes through the interpreter's file loader, which serves one
  virtual folder. Each path is looked up in the theme first, then in each parent. So a theme that
  ships only `components/card.slint` gets its card on every screen of `default` that draws a
  `Card`. `tests/tests/seams/themes.rs` (`a_child_theme_falls_back_to_its_parent_for_what_it_leaves_out`)
  checks exactly that.
- **Tokens, images, fonts, sounds and options** merge, the child winning. Every token must have
  a value somewhere in the chain.
- **Breakpoints** come from the nearest theme that lists any; **input** likewise; **navigation**
  from the nearest theme that sets it.

Because a parent's screens use a child's components, a component's interface is a contract.
The default screens pass a `Card` only `game` and `cover-height`, and handle `activated`, so a
child's card declares those and may have nothing else. Keep the same properties and callbacks
when you replace a component.

## The contract

The host serves four files in every theme. Import them; never ship a file with these names
(`RESERVED_FILES`):

| File | Global | What it holds |
|---|---|---|
| `types.slint` | `Vm`, `Host`, `Json`, and the structs | The data to show, the callbacks, and the helpers that build a command's args. Fixed: `crates/nexus-ui-api/contract/types.slint`. |
| `tokens.slint` | `Tokens` | One property per design token, with the merged values. |
| `assets.slint` | `Assets` | One `image` per name in `[theme.assets.images]`, and `loaded`. |
| `options.slint` | `Options` | One property per theme option, holding its value. |

Import with a path relative to the importing file: `"types.slint"` from `shell.slint`,
`"../types.slint"` from a screen or component.

`crates/nexus-ui-api/contract/types.slint` is the reference. Every field has a comment there;
read it before writing a screen. This section gives the shape.

### Vm: what to show

`Vm` is a global of `in-out` properties. The host sets them after every change; themes only read
them. They are the core's view models, `AppView` in `crates/nexus-ui-api/src/vm.rs`, field for
field with the names kebab-cased: one converter turns them into Slint values
(`crates/nexus-ui-host/src/values.rs`), and a drift test fails when `types.slint` and the view
models name other fields. An optional part comes with a `has-` flag, such as `has-detail`. The
main groups:

| Properties | What they are |
|---|---|
| `screen`, `game`, `can-go-back` | The current screen id, the selected game's key, whether Back goes anywhere |
| `home` (`Home`) | The games, the selection, `loading`, the games by platform (`groups: [Group]`), and the Library (`library: Library`) |
| `has-detail`, `detail` (`GameDetail`) | The open game: text, art, slots, compatibility, releases, "This PC", and what it can do |
| `catalog` (`Library`), `catalog-sources`, `catalog-releases`, `catalog-updated`, `catalog-loading`, `catalog-available`, `catalog-actions`, `catalog-status` | The games the catalogs offer, and the catalog plugins' state and buttons |
| `downloads` (`[Download]`) | Emulator build downloads |
| `torrents` (`[Torrent]`), `seed-downloads`, `torrents-downloading`, `torrents-sharing`, `vpn` | The catalogs' downloads, and "Download only through a VPN" |
| `settings` (`[SettingsPanel]`), `folders` (`[Folder]`), `sounds`, `fullscreen`, `return-on-exit`, `auto-update`, `scale`, `can-start-at-login`, `start-at-login`, `start-at-login-error`, `monitors`, `monitor` | Settings |
| `themes` (`[ThemeChoice]`), `theme-options` (`[Field]`) | The themes, and the current theme's options as fields |
| `emulators`, `plugins`, `emulator-plugins`, `theme-plugins`, `has-plugin-page`, `plugin-page` | The plugin lists and the open plugin's page |
| `boot`, `art-ready` | The boot and Welcome screens |
| `has-running`, `running` | The running game |
| `has-install-prompt`, `install-prompt` | "Install the emulator first?" before a first play |
| `has-result-prompt`, `result-prompt` | "How far did it get?" after a game closes |
| `notices`, `notice-cards`, `notice-art`, `notice-opens` | Messages for the player, newest last, at most 3 |
| `hardware`, `launcher-update`, `task`, `clock` | This PC, the launcher's update, background work, the time |
| `can-play-trailers`, `has-trailer`, `trailer`, `trailer-frame` | The trailer player |
| `viewport` (`Viewport`) | Window size, scale, size of everything, aspect, breakpoint |
| `input-mode`, `pad-name` | `pointer`, `gamepad` or `touch`; the controller's name |
| `system` (`System`) | The machine: `whole-screen`, `nexus-os`, `stick`; the network, Wi-Fi list, sound, Bluetooth and devices, battery, power, Nexus OS's update, the stick's disks, and running jobs. Each part has `*-available` and a `*-reason`. The `System` commands change it. `Host.log-out` ends the Nexus session on a desktop |
| `pad-key` | True while a controller's button is delivered as a key: a keyboard's keys leave it false |
| `power-menu` | Goes up by one each time the PS or Xbox button is held with no game running (UI API 0.1.1): a theme opens its power menu when it changes |

Recent additions to know about:

- `Vm.catalog-available` is true while a plugin that gives release catalogs is turned on.
  Without one there is no Catalog: the core refuses to open it, and a theme hides the ways to
  it. The default theme hides Search, and its shoulder buttons skip the Catalog.
- **A game's actions**, `GameCard.can` and `GameDetail.can` (`GameActions`), are worked out by
  the core, the same for every game of every plugin, Library or Catalog: draw them, don't work
  them out. `primary` is the main action: `play` when it is installed (the command `Session` `Launch`;
  a Catalog game starts the installed game it is), else `get` (its label in `get`: "Download"
  opens its Game Hub's releases; with a `get-store`, "Install in Steam",
  the command `Torrent` `Download` of `get-release` asks the store). `uninstall` is the key for
  the command `UninstallGame`, after asking; `uninstall-store` names the store that then asks its
  own way. Uninstall is never the main action: the default theme puts it in the game's menu.
- `GameCard.system-name` and `system-color` (and `GameDetail`'s) are the game's system as its
  plugin describes it (`[system]` in the plugin's manifest): its full name, "PlayStation 5",
  and its color. The color is transparent when the plugin gives none: draw the neutral look.
  Draw systems from these, never from `platform`'s code, so a new plugin's games need no
  change to the theme. The default theme makes the badge's fill, edge and text, and the art of
  a game without a cover, from the one color.
- `Torrent.store` names the store doing an install ("Steam"): its card is the store's install
  of a game, with the state `queued`, `downloading` or `paused` (or `install` `installing`),
  one action, `open-store` (the store's own Downloads page), and no folder. Empty for a torrent.
- `Release.store` names the store that installs a release ("Steam"); empty for a torrent. Its
  Download asks the store, with no download sheet: send `Torrent` `Download` with no folder,
  and stay on the Game Hub. The default theme's button says "Install in Steam".
- `GameCard.hidden` and `GameDetail.hidden` say the player hid a Catalog game. Send
  `Library` `Hide` to hide one or show it again. A hidden game leaves the Catalog,
  but for the systems row's "Hidden" filter (id `hidden`), which shows only hidden games. The
  default theme puts an eye on the Catalog's cards and in the Game Hub, and an item in the
  game's menu.

### Host: what to ask for

`Host` is a global of callbacks. One, `command(name, args)`, asks the core; the others are the
app's own and never reach it.

`name` is a command of the core's `Command` enum (`crates/nexus-core/src/app.rs`), and `args` its
payload as JSON, `""` for a command with none. The host reads the two with `serde`, as
`{name: args}`, so a command is declared once, in the core. A command whose args don't read is
dropped, with a warning in the log. Build the args with the `Json` global: `Json.text(s)` is a
JSON string, escaped; `Json.text-or-null(s)` is `null` for `""`; `Json.flag(b)` is `true` or
`false`. Text fields that hold JSON (`Slot.args`, `Slot.scope`) go in as they are.

```slint
Host.command("Back", "");
Host.command("Select", Json.text(game.key));
Host.command("Session", "{\"Launch\":" + Json.text(game.key) + "}");
Host.command("Pref", "[\"sounds\"," + Json.flag(on) + "]");
```

The default theme puts its commands in one global, `Commands` in `commands.slint`, one function
per thing the player does. Copy it to start a theme.

The commands, by name. A key is a game's key; `?` marks a value that may be `null`.

| Name | Args | What it does |
|---|---|---|
| `Navigate` | `"Home"`, `"Catalog"`, `"Downloads"`, `"Settings"`, `"Emulators"`, `"Themes"`; `{"GameDetail": key}`, `{"Plugin": id}`, `{"GameSettings": key}` | Opens a screen |
| `Back` | none | Goes back |
| `Select`, `OpenGame` | key | Selects a game; opens its Game Hub |
| `LibrarySearch`, `LibrarySort` | text | The Library's search and sort (`title`, `played`, `size`, `smallest`, `rating`, `release`, `compat`, `added`) |
| `LibrarySystem`, `LibraryGenre` | text? | A filter's id; `null` for All |
| `LibraryMore` | none | The next page |
| `CatalogSearch`, `CatalogSort`, `CatalogSystem`, `CatalogGenre`, `CatalogMore` | as the Library's | The Catalog's |
| `GridColumns` | `{"grid": "Library" or "Catalog", "columns": n}` | How many columns a grid shows |
| `Library` | `"Refresh"`; `{"Hide": {"game": key, "hidden": bool}}` | Reads the games again; hides a Catalog game or shows it again |
| `Session` | `{"Launch": key}`, `{"Stop": session}`, `{"ShowGame": session}` | Plays, stops, and brings a game's window back |
| `ConfirmInstall`, `CancelInstall` | none | Answers "Install the emulator first?" |
| `AskResult` | key | Asks "How far did it get?" |
| `ReportResult` | text? | `ingame`, `menus`, `boots` or `nothing`; `null` for Not now |
| `UninstallGame` | key | Deletes an installed game |
| `Torrent` | `{"Download": {"offer": key, "release": id, "folder": text?}}`; `{"Act": {"id": id, "action": action}}`; `{"SeedDownloads": bool}` | The catalogs' downloads. A blank folder is the one the sheet offers. See [Torrent actions](#torrent-actions). |
| `GameInstall` | `{"Download": {"id": id, "folder": text?}}`, `{"RemoveUnpacked": {"id": id}}`, `{"Cancel": {"id": id}}` | Installs a finished download, deletes its unpacked archives, stops its install |
| `PauseDownload`, `ResumeDownload`, `CancelDownload` | a job's `id` | Build downloads |
| `InstallEmulator` | `{"plugin": id, "version": text?}` | `null` installs the newest |
| `UpdateEmulator` | `{"plugin": id}` | |
| `UninstallEmulator` | `{"plugin": id, "version": text}` | |
| `RunAction` | `{"plugin": id, "action": text, "game": key?, "args": Slot.args or null}` | A slot's action button |
| `SetConfigField` | `{"plugin": id, "scope": Slot.scope, "key": text, "value": text}` | A settings field |
| `SetPluginEnabled` | `{"plugin": id, "enabled": bool}` | Turns a plugin on or off |
| `SetThemeOption` | `{"key": text, "value": text}` | The current theme's option |
| `SetFolders` | `{"name": text, "paths": text}` | A named folder: its folders as typed, separated by `;`; `""` for the default |
| `Pref` | `[name, value]` | A preference: `sounds`, `fullscreen`, `return_on_exit`, `auto_update`, `start_at_login` (bool), `scale` (0.8 to 1.4), `monitor` (text, `""` for the one it is on), `welcomed` (true: the Welcome screen's Start) |
| `Vpn` | `{"SetOnly": bool}`, `{"Choose": name}`, `{"Connect": name}`, `{"Disconnect": name}`, `{"Import": path}` | "Download only through a VPN", and the VPNs |
| `Notice` | `"DismissAll"`, `{"Dismiss": id}`, `{"Open": id}` | Notices |
| `LauncherUpdate` | `"Check"`, `"Download"` | The launcher's own update |
| `System` | `"Refresh"`, `"ScanWifi"`, `{"JoinWifi": {"ssid": text, "password": text?}}`, `{"ForgetWifi": ssid}`, `{"SetBluetooth": bool}`, `"ScanBluetooth"`, `{"Pair": address}`, `{"ConnectDevice": address}`, `{"DisconnectDevice": address}`, `{"RemoveDevice": address}`, `{"SetOutput": name}`, `{"SetVolume": 0 to 100}`, `{"SetMuted": bool}`, `"Restart"`, `"ShutDown"`, `"CheckOsUpdate"`, `{"InstallOs": path}`, `"ChoosePartitions"`, `{"CancelJob": id}` | The machine (`Vm.system`) |

The app's own callbacks:

| Callbacks | Use |
|---|---|
| `grid-window(grid, first, last)` | Which cards are on screen, for images; see [Images](#images) |
| `switch-theme(id)` | Shows another theme |
| `play-sound(name)` | A sound from `[theme.assets.sounds]` |
| `open-url(url)`, `open-game-folder(key)`, `open-game-log(key)`, `open-download-folder(id)` | Opens a page, a folder or a log with the system's opener |
| `play-trailer(url)`, `pause-trailer()`, `seek-trailer(seconds)`, `close-trailer()` | The trailer player |
| `quit()`, `restart()`, `log-out()` | Closes the launcher; starts it again; ends the Nexus session |

### Torrent actions

`Torrent.actions` lists, in order, what the player can do with a catalog download now. Most
are a `TorrentAction`: send `Torrent` `{"Act": {"id": id, "action": name}}` with its variant
name, `Start`, `Pause`, `Cancel`, `Remove`, `DeleteFiles`, `StopSeeding` or `OpenStore`. The
others are game installs. The default theme's `Commands.torrent-action` does the mapping:

| Action | Effect |
|---|---|
| `start` | Download it (ready, paused, failed or cancelled) |
| `pause` | Pause it |
| `cancel` | Stop it; the files stay. While its archives unpack, stop the unpacking. |
| `remove` | Forget a download that isn't running; the files stay |
| `stop-seeding` | Stop sharing the finished files |
| `install` | `GameInstall` `Download`: hand the finished download to its emulator's game installer, into the folder the player picks on a sheet, or the one it offers |
| `play` | Not a torrent action: `Session` `Launch` of `torrent.game` |
| `remove-unpacked` | `GameInstall` `RemoveUnpacked`: delete the unpacked archives; the downloaded files stay |
| `open-store` | A store's install: open the store's own Downloads page |

Two more actions are accepted but never listed:

- `delete-files`: forget the download and delete its folder, the downloaded files and their
  unpacked copies. An installed game is elsewhere and stays. Offer it beside `remove`: the
  default theme's remove sheet asks "keep the files" (`remove`) or "delete them"
  (`delete-files`).
- `cancel-install`: `GameInstall` `Cancel`, stop the download's install; what it wrote goes.

### Slots

Plugins add badges, buttons, sections and settings panels as data (see
[docs/plugins.md](plugins.md#slots)). They reach a theme as `Slot` structs in `GameCard.badges`,
`GameDetail.badges`, `actions` and `sections`, `SettingsPanel.slot`, `PluginPage.sections` and
`actions`, `Emulator.panels`, and `Vm.catalog-actions` and `catalog-status`.

`Slot.kind` is `badge`, `action`, `section` or `settings_panel`; `tone` is `neutral`, `info`,
`success`, `warning` or `danger`; `style` is `primary` or `secondary`. `label` is
`<plugin-id>:<item-id>`. Render every slot item with that label:

```slint
for item in root.items : Rectangle {
    accessible-role: text;
    accessible-label: item.label;
    accessible-value: item.text;
    // ...
}
```

A button sends `RunAction` with `item.plugin`, `item.action`, the game's key and `item.args`. A
settings panel draws its `fields` (`Field`: `kind` is `toggle`, `number`, `text` or `choice`,
with `choices` and their `labels`) and saves each with `SetConfigField`, with `item.plugin`,
`item.scope`, `field.key` and the value.

## Tokens and options

`Tokens` has one property per design token (`TOKENS` in `crates/nexus-ui-api/src/tokens.rs`).
Set them in `[theme.tokens]`:

| Group | Tokens | TOML value |
|---|---|---|
| color | `color-bg`, `color-surface`, `color-surface-raised`, `color-text`, `color-text-muted`, `color-accent`, `color-on-accent`, `color-focus`, `color-success`, `color-warning`, `color-danger`, `color-info`, `color-overlay` | `"#rrggbb"` or `"#rrggbbaa"` |
| typography | `font-family` | text |
| | `font-size-s`, `font-size-m`, `font-size-l`, `font-size-xl` | logical pixels |
| | `font-weight-regular`, `font-weight-bold` | whole number |
| spacing | `space-xs`, `space-s`, `space-m`, `space-l`, `space-xl` | logical pixels |
| radii | `radius-s`, `radius-m`, `radius-l` | logical pixels |
| elevation | `shadow-color` | color |
| | `shadow-blur-1`, `shadow-blur-2` | logical pixels |
| motion | `motion-fast`, `motion-normal`, `motion-slow` | milliseconds |
| layout | `layout-direction` | `"ltr"` or `"rtl"` |

An unknown token name or a value of the wrong kind refuses the theme.

Options are settings the player changes in the theme's page; they set tokens without a
recompile. Two kinds (`OptionKind`):

```toml
# A color option sets one color token.
[theme.options.accent]
label = "Accent color"
type = "color"
token = "color-accent"
default = "#0f9d8a"

# A choice sets a group of tokens per choice. From the default theme:
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

Option names are lowercase letters, digits and `-`, starting with a letter. The current value of
each is on the `Options` global (`options.slint`): a choice as its name (`string`), a color as a
`color`. An unset option, or an unknown choice, reads as its default. Use `Options` for markup
that depends on a choice, not only on tokens:

```slint
import { Options } from "../options.slint";
Text { text: Options.motion; }
```

`Vm.theme-options` lists the options as `Field`s for a settings form, and the command
`SetThemeOption` changes one.

## Responsive layout

`Vm.viewport` is a `Viewport`: `width` and `height` (logical), `scale` (the screen's pixel
density), `size` (the player's "Size of everything", 0.8 to 1.4, or `NEXUS_SCALE` for a run),
`aspect` and `breakpoint`.

`breakpoint` is the name of the first `[[theme.breakpoints]]` entry whose limits all hold. An
entry has `name` and any of `min_width`, `max_width` (logical pixels), `min_aspect`,
`max_aspect` (width over height) and `min_scale`. An entry with no limits matches everything, so
put it last as the fallback. With no match, `breakpoint` is empty.

The host updates the viewport when the window changes size. The default theme lays everything out
on a 1920 by 1080 canvas and scales it to the window.

## Input modes and pad keys

`[theme] input` lists the devices the theme supports. `Vm.input-mode` starts as the first one
and becomes `gamepad` when a controller button is pressed. Nothing in the host sets `touch`
today, or sets `pointer` again after a pad press.

A theme needs no gamepad code. The app reads controllers with `gilrs`, and the UI host turns each
button into a key press the theme already handles (`PadButton::action` in
`crates/nexus-ui-host/src/input.rs`). A keyboard does the same as a pad:

| Pad (PlayStation / Xbox) | `PadButton` | Key the theme gets |
|---|---|---|
| D-pad, left stick | `Up`, `Down`, `Left`, `Right` | `Key.UpArrow`, `Key.DownArrow`, `Key.LeftArrow`, `Key.RightArrow` |
| Cross / A | `South` | `Key.Return` |
| Circle / B | `East` | `Key.Escape` |
| Triangle / Y | `North` | `"/"` |
| Square / X | `West` | `Key.Insert` |
| Options / Menu | `Start` | `Key.Menu` |
| Create, Share / View | `Select` | `Key.Delete` |
| R1 / RB | `RightShoulder` | `Key.Tab` |
| L1 / LB | `LeftShoulder` | `Key.Backtab` |
| L2 / LT | `LeftTrigger` | `Key.PageUp` |
| R2 / RT | `RightTrigger` | `Key.PageDown` |
| PS / Xbox (Logitech's too) | `Guide` | A tap, no key: the host opens Home; while a game runs, a tap does nothing |
| PS / Xbox, held 600 ms | `GuideHeld` | No key: while a game runs, the host closes it at once. Else `Vm.power-menu` goes up by one, and the default theme opens its power menu |

A held button or stick repeats after 380 ms, then every 85 ms, as a key press with
`event.repeat` set, so a theme can ignore repeats where a second press would do harm. The default
theme ignores a repeated Return. While a game's window is in front, or the launcher's window
doesn't have the focus, only the PS button reaches the launcher. The PS button never repeats:
the host waits for it to go up (a tap) or to be held 600 ms (close the game, or the power menu).

Handle the keys in a `FocusScope`:

```slint
keys := FocusScope {
    key-pressed(event) => {
        if (event.repeat && event.text == Key.Return) { return accept; }
        if (event.text == Key.Escape) { Host.command("Back", ""); return accept; }
        reject
    }
}
```

## Images

Markup never names an image file. Images reach a theme three ways:

- **The theme's own images**: `[theme.assets.images]` maps a name to a file in the theme
  folder, PNG or JPEG. Names are lowercase letters, digits and `-`. Each is an `image` property
  on `Assets`: `Image { source: Assets.logo; }`. `Assets.loaded` is true once they are set.
- **Art from the view models**: `GameCard.cover`, `GameDetail.cover`, `background`, `logo`,
  `screenshots`, `Boot.covers`, `Notice.art`, slot icons. The host loads them in the background
  and sets them when they are ready.
- **Slot icons** that plugins give.

Covers load only near the screen. A grid tells the host which cards it shows with
`Host.grid-window(grid, first, last)`, where `grid` is `library` or `catalog` and `first` and
`last` are card indices. Cards within one screen of that window load; cards further away ask for
nothing and show empty. Before a grid reports, the host assumes cards 0 to 29.
The command `GridColumns` tells the core how many columns the grid shows, so it fetches
the art of the games above and below the selection. The default theme's
`components/library-view.slint` reports both whenever it scrolls or resizes:

```slint
function report() {
    if (root.active) {
        Host.grid-window(Nav.catalog ? "catalog" : "library", grid.first-row * grid.columns,
            (grid.first-row + grid.screen-rows) * grid.columns - 1);
    }
}
```

`GameCard.cover-state` is `loading` while a cover is on its way, `ready` once it shows, and
`none` when the game has none or it failed: show a placeholder for `loading` and `none`.

Card art is made at most 512 by 768 pixels, and other art at most 1920 by 1080
(`crates/nexus-ui-host/src/images.rs`).

## Fonts and sounds

**Fonts**: list TrueType or OpenType files in `[theme.assets] fonts = [...]`, and name the
family with the `font-family` token. The host imports them through `tokens.slint`, so every
file that imports `Tokens` has them. A theme may not import a font itself.

**Sounds**: `[theme.assets.sounds]` maps a name to a sound file, WAV or Ogg Vorbis (what the
app's `rodio` build decodes). `Host.play-sound(name)` plays one, unless the player turned
sounds off. A missing file fails the load, so the old theme stays. The default theme has
`move`, `select`, `back`, `error` and `start`; `move` plays at most once every 40 ms, so a held
key doesn't stack clicks.

```toml
[theme.assets.sounds]
move = "sounds/move.wav"
select = "sounds/select.wav"
```

## Rules the host checks

Before compiling, the host refuses a theme (`contract::lint` and `ResolvedTheme::check`) that:

- ships `types.slint`, `tokens.slint`, `assets.slint` or `options.slint`;
- uses `@image-url` anywhere;
- imports anything but a `.slint` file (a font import, for example);
- declares a screen or component in `theme.toml` and doesn't ship its file;
- has a screen or component file that doesn't export the component named for it;
- has no `shell.slint` anywhere in its chain;
- imports a path outside the theme's virtual folder.

Each error names the theme and the file. Compile errors list each file, line and column. When a
theme fails, the old theme stays on screen; at start, the app falls back to `default` and keeps
the saved choice.

## Dev mode

With `--dev`, the app checks the current theme's files four times a second and calls
`ThemeHost::reload_if_changed`. When a file in a theme folder on disk has a newer modification
time, the host loads the theme again: validate, compile, swap. A broken edit logs the error and
keeps the last good version on screen, and isn't retried until the next change.

Only themes loaded from a folder reload. The default theme is embedded in the binary, so an
edit to `plugins/themes/default/theme/` needs a rebuild (`cargo run -p nexus-app`). To work on
a look quickly, make a child theme in an extensions folder and run it with `--dev`.

## Testing a theme

The theme tests run the core and the theme host together on Slint's testing backend, with no
window (`tests/tests/seams/common/mod.rs`):

- `composed(|host| ...)` boots a `FakeHost` with the default theme and what the closure adds
  (stub plugins, `discover(dir)` for theme folders), and a `ThemeHost` on it.
- `write_theme(parent, id, theme_section, files)` writes a data-only theme folder for a test.
- `fixture_themes()` is `tests/fixtures/themes`, holding `fixture-grid`.
- `host.ui.start()` shows the saved theme, `host.ui.switch(&id)` another one.
- `host.dispatch(Command::...)` drives the core and renders; `host.pump()` carries out what the
  theme asked through `Host`; `settle(&mut host)` lets animations finish.
- `host.count(label)` and `host.first(label)` find rendered elements by accessible label.

Accessible labels are the test seam. Give the elements a test needs to find an
`accessible-label`, and an `accessible-value` for what it shows, and an
`accessible-action-default` for what a click does. Slot items carry `<plugin-id>:<item-id>`. The
default theme's tests in `tests/tests/seams/default_theme.rs` find, for example, the top bar's tabs:

```rust
fn press(host: &mut Composed, label: &str, value: &str) {
    let found = ElementHandle::find_by_accessible_label(host.instance(), label);
    let target = found
        .into_iter()
        .find(|e| e.accessible_value().as_deref() == Some(value))
        .unwrap_or_else(|| panic!("no {label} {value}"));
    target.invoke_accessible_default_action();
    host.pump();
    settle(host);
}

press(host, "default-system", "Library");
```

`nexus_ui_host::vm_text(instance, "screen")` reads a `Vm` property as text.

The theme test files:

| File | What it checks |
|---|---|
| `tests/tests/seams/themes.rs` | The default theme compiles and renders every screen; inheritance; the rules above; options |
| `tests/tests/seams/switching.rs` | Switching, rollback on a broken theme, dev-mode reload |
| `tests/tests/seams/slots.rs` | Slot items in the default theme and in `fixture-grid` |
| `tests/tests/seams/default_theme.rs` | The default theme's Home, Library, filters and options |
| `tests/tests/seams/screenshots.rs` | The README's screenshots, rendered offscreen |

```sh
cargo test -p nexus-tests --test seams themes::
cargo test -p nexus-tests --test seams default_theme::
scripts/check.sh    # everything, as CI runs it
```
