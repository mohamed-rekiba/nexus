# The default theme

This page says what the built-in `default` theme does, screen by screen. It is for theme
authors, who need to know what a child theme starts from, and for contributors who change the
theme.

Terms follow [GLOSSARY.md](../../GLOSSARY.md). For the contract between a theme and the host,
read [../themes.md](../themes.md). For the layers around it, read
[../architecture.md](../architecture.md). For plugins in general, read
[../plugins.md](../plugins.md). For the check commands, read
[../development.md](../development.md).

Text in quotes is the exact text on screen, copied from the `.slint` files.

## What the theme is

The default theme is a theme plugin compiled into the app (`plugins/themes/default/src/lib.rs`,
`DefaultTheme`). It is the default, and the app falls back to it when another theme fails to
load. The code says every other theme extends it, directly or through a parent.

| Path | What is in it |
| --- | --- |
| `plugins/themes/default/theme/theme.toml` | The manifest: id `default`, kind `theme`, screens, components, input, tokens, options, assets |
| `plugins/themes/default/theme/shell.slint` | `Shell`: the window, the key router, the layers of screens, the hints |
| `plugins/themes/default/theme/commands.slint` | `Commands`: one function for each thing the player asks of the core |
| `plugins/themes/default/theme/screens/` | One file for each screen id in the manifest |
| `plugins/themes/default/theme/components/` | The parts: top bar, library view, menus, prompts, sheets, keyboard, system panel |
| `plugins/themes/default/theme/fonts/`, `images/`, `sounds/` | The assets the manifest names |

`shell.slint` imports `types.slint` and `tokens.slint`. Those two files are not in the theme's
folder. The host supplies them from `crates/nexus-ui-api/contract/`.

### The manifest

`theme.toml` declares these screens, as `ScreenId` in `crates/nexus-ui-api/src/vm.rs` names them:
`home`, `game-detail`, `catalog`, `downloads`, `settings`, `emulators`, `themes`, `plugin`,
`running` and `game-settings`.

It declares these components, as `ComponentId` in `crates/nexus-ui-api/src/contract.rs` names
them: `card`, `button`, `list-item`, `progress-bar`, `dialog`, `slot-badges`, `slot-actions` and
`slot-sections`.

Other settings in the manifest:

- `input = ["gamepad", "pointer"]` and `navigation = "focus"`.
- Two breakpoints: `compact` (up to a width of 900) and `regular`. No `.slint` file reads them.
  The theme scales with the window height instead (see Sizes).
- Fonts: Inter (regular, medium, semibold, bold) and Tajawal (regular, bold).
- One image, `logo`, and five sounds.
- `ui_api = "^0.2.0"` and `host_api = "^0.2"`.

### Tokens

`[theme.tokens]` holds the values a child theme can override.

| Group | Names |
| --- | --- |
| Color | `color-bg`, `color-surface`, `color-surface-raised`, `color-text`, `color-text-muted`, `color-accent`, `color-on-accent`, `color-focus`, `color-success`, `color-warning`, `color-danger`, `color-info`, `color-overlay` |
| Font | `font-family` (`Inter`), `font-size-s`, `-m`, `-l`, `-xl`, `font-weight-regular`, `font-weight-bold` |
| Space and shape | `space-xs` to `space-xl`, `radius-s` to `radius-l`, `shadow-color`, `shadow-blur-1`, `shadow-blur-2` |
| Motion | `motion-fast`, `motion-normal`, `motion-slow` |
| Direction | `layout-direction` (`ltr`) |

Screens use many colors that have no token. Those are in `components/look.slint` (`Look`).

## Options

The theme has two options. The player sets them on the page of the theme in Themes. They show
only while the default theme is in use. Each choice sets tokens.

| Option | Label | Choices | Effect |
| --- | --- | --- | --- |
| `direction` | "Layout direction" | `ltr`, `rtl` | Sets `layout-direction`. The whole layout mirrors. |
| `motion` | "Animations" | `on`, `off` | `off` sets the three motion tokens to 0. |

The sheet shows the choices as "Left to right", "Right to left", "On" and "Off"
(`Look.choice`). The page sends `SetThemeOption` with the key and the value.

Every animation time in the `.slint` files goes through `Look.timed(...)`, so "Animations: Off"
cuts all of them. A test checks it (see Tests).

## Sizes

The theme lays out everything on a 1920 by 1080 canvas and scales it. `Look.k` is the window
height divided by 1080, never below a window height of 360, times `Vm.viewport.size`. A size in
a file is `N * Look.px`. `Look.tpx` is the same with a floor of 0.75, for text that must stay
readable (Settings).

The window asks for a width of 1280 and a height of 720 at the least, and 640 by 360 as the
smallest.

The player changes `Vm.viewport.size` with "Size of everything" in Settings, or with the
`NEXUS_SCALE` environment variable for one run. The row says "NEXUS_SCALE sets it for this run"
while the variable differs from the setting.

## The shell

`Shell` in `shell.slint` is the window. It holds the layers, from the back to the front:

1. `Backdrop`: the art of the shown game behind every screen. It uses the game's background
   once its details are loaded, and its cover until then. Home and the Game Hub draw it
   without a wash. The Catalog and Downloads dim it. The sheets and the running card draw
   their own wash.
2. `HomeScreen`. It stays under the other screens so a return does not rebuild it. Under a side
   sheet it steps back and dims.
3. The screen for `Vm.screen`: Game Hub, Catalog or Downloads, or a sheet (Settings, Emulators,
   Themes, a plugin, game settings). A sheet slides in from the edge and slides out before it
   goes (`Lingering`).
4. The top bar, over Home, Running, the Catalog and Downloads.
5. The running card, the install prompt, the screenshot viewer, the trailer player, the game
   menu, the result prompt, the Nexus OS installer and the power menu.
6. The key hints, the notices, the task pill and the on-screen keyboard.
7. The boot screen, over everything, from the first frame.

When a game starts (`Vm.has-running` turns true) the shell plays the sound `start`, selects the
running game, goes to Home and puts the keys on the Resume button.

### Who takes a key

`keys` in `Shell` receives every key. It tries these in order and stops at the first that takes
the key:

1. The on-screen keyboard, while it is open. `Vm.pad-key` tells the shell the key came from a
   pad.
2. The power menu, if open. `Ctrl+P` opens it, and so does `Vm.power-menu` going up (the PS or
   Xbox button held with no game running).
3. The Welcome screen: `Enter` is Start once the library and the art are in.
4. The Nexus OS installer, if shown.
5. A panel of the machine's settings, if open.
6. The launch splash, when it lists the game's keys. It takes every key except Escape and
   `Ctrl+Q`.
7. `Delete` closes the newest notice.
8. The trailer, the result prompt, the game menu, the screenshot viewer, the list of sorts, the
   install prompt, each when open.
9. `Menu` opens the game menu for the shown game.
10. `Ctrl+Q` quits. `Ctrl+D` opens Downloads.
11. `/` searches: the Catalog on its screen, else the Library.
12. `Insert` (Square on a pad) or `T` plays the trailer of the shown game. `T` does not do
    this in the Library or the Catalog, where a typed letter searches.
13. `Backtab` (L1) goes to the screen before. `Tab` (R1) goes to the next screen.
14. The keys of the screen that has them: Home and Catalog (`Nav`), Settings and plugin pages
    (`Prefs`), the lists of plugins, the Game Hub (`Hub`), Downloads (`Dl`).
15. `Escape` and `Backspace` send `Back`. A back sound plays unless the screen is Home.

A held `Enter` does not repeat. It would play, download or install again.

The order of the screens for `Tab` is: Home, Search (the Catalog, only when
`Vm.catalog-available`), Downloads, Emulators, Themes, Settings, then Home again. `Backtab`
goes the other way.

### The pad

The host turns pad buttons into keys. The theme only sees keys. `Vm.pad-key` is true while the
shell handles a key that came from the pad, and `Vm.input-mode` is `gamepad` after the last
input came from it.

| Pad button | Key the theme gets | Meaning in the theme |
| --- | --- | --- |
| D-pad, left stick | Arrow keys | Move |
| Cross (South) | `Enter` | Select |
| Circle (East) | `Escape` | Back |
| Triangle (North) | `/` | Search |
| Square (West) | `Insert` | Trailer. Delete in the on-screen keyboard |
| Options (Start) | `Menu` | Options: the game's menu |
| Create (Select) | `Delete` | Close the newest notice |
| L1 | `Backtab` | The screen before |
| R1 | `Tab` | The next screen |
| L2, R2 | Page up, Page down | A page in a grid |
| PS button, tap | Not a key | Go to Home, host action |
| PS button, hold | Not a key | Close the running game, else open the power menu |

For how the host reads the buttons, see [desktop.md](desktop.md).

### Key hints

`Hints` draws a pill at the bottom corner that lists the keys that work now. A pad shows its
buttons and a keyboard shows its keys, by `Vm.input-mode`. The pill sits at the bottom corner on the side away from the
reading start, and moves to the other corner when a sheet is open. It hides over the Library's and Catalog's
cards (it would cover the titles), and over a playing trailer while its controls are hidden,
and over the launch splash's keys. Its text changes with the state. Examples:

- Library or Catalog header row: "Move", "Choose", "Rows", "Back".
- Home with a game: "Select", "Next" or "Search" or "Downloads", and "Options". With a trailer:
  also "Trailer".
- Game Hub: "Play" or "Select" or "View", "Back", "Options", and "Trailer" if the game has one.
- Download remove sheet: "Choose" and "Remove" or "Delete files" or "Not now".
- Trailer: "Pause" or "Play", "10 s", "Close".
- On-screen keyboard: "Type", "Delete", "Space", "Done", "Close".

### The task pill

While `Vm.task` is not empty, on Home (not under a sheet), a pill at the bottom corner shows
the text with a turning ring.

### Sounds

Five sounds ship in `sounds/`: `move`, `select`, `back`, `error` and `start`. The theme calls
`Host.play-sound(name)`. The host returns nothing when the player turned "Interface sounds"
off.

- `move`: a move of the focus.
- `select`: a pick.
- `back`: Back, closing a menu, Stop.
- `error`: a new notice of kind `error`.
- `start`: a game starts, or the Welcome screen closes.

## Screens

### Boot and Welcome

`components/boot.slint` (`BootScreen`) shows from the first frame. It fades out over Home when
the library is in and Home's art is in (`Vm.boot.ready` and `Vm.art-ready`).

It shows:

- A slow wall of covers from `Vm.boot.covers`.
- The emblem, and the word "NEXUS".
- A progress bar from `Vm.boot.progress`, with the percentage, and `Vm.boot.status`.

On the first run, `Vm.boot.welcome` is true. The screen then shows "Welcome" and "Setting up
Nexus", the checklist `Vm.boot.steps` (each with a label and a detail), and a Start button. The
button says "Setting up…" until the boot is ready, then "Start". It shows "Enter" on a
keyboard and a drawn mark on a pad.

Start runs `Commands.finish-welcome`, which sends the `Pref` command to set `welcomed`. A click
on the screen does the same when ready.

### Home

`screens/home.slint` (`HomeScreen`) shows a row of tiles from `Vm.home.games`, the selected one
larger with its name beside it, and the game's hero over the backdrop.

The hero shows, for the selected game, once `Vm.detail` is loaded for that game:

- A kicker line: "PLAYED " and `Vm.detail.played` in capitals, or while it runs "● PLAYING · "
  and `Vm.running.elapsed`.
- The title, or the game's logo (`Vm.detail.logo`) when it has one.
- Chips: the player's result with " · your result" (`Vm.detail.your-result`), the game's badges,
  the rating, the genres (the first two, joined by " · "), the size and the release.
- The summary (`Vm.detail.summary`).
- Buttons.

A new game's hero waits for its details, so it fades in whole. It waits at most
`Look.wait-for-details`.

The line beside the selected tile is "Playing now", or "Last played " and the date in lower
case, or "Not played yet".

The buttons depend on state:

| State | Buttons |
| --- | --- |
| A game runs | "Resume", "Stop", "Game Hub", "Store" if it has a store page, "Options" |
| Game can start (`Vm.detail.can.play`) | "Play", "Game Hub", "Store", "Options" |
| Game cannot start | "Game Hub", "Store", "Options" |
| The Library tile is selected | "Open Library" |

A launch hint (`Vm.detail.launch-hint`) shows beside the buttons when the game cannot be played.

The tile after the last game is the Library tile. Its title is "Library" and its line is the
number of games. With no game at all it says "No games yet" and "Add a games folder in
Settings, or install an emulator.". With games it says "Every game, in a grid, by system.".

A running game's tile has a green frame and the tag "● PLAYING".

Commands used on Home:

- `Select` (the theme tells the core which game is selected).
- `OpenGame`, `Session` with `Launch`, `Stop` or `ShowGame`.
- `Navigate`.
- `Host.open-url` for Store.

#### Keys on Home

The keys act on one zone at a time. `Nav.zone` is one of `tabs`, `row`, `actions`, and in the
Library `bar`, `filters`, `genres` and `grid`.

| Zone | Keys |
| --- | --- |
| `row` | Left and right move along the tiles. Past the last game is the Library tile. Up goes to the tabs. Down goes to the buttons. `Enter` opens the Game Hub, or the Library on the Library tile. |
| `actions` | Left and right move between buttons. `Enter` runs one. Up, `Escape` or `Backspace` go back to the row. |
| `tabs` | Left and right move over Home, Library, Search (only with the Catalog), Downloads, Emulators, Themes, Settings, and when Nexus is the whole screen, the three status icons. `Enter` opens one. Down returns. |

Left and right follow the reading direction, so right to left they swap.

A move in the row shows at once. The theme tells the core about the selection with `Select`
after `Look.select-after` without another move. A key that is not a move sends the selection
first, so the core acts on the game on screen. The mouse wheel moves along the row, one game a
notch.

### Top bar

`components/top-bar.slint` (`TopBar`) is over Home, Running, the Catalog and Downloads.

- Tabs "Home" and "Library". "Library" carries the number of games.
- A running game: its title, the time played, and the buttons "Resume" and "Stop"
  (`Vm.running`).
- The first download as a bar with its label (`Vm.downloads[0]`). A click opens Downloads.
- Round buttons: "Search" (only with `Vm.catalog-available`), "Downloads", "Emulators",
  "Themes", "Settings". Their accessible labels carry these names.
- When Nexus is the whole screen (`Vm.system.whole-screen`): the network, sound and Bluetooth
  icons. Each opens a panel (see System panels).
- A battery icon when the PC has one (`Vm.system.battery-shown`), and the clock `Vm.clock`.

### Library

The Library is Home's second view (`Nav.view` is `library`). `components/library-view.slint`
draws it, and the Catalog uses the same component.

The core searches, filters and sorts (`Vm.home.library`). The theme sends a page at a time and
asks for the next page as the grid nears its end (`LibraryMore`).

- A header band: the search box ("Search games"), the sort button, the card size choice
  ("Comfortable", or "Roomy" in a narrow window, and "Compact").
- The system chips (`lib.systems`), and on the far side the count of games ("1 game" or "N
  games").
- The genre row (`lib.genres`), under the label "GENRE". It scrolls to the focused chip and
  fades where chips are hidden.
- The grid of cards (`GameCard`).

The sorts for the Library, as `Look.sort-label` names them: "Title", "Recently played",
"Size (largest first)", "Top rated", "Newest" and "Runs best". The list opens under the label
"SORT BY".

States of the grid:

- Empty: "No games here". With a search that matches nothing: `No game matches "<text>"`.
- The Catalog while it reads: "Loading catalog…". With no catalog game yet: "No catalog lists
  any game yet".

Commands: `LibrarySearch`, `LibrarySystem`, `LibraryGenre`, `LibrarySort`, `LibraryMore`,
`GridColumns` (the grid tells the core how many columns it has). `Host.grid-window` tells the
host which rows are on screen, so it loads only those covers.

Keys in the Library:

| Zone | Keys |
| --- | --- |
| `bar` | Left and right move over search, sort, Comfortable, Compact. `Enter` types, opens the sorts, or sets the size. Down goes to the systems. Up goes to the tabs. |
| `filters` | Left and right move over the systems. `Enter` picks one. Down goes to the genres, or the grid. |
| `genres` | Left and right move over the genres. `Enter` picks one. |
| `grid` | Arrows move over cards. Up from the first row goes to the genres, or the systems. `Enter` opens the Game Hub. Page up and Page down move by three lines. A letter or a digit starts a search with it. |

`Escape` and `Backspace` leave the Library for Home's row. Typing in the search box: `Escape` or
Down gives the keys back. Search runs once typing stops for a moment, and `Enter` runs it now.
On a pad, the search box opens the on-screen keyboard.

The list of sorts takes every key while open: Up and Down move, `Enter` picks, `Escape` closes.

### Catalog

`screens/catalog.slint` is the same library view over the catalogs' games (`Vm.catalog`). It
has the same zones, but not the tabs: `Escape` sends `Back`.

Differences from the Library:

- The sorts start with "Recently added", then "Title", "Size (largest first)", "Size (smallest
  first)", "Top rated", "Newest", "Runs best".
- The on-screen keyboard's title is "Search the catalog". The Library's is "Search games".
- The count reads "<n> games · <m> releases", or "<n> of <all> games · <m> releases" while a
  search or a filter narrows it. Under it: "List updated " and `Vm.catalog-updated`.
- The catalog plugin's first button (`Vm.catalog-actions[0]`, such as Refresh) sits at the
  far end of the bar. The status text of `Vm.catalog-status[0]` sits beside it.
- A card has one line more: "Availability:" and how easy the game is to download.
- The commands are `CatalogSearch`, `CatalogSystem`, `CatalogGenre`, `CatalogSort` and
  `CatalogMore`.
- A turned-off Game Catalog removes the "Search" button and the `Tab` stop for it
  (`Vm.catalog-available` is false).

The core fills the chip row (`Vm.catalog.systems`). The glossary names the grid filters it can
add: "In-game", "Installed" and "Hidden".

### Game Hub

`screens/game-detail.slint` is the page of one game. It opens from the card the player was on
(`Origin`). The cover grows out of that card's place and size. When the place is not known the
cover fades in.

The page has a cover on one side and a column that scrolls on the other. The column holds, from
the top:

1. A line over the title: the platform, the developer in capitals, the year and the region,
   joined by " · ". While the game runs, "● PLAYING " and the time.
2. The title, or the logo.
3. Chips: badges, rating, age rating (or "Ages " and the age), and genres.
4. Buttons: "Play" or "Resume", "Stop" while it runs, the plugin's actions
   (`Vm.detail.actions`), "Store", "Trailer", "Show" or "Hide" for a catalog game, and "Options".
5. The launch hint.
6. For a game a catalog offers: "RELEASES · RUNS WITH " and the emulator, one row for each
   release.
7. Fact cards, three to a row: how the game runs ("YOUR RESULT" when the player rated the game, else "COMPATIBILITY" with the date last tested),
   "ON THIS PC" with the reason, "PLAYED" (with the date) and "SIZE", the ProtonDB tier for a
   Windows game packed with Wine, and the availability of a catalog game.
8. "+  Technical details" (and "−  " when open): every fact the plugin gives
   (`Vm.detail.facts`), and the compatibility notes.
9. "SCREENSHOTS": a row of the store's shots.
10. The plugins' sections (`SlotSections`) and "ABOUT" with the description.

The compatibility card says "Not tested" when neither the player's result nor the community's
exists. The size card says "Unknown" without a size. The played card says "Not yet".

Commands: `Session` (`Launch`, `Stop`, `ShowGame`), `RunAction`, `Library` (`Hide`), `Torrent`
(`Download`), `Navigate`, and `Host.open-url`.

#### Release rows

Each release row shows its version ("Version " and the number, or "Release"), the region, its
place by version, its size and when the catalog got it, a tag "New" for a release the view marks as new (`release.new`), the line "Availability:" with the catalog's counts of seeders and
leechers, and the warnings (`not-here`, `warning`).

The row's button says:

| State | Text |
| --- | --- |
| The release comes from a store | "Install in " and the store |
| No download | "Download" |
| Downloading | "Downloading " and the percent |
| Complete | "Downloaded" |
| Paused | "Paused" |
| Failed | "Failed" |
| Otherwise | "In Downloads" |

`Enter` on a row with no download opens the download sheet. On a row with a download it opens
Downloads. On a store's release it sends `Torrent` `Download` at once, because the store asks
its own questions.

#### The download sheet

"Set up a download" asks before a release downloads. It shows the title, the version and
region and "runs with " and the emulator, then warnings (the release may not start here, the PC
warning, no room for it), the row "Download folder" (`Enter` types another folder), and the text of
the consent. The text says what the download shares, at most "128 KiB/s" while it downloads, and
whether finished downloads keep sharing (`Vm.seed-downloads`). Another line says who sees the
player's address: with `Vm.vpn.only` and a route it names the VPN; with `Vm.vpn.only` and no
route it warns.

A line says what happens next: first Nexus looks up the torrent's metadata, then the player
looks at the files in Downloads and presses Download.

The buttons are "Look up metadata" and "Not now". "Look up metadata" is off while the emulator
has no packages folder; a line then says "<emulator> can't install downloads: its packages
folder isn't set.". When the core refuses, the sheet stays open and shows `Vm.detail.download-error`.

Keys: Left and Right choose a button. Up goes to the folder row. `Escape` is "Not now".

#### The screenshots and the reading

Down from the buttons opens the technical details. Down again goes to the shots, or to the
reading when there are no shots. Left and right move over the shots, and `Enter` opens one full
screen. Down on the last shot scrolls the reading by steps. Up goes back.

The screenshot viewer (`ShotViewer`) shows the shot and "N / M". Left and right move, and
`Escape`, `Backspace` or `Enter` close it.

### The game's menu

`components/game-menu.slint` (`GameMenu`) opens from the Options button, the `Menu` key (the
pad's Options button) or the "⋯" button. Items the game does not have are left out.

| Item | Shown when | Does |
| --- | --- | --- |
| "Game Hub" | Not on the Game Hub | `OpenGame` |
| The emulator's actions | The game has them | `RunAction` |
| "Game settings" | The game has settings | `Navigate` to `GameSettings` |
| "Open game folder" | The game has a folder, and Nexus is not the whole screen | `Host.open-game-folder` |
| "View emulator log" | The game has a log, and Nexus is not the whole screen | `Host.open-game-log` |
| "Watch trailer" | The game has a trailer | The trailer player |
| "Rate how it runs" | Always | `AskResult` |
| "Compatibility list" | The game has a page, and Nexus is not the whole screen | `Host.open-url` |
| "ProtonDB page" | The game has one, and Nexus is not the whole screen | `Host.open-url` |
| "Settings" | Always | `Navigate` to Settings |
| "Show in the Catalog" or "Hide from the Catalog" | A catalog game | `Library` `Hide` |
| "Uninstall" | The game can be uninstalled (`Vm.detail.can.uninstall`) | Asks, then `UninstallGame` |

Up and down move. `Enter` picks. `Escape`, `Backspace` and `Menu` close.

"Uninstall" asks in the same card: `Uninstall <title>?`, then "Its folder is deleted from your
game folder, with its update and add-ons. A download it came from stays in Downloads.", with
"Uninstall" and "Not now". The keys start on "Not now".

### Game settings

`screens/game-settings.slint` is a side sheet for one game's own settings from its emulator.
The title is the game's title. The section is "THIS GAME", with one row for each field of
`Vm.detail.settings`. A change sends `SetConfigField` with the plugin, scope, key and value.
Back returns to the Game Hub. The rows and keys are those of every sheet (see Settings).

### Downloads

`screens/downloads.slint` lists one card for each torrent (`Vm.torrents`), then one card for
each emulator download (`Vm.downloads`).

The summary line shows the VPN state when "Download only through a VPN" is on ("Protected by
your VPN · <route>" or "Waiting for your VPN"), then the counts: "<n> downloading · <n> sharing
· <n> in all". A note says how transfers behave: they go on while Nexus is open and come back
paused at the next start, and it says whether finished downloads keep sharing.

With nothing to show: "Install an emulator in Emulators, or download a game from its Game
Hub.".

A torrent card shows the title (with version and region), its state, its bar, and its buttons.
The state text is one of: "Installing", "Installed", "Queued", "Unpacking", "Unpacking
failed", "Reading its files", "Ready: look at the files, then download", "Checking",
"Downloading", "Paused" (or "Paused · waiting for your VPN"), "Complete" (with " · sharing" or
" · checking"), "Failed", "Cancelled · the files stay". For a store's install it adds " in " and
the store.

A card in the "ready" state lists the files to look at, in a scroll area, before the player
presses Download.

The buttons come from the torrent's `actions`:

| Action | Text |
| --- | --- |
| `start` | "Download" (ready), "Resume" (paused), "Retry" |
| `pause` | "Pause" |
| `cancel` | "Cancel" ("Stop unpacking" while unpacking) |
| `remove` | "Remove" |
| `stop-seeding` | "Stop sharing" |
| `install` | "Install", "Retry install", or "Unpack and install" |
| `remove-unpacked` | "Remove unpacked files" |
| `cancel-install` | "Cancel install" |
| `open-store` | "Open in " and the store |
| `play` | "Play" |

Then "Open folder" (`Host.open-download-folder`), once the folder exists. While a card looks up
its files or installs, its first button reads "Resolving…" or "Installing…" and does nothing.
The keys skip it.

An emulator download's card shows its state ("Waiting", "Downloading", "Checking", "Unpacking",
"Paused", "Installing", "Done", "Failed", "Cancelled") and the buttons "Resume", "Pause" and
"Cancel", as `can-resume`, `can-pause` and `can-cancel` allow.

A footer reads: "Cancel never deletes your downloaded files, and Remove asks first whether they
go too. Remove unpacked files deletes only the copies Nexus unpacked."

Keys: Up and Down move between cards. Left and right move between the buttons of a card.
`Enter` presses one.

Commands: `Torrent` (`Act`), `GameInstall` (`Download`, `RemoveUnpacked`, `Cancel`),
`PauseDownload`, `ResumeDownload`, `CancelDownload`, `Session` `Launch` for "Play".

#### The install sheet

"Install" asks first. The sheet is titled "Install game", names the game, and says "Nexus
copies or unpacks this finished download into your game folder. The install runs in the
background and checks the free space first. It never writes over a game that is there, and the
downloaded files stay.". Its row is "Game folder". If the emulator has no game folder, it says
"This emulator has no game folder: add one in Settings.". The buttons are "Install" and "Not
now". When the core refuses, the sheet stays open and shows `install-error`.

#### The remove sheet

"Remove" asks, with the question `Remove <title>?`, and three answers in a list: "Remove from
Downloads" (first, and the keys start there), "Delete files too" (red) and "Not now". Each has
a line that says what it does. Up and Down, or Left and Right, move. `Escape` is "Not now".

### Settings

`screens/settings.slint` is a side sheet. The sections and rows are below. Up and Down move,
Left and Right change the row (`Prefs.act`), and `Enter` or `Space` presses it.

| Section | Rows |
| --- | --- |
| "GAMES" | One row for each game folder (`Vm.folders`; `Enter` types a path, several separated by ";"), then "Rescan game folders" |
| "UPDATES" | "Update automatically", "Nexus updates" |
| "LAUNCHER" | "Full screen", "Size of everything", "Back to the launcher", "Interface sounds", "Start at login" (only when `Vm.can-start-at-login`), "Screen" (only with two or more monitors), "Quit Nexus" |
| "DOWNLOADS" | "Download only through a VPN", one row for each VPN, "Add a VPN" (where the system can), "Keep sharing finished downloads" |
| "THIS PC" | "Processor", "Memory", "Graphics", and a row for each emulator (`<emulator> games`) |
| "SYSTEM" | Only when Nexus is the whole screen: "Internet", "Sound", "Bluetooth", "Nexus OS updates", "Install Nexus OS on this PC", "Log out", "Restart", "Shut down" |
| "ABOUT" | The emblem, "Nexus", the counts of plugins and emulators, and "Games from every emulator, in one place" |

Notes by row:

- "Nexus updates": the button follows `Vm.launcher-update.state`. It checks, downloads, or
  restarts (`Host.restart`). Commands: `LauncherUpdate` (`Check`, `Download`).
- "Size of everything": ranges from 80% to 140% in steps of 10%. `Enter` goes round; Left and
  Right stop at the ends. Command: `Pref` `scale`.
- "Back to the launcher": "When a game closes, the launcher comes to the front"
  (`return_on_exit`).
- "Start at login": the hint is "Nexus opens when you log in", or the system's error
  (`Vm.start-at-login-error`) in red. The pref write fails in a build that is not installed,
  so the row is hidden there.
- "Quit Nexus": "Restart Nexus" when Nexus is the whole screen (the session starts it again).
  It calls `Host.quit`.
- "Download only through a VPN": `Vpn` `SetOnly`. Each VPN row connects or disconnects
  (`Vpn` `Connect`, `Disconnect`) where the system can switch a VPN, and otherwise picks the
  network the VPN uses (`Vpn` `Choose`: "Use" or "Picked"). "Add a VPN" takes the path of a
  WireGuard `.conf` or OpenVPN `.ovpn` file (`Vpn` `Import`). Without a VPN, a row "Your VPN"
  says "None found" or "No network found".
- "Keep sharing finished downloads": "Other players download from you, at most 128 KB/s".
  Command: `Torrent` `SeedDownloads`.
- "THIS PC": the rows only show. They take the focus so a pad can scroll to them. Before the
  check ends, one row says "Checking this PC…".
- "Log out", "Restart" and "Shut down" ask twice: the first press arms the row ("Press again to
  log out", "Press again to restart", "Press again to shut down"), the second acts. "Log out"
  calls `Host.log-out` and shows on a desktop's Nexus session, not on Nexus OS.
- Folder rows: `SetFolders` with the name and the paths.
- "Rescan game folders": `Library` `Refresh`.

### Emulators, Themes and plugin pages

Emulators and Themes (`screens/emulators.slint`, `screens/themes.slint`, both use
`components/plugin-list.slint`) are side sheets that list plugins. The Emulators sheet is titled
"Emulators" and its subtitle is "Open one to turn it on or off, install it and change its
settings". The Themes sheet is titled "Themes" and its subtitle is "Open one to use it and
change its options". Both list under "AVAILABLE": the plugin's name, its build or version, and
its status, or "Off". Up and Down move; `Enter` opens the plugin (`Navigate` to `Plugin`).

The plugin's page (`screens/plugin.slint`) opens at the edge beside the list, which stays and
moves over. Its sections:

- A theme: "THEME" with "Use this theme" (`Host.switch-theme`), then "OPTIONS" while it is the
  one in use, else "Use it to change its options".
- An emulator or a service: "PLUGIN" with the plugin's note and "Turned on"
  (`SetPluginEnabled`), then "BUILD" or "ACTIONS", then "SETTINGS" (one panel for each slot),
  "FOLDERS", and the plugin's blocks (`SlotSections`). An emulator shows "Controllers: " and
  the controllers it supports.
- The build row reads "Install" (`InstallEmulator`), "Update to <version>" (`UpdateEmulator`),
  or "<version> · Up to date", or "Working…". "Uninstall <version>" removes the build
  (`UninstallEmulator`). The plugin's actions run with `RunAction`.
- A plugin that is turned off shows only the switch, with the hint "Turn it on to see its
  settings".

### Running

`screens/running.slint` (`RunningScreen`) is the launch splash. It shows while
`Vm.running.state` is `starting`: the game's art dimmed, its icon, the title (`Vm.running.title`),
"Starting with <emulator>…" (or "Starting…"), and a turning ring. When the game's window shows,
it fades. Home is under it, with the keys on Resume.

A line names the input: "Controller: <name>" when a pad is connected (`Vm.pad-name`), else
"Keyboard: the keys are on the <page> page" (or "the emulator's page").

Without a controller, when the emulator names its keyboard keys (`Vm.running.keymap`), the splash
lists them under "Keyboard" in two columns. The keys are the game's then, so the shell takes
every key except `Escape` (Back) and `Ctrl+Q`.

### Power menu

`components/power-menu.slint` opens over any screen with `Ctrl+P`, or when the PS or Xbox
button is held with no game running. The items are:

- "Log out": when Nexus is the whole screen and not Nexus OS.
- "Quit Nexus", or "Restart Nexus" when Nexus is the whole screen.
- "Restart" and "Shut down": when Nexus is the whole screen and the power calls are available
  (`Vm.system.power-available`). These ask twice, as in Settings.

Up and Down move, `Enter` picks, `Escape` and `Backspace` close.

### Result prompt

After a game closes, the core may ask "How far did it get?" (`Vm.has-result-prompt`). The prompt
reads `How far did <game> get?` and lists:

- "In game: reaches gameplay" (`ingame`)
- "Main menu, but not gameplay" (`menus`)
- "Intro logos, then stops" (`boots`)
- "Doesn't start" (`nothing`)
- "Not now"

`Enter` answers with `ReportResult`. `Escape` is "Not now". The same prompt opens from the
game's menu with "Rate how it runs" (`AskResult`).

### Install prompt

A first play can ask before the emulator downloads. The prompt (`Vm.has-install-prompt`) reads
`Install <emulator>?` and "To play <game>, <emulator> <version> downloads first<size>. It checks
the download, then the game starts.". The buttons are "Install and play" and "Not now".
`Enter` runs the focused one (`ConfirmInstall` or `CancelInstall`), and `Escape` cancels. It
takes every key. The keys start on "Install and play".

### Trailer player

`Trailers.watch` plays the shown game's trailer. If the app can play video
(`Vm.can-play-trailers`), `Host.play-trailer` plays it in the window. If not, `Host.open-url`
opens it in the browser.

In the window: `Enter` or `Space` pauses or goes on (`Host.pause-trailer`). Left and right seek
back or on (`Host.seek-trailer`). `Escape` or `Backspace` closes it (`Host.close-trailer`). The
screen shows "Loading the trailer…", the game's title, the time as "m:ss / m:ss" and a bar. The
controls hide while it plays and no key comes for a while; any key or the pointer shows them. They
stay while it is paused.

### Notices

Notices stack on the far side from the reading start, oldest at the top. The stack shows at
most the three the core keeps (`Vm.notice-cards`). A card shows the game's art or the sign of
its kind (info, success, error), the text, a "Dismiss" button, and, when the notice has a
subject, a link that opens it (`Notice` `Open`, shown as the subject with " ›").

A card slides in, stays, and slides out, and then the theme sends `Notice` `Dismiss`. A notice
with something to open stays longer than one without. `Delete` closes the newest card that is
not closing yet. A new notice of kind `error` plays the sound `error`.

### System panels

When Nexus is the whole screen, the top bar has the network, sound and Bluetooth icons. Each
opens a panel under it (`components/system-panel.slint`, `Sys`). Left and right go to the panel
beside. Up and Down move over rows. `Enter` or `Space` acts. `Escape` closes.

| Panel | Rows | Commands |
| --- | --- | --- |
| Network | "Look for networks", then one row for each Wi-Fi network | `System` `ScanWifi`, `JoinWifi`; `Delete` forgets a saved network (`ForgetWifi`) |
| Sound | The volume (left and right change it), mute, then one row for each output | `SetVolume`, `SetMuted`, `SetOutput` |
| Bluetooth | On or off, "Pair a device", then one row for each device | `SetBluetooth`, `ScanBluetooth`, `Pair`, `ConnectDevice`, `DisconnectDevice`; `Delete` removes a device (`RemoveDevice`) |

A secured network that is not saved asks for its password first, with the on-screen keyboard.
When a part is not available, the panel shows why (`network-reason`, `audio-reason`,
`bluetooth-reason`).

### The on-screen keyboard

`components/keyboard.slint` (`Osk`) is for a controller. Every text field in the theme is a
`TextField`. It opens the keyboard when the field gets the focus with the pad and the last key
was the pad's. A physical keyboard types straight into the field and the on-screen keyboard
stays shut.

It has rows of ten keys, a numbers row, three letter rows (or the symbols) and an action row:
"Shift", "Symbols" (or "ABC"), "Space", "Delete", "Show" or "Hide" (for a password only) and
"Done".

On a pad: the d-pad moves, Cross presses the key, Square deletes, Triangle adds a space, L1 is
Shift for one letter, R1 switches to the symbols, R2 or Options is Done, and Circle closes and
keeps the text. On a keyboard, `Esc` closes and `Tab` shows a password.

### Nexus OS installer

When Nexus runs from the Nexus OS USB stick (`Vm.system.stick`), `components/install-os.slint`
(`Setup`) shows before anything else, after Welcome. Its steps are `welcome`, `disks`,
`confirm`, `installing`, `done` and `failed`.

- Welcome: "Welcome to Nexus OS", with "Try it" and "Install on this PC".
- Disks: "Where should Nexus OS go?", one row for each disk, "Choose partitions instead"
  (`System` `ChoosePartitions`), and Back. With no disk of enough size: "No disk of 40 GB or
  more is free to install on.".
- Confirm: `Erase <model> (<size>)?` and "Erase and install" (`System` `InstallOs`).
- Installing: "Installing Nexus OS". No keys; it cannot stop halfway.
- Done: "Nexus OS is installed" and Restart (`System` `Restart`).
- Failed: "Installation didn't finish" and Back.

## Accessible labels

The seam tests drive the theme only through accessible labels, values, keys and clicks. The
labels are part of what the tests pin, so keep them when you change a screen. Examples:

| Area | Labels |
| --- | --- |
| Boot | `boot`, `welcome`, `welcome-step`, `welcome-start` |
| Home | `default-tile`, `default-library-tile`, `default-title`, `default-sub`, `default-description`, `default-resume`, `default-stop`, `game-options` |
| Library | `default-game`, `library-sort`, `density`, `grid-empty`, `grid-progress`, `genres`, `default-count`, `catalog-updated`, `filter:<name>` |
| Game Hub | `hub-cover`, `hub-title`, `hub-stop`, `hub-store`, `hub-trailer`, `hub-hide`, `hub-about`, `release-<id>`, `release-download`, `download-title`, `download-agree`, `download-not-now`, `download-folder` |
| Downloads | `torrent`, `torrent-state`, `torrent-action:<action>`, `torrent-open-folder`, `downloads-summary`, `install-agree`, `remove-title` |
| Settings | `rescan`, `auto-update`, `fullscreen`, `scale`, `start-at-login`, `monitor`, `vpn-only`, `seed-downloads`, `folder:<name>` |
| Plugins | `plugin:<id>`, `plugin-enabled`, `build`, `uninstall`, `theme-current`, `option:<key>` |
| Other | `launch-title`, `launch-input`, `result-prompt`, `install-prompt`, `notice`, `system-panel`, `status:network` |

## Tests

The tests of the theme are in `tests/tests/seams/`:

- `default_theme/` has one module for each area: `home`, `library`, `catalog`, `game_hub`,
  `releases`, `downloads`, `installs`, `emulators`, `settings`, `keyboard`, `notices`,
  `trailer`, `play` and `theme_files`. Each drives the shown theme with keys, clicks and
  accessible labels (the theme driver), against fake plugins.
- `theme_files.rs` checks two rules of the files. Every font a file names ships in `fonts/`.
  Every `duration:` and `delay:` time goes through `Look.timed`, so "Animations: Off" cuts it.
- `screenshots.rs` renders the screens off screen with the software renderer and writes
  pictures. It checks that the pictures of Home, Library, Game Hub, Settings and Running are
  not blank. It renders the default screens right to left, and renders a child theme
  (`fixture-grid`) over the same library.

For how to run the tests, read [../development.md](../development.md).

## Limitations

- The theme reads no breakpoint. The `compact` and `regular` breakpoints in the manifest have
  no effect on the default theme's layout.
- The "Hide" and "Show" button on the Game Hub is for the pointer only, outside the key order.
  A pad uses the "Hide from the Catalog" item of the game's menu.
- The Welcome step labels and details, the boot status text, and the notice texts come from
  the core, not from the theme. This page does not list them.
- I did not run the theme or look at a screenshot for this page. Every statement comes from
  reading the files and the tests.
