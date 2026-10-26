# Nexus

Nexus is a game launcher for PS5, PS4 and PC games, made for a TV and a controller as much as a
desk. It plays PS5 games with [KytyPS5](https://github.com/KytyPS5/KytyPS5), PS4 games with
[shadPS4](https://github.com/shadps4-emu/shadPS4), and PC games natively or through Wine or
Proton. It runs on Linux, macOS and Windows. Each emulator is a plugin, and each look is a theme
that changes the whole layout, not just the colors.

| Home | Library |
|---|---|
| ![Home](docs/screenshots/home.jpg) | ![Library](docs/screenshots/library.jpg) |

| Game Hub | Settings |
|---|---|
| ![Game Hub](docs/screenshots/detail.jpg) | ![Settings](docs/screenshots/settings.jpg) |

The screenshots are the built-in theme, `default`, on Linux, with a real library of PS5 and PC
games. The tests also render every screen offscreen with made-up games
(`tests/tests/seams/screenshots.rs`, into `target/tmp/screenshots`).

## What it does

- **Library**: finds your PS5, PS4 and PC games in your game folders. Home is a row of games
  over the selected game's art; Library is a grid with search, sort and filters by system,
  genre and compatibility.
- **Game Hub**: one game's page, with official art, facts, the community's compatibility
  result, your own result, how it should run on this PC, screenshots and the trailer.
- **Catalog and Downloads**: the games each console's catalog offers. Nexus downloads a release
  by BitTorrent into your packages folder and shows it in Downloads.
- **Installs**: PS5 and PS4 packages, disc images, and PC releases install into your game
  folders, with progress in Downloads.
- **Emulators**: Nexus downloads and updates the official KytyPS5 and shadPS4 builds, or runs a
  build of your own.
- **Themes**: switching the theme keeps the screen you are on, your selection, the downloads
  and a running game.
- **Controller**: every screen works with a gamepad, a keyboard or a pointer. Holding the PS
  (Guide) button closes a running game, or opens the power menu when no game runs.
- **VPN**: catalog downloads can be bound to your own VPN, and pause when it drops.
- **Updates**: Nexus updates its emulators by itself, and updates itself from its GitHub
  releases.

## Install

### Release packages

Each [release](https://github.com/mohamed-rekiba/nexus/releases/latest) has these files, with a
`.sha256` file beside each and signed build provenance
(`gh attestation verify <file> --repo mohamed-rekiba/nexus` checks it).

| System | File | How to use it |
|---|---|---|
| Fedora (x86-64) | `nexus-<version>-1.x86_64.rpm` | `sudo dnf install ./nexus-<version>-1.x86_64.rpm` |
| Debian, Ubuntu (x86-64) | `nexus_<version>-1_amd64.deb` | `sudo apt install ./nexus_<version>-1_amd64.deb` |
| Arch (x86-64) | `nexus-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U nexus-<version>-1-x86_64.pkg.tar.zst` |
| Linux (x86-64) | `nexus-linux-x86_64.tar.gz` | Unpack it and run `./install.sh` (below) |
| Linux (x86-64) | `Nexus-x86_64.AppImage` | Make it executable and run it |
| macOS (Apple Silicon and Intel, 12 or newer) | `nexus-macos-universal.dmg` | Open it and drag Nexus to Applications |
| macOS | `nexus-macos-universal.zip` | Unzip it and move `Nexus.app` to Applications |
| Windows (x86-64) | `nexus-windows-x86_64-setup.exe` | Run it: it installs for you only, with no administrator, and adds Nexus to the Start menu |
| Windows (x86-64) | `nexus-windows-x86_64.zip` | Unzip it anywhere and run `nexus.exe` |

Every kind keeps itself up to date from Settings → Nexus updates. The deb, rpm and Arch packages
update through your package manager, which asks for your password. The installer runs the next
release's installer by itself.

The deb, rpm and Arch packages also add a **Nexus** session at the login screen: Nexus full
screen in gamescope, with no desktop, for a PC under the TV. Install gamescope for it (cage is
the fallback for graphics cards gamescope can't drive). To change its settings, copy a line from
`/usr/share/nexus/session.conf` to `/etc/nexus/session.conf`.

`install.sh` installs the Linux package for you, with no `sudo`:

- the package folder goes to `~/.local/lib/nexus` (set `PREFIX` for another place than
  `~/.local`);
- `~/.local/bin/nexus` links to it;
- Nexus gets an entry and an icon in your app menu.

Run `./install.sh` from a newer package to put it in place of the installed one.
`./install.sh --uninstall` removes Nexus; your settings, games and downloads stay.

The macOS app and disk image are signed with a Developer ID and notarized, and `nexus.exe` and
the installer are signed, only when the release has the signing secrets
([docs/signing.md](docs/signing.md)). Without them, macOS Gatekeeper asks before the first start
and Windows SmartScreen warns.

Each package carries its licences in `licenses/`. The release also carries the source of the
mpv and FFmpeg builds it ships, as `nexus-<os>-<arch>-third-party-sources.tar`.

### From source

You need [Rust](https://rustup.rs) 1.99 or newer and the build packages below. Then, in the
repository:

```sh
make run            # build and run the launcher
make run-windowed   # the same, in a normal window
make package        # a release build in dist/nexus-<os>-<arch>/
```

`make` alone lists every target.

### System packages

**Linux, to build**: a C toolchain and the development files for fontconfig, xkbcommon, udev
(controllers) and ALSA (sounds).

- Fedora: `sudo dnf install gcc gcc-c++ make pkgconf-pkg-config fontconfig-devel libxkbcommon-devel systemd-devel alsa-lib-devel`
- Debian and Ubuntu: `sudo apt-get install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev libudev-dev libasound2-dev`

**Linux, to run**: Nexus starts without any of these; each one turns on a feature.

| For | Fedora | Debian and Ubuntu | Without it |
|---|---|---|---|
| Games and Nexus coming to the front, on X11 | `xdotool` | `xdotool` | A notice names it |
| The same under Gamescope | `xdotool xprop` | `xdotool x11-utils` | A notice names them |
| The same on KDE Plasma (Wayland) | Nothing: KWin, over D-Bus | Nothing | |
| Trailers in the window | `mpv-libs` | `libmpv2` | Trailers open in the browser |
| RAR, tar.xz, tar.bz2, tar.zst and ISO downloads (the release package ships it) | `libarchive` | `libarchive13` | Those archives don't unpack, and the error names the package |
| A RAR libarchive can't read: a RAR 5 with a big dictionary, as in some script installers (Cuphead's). The Nexus packages recommend it | `unar` | `unar` | Its unpacking fails, and the error says to install unar |
| The "This PC" check | `vulkan-loader mesa-vulkan-drivers` (or your GPU vendor's driver) | `libvulkan1 mesa-vulkan-drivers` | The check lists the graphics cards without their Vulkan version |
| Windows PC games | [umu-launcher](#umu-launcher-for-windows-games) (runs them with Proton), or `wine` | [umu-launcher](#umu-launcher-for-windows-games), or `wine` | The game's Game Hub says to install one |
| An old game filling the screen at its own resolution (its Resolution setting) | `gamescope` | `gamescope` (Debian 12, backports for 13; Ubuntu where your release has it) | Play says to install it, or to set the game's Resolution back to Your screen |
| Saved VPN connections in Settings | `NetworkManager` | `network-manager` | Only the VPNs already up show |
| Old Linux games whose start script opens the game's own launcher (Trine 3 from GOG). The Nexus packages depend on these | `gtk2 libpng12` | `libgtk2.0-0t64` (no libpng 1.2 package) | The game closes right after it starts, with code 127 in its log |
| A game that brings its own 32-bit Wine (a portable Wine wrapper's `start.sh`). The Nexus rpm depends on these | `glibc.i686 libgcc.i686 libX11.i686 libXext.i686 libXcomposite.i686 libXcursor.i686 libXfixes.i686 libXinerama.i686 libXi.i686 libXrandr.i686 libXrender.i686 libXxf86vm.i686 mesa-libGL.i686 mesa-dri-drivers.i686 mesa-vulkan-drivers.i686 vulkan-loader.i686 alsa-lib.i686 pulseaudio-libs.i686 freetype.i686 fontconfig.i686` | The same libraries for `i386`, after `sudo dpkg --add-architecture i386` | Its script says "There is a problem running Wine binary!", or Wine says no driver could be loaded |

All of them on Fedora: `sudo dnf install xdotool xprop mpv-libs libarchive vulkan-loader mesa-vulkan-drivers wine gamescope NetworkManager gtk2 libpng12 unar`.
On Debian and Ubuntu: `sudo apt-get install xdotool x11-utils libmpv2 libarchive13 libvulkan1 mesa-vulkan-drivers wine network-manager libgtk2.0-0t64 unar`.

#### umu-launcher, for Windows games

Nexus runs a Windows game through [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)
when it is installed, else through Wine. umu-launcher runs games with Proton (GE-Proton), the
way Steam does, and downloads and updates Proton by itself: the first game takes a while to
start while it does. A repack that brings its own 32-bit Wine can't run its script on a system
without 32-bit libraries, such as a default Fedora: pick its `.exe` in its Game settings
(Starts) and it runs through umu-launcher.

The easiest way: **Install umu-launcher** on the PC page (Emulators, then PC). Nexus installs
the latest release's zipapp for itself, checked against the checksum GitHub lists, and
**Update umu-launcher** brings it up to date. It needs only Python 3. The page's "Windows games"
section says what they run with.

To install it for the whole system instead: it isn't in Fedora's repositories, so install it
from its [latest release](https://github.com/Open-Wine-Components/umu-launcher/releases/latest). First
ask GitHub which version that is:

```sh
UMU_VERSION=$(curl -s https://api.github.com/repos/Open-Wine-Components/umu-launcher/releases/latest | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
echo "$UMU_VERSION"
```

Then, in the same terminal:

- Fedora 43 or 44 (x86-64):

  ```sh
  sudo dnf install "https://github.com/Open-Wine-Components/umu-launcher/releases/download/$UMU_VERSION/umu-launcher-$UMU_VERSION.fc$(rpm -E %fedora).x86_64.rpm"
  ```

- Debian 12 or 13, Ubuntu 24.04 (x86-64): its two packages.

  ```sh
  . /etc/os-release
  case "$ID" in debian) OS="debian-$VERSION_ID" ;; ubuntu) OS="ubuntu-$VERSION_CODENAME" ;; esac
  for f in "umu-launcher_$UMU_VERSION-1_all_$OS.deb" "python3-umu-launcher_$UMU_VERSION-1_amd64_$OS.deb"; do
    curl -LO "https://github.com/Open-Wine-Components/umu-launcher/releases/download/$UMU_VERSION/$f"
  done
  sudo apt-get install "./umu-launcher_$UMU_VERSION-1_all_$OS.deb" "./python3-umu-launcher_$UMU_VERSION-1_amd64_$OS.deb"
  ```

- Any other system: the zipapp, which needs only Python 3.

  ```sh
  curl -LO "https://github.com/Open-Wine-Components/umu-launcher/releases/download/$UMU_VERSION/umu-launcher-$UMU_VERSION-zipapp.tar"
  tar xf "umu-launcher-$UMU_VERSION-zipapp.tar"
  install -Dm755 umu/umu-run ~/.local/bin/umu-run
  ```

  `~/.local/bin` must be on your `PATH`.

Run the same commands again to update it.

`umu-run --help` answering means Nexus will find it. Nexus looks for it each time a Windows game
starts, so there is no need to restart Nexus.

#### gamescope, for old games

An old game made for a small screen (800 × 600, say), Windows or Linux, can't change a Wayland screen's
resolution: it shows in a corner of a black screen. Set its **Resolution** in its Game Hub (its
Game settings) to the one it runs at: it then runs inside
[gamescope](https://github.com/ValveSoftware/gamescope), Valve's compositor from the Steam Deck,
which scales it up to fill the screen. **Your screen**, the default, runs the game as it is.
Install it with `sudo dnf install gamescope` on Fedora, `sudo apt-get install gamescope` on
Debian and Ubuntu.

`ydotool` isn't needed, and doesn't help on Wayland: it types keys and moves the mouse, but it
can't find a game's window or bring one to the front. On a Wayland desktop other than KDE
Plasma, only games under XWayland come to the front (through `xdotool`).

**macOS**: the Xcode Command Line Tools (`xcode-select --install`) to build. To run: the
Accessibility permission for the app that starts Nexus, and optionally `brew install mpv`
(trailers in the window) and Wine (Windows PC games).

**Windows**: the Visual Studio Build Tools, with "Desktop development with C++", to build.
To run, nothing more; optionally `libmpv-2.dll` beside `nexus.exe` for trailers in the window.

#### UMIP off, for some Windows repacks

Some Windows repacks bypass the game's copy protection with a driver of their own: their
`game` folder holds `driver_amd/SimpleSvm.sys` or `driver_intel/hyperkd.sys`, and their ReadMe
asks you to turn off UMIP, a CPU feature. The ReadMe says which CPUs need it (Sonic
Superstars': AMD after Zen 3, Intel after 8th gen). Without it the game closes as it starts.
Nexus doesn't change how your system boots: add the kernel argument yourself, then restart.

- Fedora: `sudo grubby --update-kernel=ALL --args="clearcpuid=514"`
- Debian and Ubuntu: add `clearcpuid=514` to `GRUB_CMDLINE_LINUX_DEFAULT` in
  `/etc/default/grub`, then `sudo update-grub`

`grep -o clearcpuid=514 /proc/cmdline` prints it once it is on. To undo it on Fedora:
`sudo grubby --update-kernel=ALL --remove-args="clearcpuid=514"`.

## Download Nexus OS

Nexus OS turns a spare PC into a console: it starts straight into Nexus, with no desktop, no
passwords and no pop-ups. It's a small, read-only Fedora system. It updates itself and Nexus in
the background, applies an update at the next restart, and keeps the previous version in the
boot menu. On a PC with an NVIDIA card it gets NVIDIA's driver by itself. It needs a 64-bit PC
that starts with UEFI.

<!-- x-release-please-start-version -->
The current version is Nexus OS 0.1.1:
[its release](https://github.com/mohamed-rekiba/nexus/releases/tag/nexus-os-v0.1.1).
<!-- x-release-please-end -->
All Nexus OS releases are tagged `nexus-os-v…` on the
[releases page](https://github.com/mohamed-rekiba/nexus/releases?q=nexus-os).

The release has one ISO, `nexus-os-x86_64.iso`, to write to a USB stick of 8 GB or more. When
the ISO is over 2 GiB it comes in parts (`nexus-os-x86_64.iso.part-aa`, `-ab`, …): download them
all and join them into the ISO.

- Linux and macOS:

  ```sh
  cat nexus-os-x86_64.iso.part-* > nexus-os-x86_64.iso
  ```

- Windows (Command Prompt), naming every part in order:

  ```bat
  copy /b nexus-os-x86_64.iso.part-aa + nexus-os-x86_64.iso.part-ab nexus-os-x86_64.iso
  ```

Then check the ISO against `nexus-os-x86_64.iso.sha256` (each part also has its own `.sha256`):

- Linux: `sha256sum -c nexus-os-x86_64.iso.sha256`
- macOS: `shasum -a 256 -c nexus-os-x86_64.iso.sha256`
- Windows (PowerShell): `Get-FileHash nexus-os-x86_64.iso`, and compare it with the file's
  first word.

Write it to the USB stick with [balenaEtcher](https://etcher.balena.io): choose the ISO, the USB
stick, then **Flash**. Start the PC from the stick (its boot menu key is usually F12, F11, F8 or
Esc). Nexus OS starts from the stick without changing anything on the PC. From there:

- **Try it:** play from the stick.
- **Install on this PC:** pick a disk in Nexus. It erases that whole disk.
- **Choose partitions instead:** Fedora's installer, to keep Windows or use free space. It needs a
  keyboard and a mouse. It's also the boot menu entry "Install Nexus OS (choose partitions)";
  hold Esc or Shift while the stick starts to see the menu.

Both installs copy the system stored on the stick, so neither needs internet. With an NVIDIA
card and Secure Boot on, a blue "MOK management" screen appears once at the next restart: choose
**Enroll MOK**, **Continue**, **Yes**, type `universalblue`, then **Reboot**. How Nexus OS is
built is in [os/README.md](os/README.md).

## Using Nexus

### First start

Nexus opens full screen. On the first start a Welcome screen shows what it sets up (game
folders, each emulator, official artwork and details, covers and Home screen); press Start
once it is ready. The top bar has the Home and Library tabs, then Search (the Catalog), Downloads,
Emulators, Themes and Settings.

Nexus keeps its settings and plugin data in one folder:

| System | Folder |
|---|---|
| Linux | `~/.local/share/nexus` (or `$XDG_DATA_HOME/nexus`) |
| macOS | `~/Library/Application Support/nexus` |
| Windows | `%APPDATA%\nexus` |

The log goes to the terminal; `RUST_LOG` filters it (`info` by default).

### Game folders and packages folders

Settings, GAMES, has two lists of folders. Separate several folders with `;`, and start a path
with `~` for your home folder.

- **Game folders**, `~/Games` until you change it: games of any console, in any layout up to
  three folders down. Each emulator finds its own games there. Installs go into the console's
  folder inside the first one: `PS5/`, `PS4/` or `PC/`.
- **Packages to install**, `~/Downloads` until you change it: packages and disc images, each in
  its console's folder, such as `~/Downloads/PS5` and `~/Downloads/PS4`. Catalog downloads go
  here too.

Rescan game folders, under the list, looks for games again. An emulator's own folders, such as
"Your own KytyPS5 build" or "shadPS4 data", are on its page under Emulators.

### Catalog and Downloads

The Catalog lists the games each console's catalog offers, newest added first, with the same
search, sort and filters as the Library. A release's row in the Game Hub shows its version,
region, size and availability. Download opens "Set up a download": the release, the download
folder (inside a packages folder) and what downloading means. Look up metadata adds the
download to Downloads, which reads its files first; press Download there to start it.

- A download goes on while Nexus is open, also minimized, and stops when it closes. Unfinished
  downloads come back paused at the next start. Nexus remembers which pieces it checked, so a
  start doesn't read every finished download again.
- With Settings, DOWNLOADS, "Keep sharing finished downloads" on (the default), a finished
  download goes on sharing, at most 128 KB/s. Stop sharing stops it until the next start.
- Cancel never deletes the downloaded files. Remove asks first: Remove from Downloads keeps the
  folder, Delete files too also deletes the download's own folder with its unpacked copies, but
  never an installed game.
- Ctrl+D opens Downloads from anywhere.
- A game you don't want to see hides: the eye on its card (on the focused or pointed card), Hide
  in its Game Hub, or "Hide from the Catalog" in its menu. The "Hidden" filter, there once a game
  is hidden, shows only the hidden games, each with the eye to show it again. The Game Catalog's
  "Show the Hidden filter" setting turns the chip off; hidden games then stay hidden.
- A thin bar under the header shows how far down all the games you are, as you scroll.
- The focused or pointed card has a button: Uninstall for a game you have (in the Catalog, the
  installed game it is), which asks first, and Download for one you don't, which opens its Game
  Hub on its releases.

The catalogs come from the Game Catalog plugin (Emulators, then Game Catalog). It uses the copies
bundled in `assets/catalog/` (`ps5.json`, `ps4.json`, `pc.json`), or newer ones from its
**Catalog address** setting. With **Refresh from the forums** on, it brings each catalog up to
date from the forums named in `assets/catalog-sources.json` (or the **Sources address**): new
releases, and torrents, sizes and peers. Its page shows the last refresh and has Refresh now.
Its "Show PS5 games" (one for each console you have an emulator for) turned off takes that
console out of the Catalog: none of its games can be downloaded, and it isn't fetched or
refreshed. The games of that console you installed stay in your library and play.
With the Game Catalog turned off there is no Catalog: the top bar has no Search, and the
shoulder buttons skip it. How the catalog files are made is in
[scripts/catalog/README.md](scripts/catalog/README.md).

### Installing games

- **From a catalog download**: when the download is complete, press Install on its card. Nexus
  unpacks its archives (zip, 7z, tar and tar.gz built in; RAR and the rest through libarchive)
  and opens "Install game", where you can change the game folder. On the same disk, an unpacked
  PS5 game is moved into the game folder, not copied. Remove unpacked files frees the space
  after the install.
- **From your packages folder**: each emulator's page (Emulators, then the emulator) offers
  "Install <game>" for each package it finds: unsigned (debug) PS5 packages and exFAT disc
  images under PS5, PS4 fake packages (games, updates and DLC) under PS4.

Where installs go:

| What | Where |
|---|---|
| A PS5 game | `PS5/<title id>-app0` in the first game folder |
| A PS4 game | `PS4/<title id>` in the first game folder |
| A PS4 update | Beside its game, as `<game folder>-patch` |
| PS4 DLC | `addcont/` in shadPS4's data folder (`~/.local/share/shadPS4`, on macOS `~/Library/Application Support/shadPS4`) |
| A PC game | `PC/` in the first game folder, from its folder, a GOG installer or an AppImage |

The install runs in Downloads. Installing checks that a package holds a game, and never runs
what it installs. To delete an installed game, open its menu and choose Uninstall: it asks
first, then deletes the game's folder. It refuses while the game runs.

### Playing

Press Play on Home or in the Game Hub. When the game's emulator isn't installed yet, Play
installs it first. A splash shows while the game starts; without a controller it lists the
emulator's keyboard keys. When the game closes, Nexus asks "How far did it get?" (once per
emulator build); the answer is "YOUR RESULT" in the Game Hub. "Rate how it runs" in the
game's menu asks again.

| Controller | Keyboard | What it does |
|---|---|---|
| Cross | Enter | Select |
| Circle | Escape | Back |
| D-pad, left stick | Arrows | Move |
| Options | Menu | The game's menu (Game settings, Open game folder, View emulator log, Rate how it runs, Uninstall…) |
| Triangle | `/` | Search |
| Square | T | The game's trailer |
| L1, R1 | Shift+Tab, Tab | The screen before and after |
| L2, R2 | Page Up, Page Down | A page up or down |
| Create (Share, View) | Delete | Close the newest notice |
| PS (Guide) | | Hold: closes the running game, or with no game running opens the power menu. Tap: back to Home; nothing while a game runs |
| | Ctrl+P | The power menu |
| | Ctrl+Q | Quit |

While a game is in front, the controller is the game's; Nexus takes it back only while its
own window has the focus. Quit, Ctrl+Q, Ctrl-C in the terminal, `SIGTERM` and `SIGHUP` all close
Nexus cleanly: the downloads stop and are saved first. A second Ctrl-C quits at once.

How a game and Nexus come to the front differs by system:

- **Linux on X11, and Gamescope**: `xdotool` (and `xprop` under Gamescope).
- **Linux on KDE Plasma (Wayland)**: KWin's scripting over D-Bus, with nothing to install. It
  sees Wayland and XWayland windows alike.
- **Linux on another Wayland desktop**: no program may raise another's window. Nexus raises
  itself, and a game under XWayland still comes to the front through `xdotool`.
- **macOS**: `osascript` and System Events, which need the Accessibility permission. Full
  screen stays in the current Space.
- **Windows**: the Win32 API.

### Settings

Changes save as you make them.

| Section | Setting | What it does |
|---|---|---|
| UPDATES | Update automatically | Checks for emulator updates every few hours; an update waits until you close your game |
| UPDATES | Nexus updates | Check now, Download, then Restart to put the new version in. Only for an installed package, AppImage or app |
| LAUNCHER | Full screen | On by default. `--windowed` opens a normal window for one run |
| LAUNCHER | Size of everything | Text, art and space from 80% to 140%. `NEXUS_SCALE=<0.8 to 1.4>` sets it for one run |
| LAUNCHER | Back to the launcher | When a game closes, Nexus comes to the front |
| LAUNCHER | Interface sounds | The theme's sounds for moves, selects, back, starts and errors |
| LAUNCHER | Start at login | Only in an installed build: an autostart entry on Linux, a launch agent on macOS, a `Run` registry value on Windows |
| LAUNCHER | Screen | With two monitors or more: the one Nexus fills |
| DOWNLOADS | Download only through a VPN | Binds the catalog downloads to your VPN, and pauses them while it is down |
| DOWNLOADS | Your VPN, Add a VPN | Linux: WireGuard and OpenVPN interfaces are found by themselves; with NetworkManager you connect, disconnect and add one from a `.conf` or `.ovpn` file. macOS: pick the network your VPN app made |
| DOWNLOADS | Keep sharing finished downloads | Other players download from you, at most 128 KB/s |
| THIS PC | | Your processor, memory and graphics, and whether each emulator's games should run well |

Command-line options (`nexus --help` lists them):

| Option | What it does |
|---|---|
| `--windowed` | A normal window for this run |
| `--monitor <name>` | This monitor for this run, such as `DP-2` (a screen picked in Settings wins) |
| `--theme <id>` | This theme for this run |
| `--safe-mode` | Built-in plugins and the default theme only |
| `--data-dir <path>` | Another settings and data folder |
| `--extensions <path>` | Another folder of third-party themes (default: `<data-dir>/extensions`) |
| `--folder <name>=<path>` | Map a named folder, such as `games=/srv/games` (saved; repeat it for more) |
| `--dev` | Reload a theme when its files change |
| `--supports <feature>` | Exits 0 when Nexus has the feature: `gamescope`, `system-settings`, `os-install` ([Nexus OS](docs/nexus-os.md#what-the-os-asks-of-nexus)) |

### Emulators

Emulators lists each plugin by the console it plays: PS5 (KytyPS5), PS4 (shadPS4) and PC, and
the services: Game Catalog and PlayStation Store (official art and details for PS4 and PS5
games). Each page has "Turned on", the build, its actions, settings, folders and keys.

- **Official builds**: Nexus downloads them from GitHub, runs each new one once with `--help`
  to check it, and keeps the build before it. KytyPS5's builds come from `KytyPS5/KytyPS5`; on
  macOS from `mohamed-rekiba/KytyPS5`, the test builds of upstream PR #1469, which makes games
  start on macOS. The "Release repo" and "Pre-releases" settings change this. shadPS4's come
  from `shadps4-emu/shadPS4`.
- **Use your own build**: runs a KytyPS5 or shadPS4 you built or installed. Map its folder on
  the emulator's page ("Your own KytyPS5 build", "Your own shadPS4 build"), or put
  `kyty_emulator` or `shadps4` on your PATH. For KytyPS5 the mapped path can be the program
  itself. Until you map it, KytyPS5 looks in `~/KytyPS5`, `~/kyty`, `~/Kyty` and the same names
  in `~/projects`, `~/Games`, `~/Applications` and `~/src`. Installing an official KytyPS5 build
  turns your own build off and copies its saves and caches.
- KytyPS5 asks each build which options it knows (its `--help`) and leaves out a setting the
  build doesn't know, with a line in the log at info level. Builds from upstream's main branch
  have no `--video-out-resolution`, so they skip "Video out"; builds of `mohamed-rekiba/KytyPS5`
  have it, so turn on "Use your own build" to use it with a build of that fork.
- **PC games** run on their own system; a Windows game runs on Linux through umu-launcher
  (Proton) or Wine, and on macOS through Wine.
- A game's menu has Game settings: the settings of that game only, such as KytyPS5's "Video out"
  and "Extra arguments".

### Themes

Themes lists the built-in theme, `default`, and the themes in `<data-dir>/extensions` (one
folder each) and in `extensions/` next to the program. Open one and choose "Use this theme". The
default theme's options are "Layout direction" (right to left) and "Animations" (off makes the
transitions cuts). Third-party plugins are themes only: they are data, and run no code.

## Limits

- Retail packages don't install: their keys are on the console. Only unsigned PS5 packages,
  exFAT images and PS4 fake packages do.
- Nexus doesn't notice a game it didn't start.
- KytyPS5 and shadPS4 don't provide the shader cache screen.
- shadPS4's own build shows no version on its page.
- The PC library shows only games Nexus installed: each has a `.nexus-game.json` in its folder.
- "This PC" judges by the numbers in each emulator's manifest, not by how fast a game runs here.
- "Download only through a VPN" isn't available on Windows: the torrent library (librqbit)
  can't bind a connection to an interface there. On macOS Nexus can't tell a VPN from the
  system's own tunnels, so you pick the network; connect it with the VPN's own app.
- The catalog refresh brings an existing catalog up to date; it can't build one from nothing.
  With "Download only through a VPN" on it waits for the VPN, but its own requests don't go
  through the VPN. A release it adds has no cover and no title ID, so no store can describe it:
  the forum's topic pages ask for a browser check, which only the catalog collector (a Chrome
  window, `make catalog`) passes. The next catalog it makes brings them.
- On a Wayland desktop other than KDE Plasma, a native Wayland game doesn't come to the front
  by itself.
- On macOS, without the Accessibility permission a notice says so, and the controller stays
  with the game until the PS button.
- macOS and Windows have no installer: the app and the folder run from where you put them.

## For contributors

Nexus is a Rust workspace with a Slint UI. A small core runs plugins: emulator and service
plugins are compiled in and talk to the host only through `nexus-api`; themes are data
(`theme.toml` and Slint markup) that the UI host loads, swaps and rolls back. `make check` runs
what CI runs: the format check, clippy with warnings as errors, and every test.

| Document | What it covers |
|---|---|
| [docs/architecture.md](docs/architecture.md) | How the core, the plugins and the UI host fit together |
| [docs/development.md](docs/development.md) | Building, testing, packaging and CI |
| [docs/plugins.md](docs/plugins.md) | Writing an emulator or service plugin |
| [docs/themes.md](docs/themes.md) | Writing a theme |
| [docs/reference/desktop.md](docs/reference/desktop.md) | Windows, focus and start at login on each system |
| [docs/reference/archives-and-pc-games.md](docs/reference/archives-and-pc-games.md) | Archives in downloads, and PC games |
| [docs/reference/default-theme.md](docs/reference/default-theme.md) | The default theme, screen by screen |
| [docs/signing.md](docs/signing.md) | Signing and notarizing the release packages |
| [docs/nexus-os.md](docs/nexus-os.md) | What Nexus and Nexus OS expect from each other |
| [os/README.md](os/README.md) | Building and testing Nexus OS, its images and its USB stick |
| [docs/third-party-notices.md](docs/third-party-notices.md) | Code and libraries from other projects, and their licences |
| [scripts/catalog/README.md](scripts/catalog/README.md) | Making and refreshing the catalog files |

### What is where

| Path | What it is |
|---|---|
| `crates/nexus-api` | The plugin contract. The only crate a plugin depends on. |
| `crates/nexus-ui-api` | The theme contract: view models, screens, slots, tokens, `theme.toml`. No toolkit code. |
| `crates/nexus-core` | The core: plugin lifecycle, settings, downloads, torrents, installs, launches, updates. No UI code. |
| `crates/nexus-ui-host` | Shows themes with Slint: inheritance, assets, input, switching with rollback. |
| `crates/nexus-app` | The `nexus` binary: the desktop, controllers, sounds, trailers, archives, torrents and VPNs of each system. |
| `crates/nexus-testkit` | The fake host, fakes for HTTP and processes, and the plugin contract suites. |
| `plugins/emulators/kytyps5` | PS5, with KytyPS5: its builds, PS5 games, installs, community compatibility. |
| `plugins/emulators/shadps4` | PS4, with shadPS4: its builds, PS4 games, installs with updates and DLC, community compatibility. |
| `plugins/emulators/pc` | PC games: installs a release, finds what starts it, and runs it. |
| `plugins/services/release-catalog` | The Game Catalog: each console's catalog, fetched or refreshed from its forums. |
| `plugins/services/playstation-store` | Official art and details for PS4 and PS5 games. |
| `plugins/shared/emulator-kit` | Code the emulator plugins share (not a plugin): GitHub releases, build folders, the game folder scan, compatibility lists, the PS5 package and exFAT readers. |
| `plugins/themes/default` | The built-in theme, `default`. It has every screen, component and token. |
| `tests/` | Tests across the crates: the core, the contracts, and the core with the theme host. `tests/fixtures/themes` holds the test theme `fixture-grid`. |
| `third_party/oozextract` | The Kraken decompressor, vendored. |
| `assets/` | The bundled catalogs, the catalog sources and the icons. |
| `scripts/` | Checks, packaging, the AppImage, `install.sh`, and the catalog collector. |
| `os/` | Nexus OS: its images, its USB stick, and its own releases. |

### Licence

Nexus is licensed under the GNU General Public License, version 2 or later (`LICENSE`), because
the PS4 package reader is a port of shadPS4 code. The default theme's fonts, Inter and Tajawal,
are under the SIL Open Font License 1.1. Other code and the libraries the packages ship (libmpv,
FFmpeg, libarchive) keep their own licences: see
[docs/third-party-notices.md](docs/third-party-notices.md).
