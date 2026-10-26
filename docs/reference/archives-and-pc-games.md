# Downloads, archives, installs and PC games

This page describes how a release goes from the Catalog to the Library today: the torrent
download, unpacking its archives, the install, removing and uninstalling, and the PC games
plugin. The host and its ports are in [architecture.md](../architecture.md). Plugins are in
[plugins.md](../plugins.md). The Downloads screen and its sheets are in
[default-theme.md](default-theme.md#downloads).

## The flow

1. **Download.** The Game Hub's Download button opens the download sheet. "Look up metadata"
   reads the torrent's file list (`resolving`), then the download waits for the player
   (`ready`). Download checks the free space and the pieces already on disk (`checking`), then
   downloads the rest (`downloading`).
2. **Unpack.** When the download is `complete`, the core looks for archive sets in it. When
   there are any, it unpacks them in the background, each into its own folder.
3. **Install.** Install on the download's card opens the install sheet. Install hands the
   download's folder to the emulator's game installer, which copies (or moves) the game into a
   game folder. Install unpacks first when the archives aren't unpacked yet.
4. **Play.** The game is in the Library. The download's card offers Play.

Each step is separate. Nothing in a download runs on the computer during these steps.

## Torrents

The core's side is `crates/nexus-core/src/torrents.rs`. The app plugs in the `Torrents` port
on librqbit 9.0.1 (`crates/nexus-app/src/torrents.rs`).

### Where files go

A download goes to `<folder>/torrent-<info hash>`. `<folder>` is, in order:

- the folder the player typed on the download sheet, which must be inside one of the
  emulator's packages folders;
- the folder last picked for that console (`app.download_folder.<console>`);
- `<console>` in the emulator's first packages folder (`PS5` when the release doesn't say).

A typed folder is saved for the next download of the same console. At most 32 downloads are
kept. A start checks the free space first: "Not enough free space in the download folder:
<needed> needed, <free> free".

### States

| State | Meaning |
|---|---|
| `resolving` | Reading the torrent's file list from the swarm, at most 3 minutes |
| `ready` | The file list is in. The player looks at the files, then presses Download |
| `checking` | Checking the pieces already on disk |
| `downloading` | Downloading |
| `paused` | Paused by the player, by a restart, or by the VPN rule |
| `complete` | Every piece is there |
| `failed` | It stopped. The error says why |
| `cancelled` | Stopped by Cancel. The files stay |

Each download is saved in the config under `app.torrents.<info hash>`. After a restart:

- a download that was `resolving` comes back `failed`: "The app stopped before the torrent's
  files were read: try again.";
- one that was checking or downloading comes back `paused`;
- a finished one seeds again after a check of its files, while seeding is on.

A finished download posts a success notice: "Download complete: <title>. Install it from
Downloads." A download that fails posts an error notice: "Download stopped: <title>. Open
Downloads to see why". Both open Downloads.

### The session and fastresume

There is one librqbit session, made at the first use:

- no local peer discovery, at most 50 peers, two torrents starting at once;
- uploads capped at `SEED_UPLOAD_LIMIT`, 128 KiB/s;
- each torrent's metadata kept as `torrents/<info hash>.torrent` in the data folder, so a start
  after a restart doesn't ask the swarm for it again.

librqbit keeps which pieces each torrent was checked to have, in `torrents/resume/` (fastresume).
A launch then reads one piece of each file instead of every byte. The core decides which
downloads start, so librqbit's own list of torrents, `torrents/resume/session.json`, is deleted
each time a session is made. Stopping a torrent forgets its pieces, and its next start checks it
all.

### Seeding

"Keep sharing finished downloads" (`app.seed_downloads`) is on by default. While it is on, a
finished download seeds at most 128 KiB/s. After a restart it is added paused, which checks the
pieces on disk without downloading. It seeds when every piece is there. When one is missing it
says: "The downloaded files were moved, changed or deleted, so this download can't seed.
Remove it, or download it again."

Stop sharing stops one download until the next start. Turning the setting off stops every
download from seeding. A download that finishes with the setting off is paused at once.

### Quitting

At quit the core stops every torrent first (`Torrents::stop_all`), then deactivates the
plugins. The session stops within 5 s, then gets 200 ms to write down each torrent's pieces.
The downloads stay as they were, and the next launch starts them again. The rest of quitting is
in [desktop.md](desktop.md#quitting).

### Downloads through a VPN

"Download only through a VPN" (`app.vpn_only`) is off by default. The rules are in
`crates/nexus-core/src/vpn.rs`; what each system can do is in
[desktop.md](desktop.md#downloads-through-a-vpn).

- The VPN in use is the one the player picked (`app.vpn`). With none picked, it is the first
  connected VPN, where the system can tell VPNs from other networks.
- The torrent port binds every socket of the session to the VPN's interface: peers, the
  listener, DHT, trackers and UPnP. A new interface stops the session, and the next start makes
  a new one.
- Without the VPN, a download doesn't start, not even to read its file list. It is paused and
  marked waiting ("Paused · waiting for your VPN").
- The kill switch: while a download runs or waits, the core checks the VPN every 2 s. When the
  VPN drops or another one takes its place, every running download is paused and marked
  waiting, and the connections close. When a VPN is back, only the waiting downloads start
  again.
- A restart forgets the waiting: those downloads come back paused.
- Plugins ask the same gate before they fetch in bulk, so the catalog refresh waits too.

## Archives

### Sets

An archive set is one archive, or one archive cut into parts. The core finds sets by their
names (`crates/nexus-core/src/archives.rs`), at the top of the download or one folder down.
Archives deeper than that are a game's own data, and are left as they are.

| Names | Kind |
|---|---|
| `name.part1.rar`, `name.part2.rar`, … | RAR, in parts |
| `name.rar` with `name.r00`, `name.r01`, … | RAR, in parts (`.r00` is part 2) |
| `name.rar` | RAR |
| `name.7z.001`, `name.7z.002`, … | 7z, in parts |
| `name.7z` | 7z |
| `name.zip` | zip |
| `name.tar` | tar |
| `name.tar.gz`, `name.tgz` | tar.gz |
| `name.tar.xz`, `name.txz` | tar.xz |
| `name.tar.bz2`, `name.tbz2`, `name.tbz` | tar.bz2 |
| `name.tar.zst`, `name.tzst` | tar.zst |
| `name.iso` | ISO |
| `name.cab` | cab |
| `name.sh` that starts with `#!` and has a zip's end record in its last 64 KiB | GOG installer |
| `name.sh` or `name.run` that starts with `#!` and has an `arcsz=<bytes>` line in its first 128 KiB | script installer (Kron4ek's YAD installers): the archive is the file's last `<bytes>` |

Names match in any case. A set can have at most 999 parts. A set with a missing part ("part 2
of game is missing") or a doubled one ("<a> and <b> are the same part of one archive") is
refused before anything is written.

The emulator kit's `archive_sets::set_name` (`plugins/shared/emulator-kit/src/archive_sets.rs`)
names a file's set by the same rules, so a plugin finds each set's unpacked folder.

### Readers

The app plugs in the `Archives` port (`crates/nexus-app/src/archives.rs`). It has one call:
`unpack(kind, parts, into, progress, cancel)`, on real paths in the download's folder.

| Kind | Reader |
|---|---|
| zip, GOG installer | the `zip` crate, which finds the zip from its end, past the script |
| tar, tar.gz | the `tar` crate, with `flate2` for gzip |
| 7z | `sevenz-rust2` 0.23.0, with a split 7z read as one stream |
| RAR (3, 4 and 5), tar.xz, tar.bz2, tar.zst, ISO, cab | libarchive, loaded at run time |
| script installer | libarchive, reading the file from the archive's start as a stream |
| a RAR or script installer libarchive can't read | `unar` (The Unarchiver), when it is installed: `lsar -j` lists it for the checks, then `unar` unpacks it; a script installer's archive is copied out beside the unpack folder first and removed after |
| 7z with BZip2, Zstandard, Brotli, LZ4, LZS, Lizard or Deflate64 | libarchive |

`sevenz-rust2` is built without those codecs, so a 7z that uses one goes to libarchive.

libarchive (`crates/nexus-app/src/libarchive.rs`) is loaded with `libloading`, and must have
RAR 5 support. The app looks for it in this order:

- the package's copy: `lib/` beside the binary, `../lib/` in the AppImage, `../Frameworks/` in
  `Nexus.app`, the binary's own folder on Windows;
- the system's: `libarchive.so.13` on Linux; Homebrew's keg and then the system's on macOS.

`scripts/bundle-libarchive.sh` ships libarchive 3.8.9, built from its release with only its
compression codecs, on Linux and macOS, and MSYS2's build on Windows. Without libarchive, such
an archive fails with the fix, for example "RAR archives need libarchive, which isn't here:
install libarchive13 on Debian and Ubuntu, libarchive on Fedora".

RAR goes through libarchive, which reads RAR with its own BSD-licensed code, and not through
UnRAR. UnRAR's licence adds terms the GPL doesn't allow, and Nexus is GPL-2.0-or-later. See
[third-party-notices.md](../third-party-notices.md).

### Checks

Each archive is read twice. The first pass lists it and refuses before anything is written:

| Problem | Error |
|---|---|
| An entry needs a password | "This archive needs a password, which Nexus can't ask for" |
| A path is absolute or has `..` | "<name> leaves the folder" |
| A link points out of the folder | "<name> links out of the folder" |
| Not enough room | "This archive needs <size> free, and the disk has <free>" |

The room check adds 64 MiB of spare to the unpacked size. A disk that doesn't report its free
space counts as having room.

The second pass writes. Before each entry it checks again on the disk: nothing is written
through a link ("<name> would write through a link"), and the path stays inside the folder once
links are followed. Files keep their Unix mode, so a program stays a program. Links are made
only on Unix. Devices and pipes are left out. Progress is reported every MiB, and a cancel
stops the work at the next MiB: "The unpacking was cancelled".

### Where sets go, and the phases

Each set unpacks into `unpacked/<folder>/<name>/` in the download's folder, for example
`unpacked/game/` for `game.part1.rar`, or `unpacked/Extras/dlc/` for `Extras/dlc.rar`. The
folder is named as the set's first part spells it. A set is written into
`unpacked/<name>.part/`, which moves into place when the set is done. A set whose folder is
already there is skipped.

A download's unpacking is saved with it, in these phases:

| Phase | Meaning |
|---|---|
| (none) | The download holds no archive. It installs as it is |
| `waiting` | Its archives aren't unpacked yet |
| `unpacking` | Unpacking. Progress goes from 0 to 1 over every set, saved at each whole percent |
| `unpacked` | Every set is unpacked |
| `failed` | A set failed. The error says which and why |
| `cancelled` | The player stopped it |

When a set fails, its `.part` folder goes, and so do the sets this run already unpacked. A
download that was `unpacking` when the app closed comes back `failed`: "The unpacking stopped
when Nexus closed: Install unpacks again." Its `.part` folders are removed.

Unpacking starts by itself when the download finishes. On the card, Cancel reads "Stop
unpacking" while it runs. After a failed or stopped unpacking, Install reads "Unpack and
install": it unpacks, then installs.

"Remove unpacked files" deletes the download's `unpacked/` folder only. The downloaded files
stay, and the download can still seed. The phase goes back to `waiting`.

An archived download takes about twice the game's size until "Remove unpacked files", unless
the install moved the unpacked game (see below).

## Installing

### The install sheet

Install on a finished download opens the install sheet, with its "Game folder" row. The game
folder is, in order: the folder the player typed (which must be inside one of the emulator's
game folders), the one last picked for the console (`app.install_folder.<console>`), or
`<console>` in the first game folder. A typed folder is saved for the next install.

Before it starts, Install checks that the game folder's disk has room for the download's size:
"Not enough free space in the game folder: <needed> needed, <free> free". The reason it doesn't
start stays on the sheet.

The sheet's note comes from the emulator's manifest (`install_note`), then the host's: "Nexus
first unpacks the RAR, ZIP, 7z and TAR archives in the download, but not one with a password."

### The install job

The core owns the job (`crates/nexus-core/src/game_installs.rs`). The plugin's `GameInstaller`
gets the download's folder as a `HostPath` and an `InstallJob`. Plugins never see real paths.

- The plugin writes into a work folder, `<dest>.part`, and moves it into place at the end. On
  an error the work folder goes.
- Progress reaches the screen at most every 200 ms.
- Cancel install stops the job at the plugin's next check.
- A running install's work folder is saved under `app.game_installs`. The next start removes
  the work folders of installs that were running when the app stopped.
- One install of the same source at a time: "<source> is already installing".
- A failed install posts "The install failed: <error>", which opens Downloads.
- When the install ends, the Library refreshes, and the card offers Play.

### Moving instead of copying

`InstallJob::take(from, to)` lets a plugin move a folder instead of copying it. The host agrees
only when:

- `from` is inside the download's `unpacked/` folder (`disposable`): a copy the host made itself,
  set only while the download's phase is `unpacked`;
- `to` is not there yet;
- both are on one disk. A rename across disks fails with "crosses devices", and the plugin
  copies.

After a move, the empty folders it left go, up to `unpacked/`. When `unpacked/` is gone, the
download's phase goes back to `waiting`.

KytyPS5 uses it: an unpacked PS5 game folder from an archive moves into the game folder in one
step. shadPS4 and the PC plugin copy.

### PS5 (KytyPS5) and PS4 (shadPS4)

Both read a download up to 8 folders deep, and the archives' unpacked folders are part of it.

| | KytyPS5 (PS5) | shadPS4 (PS4) |
|---|---|---|
| Unpacked game folder | `sce_sys/param.json` with a non-empty `eboot.bin` beside `sce_sys` | `sce_sys/param.sfo` of category `gd` with a title id, and a non-empty `eboot.bin` |
| Which folder | The one nearest the top. Two at the same depth: "Several folders hold a game (…); choose a single release folder" | The first game folder found |
| Packages | The PS5 package or exFAT image that holds the game, nearest the top. Two at the same depth are refused | Every PS4 package, as one job: the game, then its update, then its DLC |
| Goes to | `<game folder>/<title id>-app0` | The game: `<game folder>/<root>`. Its update: `<game>-patch` beside it. DLC: `addcont/<root>` in shadPS4's data folder |

shadPS4's checks, before anything is written:

- "This download has 2 games; install one game at a time"
- "This download has 2 updates for one game; it can install only one"
- an update in the same download as its game must be for that game
- an update alone needs its game installed: "this is an update for <title id>: install the game
  first"
- a game or update that is there already: "<path> is already installed"
- an add-on that is installed already stays as it is; when every add-on is there: "every add-on
  in this download is already installed"

The job reports one progress for all its packages.

## Removing a download

Remove on a card opens a sheet with three answers:

| Answer | Action | Effect |
|---|---|---|
| "Remove from Downloads" | `remove` | Forgets the download. Its folder stays |
| "Delete files too" | `delete-files` | Forgets the download and deletes its folder |
| "Not now" | | Closes the sheet |

Remove is offered only when the download isn't running or seeding. "Delete files too" also
waits while its archives unpack or its install runs. It deletes the folder only when its name
is `torrent-<info hash>`, the folder Nexus made for that download. The downloaded files and
their unpacked copies go with it. An installed game is elsewhere, and stays.

Cancel never deletes files. It stops the download, which reads "Cancelled · the files stay".

## Uninstalling a game

The game's menu has "Uninstall", last and in red, when the Game Hub's details say
`can_uninstall`:

- the game's details name its folder;
- that folder is not one of the player's folders itself;
- the emulator's manifest grants it write access to that folder;
- the folder resolves on the disk.

It asks first, "Uninstall <title>?", with the keys on "Not now". Uninstall deletes the game's
folder through the plugin's folder grants, refreshes the Library, closes the Game Hub and posts
"Uninstalled <title>". The core refuses while the game runs ("Stop the game before you
uninstall it") or while its emulator installs a game ("Wait until the install ends"). A
download it came from stays in Downloads.

## PC games

The PC plugin is `emu-pc` (`plugins/emulators/pc`). It plays the console `PC` on Linux, macOS
and Windows, with the capabilities catalog, launcher, slots, game installer and configurator.
It has no settings of its own; each game has Starts, and on Linux a Resolution (see Game Hub).

Its modules: `install.rs` (the install), `probe.rs` (what a file is, and the order of a game's
files), `manifest.rs` (the manifest file and the default start), `library.rs` (the installed
games, their details and Game Hub section), `launch.rs` (Play), `settings.rs` (a game's
settings) and `runner.rs` (umu-launcher and Wine). `lib.rs` joins them.

Permissions:

- the games folder, with write; the packages folder, read only;
- processes: `folder:games` (any file in a game folder), the umu-launcher it installs,
  `system:umu-run`, `system:wine`, `system:wine64`, `system:sh`, `system:gamescope` and
  `system:gamemoderun`.

### Library

Games install to `PC/` in the player's first game folder, or the folder picked on the install
sheet. A game is a folder in `PC/` with a `.nexus-game.json` manifest (version 2): its `name`,
and `starts`, every file in it that can start the game, best first, each a `path` inside the
folder and its `kind`. The manifest holds what the install found and doesn't change after. The
folder's name is the game's id. Search matches the name.

A folder the player copied into `PC/` by hand, with no manifest, gets one the first time the
library or Play sees it: its files are probed as an install probes them (the nearest 4000 first),
and the folder's files that should be programs get the flag. Its name is GOG's `gameinfo` (first
line, at the folder's top), else a portable Wine wrapper's, else the folder's name, with a
capital at each word when it has none (`my_game` is "My Game"). A folder with nothing that
starts a game stays as it is and isn't probed again until Nexus restarts. Hidden folders and
`<name>.part` (an install still writing) are skipped.

A `cover.jpg`, `cover.png` or `cover.webp` in a game's folder, in that order, is its cover. It
comes before the cover of a catalog release the game matches.

The first manifest (version 1: `launch`, `kind` and up to 8 `alternatives`) reads as the same
list, `launch` first. A game with one is named by GOG's `data/noarch/gameinfo` (first line) or a
portable Wine wrapper's `game_info/game_info.txt` beside its first file (sixth line) when it
has one: it was installed before its name came from them.

### What a file is

Files are known by their first 4096 bytes, not their names, since downloads lose the program
flag:

| First bytes | Probe | Kind |
|---|---|---|
| `#!` | Script | `script` |
| ELF with `AI\x02` at byte 8 | AppImage | `appimage` |
| ELF program, or a PIE with an interpreter, x86 or x86-64 | Elf | `native` |
| Mach-O, 32 or 64 bit, or a universal binary | MachO | `macapp` |
| `MZ` with a `PE\0\0` header | Exe | `windows` |

At most 4000 files of a release are probed: the nearest the game's folder first, so a start
script beside a bundled runtime's thousands of files is still found.

### Install

The installer reads the download after the host unpacked it. For each archive set, it decides
whether the set is the release's packaging, by what its folder holds beside the set:

| Beside the set | Its unpacked folder is used |
|---|---|
| Nothing that starts a game | Yes |
| Only a start script | Only when the unpacked files hold a program |
| A program | No: the archive is the game's own data, and is copied as it is |
| (a GOG or script installer) | Always |

Everything else is copied as it is. The files of a used set go where the archive was. The
torrent's own top folder is the game's folder, and the game's name comes from it, cleaned of
release tags: "[GOG] Celeste (v1.4.0.0) [amd64] [Multi]" is "Celeste".

A script installer's archive usually holds the game's folder ("Human Fall Flat/"): its files go
at the game's top, and the folder names the game. Without one, the installer's file name does,
without its version, each word with a capital: "cuphead_v1.3.4.run" is "Cuphead".

libarchive can't read every RAR: Cuphead's installer holds a RAR 5 with a 1 GB dictionary
("Declared dictionary size is not supported"). Nexus then unpacks it with `unar`, which needs
room for a copy of the archive too. Without `unar` the unpacking fails: "… Install unar to unpack
it, or run the installer by hand". Nothing installs then.

The plugin then lists every file that can start the game. The order compares, one after the
other:

1. helpers last: a name with `setup`, `install`, `update`, `config`, `crashhandler` and the like;
2. what the file is: a start script by its name (`start`, `start.sh`, `play.sh`, `run.sh`, …),
   an AppImage, a Mac program inside a `.app`, a Linux program, another Mac program, another
   script, a Windows `.exe`;
3. how deep it is: nearer the game's folder first;
4. a 64-bit program before a 32-bit one;
5. its path.

The game's name plays no part. Libraries (`.so`, `.dll`) and the programs in bundled runtimes'
folders (`lib/`, `wine/`, `runtime/`, `jre/`, …) aren't listed. The order doesn't depend on the
system; what Play starts by default does (see Game Hub). A GOG installer's `gameinfo` or a
portable Wine wrapper's `game_info.txt` beside the first file names the game; neither decides
what starts.

When one unpacked folder alone is the game, the host moves it into the games folder whole, on
the same disk: links and empty folders stay, as a bundled Proton's libraries and Wine prefix need
(Proton makes its drive links in the prefix's empty `dosdevices`). Otherwise the files are copied
one by one, and a link arrives as a copy of what it points to.

On Linux and macOS the program flag is set again on scripts, AppImages and programs. A game
whose folder exists already is refused: "<name> is already installed". A download with nothing
that starts a game is refused: "Nothing in this download starts a game".

### What runs where

| Kind | Linux | Windows | macOS |
|---|---|---|---|
| `script` | runs | can't | can't |
| `appimage`, `native` | runs | can't | can't |
| `macapp` | can't | can't | runs |
| `windows` | `umu-run`, else `wine`, else `wine64` | runs | `wine`, else `wine64` |

So on a Mac, a release with a start script and a `.exe` starts the `.exe` through Wine.

Play runs the file Starts chose, as it is: its kind decides how, and nothing else. Nexus never
starts another file in its place. When the file fails (a wrapper's 32-bit Wine on a system
without 32-bit libraries, say), the notice "closed right after it started" names its log, and
the player picks another file in Starts. `prepare` builds the launch:

- The file starts from its own folder, which is the working folder.
- A script without `#!` runs through `sh`.
- An AppImage gets `APPIMAGE_EXTRACT_AND_RUN=1`, so it runs without FUSE.
- A Windows program off Windows runs through the runner, with the `.exe` as its argument and
  its own Wine prefix, `prefixes/<game id>` in the plugin's data folder (`WINEPREFIX`). With
  `umu-run`, `GAMEID=umu-default` and `PROTONPATH=GE-Proton`: umu downloads and updates Proton
  by itself. On a system without 32-bit libraries it also gets `PROTON_USE_WOW64=1`.
- With a Resolution, all of it runs inside gamescope: `gamescope -f -w <width> -h <height> --`
  and then the program and its arguments.
- On Linux with GameMode installed (`gamemoderun` on the `PATH`), all of that runs through
  `gamemoderun`. GameMode raises the game's priority and the processor's speed while it runs;
  without a password only for a player in the `gamemode` group.

A game this system can't start installs anyway. Play then fails with the reason, for example
"Can't start Celeste: This is a Linux game: it can't run on Windows". Without a runner it is
"This is a Windows game: install umu-launcher (it runs the game with Proton) or Wine to play
it" on Linux, and "… install Wine to play it" on a Mac.

### Game Hub

- A "Starts" fact names the file Play starts.
- A "PC" section has the rows "Starts" and "Runs". "Runs" is "As it is", "With Proton, through
  umu-launcher", "With Wine", the runner hint, or why it can't run here.
- Every PC game has Game settings (the Game Hub's ⋯ menu). "Starts" lists each file it can
  start, in the manifest's order. Each is named by its file name, with the nearest folder that
  tells it apart when another file has the same name ("ENG/start.sh", "ENG/…/Ra2.exe"). The
  line under the row gives the whole path of the file Play starts.
- The default is the first start script by its name (`start.sh`, `run.sh`, …) that runs on
  this system, else the first file that runs here, else the first file.
- Picking another file is kept as the game's `starts` setting, and Play and the "Starts" fact
  and row then use it. Only a file in the manifest can be picked. The manifest file doesn't
  change. A `starts` setting that names a file the manifest no longer has is ignored.
- On Linux, every game has a "Resolution" setting there too: "Your screen" (the default) or a
  size, which runs it inside gamescope.
- The game comes to the front, takes the pad and gives the launcher back as any game does (see
  [desktop.md](desktop.md#the-running-game)).

### Catalog

The release catalog plugin reads the PC catalog for the console the PC plugin names. Each
release gets the system it is for, from its game info's platform first, then from its title's
tags:

| Word | System |
|---|---|
| `linux`, `native`, `wine`, `amd64`, `x86_64`, `x64` | Linux |
| `windows`, `win`, `win64`, `win32` | Windows |
| `macos`, `mac`, `osx`, `mac os` | macOS |

A release with neither has no system mark. A Windows release counts as runnable everywhere. A
Linux or macOS release on another system says "This is a Linux release: it can't run on
Windows" on its row and on the download sheet. It still downloads and installs. A Catalog card
whose every release can't run here gets the badge "Can't run here".

## Emulator builds

Separate from downloads, an emulator plugin's own build is unpacked by the core
(`crates/nexus-core/src/unpack.rs`) into the plugin's data folder: tar.gz, zip, or an AppImage
inside a zip. The AppImage runs `--appimage-extract` (Linux only, at most 10 minutes), and only
when the plugin's process permission allows that file. The same path checks apply: no absolute
paths, no `..`, no links out of the folder, nothing written through a link. It unpacks into a
sibling `.unpacking` folder first, so a failure leaves the old build.

## Tests

- `tests/tests/seams/torrents.rs`, `unpack.rs`, `release_archives.rs`, `game_installs.rs`, `kyty.rs`,
  `shadps4.rs`, `pc.rs` and `vpn.rs` drive the core with fakes, as a player or a plugin does.
- The tests in `crates/nexus-app/src/archives.rs` unpack real zip, tar.gz, GOG, script installer and RAR files,
  and check the refusals and cancel. The RAR files in `crates/nexus-app/testdata/` come from the
  `unrar` crate's test data.
- `tests/tests/seams/real_archives.rs` installs the real KytyPS5 and shadPS4 release archives, with
  `NEXUS_REAL_ARCHIVES` set to a folder of them.
- A real transfer between two local librqbit sessions runs with `NEXUS_TORRENT_TESTS=1`.

## Limits

- Archives with a password are refused.
- Nothing in a download runs during install, GOG and script installers included.
- Nexus doesn't download Wine, Proton or umu-launcher.
- Downloads only through a VPN is not available on Windows.
