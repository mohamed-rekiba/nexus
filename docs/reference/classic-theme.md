# The Classic theme

This page lists what the built-in Classic theme (`classic`) ships and draws, file by file. It is
for theme authors who want to see a complete theme, and for contributors who change it. How a
theme works in general is in [../themes.md](../themes.md). Why every theme stands alone is in
[ADR 0003](../adr/0003-themes-stand-alone-with-a-shared-kit.md). Terms follow
[GLOSSARY.md](../../GLOSSARY.md).

Text in quotes is the text on screen, copied from the `.slint` files.

## What it is

Classic is the old launcher's look: a hero and a row of game tiles on Home, over the selected
game's art. It is a theme plugin compiled into the binary (`ClassicTheme` in
`plugins/themes/classic/src/lib.rs`). Its `build.rs` embeds every file under
`plugins/themes/classic/theme/`.

It stands alone. It ships its shell and every screen, and draws Downloads, the Game Hub, the
plugin pages, the Emulators and Themes lists, the sheets and the prompts with the kit
(`crates/nexus-ui-api/kit/`). It is not the fallback theme: Default is.

## Files

All paths are under `plugins/themes/classic/theme/`.

| File | What it holds |
| --- | --- |
| `theme.toml` | The manifest. |
| `shell.slint` | `Shell`: the window, the key router, the layers of screens and sheets, the hints, the notices, the task pill. |
| `screens/home.slint` | `HomeScreen`: the tile row and the hero, and the Library. |
| `screens/game-detail.slint` | `GameDetailScreen`: the kit's `GameDetailPage`. |
| `screens/catalog.slint` | `CatalogScreen`: `LibraryView` on the catalogs' games. |
| `screens/downloads.slint` | `DownloadsScreen`: the kit's `DownloadsPage`. |
| `screens/settings.slint` | `SettingsScreen`: a side sheet with tabs, made of kit sections. |
| `screens/emulators.slint` | `EmulatorsScreen`: the kit's `PluginListSheet`. |
| `screens/themes.slint` | `ThemesScreen`: the kit's `PluginListSheet`. |
| `screens/plugin.slint` | `PluginScreen`: the kit's `PluginPage`. |
| `screens/game-settings.slint` | `GameSettingsScreen`: a side sheet of one game's settings. |
| `screens/running.slint` | `RunningScreen` and the `Splash` global: the launch splash. |
| `components/backdrop.slint` | `Backdrop`: the art behind every screen. |
| `components/boot.slint` | `BootScreen`: the boot screen and Welcome. |
| `components/card.slint` | `Card`: a Library or Catalog card. |
| `components/library-view.slint` | `LibraryView`: the Library's header and grid, for the Library and the Catalog. |
| `components/nav.slint` | `Nav`: where the keys are on Home, the Library and the Catalog, and what each key does there. |
| `components/top-bar.slint` | `TopBar`, `PadGlyph`, `PadArrows`, `Hints`: the top bar and the key hints. |
| `fonts/` | Inter (Regular, Medium, SemiBold, Bold) and Tajawal (Regular, Bold), with `OFL-Inter.txt` and `OFL-Tajawal.txt`. |
| `images/logo.png` | The Nexus emblem. |
| `sounds/` | `move.wav`, `select.wav`, `back.wav`, `error.wav`, `start.wav`. |

The Default theme has the same files, except: its own `theme.toml` (its id and name,
`fallback = true`, no tokens, two more images); `shell.slint` differs in one comment;
`screens/home.slint`, `components/nav.slint`, `components/top-bar.slint` and
`components/library-view.slint` are Default's own; Default adds `components/chip-row.slint`
and two images.

## The manifest

From `theme.toml`:

| Key | Value |
| --- | --- |
| `id`, `name` | `classic`, `Classic` |
| `version` | `0.3.0` |
| `host_api` | `^0.2` |
| `activation` | `eager` |
| `ui_api` | `^0.2.0` |
| `input` | `["gamepad", "pointer"]` |
| `navigation` | `focus` |
| breakpoints | `compact` (`max_width = 900`), then `regular` |
| fonts | the six font files above |
| images | `logo` |
| sounds | `move`, `select`, `back`, `error`, `start` |

It has no `fallback` key.

### Tokens

`[theme.tokens]` sets all 42 tokens. Each value equals the token's default in
`crates/nexus-ui-api/src/tokens.rs` (the defaults are the Classic look). The table of tokens and
defaults is in [../themes.md](../themes.md#tokens). So Classic and Default have the same token
values.

Classic's own files also use fixed hex colors, which no token changes:

| File | Fixed colors (count, comments left out) |
| --- | --- |
| `components/boot.slint` | 29 |
| `components/top-bar.slint` | 29 |
| `components/library-view.slint` | 21 |
| `components/backdrop.slint` | 15 |
| `components/card.slint` | 12 |
| `screens/running.slint` | 5 |
| `screens/home.slint` | 2 |
| `shell.slint` | 1 |

For example, the backdrop's washes are `#04060e` at several alphas, and a card's rating star is
`#ffd666`. The kit files have none: a test in `contract.rs` refuses them.

### Options

| Option | Label | Type | Default | Sets |
| --- | --- | --- | --- | --- |
| `direction` | "Layout direction" | choice, `hidden = true` | `ltr` | `layout-direction`: `ltr` or `rtl` |
| `motion` | "Animations" | choice | `on` | `motion-fast`, `motion-normal`, `motion-slow`: 160, 260, 420 for `on`; 0 for `off` |

"Layout direction" is hidden because the right-to-left layout is not finished (comment in
`theme.toml`).

## Sizes

Classic lays everything out on a 1920×1080 canvas and scales it to the window, through the kit's
`Look` (`kit/look.slint`):

- `Look.k` is the window's height over 1080 px (at least 360 px), times the player's "Size of
  everything" (`Vm.viewport.size`). `Look.px` is `1px * Look.k`.
- `Look.t` is `Look.k`, but at least 0.75, for text that must stay readable. `Look.tpx` is
  `1px * Look.t`. Settings uses it.
- The shell sets `Look.window-height` from its own height, so the first frame has the right
  size before the host's `Vm.viewport` comes.

## The shell

`shell.slint` exports `Shell`, which inherits `Window`:

- title "Nexus", background `Tokens.color-bg`;
- preferred size 1280×720 px, smallest 640×360 px;
- font `Tokens.font-family` at `Tokens.font-size-m`.

### Layers

From the back to the front:

1. `Backdrop`, with the art of the shown game. It dims (0.84) on every screen but Home, the Game
   Hub and the sheets, which draw their own wash.
2. `HomeScreen`, always there. It shows on Home and while a game runs. Under a side sheet it
   dims to 0.3; on other screens it hides.
3. The Game Hub, the Catalog and Downloads, each while it is the screen.
4. The side sheets: Emulators, Themes, a plugin's page, Settings, a game's settings. Each is
   kept on screen by a kit `Lingering` while it slides out. A plugin's page opens beside the
   Emulators or Themes list, which stays.
5. `TopBar`, on Home, the Library, the Catalog and Downloads.
6. The kit's `SystemSheet`, while a machine panel is open and Nexus is the whole screen.
7. `RunningScreen`.
8. The kit's prompts and sheets, each while it is open: `InstallPrompt`, `ShotViewer`,
   `TrailerPlayer`, `GameMenuSheet`, `ResultPrompt`, `BackupBrowser`, `InstallOs`,
   `PowerMenuSheet`.
9. `Hints`, the key hints, in a pill at the bottom.
10. The kit's `NoticeStack`, on the far side.
11. The task pill: `Vm.task` with a turning ring, at the bottom corner, on Home with no sheet.
12. The kit's `OnScreenKeyboard`, while it is open.
13. `BootScreen`, over everything, from the first frame.

### Who takes a key

Every key reaches the shell's `FocusScope` first. In `capture-key-pressed`, whatever has the
focus:

1. While the backup browser is open (and the power menu isn't), it takes every key but Ctrl+P
   (`Browser.key`).
2. While the on-screen keyboard is open, it takes the keys (`Osk.key`).
3. A controller's Enter on a text field opens the keyboard again (`Osk.reopen`).

Then `key-pressed` tries these in order and stops at the first that takes the key:

1. A held Enter does nothing: it would play, download or install again.
2. The power menu, while open; Ctrl+P opens it.
3. The Welcome screen: Enter is Start once the library and its art are in. It takes every key.
4. Nexus OS's installer (`Setup`), then a machine panel (`Sys`).
5. The launch splash with the emulator's keys takes every key but Escape and Ctrl+Q.
6. Delete closes the newest notice.
7. The trailer, the result prompt, the game's menu, the screenshot viewer, the list of sorts,
   each while open.
8. Menu opens the shown game's menu.
9. The install prompt, while open.
10. Ctrl+Q quits. Ctrl+D opens Downloads.
11. `/` (△) searches: in the Catalog on its screen, else in the Library.
12. Insert (□) plays the shown game's trailer. T does too, but not in the Library or the
    Catalog, where it types.
13. Tab and Shift+Tab (R1 and L1): on a sheet with tabs, the next or the previous tab; on Home,
    along the top bar; elsewhere, the next or the previous screen. The order is Home, Catalog
    (when there is one), Downloads, Emulators, Themes, Settings, and round to Home.
14. Home, the Library and the Catalog: `Nav.key`. A settings sheet: `Prefs.key`. Emulators and
    Themes: `PluginList.key`. The Game Hub: `Hub.key`. Downloads: `Dl.key`.
15. Escape or Backspace goes back, with the `back` sound (no sound on Home).

### Hints

`Hints` shows the keys that do something now, in a pill at the bottom, on the far side (the near
side under a sheet, which is on the far side). With a controller (`Vm.input-mode == "gamepad"`) it shows the pad's buttons
(✕, ○, △, □, L1 R1, ☰); with a keyboard, the keys (Enter, Esc, Tab, Menu). The face buttons are
drawn as paths (`PadGlyph`, `PadArrows`), because no font the theme ships has them.

The hints hide over the Library's and the Catalog's cards (they would cover the titles), while a
trailer plays with its controls hidden, over the backup browser, and over the splash's keys.

### Sounds

| Sound | When |
| --- | --- |
| `start` | A game starts; the Welcome screen closes. |
| `error` | A new notice of kind `error` comes. |
| `back` | Back, except on Home. |
| `move`, `select` | Moves and choices on Home, the Library and the Catalog (`Nav`). |

## Screens

### Home (`screens/home.slint`)

`HomeScreen` shows Home or the Library (`Nav.view`, `home` or `library`).

Home is a row of tiles, the selected one larger with its name beside it, and a hero over the
game's art. Past the last game is the Library tile. The tile of the running game has a green
frame and "● PLAYING".

The hero comes in a piece at a time when the selection changes: the line over the title, the
title (or the game's logo from the store), the chips, the description, then the buttons. It
waits for the game's details, at most `Look.wait-for-details` (450 ms).

- Chips: the player's result ("… · your result"), the plugin's badges, the rating, the genres,
  the size, the release date.
- Buttons while the game runs: "Resume", "Stop", "Game Hub". Otherwise: "Play" (when the game
  can start), "Game Hub". Then "Store" when the game has a store page, and "Options" (the game's
  menu) once its details are in. On the Library tile past the last game: "Open Library" (only when the library has games).
- Under the buttons: `Vm.detail.launch-hint`, such as "Play installs KytyPS5 first".

The mouse wheel moves along the row, one game a notch.

The Library is `LibraryView` (see below), made again each time it opens.

### Keys on Home and in the Library (`components/nav.slint`)

The keys act on one zone at a time:

| Zone | What it is | Up | Down | Enter | Escape |
| --- | --- | --- | --- | --- | --- |
| `tabs` | The top bar | | Home's row, or the Library's bar | Opens the item | Back to the row or the grid |
| `row` | Home's tiles | The top bar | The buttons | Opens the Game Hub, or the Library from its tile | |
| `actions` | The buttons under the game | The row | | Runs the button | The row |
| `bar` | Search, sort, card sizes, the catalog's button | The top bar (Library only) | The systems | Runs the item | Home |
| `filters` | The systems | The bar | The genres, or the grid | Filters by the system | Home |
| `genres` | The genres | The systems | The grid | Filters by the genre | Home |
| `grid` | The cards | The row above, or from the first row the genres (the systems when there is one genre or none) | The row below | Opens the Game Hub | Home |

Left and Right follow the reading direction. In the grid, Page Up and Page Down (L2 and R2) move
three rows. A letter or a digit typed in the grid starts a search with it. In the Catalog,
Escape goes back to the screen it opened from.

A move shows at once. The core hears of the selection `Look.select-after` (100 ms) after the
last move: the shell's timer calls `Nav.send`.

### Game Hub (`screens/game-detail.slint`)

The kit's `GameDetailPage`, in Classic's tokens. It opens out of the card the player was on
(`Origin`): the cover grows from the card's place. It has the line over the title, the
buttons (Play, Resume and Stop, the plugin's actions), the releases of a game a catalog offers,
fact cards, "Technical details", and the screenshots.

### Catalog (`screens/catalog.slint`)

`LibraryView` on the catalogs' games (`Vm.catalog`), sliding in from the side. The top bar is
over it. Its bar adds the catalog plugin's first button, such as a refresh.

### Downloads (`screens/downloads.slint`)

The kit's `DownloadsPage`: one row per torrent, then one per emulator download, with their
buttons. The install sheet and the remove sheet are the kit's.

### Settings (`screens/settings.slint`)

A side sheet (`SheetFrame`, 1000 px wide at 1080p, at least 0.75 of that). It has a back
button, "Settings", "Changes are saved automatically", then the tabs:

"Launcher", "Games", "Updates", "Downloads", "This PC", "System" (only when Nexus is the whole
screen), "Backup", "Power".

Each tab's rows are a kit section: `LauncherSection`, `GamesSection`, `UpdatesSection`,
`DownloadsSection`, `PcSection` (with `Assets.logo`), `SystemSection`, `BackupSection`,
`PowerSection`. The title and the tabs stay; the rows scroll under them, and the sheet scrolls
to keep the focused row in view. When the System tab goes, the sheet goes back to the last tab.

### Emulators and Themes (`screens/emulators.slint`, `screens/themes.slint`)

The kit's `PluginListSheet`, a side sheet listing plugins.

- Emulators: "Emulators", "Open one to turn it on or off, install it and change its settings".
- Themes: "Themes", "Open one to use it and change its options".

### A plugin's page (`screens/plugin.slint`)

The kit's `PluginPage`, a sheet beside the list. An emulator: "Turned on", its build, actions,
settings, folders and blocks. A theme: "Use this theme", then its options while it is the one in
use.

### Game settings (`screens/game-settings.slint`)

A side sheet (760 px wide at 1080p) with a back button, the game's title, the panel's text, then
"THIS GAME" and one kit `FieldRow` per field of `Vm.detail.settings`. A change sends
`Commands.set-setting` with the panel's plugin and scope. Back returns to the Game Hub.

### Running (`screens/running.slint`)

While a game starts: its art dimmed behind its icon, its name, "Starting with <emulator>…" (or
"Starting…") and a turning ring. Under it, "Controller: <name>", or "Keyboard: the keys are on
the <page> page". Without a controller, it lists the emulator's keyboard keys
(`Vm.running.keymap`) under "Keyboard", and the shell gives every key but Escape and Ctrl+Q to
the game (`Splash.keys`). It fades once the game's window shows; Home is under it with Resume
and Stop.

## Components

### Backdrop (`components/backdrop.slint`)

The art behind every screen: a deep blue glow, the game's art, and dark washes from the text
side, from the bottom and under the top bar. New art fades in over the old and settles from a
slight zoom in about 1.7 s (`Tokens.motion-slow` × 4). Once it settles, nothing is drawn every frame.

### Boot screen and Welcome (`components/boot.slint`)

A slowly drifting wall of the library's covers behind the Nexus emblem, "NEXUS", a progress bar
with its percentage and `Vm.boot.status`. It shows from the first frame until the library and
Home's art are in, then fades out.

On the first run it is Welcome: "Welcome", "Setting up Nexus", a checklist of `Vm.boot.steps`,
and a button that says "Setting up…" until it is ready, then "Start". A click or Enter is Start
(`Commands.finish-welcome`).

### Card (`components/card.slint`)

A 2:3 cover with the console's name on top (in the system's color), how the game runs and its
rating at the bottom; then two lines for the title, and the size (or the system's name). A game
a catalog offers also shows its releases beside the size, and "Availability:" under it. On the
focused or pointed card: the main action ("Play", or getting it), and in the Catalog an eye that
hides or shows the game. The focused card grows a little and gets the white ring.

### Library view (`components/library-view.slint`)

The Library and the Catalog. A header band with "Search games", the sort button, and the card
sizes ("Comfortable" or "Roomy" in a window under 1000 px, and "Compact"); the systems as chips
with their counts; the count of games ("N games", in the Catalog "N of M games · N releases" and
"List updated …"); "GENRE" and the genres. The grid of cards scrolls under the band, asks for
the next page as it nears the end, and keeps the selected card in view. The list of sorts opens
under the sort button with "SORT BY".

An empty grid says "Loading catalog…", "No game matches "…"", "No catalog lists any game yet"
or "No games here".

### Navigation (`components/nav.slint`)

The `Nav` global: the view (`home` or `library`), the zone, the focused item of each zone, the
grid's columns, the search box, the list of sorts, and each grid's scroll. It holds the theme's
own selection (`cursor`) and sends it to the core after the moves stop. It re-exports the kit's
`Origin`. The keys are in [Keys on Home](#keys-on-home-and-in-the-library-componentsnavslint).

### Top bar (`components/top-bar.slint`)

On the reading side: the "Home" and "Library" tabs (the Library with its count). On the far
side: the running game (its name, the time played, "Resume" and "Stop"), the active emulator
download, the screen buttons (Search, Downloads, Emulators, Themes), the network, sound and
Bluetooth icons when Nexus is the whole screen, the battery when the PC has one, Settings, and
the clock. Search is left out when there is no Catalog.

## Kit files Classic uses

`backup-browser`, `button`, `commands`, `downloads`, `game-detail`, `game-menu`, `icon`,
`install-os`, `install-prompt`, `keyboard`, `lingering`, `look`, `notices`, `origin`,
`plugin-list`, `plugin`, `power-menu`, `result-prompt`, `settings-rows`, `settings-sections`,
`sheet-frame`, `shot-viewer`, `slot-badges`, `system-panel`, `trailer-player`, `widgets`.
Classic replaces no kit file. What each one gives is in [../themes.md](../themes.md#the-kit).

## Accessible labels

Tests find Classic's elements by these labels (`accessible-label`), among others:

| Where | Labels |
| --- | --- |
| Home | `default-tile`, `default-library-tile`, `default-sub`, `default-title`, `default-played`, `default-description`, `default-size`, `your-result`, `default-resume`, `default-stop`, `game-options` |
| Library and Catalog | `default-game`, `default-count`, `catalog-updated`, `grid-empty`, `grid-progress`, `library-sort`, `sort-option`, `sort-current`, `density`, `genres`, `genres-more-start`, `genres-more-end`, `filter:<name>` |
| Card | `card-console`, `card-hide`, `card-play`, `card-get`, `card-size`, `card-availability` |
| Top bar and shell | `default-system`, `clock`, `hint`, `hint-key`, `task` |
| Boot | `boot`, `welcome`, `welcome-step`, `welcome-start` |
| Running | `launch-title`, `launch-input`, `launch-key` |
| Game settings | `game-settings-title` |

The `default-` labels are names only: Classic uses them too.

## Tests

`tests/tests/seams/classic_theme/` tests the theme, one module for each part: `catalog.rs`,
`downloads.rs`, `emulators.rs`, `game_hub.rs`, `home.rs`, `installs.rs`, `keyboard.rs`,
`library.rs`, `notices.rs`, `play.rs`, `releases.rs`, `settings.rs`, `theme_files.rs`,
`trailer.rs`. `theme_files.rs` checks that every font the theme names ships with it, and that
every animation time goes through `Look.timed`. `every_theme.rs` runs its suite on Classic and
Default. `themes.rs` checks that Classic ships every screen and compiles.

```sh
cargo test -p nexus-tests --test seams classic_theme::
```
