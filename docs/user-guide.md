# User guide

This guide is for people who play games with Nexus. It covers setting up each system, then using
Nexus: folders, the Catalog, installs, playing, settings, emulators and themes. For what Nexus is
and how to install it, start at the [README](../README.md). To build Nexus from source, see
[development.md](development.md).

## Set up your system

Nexus starts without any of the extra packages below. Each one turns on one feature.

### Linux

| For | Fedora | Debian and Ubuntu | Without it |
|---|---|---|---|
| Games and Nexus coming to the front, on X11 | `xdotool` | `xdotool` | A notice names it |
| The same under Gamescope | `xdotool xprop` | `xdotool x11-utils` | A notice names them |
| The same on KDE Plasma (Wayland) | Nothing: KWin, over D-Bus | Nothing | |
| Trailers in the window | `mpv-libs` | `libmpv2` | Trailers open in the browser |
| RAR, tar.xz, tar.bz2, tar.zst and ISO downloads (the release packages ship it) | `libarchive` | `libarchive13` | Those archives do not unpack, and the error names the package |
| A RAR that libarchive cannot read (a RAR 5 with a big dictionary) | `unar` | `unar` | The unpack fails, and the error says to install `unar` |
| The "This PC" check | `vulkan-loader mesa-vulkan-drivers` (or your GPU vendor's driver) | `libvulkan1 mesa-vulkan-drivers` | The check lists the graphics cards without their Vulkan version |
| Windows PC games | [umu-launcher](#umu-launcher-for-windows-games), or `wine` | [umu-launcher](#umu-launcher-for-windows-games), or `wine` | The game's Game Hub says to install one |
| An old game that fills the screen at its own resolution | `gamescope` | `gamescope` | Play says to install it, or to set the game's Resolution back to Your screen |
| [Steam games](#steam-for-steam-games) | `steam` (RPM Fusion nonfree) | `steam-installer` (Debian non-free), `steam` (Ubuntu multiverse) | The Steam plugin finds no games |
| GameMode | `gamemode` | `gamemode` | Games run without it |
| Saved VPN connections in Settings | `NetworkManager` | `network-manager` | Only the VPNs already up show |

The deb, rpm and Arch packages recommend or depend on some of these. They also add a **Nexus**
session at the login screen: Nexus full screen in gamescope, with no desktop, for a PC under the
TV. Install gamescope for it. Cage is the fallback for graphics cards that gamescope cannot drive.
To change the session's settings, copy a line from `/usr/share/nexus/session.conf` to
`/etc/nexus/session.conf`.

`install.sh`, in the Linux tarball, installs Nexus for you, with no `sudo`. The package folder goes
to `~/.local/lib/nexus` (set `PREFIX` for another place than `~/.local`), `~/.local/bin/nexus`
links to it, and Nexus gets an entry and an icon in your app menu. Run `./install.sh` from a newer
package to put it in place of the installed one. `./install.sh --uninstall` removes Nexus and keeps
your settings, games and downloads.

### macOS

Nexus drives System Events through `osascript` to bring a game and itself to the front. Give the
app that starts Nexus (Terminal, your editor, or Nexus itself) the Accessibility permission, in
System Settings, Privacy & Security, Accessibility. Without it, a notice says so once. Optional:
`brew install mpv` (trailers in the window, for a build from source) and Wine (Windows PC games).

### Windows

Nothing more is needed. The graphics driver ships the Vulkan loader. For trailers in the window
from a build from source, put `libmpv-2.dll` beside `nexus.exe`.

### umu-launcher, for Windows games

Nexus runs a Windows game through [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)
when it is installed, and through Wine when it is not. umu-launcher runs games with Proton
(GE-Proton), as Steam does, and downloads and updates Proton by itself. The first game takes a
while to start while it does. A repack that brings its own 32-bit Wine cannot run its script on a
system without 32-bit libraries: pick its `.exe` in its Game settings (Starts) and it runs through
umu-launcher.

The easiest way: choose **Install umu-launcher** on the PC page (Emulators, then PC). Nexus installs
the latest release's zipapp for itself, checks it against the checksum GitHub lists, and **Update
umu-launcher** brings it up to date. It needs only Python 3.

To install it for the whole system instead, use its
[latest release](https://github.com/Open-Wine-Components/umu-launcher/releases/latest). It is not in
Fedora's repositories. Its page lists the Fedora, Debian and Ubuntu packages and the zipapp, which
needs only Python 3 and works on any system. `umu-run --help` answering means Nexus finds it. Nexus
looks for it each time a Windows game starts, so there is no need to restart Nexus.

### Steam, for Steam games

The games Steam installed show in the Library with Steam's covers, and Play starts them through
Steam (Linux only). Nexus follows the game that Steam starts: the Running screen, the PS button and
Stop work as for any game, and Steam keeps running.

1. Install Steam from your distribution, or as the Flatpak (`com.valvesoftware.Steam`). Nexus finds
   the Flatpak's folder too.
2. Start Steam once, sign in, and install your games in it.
3. For GameMode, put yourself in its group, then log out and in again:
   `sudo usermod -aG gamemode $USER`. Nexus runs PC games through GameMode by itself. For a Steam
   game, set its Launch Options in Steam to `gamemoderun %command%`.
4. Set nothing for your Steam libraries: Nexus reads Steam's folder and every library Steam lists
   (`steamapps/libraryfolders.vdf`), on any drive. A library you add in Steam shows at the next
   rescan (Settings, GAMES, **Rescan game folders**). **Steam libraries** in Settings is only for
   reading other folders. Once set, it replaces Steam's list.

A Steam game's Game Hub has Steam's store's words, genres, developer, release date, screenshots and
trailer. Your Steam library is also in the Catalog, under **Steam**: the games in your Steam
collections (Favorites, and the ones you made), read from Steam's own files on this PC. **Install in
Steam** opens Steam's install dialog, and Downloads shows its progress. **Uninstall**, in a Steam
game's menu, asks Steam, which confirms in its own dialog. A game you never put in a collection is
not in those files, so it is not listed.

Steam may ask to sign in or update before the game starts. Nexus waits for the game for
`hand_over_timeout` (2 minutes), then says it did not start. Steam Input may take the controller
while Steam runs: turn it off for a game in Steam if both read the controller.

### gamescope, for old games

An old game made for a small screen (800 × 600, say) cannot change a Wayland screen's resolution:
it shows in a corner of a black screen. Set its **Resolution** in its Game Hub (its Game settings)
to the one it runs at. It then runs inside [gamescope](https://github.com/ValveSoftware/gamescope),
Valve's compositor from the Steam Deck, which scales it up to fill the screen. **Your screen**, the
default, runs the game as it is.

On a Wayland desktop other than KDE Plasma, only games under XWayland come to the front, through
`xdotool`. `ydotool` does not help: it types keys and moves the mouse, but it cannot find a game's
window.

### UMIP off, for some Windows repacks

Some Windows repacks bypass the game's copy protection with a driver of their own. Their `game`
folder holds `driver_amd/SimpleSvm.sys` or `driver_intel/hyperkd.sys`, and their ReadMe asks you to
turn off UMIP, a CPU feature. The ReadMe says which CPUs need it. Without it the game closes as it
starts. Nexus does not change how your system boots: add the kernel argument yourself, then restart.

- Fedora: `sudo grubby --update-kernel=ALL --args="clearcpuid=514"`
- Debian and Ubuntu: add `clearcpuid=514` to `GRUB_CMDLINE_LINUX_DEFAULT` in `/etc/default/grub`,
  then run `sudo update-grub`

`grep -o clearcpuid=514 /proc/cmdline` prints it once it is on. To undo it on Fedora:
`sudo grubby --update-kernel=ALL --remove-args="clearcpuid=514"`.

## First start

Nexus opens full screen. On the first start a Welcome screen shows what it sets up (game folders,
each emulator, official artwork and details, covers and the Home screen). Press Start once it is
ready. The top bar has the Home and Library tabs, then Search (the Catalog), Downloads, Emulators,
Themes and Settings.

Nexus keeps its settings and plugin data in one folder:

| System | Folder |
|---|---|
| Linux | `~/.local/share/nexus` (or `$XDG_DATA_HOME/nexus`) |
| macOS | `~/Library/Application Support/nexus` |
| Windows | `%APPDATA%\nexus` |

The log goes to the terminal. `RUST_LOG` filters it (`info` by default).

## Game folders and packages folders

Settings, GAMES, has two lists of folders. Separate several folders with `;`, and start a path with
`~` for your home folder.

- **Game folders**, `~/Games` until you change it: games of any console, in any layout up to three
  folders down. Each emulator finds its own games there. Installs go into the console's folder
  inside the first one: `PS5/`, `PS4/` or `PC/`.
- **Packages to install**, `~/Downloads` until you change it: packages and disc images, each in its
  console's folder, such as `~/Downloads/PS5` and `~/Downloads/PS4`. Catalog downloads go here too.

**Rescan game folders**, under the list, looks for games again. An emulator's own folders, such as
"Your own KytyPS5 build" or "shadPS4 data", are on its page under Emulators.

## Catalog and Downloads

The Catalog lists the games each console's catalog offers, newest added first, with the same search,
sort and filters as the Library. A release's row in the Game Hub shows its version, region, size and
availability. **Download** opens "Set up a download": the release, the download folder (inside a
packages folder) and what downloading means. **Look up metadata** adds the download to Downloads,
which reads its files first. Press Download there to start it.

- A download goes on while Nexus is open, also minimized, and stops when it closes. Unfinished
  downloads come back paused at the next start. Nexus remembers which pieces it checked, so a start
  does not read every finished download again.
- With Settings, DOWNLOADS, "Keep sharing finished downloads" on (the default), a finished download
  goes on sharing, at most 128 KB/s. **Stop sharing** stops it until the next start.
- **Cancel** never deletes the downloaded files. **Remove** asks first: "Remove from Downloads" keeps
  the folder, and "Delete files too" also deletes the download's own folder with its unpacked
  copies, but never an installed game.
- Ctrl+D opens Downloads from anywhere.
- To hide a game, use the eye on its card, **Hide** in its Game Hub, or "Hide from the Catalog" in
  its menu. The "Hidden" filter shows only the hidden games, each with the eye to show it again.
  The Game Catalog's "Show the Hidden filter" setting turns the chip off.
- The focused card has a button: Uninstall for a game you have, which asks first, and Download for
  one you do not, which opens its Game Hub on its releases.

The catalogs come from the Game Catalog plugin (Emulators, then Game Catalog). It uses the copies
bundled in `assets/catalog/` (`ps5.json`, `ps4.json`, `pc.json`), or newer ones from its **Catalog
address** setting. With **Refresh from the forums** on, it brings each catalog up to date from the
forums named in `assets/catalog-sources.json` (or the **Sources address**). Its page shows the last
refresh and has Refresh now. Its "Show PS5 games" setting (one for each console you have an emulator
for), when turned off, takes that console out of the Catalog. The games of that console that you
installed stay in your library and play. With the Game Catalog turned off there is no Catalog: the
top bar has no Search. How the catalog files are made is in
[../scripts/catalog/README.md](../scripts/catalog/README.md).

## Installing games

- **From a catalog download.** When the download is complete, press Install on its card. Nexus
  unpacks its archives (zip, 7z, tar and tar.gz built in; RAR and the rest through libarchive) and
  opens "Install game", where you can change the game folder. On the same disk, an unpacked PS5 game
  is moved into the game folder, not copied. **Remove unpacked files** frees the space after the
  install.
- **From your packages folder.** Each emulator's page (Emulators, then the emulator) offers "Install
  <game>" for each package it finds: unsigned (debug) PS5 packages and exFAT disc images under PS5,
  and PS4 fake packages (games, updates and DLC) under PS4.

Where installs go:

| What | Where |
|---|---|
| A PS5 game | `PS5/<title id>-app0` in the first game folder |
| A PS4 game | `PS4/<title id>` in the first game folder |
| A PS4 update | Beside its game, as `<game folder>-patch` |
| PS4 DLC | `addcont/` in shadPS4's data folder |
| A PC game | `PC/` in the first game folder, from its folder, a GOG installer or an AppImage |

The install runs in Downloads. It checks that a package holds a game and never runs what it
installs. To delete an installed game, open its menu and choose Uninstall: it asks first, then
deletes the game's folder. It refuses while the game runs.

### Adding a PC game by hand

A PC game you already have, unpacked and ready to run, needs no install:

1. Copy the game's folder into `PC/` of a game folder (`~/Games/PC/` by default). One folder is one
   game; keep the game's own files as they are inside it.
2. Optional: put its cover in the game's folder, named `cover.jpg`, `cover.png` or `cover.webp` (a
   portrait picture, as on a box).
3. In Settings, GAMES, choose **Rescan game folders**, or restart Nexus. The game shows in the
   Library, named after its folder.

```text
~/Games/PC/
└── Hollow Knight/
    ├── cover.jpg            ← its cover (optional)
    ├── start.sh             ← what Play starts (found by itself)
    └── game/…
```

Nexus looks through the folder once and writes `.nexus-game.json` in it: the game's name, and every
file that can start it, best first. Play starts the first one that runs on this system; **Starts**,
in the game's Game settings, picks another. A Windows game runs through umu-launcher (Proton) or
Wine. The name comes from the folder (`hollow_knight` shows as "Hollow Knight"), or from GOG's
`gameinfo` file when the game has one. To rename the game, rename its folder before Nexus first sees
it, or edit `name` in `.nexus-game.json`.

A folder with nothing that starts a game stays out of the Library until Nexus restarts. Folders
whose name starts with `.` or ends with `.part` are never read. The rules are in
[reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).

## Playing

Press Play on Home or in the Game Hub. When the game's emulator is not installed yet, Play installs
it first. A splash shows while the game starts; without a controller it lists the emulator's
keyboard keys. When the game closes, Nexus asks "How far did it get?" (once for each emulator
build). The answer is "YOUR RESULT" in the Game Hub. "Rate how it runs" in the game's menu asks
again.

| Controller | Keyboard | What it does |
|---|---|---|
| Cross | Enter | Select |
| Circle | Escape | Back |
| D-pad, left stick | Arrows | Move |
| Options | Menu | The game's menu (Game settings, Open game folder, View emulator log, Rate how it runs, Uninstall) |
| Triangle | `/` | Search |
| Square | T | The game's trailer. The letter T works outside the Library's search box and the Catalog |
| L1, R1 | Shift+Tab, Tab | The screen before and after |
| L2, R2 | Page Up, Page Down | A page up or down |
| Create (Share, View) | Delete | Close the newest notice |
| PS (Guide) | | Hold: closes the running game, or with no game running opens the power menu. Tap: back to Home; nothing while a game runs |
| | Ctrl+P | The power menu |
| | Ctrl+D | Downloads |
| | Ctrl+Q | Quit |

While a game is in front, the controller is the game's. Nexus takes it back only while its own
window has the focus. Quit, Ctrl+Q, Ctrl-C in the terminal, `SIGTERM` and `SIGHUP` all close Nexus
cleanly: the downloads stop and are saved first. A second Ctrl-C quits at once. How a game and Nexus
come to the front on each system is in [reference/desktop.md](reference/desktop.md).

## Settings

Changes save as you make them.

| Section | Setting | What it does |
|---|---|---|
| UPDATES | Update automatically | Checks for emulator updates every few hours. An update waits until you close your game |
| UPDATES | Nexus updates | Check now, Download, then Restart to put the new version in. Only for an installed package, AppImage or app |
| LAUNCHER | Full screen | On by default. `--windowed` opens a normal window for one run |
| LAUNCHER | Size of everything | Text, art and space from 80% to 140%. `NEXUS_SCALE=<0.8 to 1.4>` sets it for one run |
| LAUNCHER | Back to the launcher | When a game closes, Nexus comes to the front |
| LAUNCHER | Interface sounds | The theme's sounds for moves, selects, back, starts and errors |
| LAUNCHER | Start at login | Only in an installed build |
| LAUNCHER | Screen | With two monitors or more: the one Nexus fills |
| DOWNLOADS | Download only through a VPN | Binds the catalog downloads to your VPN, and pauses them while it is down |
| DOWNLOADS | Your VPN, Add a VPN | On Linux, WireGuard and OpenVPN interfaces are found by themselves. With NetworkManager you connect, disconnect and add one from a `.conf` or `.ovpn` file. On macOS, pick the network your VPN app made |
| DOWNLOADS | Keep sharing finished downloads | Other players download from you, at most 128 KB/s |
| THIS PC | | Your processor, memory and graphics, and whether each emulator's games should run well |

The command-line options are listed by `nexus --help` and in [development.md](development.md).

## Emulators

Emulators lists each plugin by the console it plays: PS5 (KytyPS5), PS4 (shadPS4) and PC, and the
services: Game Catalog and PlayStation Store (official art and details for PS4 and PS5 games). Each
page has "Turned on", the build, its actions, settings, folders and keys.

- **Official builds.** Nexus downloads them from GitHub, runs each new one once with `--help` to
  check it, and keeps the build before it. The "Release repo" and "Pre-releases" settings change
  where KytyPS5's builds come from.
- **Use your own build.** This runs a KytyPS5 or shadPS4 you built or installed. Map its folder on
  the emulator's page ("Your own KytyPS5 build", "Your own shadPS4 build"), or put `kyty_emulator`
  or `shadps4` on your `PATH`. Installing an official KytyPS5 build turns your own build off and
  copies its saves and caches.
- KytyPS5 asks each build which options it knows (its `--help`) and leaves out a setting that the
  build does not know, with a line in the log at info level.
- **PC games** run on their own system. A Windows game runs on Linux through umu-launcher (Proton)
  or Wine, and on macOS through Wine.
- A game's menu has Game settings: the settings of that game only, such as KytyPS5's "Video out" and
  "Extra arguments".

## Themes

Themes lists the built-in theme, `default`, and the themes in `<data-dir>/extensions` (one folder
each) and in `extensions/` next to the program. Open one and choose "Use this theme". Switching the
theme keeps the screen you are on, your selection, the downloads and a running game. The default
theme's options are "Layout direction" (right to left) and "Animations" (off makes the transitions
cuts). Third-party plugins are themes only: they are data and run no code. To write a theme, see
[themes.md](themes.md).
