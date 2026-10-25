# The default theme

This page describes the default theme as it is today: its screens, what each shows, how the
keys act, and its options. The theme is in `plugins/themes/default/theme/`. It follows the
PlayStation 5 home screen look, and every other theme falls back to it. How themes work, and
the contract between a theme and the host, are in [themes.md](../themes.md). The desktop side
(the pad's focus rule, the PS button, full screen) is in [desktop.md](desktop.md). Downloads and
installs are in [archives-and-pc-games.md](archives-and-pc-games.md).

## Files

| Path | What it holds |
|---|---|
| `theme.toml` | The manifest: screens, input, breakpoints, fonts, images, sounds, tokens, options |
| `shell.slint` | The window, the screens, the overlays, the key handler and the key hints |
| `screens/*.slint` | `home`, `catalog`, `game-detail`, `downloads`, `settings`, `emulators`, `themes`, `plugin`, `game-settings`, `running` |
| `components/nav.slint` | The keys on Home, the Library and the Catalog (`Nav`) |
| `components/look.slint` | Shared sizes, colours and words (`Look`) |
| `components/*.slint` | Top bar, library view, cards, notices, boot screen, game menu, prompts, trailer player, settings rows |
| `fonts/` | Inter (Regular, Medium, SemiBold, Bold) and Tajawal (Regular, Bold), with their OFL licences |
| `sounds/` | `move`, `select`, `back`, `error`, `start` |
| `images/logo.png` | The Nexus emblem, the `logo` asset |

The window is titled "Nexus". Its preferred size is 1280 × 720, and it is at least 640 × 360.

## Screens

The screens are `Vm.screen` values: `home`, `game-detail` (the Game Hub), `catalog`,
`downloads`, `settings`, `emulators`, `themes`, `plugin`, `game-settings` and `running`. Home
stays in the tree under every other screen.

### Boot screen and Welcome

The boot screen (`components/boot.slint`, label `boot`) covers the start until the library is
in and Home's art is loaded: it shows while `!(Vm.boot.ready && Vm.art-ready)`. The UI host sets
`Vm.art-ready` once every image the screen wants is loaded or failed, or 5 s after the core is
ready, whichever comes first. It never goes back to false. The status reads "Loading...", then
"All set · N installed games found". The boot screen fades out over 600 ms.

On the first run it is the Welcome screen (label `welcome`): "Welcome", "Setting up Nexus", a
checklist, a bar with a percentage, and a Start pill (`welcome-start`). The checklist steps
(`welcome-step`) are:

| Step | Detail |
|---|---|
| "Game folders" | "Looking…", then "N games" |
| "<Emulator> emulator" | "Your own build", "Installs on first play", "Update available" or "Latest build" |
| "Official artwork & details" | "done/total", then "Done" |
| "Covers & Home screen" | "Loading…", then "Ready" |

The pill reads "Setting up…" until everything is ready, then "Start". Enter, Cross or a click
is Start once ready. Every other key is ignored. Start calls `Host.finish-welcome`, and the
Welcome never shows again.

### Home

Home (`screens/home.slint`) has a top bar, a row of tiles, and the selected game's hero over its
background art.

- **Row.** Every game, the selected tile larger (172 px against 112 px at 1080p), then a Library
  tile. The sub line under the selected tile (`default-sub`) reads "Playing now", "Last played
  today", "Last played yesterday", "Last played 3 days ago" or "Not played yet".
- **Hero.** A kicker ("PLAYED <time>", or "● PLAYING · <elapsed>" in green), the logo or title,
  chips (the player's result as "<result> · your result", the plugin's badges, the rating, two
  genres, the size, the release date), and one line about the game (`default-description`), cut
  after a word at 75 characters with "…".
- **Buttons.** "Play" (when the plugin can launch the game), "Game Hub", "Store" and "Options"
  (`game-options`, the ⋯ button). While the game runs: "Resume" (`default-resume`) and "Stop"
  (`default-stop`, red). When the emulator isn't installed, a line after the buttons says "Play
  installs <emulator> first", and Play opens the install prompt.
- **Library tile.** The hero says "Library", "N games" and "Every game, in a grid, by system.".
  With no games: "No games yet" and "Add a games folder in Settings, or install an emulator.".
- **Task pill** (`task`), bottom corner, Home only: work going on, such as fetching art.

### Top bar

From the near side: the tabs "Home" and "Library" (`default-system`; Library shows the game
count). On the far side: a pill for the running game ("Resume", "Stop"), a pill for an active
emulator download, round buttons "Search" (the Catalog), "Downloads", "Emulators", "Themes" and
"Settings", then the clock (`clock`), as "9:38 PM".

### Library and Catalog

The Library is Home's second tab. The Catalog is its own screen. Both are the same component
(`components/library-view.slint`), on `Vm.home.library` and `Vm.catalog`. The core searches,
filters, sorts and pages them.

- **Bar.** A search box ("Search games"), Sort (`library-sort`), and the card size: "Comfortable"
  (or "Roomy" when the view is under 1000 px wide) and "Compact" (`density`). In the Catalog the
  catalog plugin's first button follows, such as Refresh, with its last result beside it.
- **Keys.** Up from the grid's first row goes to the genres, then the systems, then the bar;
  Left and Right move along a row, Enter chooses, Escape goes back. The focused chip or button
  has the cards' white ring; a chip that is on is white. While the keys are
  on these rows the hint pill says so: "Move", "Choose", "Rows", "Back".
- **Systems.** "All", each platform, then "In-game" when a game reaches gameplay (by the
  player's own result first, then the community's), and in the Catalog "Installed", and
  "Hidden" once a game is hidden: it shows only the hidden games. The Game Catalog's "Show the
  Hidden filter" setting turns it off. Labels `filter:<label>`.
- **The eye** (`card-hide`, valued "Hide <title>" or "Show <title>"). On a Catalog card's cover,
  top left: on the focused or pointed card, and always on a hidden one, crossed out. It hides
  the game from the Catalog, or shows it again (`Host.hide-game`). The Game Hub of a Catalog game
  has the same as a "Hide" or "Show" button (`hub-hide`), and its menu has "Hide from the
  Catalog" or "Show in the Catalog", the pad's way.
- **The card's button**, on the focused or pointed card, above its badges: "Uninstall"
  (`card-uninstall`, valued "Uninstall <title>") when `GameCard.uninstall` names a game, which
  asks "Uninstall <title>?" as the game's menu does (`GameMenu.ask-uninstall`); else, on a
  Catalog card, "Download" (`card-download`), which opens the Game Hub with the keys on its
  first release.
- **Genres.** "GENRE", "All genres", then each genre. The row scrolls sideways; a fading edge
  means more chips past it (`genres-more-start`, `genres-more-end`).
- **Count** (`default-count`). The Library: "N games". The Catalog: "2 games · 3 releases", or
  "1 of 2 games · 3 releases" while a search or filter hides some. A release is one download
  (a version, a region, an update), so a game can have several. Hidden games count in neither. Under it, "List updated Oct
  19, 2026" (`catalog-updated`).
- **Grid.** Cards (`default-game`) with the cover, a console badge (`card-console`), the size
  (`card-size`), and in the Catalog the release count (`card-releases`, "2 releases") and
  "Availability: Good" (`card-availability`: Good in green, Low in amber, None and Unknown in
  grey). A card whose every release can't run on this computer has the badge "Can't run here".
- **Empty grid** (`grid-empty`): "Loading catalog…" while the catalogs haven't answered, "No game
  matches "q"", "No catalog lists any game yet", or "No games here".

Sorts:

| View | Sorts |
|---|---|
| Library | "Title", "Recently played", "Size (largest first)", "Top rated", "Newest", "Runs best" |
| Catalog | "Recently added", "Title", "Size (largest first)", "Size (smallest first)", "Top rated", "Newest", "Runs best" |

The sort list opens under Sort ("SORT BY", options `sort-option`, a tick on `sort-current`).

The search runs 300 ms after the last key in the box. A letter or digit typed in the grid starts
a search with it at once. A grid asks for the next page (`Host.library-more`,
`Host.catalog-more`) when it scrolls within two screens of the end, or the selection reaches
the last two rows. Only the rows on screen and a screen around them build their cards; the
others keep an empty place (`default-game`), so a long grid scrolls as fast as a short one.
A thin bar under the header (`grid-progress`, valued "35%") shows how far down the grid the
screen goes, against every game the search and filters leave, not only the loaded pages; only
for a grid longer than the screen.
Covers load for the cards on screen and one screen around them. Each grid
keeps how far it is scrolled (`Nav.catalog-scroll`, `Nav.library-scroll`): back from a game's
details, the grid is where it was, not at the top.

**When no catalog is available** (`Vm.catalog-available` is false, for example with the Game
Catalog plugin turned off), there is no Catalog: the top bar has no "Search" button, the keys
step over its place, R1 from Home goes to Downloads, and L1 from Downloads goes Home. The core
refuses to open the Catalog. "/" still opens the Library's search.

### Game Hub

The Game Hub (`screens/game-detail.slint`) is one game's page. Its cover (`hub-cover`) grows
from the card or tile it opened from, or fades in when that place isn't known (the value is
`grows` or `fades`).

- Kicker ("PS5 · <developer> · <year> · <region>"), logo or title (`hub-title`), chips.
- Buttons: "Play" or "Resume", "Stop" (`hub-stop`), the emulator's own actions, "Store"
  (`hub-store`, opens the store page in the browser), "Trailer" (`hub-trailer`) and "Options"
  (`game-options`). The launch hint goes under them.
- **Releases** (for a game a catalog offers): "RELEASES · RUNS WITH <EMULATOR>", one row per
  release (`release-<id>`): version, region and, for a PC release, its system; its place by
  version, size and when the catalog got it (`release-version`); "Availability:"
  (`release-availability`) and "5 seeders · 1 leecher" (`release-peers`); why it can't run here
  (`release-not-here`) or a warning (`release-warning`), in the warning colour. Its button
  (`release-download`) reads "Download", "Downloading N%", "Downloaded", "Paused", "Failed" or "In
  Downloads".
- **Fact cards**, three to a row: "COMPATIBILITY · <TESTED>" or "YOUR RESULT"; "ON THIS PC · <WHY>"
  (`on-this-pc`, green good, warning colour slow, red no); "PLAYED"; "SIZE"; "PROTONDB"
  (`protondb`); "AVAILABILITY" (`hub-availability`). A card the game doesn't have is left out.
- "+ Technical details" (`technical-details`): rows with the fact's name (`fact-name:<id>`) and
  value (`fact:<id>`), then the shader cache with "Clear", "SCREENSHOTS" (`screenshot`), the
  plugin's sections, and "ABOUT" (`hub-about`).
- A screenshot opens full screen. Left and Right move; Escape, Backspace or Enter close it.

### The game's menu

Options on a pad, the Menu key, or the "Options" button opens the menu for the game shown on
Home or in the Game Hub (`components/game-menu.slint`). Items, in order, each left out when the
game doesn't have it:

1. "Game Hub" (not in the Game Hub)
2. the emulator's own actions
3. "Game settings"
4. "Open game folder"
5. "View emulator log"
6. "Watch trailer"
7. "Rate how it runs"
8. "Compatibility list"
9. "ProtonDB page"
10. "Settings"
11. "Uninstall", in red, when `Vm.detail.can-uninstall`

When Nexus is the whole screen (the Nexus session, Nexus OS), the menu leaves out "Open game
folder", "View emulator log", "Compatibility list" and "ProtonDB page": no file manager or
browser is on screen to open them in.

Rows are labelled `game-menu`. The focus opens on the first item off the Game Hub, and on the
second in it. "Uninstall" asks in the same card: "Uninstall <title>?" (`uninstall-title`), "Its
folder is deleted from your game folder. A download it came from stays in Downloads.", then
"Uninstall" and "Not now" (`uninstall-choice`), with the focus on "Not now". The rules are in
[archives-and-pc-games.md](archives-and-pc-games.md#uninstalling-a-game).

"Rate how it runs" opens the result prompt for that game. Open game folder and View emulator
log name the game, never a path: the host resolves the folder through the plugin's grants.

### Game settings

"Game settings" opens a side sheet (`screens/game-settings.slint`) with the game's title
(`game-settings-title`), "THIS GAME", and the game's own settings from its emulator
(`setting:<key>`). A choice starts with "As the emulator", which leaves it to the emulator's
setting.

### Downloads

Downloads (`screens/downloads.slint`) lists the catalogs' downloads, then the emulator
downloads.

- **Summary** (`downloads-summary`): "Protected by your VPN · <interface>" or "Waiting for your
  VPN" (with the VPN rule on), then "X downloading · Y sharing · Z in all".
- **Note** (`downloads-note`): transfers go on while Nexus is open, also minimized, and stop when
  it closes; unfinished ones come back paused; and how finished ones share.
- **Torrent cards** (`torrent`): title, version and region; the state (`torrent-state`); a bar;
  one line with sizes, speed, peers and the time left ("12 min left"); while it waits for the
  player, its files (`torrent-file-summary`, `torrent-files`, up to 100, "first 100 shown"); the
  folder; and the error.
- **Footer** (`downloads-footer`): "Cancel never deletes your downloaded files, and Remove asks
  first whether they go too. Remove unpacked files deletes only the copies Nexus unpacked."
- **Empty**: "Nothing downloading yet".

Torrent states as the card reads them: "Reading its files", "Ready: look at the files, then
download", "Checking", "Downloading", "Paused", "Paused · waiting for your VPN", "Complete",
"Complete · sharing", "Complete · checking", "Unpacking", "Unpacking failed", "Installing",
"Installed", "Failed", "Cancelled · the files stay".

Buttons (`torrent-action:<action>`), as the core offers them:

| Action | Label |
|---|---|
| `start` | "Download", "Resume" or "Retry" |
| `pause` | "Pause" |
| `cancel` | "Cancel", or "Stop unpacking" while unpacking |
| `remove` | "Remove" (opens the remove sheet) |
| `stop-seeding` | "Stop sharing" |
| `install` | "Install", "Retry install" or "Unpack and install" (opens the install sheet) |
| `remove-unpacked` | "Remove unpacked files" |
| `cancel-install` | "Cancel install" |
| `play` | "Play" |

"Open folder" (`torrent-open-folder`) comes last, once the download has started. While a
download reads its files or installs, a first button reads "Resolving…" or "Installing…"
(`torrent-placeholder`), does nothing, and the keys skip it.

Emulator downloads say their state in words (`download-state`): "Waiting", "Downloading",
"Checking", "Unpacking", "Paused", "Installing", "Done", "Failed" or "Cancelled". Their buttons
are "Resume", "Pause" and "Cancel".

### Settings

Settings (`screens/settings.slint`) is a side sheet: "Settings", "Changes are saved
automatically". Sections, in order:

| Section | Rows |
|---|---|
| GAMES | One row per folder (`folder:<name>`, with `~` for the home folder), "Rescan game folders" (`rescan`) |
| UPDATES | "Update automatically" (`auto-update`), "Nexus updates" (`launcher-update`: check, download, restart) |
| LAUNCHER | "Full screen", "Size of everything" (80% to 140%), "Back to the launcher", "Interface sounds", "Start at login" (installed builds only), "Screen" (two monitors or more), "Quit Nexus" (`quit`, red, hint "Ctrl+Q"), or "Restart Nexus" (`restart-nexus`) when Nexus is the whole screen, where the session starts it again |
| DOWNLOADS | "Download only through a VPN" (`vpn-only`), one row per VPN (`vpn`) or "Your VPN" (`vpn-none`), "Add a VPN" (`vpn-import`, Linux with NetworkManager), "Keep sharing finished downloads" (`seed-downloads`) |
| THIS PC | "Checking this PC…", then "Processor", "Memory", one "Graphics" row per card, and one row per emulator ("KytyPS5 games: May run slowly · Integrated GPU") |
| SYSTEM | When Nexus is the whole screen: "Internet", "Sound", "Bluetooth", Nexus OS's updates, "Install Nexus OS on this PC" (its USB stick), "Log out" (`system-log-out`, the Nexus session on a desktop only: Nexus OS's session would start again), "Restart" (`system-restart`) and "Shut down" (`system-shut-down`). Log out, Restart and Shut down ask again: the first press arms them, the second acts. |
| ABOUT | The logo, "Nexus", "N plugins · M emulators", "Games from every emulator, in one place" |

"Size of everything" moves a tenth at a time. Left and Right stop at the ends; Enter goes round.
When `NEXUS_SCALE` sets the size for the run, its hint says "NEXUS_SCALE sets it for this run".
A refused "Start at login" keeps its toggle and shows the reason in red.

### Emulators, Themes and plugin pages

Emulators and Themes (`components/plugin-list.slint`) list the plugins under "AVAILABLE"
(`plugin:<id>`). An emulator's hint is its build, "Installed · shadPS4 v.0.20.0", or "Not
installed", or "No install needed" (PC). Enter opens the plugin's page.

A plugin's page (`screens/plugin.slint`):

- a theme: "Use this theme" (`theme-current`), then "OPTIONS" (`option:<key>`) while it is the
  one in use;
- an emulator or service: "Turned on" (`plugin-enabled`), its build ("Install", "Update to x",
  "x · Up to date") and "Uninstall <version>", its actions, "SETTINGS", "FOLDERS", and its
  blocks (`plugin-blocks`).

### Running: the launch splash

While a game starts (`Vm.running.state` is `starting`), the splash (`screens/running.slint`)
shows the game's art, "Starting with <emulator>…" (`launch-title`), and what the game gets
(`launch-input`): "Controller: DualSense Wireless Controller", or "Keyboard: the keys are on
the <page> page". Without a controller, when the emulator lists its keys, they show under
"Keyboard" in two columns (`launch-key`). The keys are then the game's: the shell takes every key
but Escape and Ctrl+Q, shows no hints, and does nothing with them.

### Result prompt

When a game closes, once for each emulator build, "How far did <game> get?" (`result-prompt`)
offers "In game: reaches gameplay", "Main menu, but not gameplay", "Intro logos, then stops",
"Doesn't start" and "Not now" (`result:<level>`). Escape, or a click outside, is "Not now".

### Trailer player

Square, T, "Trailer" or "Watch trailer" plays the trailer full screen in the window. Enter (or
Cross) pauses or plays, Left and Right go 10 s back or on, Escape (or Circle) closes. The game's
name is at the top (`trailer-title`), the time at the bottom (`trailer-time`, "m:ss / m:ss").
The controls and hints fade about 3 s after the last key while it plays; a paused trailer keeps
them, with a large play sign (`trailer-paused`). Without libmpv (`Vm.can-play-trailers` false)
the trailer opens in the browser.

## Keys

### Pad buttons

The UI host turns each pad button into a key (`crates/nexus-ui-host/src/input.rs`), so the theme
handles a keyboard and a pad the same way.

| Pad | Key | What it does |
|---|---|---|
| D-pad, left stick | Arrows | Move |
| Cross | Enter | Select, press |
| Circle | Escape | Back, close |
| Triangle | `/` | Search the Library, or the Catalog on its screen |
| Square | Insert | Play the shown game's trailer |
| Options | Menu | The game's menu |
| L1 | Shift+Tab | The screen before |
| R1 | Tab | The screen after |
| L2, R2 | Page Up, Page Down | Page the grid, three rows at a time |
| Create, Share or View | Delete | Close the newest notice |
| PS, Xbox | (none) | With a game running, switch between the game and the launcher; otherwise Home |

The pad acts only while the launcher's window has the focus and no game is in front. See
[desktop.md](desktop.md#the-pad-acts-only-while-the-launchers-window-has-focus).

### Keyboard

| Key | What it does |
|---|---|
| Enter, Space (in sheets, lists, prompts, the trailer) | Select, press |
| Escape, Backspace | Back, close |
| Arrows | Move; Left and Right follow the reading direction |
| Tab, Shift+Tab | The screen after, the screen before |
| Menu | The game's menu |
| `/` | Search |
| T | The trailer, outside the Library and the Catalog (where T types) |
| Ctrl+D | Downloads |
| Ctrl+Q | Quit |
| Page Up, Page Down | Page the grid |
| Delete | Close the newest notice |
| A letter or digit in the grid | Start a search with it |

A held Enter doesn't repeat, so it can't play, download or install twice.

### The order of the screens

Tab and R1 go Home or Library → Catalog → Downloads → Emulators → Themes → Settings → Home.
Shift+Tab and L1 go the other way from Home: Settings → Themes → Emulators → Downloads → Catalog
→ Home. From any other screen (the Game Hub, a plugin page) both go Home. Without a catalog the
Catalog is skipped. Neither works while a text row is open for typing.

### Who takes a key

The shell (`shell.slint`) offers each key in this order, and the first taker keeps it:

1. the Welcome screen;
2. the splash's game keys;
3. Delete, while a notice shows;
4. the trailer, the result prompt, the game's menu, the screenshot viewer, the sort list;
5. Menu, for the game's menu;
6. the emulator's install prompt;
7. Ctrl+Q, Ctrl+D, `/`, Insert or T;
8. Tab and Shift+Tab;
9. the screen: `Nav` (Home, Library, Catalog), the sheet rows, the plugin list, the Game Hub,
   Downloads;
10. Escape or Backspace: back, with the `back` sound unless on Home.

So Ctrl+D and Ctrl+Q do nothing while a menu, the viewer, the sort list, the trailer or the
result prompt is open.

### Zones on Home, the Library and the Catalog

`Nav` (`components/nav.slint`) acts on one zone at a time.

| Zone | Where | Left, Right | Up | Down | Enter |
|---|---|---|---|---|---|
| `tabs` | The top bar | Step over the tabs and buttons | | The bar (Library) or the row | Open the tab or screen |
| `row` | Home's tiles | Step; after the last game, the Library tile | The tabs | The buttons | Open the Game Hub, or the Library from its tile |
| `actions` | Home's buttons | Step | The row | | Press the button |
| `bar` | Search, Sort, card sizes, catalog button | Step | The tabs (Library only) | The systems | Type, open the sorts, set the size, run the action |
| `filters` | Systems | Step | The bar | The genres, or the grid | Apply |
| `genres` | Genres | Step | The systems | The grid | Apply |
| `grid` | Cards | Step | A row up; from the first, the genres or systems | A row down | Open the Game Hub |

Escape from the Library's zones goes back to Home; in the Catalog it goes back. While the search
box has the keys, Enter goes to the grid and Escape or Down goes back to the bar. While the sort
list is open it takes every key: Up and Down move round it, Enter picks, Escape closes with no
change.

### Other screens

- **Game Hub.** Left and Right move over the buttons. Down goes to the releases, then opens the
  technical details, then the screenshots and the text below. Up goes back. Enter on a
  screenshot opens it.
- **Downloads** (`Dl`). Up and Down move between cards, Left and Right between a card's buttons,
  with "Open folder" last. Enter presses the focused button, the same call a click makes; a click
  also moves the focus there. When a card's buttons get fewer, the focus stays on its last one.
  The list scrolls to keep the focused card in view.
- **Sheets** (Settings, plugin pages, game settings). Up and Down move between rows, across
  sections. Left and Right change a value. Enter changes or opens a row. A text row opens for
  typing; Enter keeps the text, Escape drops it.
- **Game menu.** Up and Down skip the items that aren't there. Escape, Backspace or Menu close
  it. In the Uninstall question, any arrow moves between the two answers, and Escape or Menu is
  "Not now".

## Focus

| Element | Focused look |
|---|---|
| Home tile, Library and Catalog card | A white 4 px ring with a blue glow; a card also grows to 1.05 |
| Home tile | The selected one is larger |
| Pill button | Turns white (red for danger), grows to 1.05, glows |
| Button | A 3 px border: the accent on a white button, white on others |
| Downloads card | A white 2 px border; its focused button has the ring |
| Top bar tab | A white rounded outline; a round button turns white |
| System or genre chip | A 3 px border, white, or the accent on the active chip |
| Search box | A white border and a blue glow |
| Settings row, release row | A 3 px white border (red for danger) |
| Screenshot | A white border, grown to 1.04 |
| Menu, sort and sheet answers | A white fill (red for danger) |

The ring colour is `Look.ring` (white).

## Key hints

A pill 30 px from the bottom shows the keys that act now (each hint labelled `hint`). It sits
in the far corner on screens and in the near corner over side sheets, mirrored right to left.
It hides over the Library and Catalog views (unless a menu or sheet is open), over a playing
trailer whose controls have faded, and over the splash's game keys.

The hints draw the pad's signs: ✕ (blue), ○ (pink), △, □, ☰, "▲ ▼", "◀ ▶" and "R1". Each hint
also carries a keyboard name ("Enter", "Esc", "Tab", "T", "Menu", "← →", "↑ ↓"), shown when
`Vm.input-mode` isn't `gamepad`. The default theme starts in `gamepad` mode (the first of its
`input` list), and only a pad press sets the mode, so the pad signs show.

| Where | Hints |
|---|---|
| Home, top bar | "Select", "Next" |
| Home | "Select", "Search" (R1; "Downloads" with no Catalog), "Trailer" (when there is one), "Options" (with a game) |
| Game Hub | "Play", "View" (on a screenshot) or "Select"; "Back"; "Trailer"; "Options" |
| Download sheet | "Look up metadata", "Not now" or "Change"; "Not now" |
| Downloads | "Move", "Buttons", "Select", "Back" |
| Install sheet | "Install", "Not now" or "Change"; "Not now" |
| Remove sheet | "Choose", then "Remove", "Delete files" or "Not now"; "Not now" |
| Install prompt | "Install and play" or "Not now"; "Not now" |
| Game menu, sort list | "Pick", "Close" |
| Trailer | "Pause" or "Play", "10 s", "Close" |
| Screenshot viewer | "Move", "Close" |
| Side sheets | "Change" (or "Open" in Emulators and Themes), "Back" |
| Elsewhere | "Next", "Back" |

The result prompt shows no hints of its own; the screen's hints stay.

## Notices

Notices (`components/notices.slint`, `Vm.notice-cards`) stack on the far side, oldest at the
top. There are at most 3: the core drops the oldest when a fourth comes.

| Kind | Sign | Colour | Used for |
|---|---|---|---|
| `error` | ! in a circle | red, `color-danger` #ff5a6a | A download that stopped, a failed install, update or launch, a game that crashed, an emulator this PC can't run, a problem the app reports |
| `success` | a tick | green, `color-success` #3ddc97 | A finished download |
| `info` | i in a circle | blue, `color-info` #7ab4ff | The rest, such as a missing desktop tool |

A notice about a game shows the game's art in place of the sign. A notice with a subject has a
link (`notice-open`): "Open Downloads", "Open the Catalog", "Open the Game Hub", "Open <plugin>"
or "Open Settings", with " ›" (" ‹" right to left). The link opens it and closes the notice.

Each card slides in, stays 4 s (10 s with a link), slides out, and the core closes it
(`Host.dismiss-notice`). "Dismiss" starts the slide out at once. Delete, or Create on a pad,
closes the newest card that isn't closing yet; the others keep their own time. A new error
notice plays the `error` sound once.

## Sheets and prompts

Side sheets (Settings, Emulators, Themes, plugin pages, game settings) slide in from the far
edge and back out. Each stays in the tree for the slide time after it closes (`Lingering`).
Home dims under a sheet. Dialogs (the download, install and remove sheets, the prompts) rise
over a dark wash.

### Download sheet

The Game Hub's Download button opens "Set up a download" (`download-title`): the game, "Version
v · region · size · runs with <emulator>", any warning (it can't run here, a retail PS5 package,
this PC can't run it, not enough room), the "Download folder" row (`download-folder`), what the
player agrees to (`download-consent`, including the 128 KiB/s upload cap), who sees their address
(`download-privacy`, naming the VPN's interface with the VPN rule on), and the steps
(`download-steps`). Buttons: "Look up metadata" (`download-agree`, off without a packages folder)
and "Not now" (`download-not-now`); the focus starts on Look up metadata. Up from the buttons
reaches the folder row; Enter opens it for typing. When the core refuses, the sheet stays and
says why in red (`download-error`).

### Install sheet

Install on a finished download opens "Install game" (`install-title`): the title, version and
region, "Nexus copies or unpacks this finished download into your game folder. …", the "Game
folder" row (`install-folder`), what the emulator installs from (`install-note`), and "Install"
(`install-agree`) and "Not now" (`install-not-now`). The focus starts on Install. Without a game
folder: "This emulator has no game folder: add one in Settings." The sheet closes once the
install or its unpacking runs; a refusal shows in red (`install-error`).

### Remove sheet

Remove opens "Remove <title>?" (`remove-title`), the version and region, then three answers in
a column, with the focus on the first:

| Answer | Line under it |
|---|---|
| "Remove from Downloads" (`remove-keep`) | "The downloaded files stay in their folder." |
| "Delete files too" (`remove-delete`, red) | "Deletes the folder and the copy Nexus unpacked. A game you installed stays." |
| "Not now" (`remove-not-now`) | |

The folder path is under the answers. Escape or a click beside the sheet is Not now.

### Install prompt

Play for a game whose emulator isn't installed opens "Install <emulator>?" (`install-prompt`):
"To play <game>, <emulator> <version> downloads first (<size>). It checks the download, then the
game starts." Buttons: "Install and play", with the focus, and "Not now".

### Button order

In the download sheet, the install sheet and the install prompt, the button that goes on comes
first from the reading side, with "Not now" beside it: on the left, and on the right from right
to left. Left and Right go to the button on that side.

## Options

`theme.toml` has two options, on the theme's page under OPTIONS:

| Option | Choices | Effect |
|---|---|---|
| "Layout direction" (`direction`) | "Left to right", "Right to left" | Sets the `layout-direction` token. `Look.rtl` mirrors the layout, the button order, the hints and the notices, and Left and Right follow it |
| "Animations" (`motion`) | "On", "Off" | Sets `motion-fast`, `motion-normal` and `motion-slow` to 160, 260 and 420 ms, or to 0. Off turns every transition into a cut and stops the loading shimmer |

## Sizes

Everything is laid out on a 1920 × 1080 canvas and scaled to the window (`Look`,
`components/look.slint`):

- `Look.k = max(window height, 360 px) / 1080 px × size`. The shell sets the window height from
  the first frame, so the first frame is already the right size.
- `size` is `Vm.viewport.size`: "Size of everything", 0.8 to 1.4, or `NEXUS_SCALE` for a run. A
  change shows at once.
- `Look.px` is `1px × k`. `Look.tpx`, for text that must stay readable (Settings), never goes
  below 0.75.

`theme.toml` defines two breakpoints, `compact` (up to 900 px wide) and `regular`. The UI host
binds `Vm.viewport.breakpoint`, but the default theme's layout doesn't use it.

## Fonts

The theme ships Inter in four weights (Regular, Medium, SemiBold, Bold) and Tajawal (Regular,
Bold), each with its OFL licence. The `font-family` token is "Inter". Tajawal is loaded with the
theme, but no element names it: it can only serve as a fallback for glyphs Inter lacks, such as
Arabic. Right to left doesn't change the font.

## Sounds

The theme's sounds play through `Host.play-sound` (`crates/nexus-app/src/audio.rs`). "Interface
sounds" in Settings turns them off.

| Sound | When |
|---|---|
| `move` | A step to the next item; at most once in 40 ms |
| `select` | Enter that opens a game, the Library, a screen, the game's menu, a sheet, a filter or a sort; a menu pick; Resume |
| `back` | Escape or Back that leaves a screen, the Library, a menu, the viewer or the trailer; Stop; Not now |
| `start` | A game starts, or the Welcome closes |
| `error` | A new error notice |

`scripts/make-sounds.py` makes `select`, `back`, `error` and `start`: synthesized tones, mono,
16 bit, 22050 Hz.

## Motion

- A focused card grows to 1.05; Library cards come in as a wave on the first rows.
- The hero's parts (kicker, logo, chips, line, buttons) come in one beat (55 ms) after another.
  A new hero waits up to 450 ms for the game's details.
- Side sheets slide from the edge over `motion-slow`; dialogs rise and fade.
- The Game Hub's cover grows from its card into place.
- Notices slide in from the far side.

Durations are the `motion-*` tokens. Easings are written in the `.slint` files.

## Accessible labels: the test seam

Every element a test or a screen reader needs has an `accessible-label`, and some an
`accessible-value`. The labels quoted on this page are those. The theme tests find elements by
label (`ElementHandle::find_by_accessible_label`, in `tests/tests/seams/common/mod.rs`) and check what
the player sees: a value, a count, whether a screen shows. They never check layout numbers.

The tests run on the composed core and theme host on Slint's testing backend:

- `tests/tests/seams/default_theme.rs`: the screens, sheets, notices, keys and Downloads;
- `tests/tests/seams/gamepad.rs`: the pad and its focus rule;
- `tests/tests/seams/sounds.rs`: which sounds play;
- `tests/tests/seams/screenshots.rs`: PNGs of the main screens on the fake library, for a person to
  look at.

## Limits

- Keyboard players see the pad's signs in the hints, since the theme stays in `gamepad` input
  mode.
- The monitor choice matches monitors by name, so two of the same model look alike.
