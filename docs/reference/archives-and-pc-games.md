# Archives and PC games

When a download finishes, Nexus unpacks the archives in it. The PC plugin then installs the
game, finds the file that starts it, and runs it on Play. Nexus never runs anything in a
download while it unpacks.

This page is for contributors and for players who want to know what Nexus does with a
download. The core and its ports are in [architecture.md](../architecture.md). Plugins are in
[plugins.md](../plugins.md).

## Where the work happens

| Part | Where | Job |
| --- | --- | --- |
| Archive sets | `crates/nexus-core/src/archives.rs` | Finds the sets in a download by name or by bytes. |
| Archive plan | `crates/nexus-core/src/archive_plan.rs` | Decides what an unpack may write. |
| Unpack flow | `crates/nexus-core/src/game_installs/unpacking.rs` | Runs the sets one by one and keeps the state. |
| `Archives` port | `crates/nexus-core/src/ports.rs` | The one call, `unpack`, that an adapter implements. |
| Real adapter | `crates/nexus-app/src/archives.rs`, `libarchive.rs` | `HostArchives`: the format readers. |
| Fake adapter | `crates/nexus-testkit/src/archives.rs` | `FakeArchives`: the same plan, no files. |
| PC plugin | `plugins/emulators/pc/` | Installs, lists and runs PC games. |
| Shared code | `plugins/shared/emulator-kit/` | Reads a download's folder; finds archive set names. |

## Archive sets

An archive set is one archive, or one archive cut into parts. `part_of` in
`crates/nexus-core/src/archives.rs` reads a file name and says which set and which part it is.

| Name | Kind | Part |
| --- | --- | --- |
| `game.rar` | `Rar` | 1 |
| `game.part1.rar`, `game.part2.rar` | `Rar` | 1, 2 |
| `game.rar` with `game.r00`, `game.r01` | `Rar` | 1, then 2, 3 |
| `game.7z`, `game.7z.001`, `game.7z.002` | `SevenZ` | 1, then 1, 2 |
| `game.zip`, `game.tar` | `Zip`, `Tar` | 1 |
| `.tar.gz`, `.tgz` | `TarGz` | 1 |
| `.tar.xz`, `.txz` | `TarXz` | 1 |
| `.tar.bz2`, `.tbz2`, `.tbz` | `TarBz2` | 1 |
| `.tar.zst`, `.tzst` | `TarZst` | 1 |
| `.iso`, `.cab` | `Iso`, `Cab` | 1 |

Names are matched in lower case. Only RAR and 7z can be cut into parts.

Two more kinds are found by their bytes, not their names. A file that ends in `.sh` or `.run`
is a script installer when:

- It is a `Gog` installer: it starts with `#!`, is at least 1024 bytes long, and has a zip end
  record (`PK\x05\x06`) in its last 64 KiB. Only a `.sh` file can be a `Gog` installer.
- It is a `Script` installer: it starts with `#!` and states its archive's size on an
  `arcsz=` line within the first 128 KiB (`SCRIPT_HEAD`). The archive is the last bytes of the
  file. `embedded_archive` finds where it starts. Kron4ek's YAD installers do this.

Nexus never runs these scripts. It reads the archive inside.

`scan` looks at the top of the download and one folder down. A set deeper than that is left
alone. The folder `unpacked` is skipped. Two files that are the same part of a set, or a
missing part, make the set carry a `problem`. Nothing is written for such a set, and the
download fails with the problem as its text. The test `a_set_with_a_missing_part_fails_before_anything_is_unpacked`
in `tests/tests/seams/release_archives.rs` checks this. A set can have at most 999 parts
(`MAX_PARTS`).

## When Nexus unpacks

`Unpack::of` scans a download when the torrent finishes (`torrents/mod.rs`). The download
then has one of these unpack phases (`UnpackPhase`):

| Phase | Meaning |
| --- | --- |
| `none` | No archive set. The download installs as it is. |
| `waiting` | Sets found, not unpacked yet. |
| `unpacking` | An unpack is running. |
| `unpacked` | Every set is unpacked. |
| `failed` | An unpack failed. The error says why. |
| `cancelled` | The player cancelled. |

Unpacking starts in two ways:

- When a download finishes with phase `waiting`, the core starts the unpack at once
  (`torrent_event` in `release_downloads.rs`).
- When the player presses Install on a download in phase `waiting`, `failed` or `cancelled`,
  the unpack runs first and the install starts after it (`install_download` in
  `game_installs/mod.rs`). In phase `unpacking`, Install does nothing.

Each set goes to `unpacked/<folder>/<name>/` inside the download, where `<folder>` is the
set's folder if it has one. For example `a/game.rar` unpacks to `unpacked/a/game/`.

`run_unpack` in `game_installs/unpacking.rs` works set by set:

1. It skips a set whose destination folder exists already.
2. It makes a work folder, `unpacked/<name>.part`, and empties it.
3. It reads the free space of the disk that holds the work folder.
4. It calls `Archives::unpack` into the work folder.
5. On success it renames the work folder to the destination.
6. On failure it removes the work folder and every set this run already unpacked, then
   reports the error.

Progress is the share of the bytes of all sets, saved at each whole percent.

Cancel sets a flag. The adapter stops soon after and returns `ArchiveError::Cancelled`. The
phase becomes `cancelled`. The download stays.

If Nexus closes during an unpack, `Unpack::restored` runs at the next start. It sets the phase
to `failed`, with the text "The unpacking stopped when Nexus closed: Install unpacks again.",
and removes the work folders.

Remove unpacked files (`remove_unpacked`) deletes the folder `unpacked` and sets the phase
back to `waiting`. The torrent's own files stay, and so does the seeding. It does nothing
while an unpack runs.

When the install moves the unpacked folder into the games folder instead of copying it, the
phase also goes back to `waiting`, because the unpacked copy is gone.

## The archive plan

Every `Archives` adapter reads an archive twice. The first pass lists it. `plan_unpack`
checks the listing. The second pass writes only what the plan holds.

`plan_unpack` refuses the whole archive at the first entry that breaks a rule:

| Rule | Error |
| --- | --- |
| An entry needs a password. | `Password` |
| A path is absolute, has `..` that climbs out, or is empty. | `Escapes` with `Leaves` |
| A link points to an absolute path, or climbs out with `..`. | `Escapes` with `LinksOut` |
| A hard link names a target that is not a plain path inside the folder. | `Escapes` with `LinksOut` |
| The files' bytes plus the spare do not fit in the free space. | `NoRoom` |

The spare is `UNPACK_SPARE`, 64 MiB. A disk that does not report its free space counts as
having room.

The plan leaves out the archive's own top, an entry such as `./` that some tars list first.
It also leaves out devices and pipes (`EntryKind::Other`).

The adapter checks the disk again during the second pass. `Work::place` in
`crates/nexus-app/src/archives.rs` refuses to write where a link is already on the disk
(`Escape::ThroughLink`), and where the path ends up outside the folder once links are
followed (`Escape::LeavesThroughLink`). If the second pass finds an entry the first pass did
not list, it fails with "the archive changed while it was unpacked".

A file written by the adapter keeps its Unix mode, so a program stays a program. Windows has
no modes. Symbolic links are made on Unix only.

## Format readers

`HostArchives` picks a reader by the set's kind.

| Kind | Reader |
| --- | --- |
| `Zip`, `Gog` | `zip` crate. It finds a zip's start from its end, past the script. |
| `Tar`, `TarGz` | `tar` and `flate2` crates. |
| `SevenZ` | `sevenz-rust2` crate. A split 7z is read as one stream. |
| `Rar`, `TarXz`, `TarBz2`, `TarZst`, `Iso`, `Cab`, `Script` | libarchive. |
| `SevenZ` with a codec `sevenz-rust2` lacks | libarchive. |

`SEVEN_ELSEWHERE` lists the codecs that send a 7z to libarchive: BZip2, Zstd, Brotli, LZ4,
LZS, Lizard and Deflate64. A 7z listing that cannot be read for lack of a codec goes there
too.

A format reader decides nothing about safety. The plan does.

### libarchive

libarchive is loaded at run time (`libarchive.rs`), not linked. `candidates` lists where it
looks, package copy first, then the system:

| System | Package copy | System copy |
| --- | --- | --- |
| Linux | `lib/libarchive.so.13`, `../lib/libarchive.so.13` | `libarchive.so.13`, `libarchive.so` |
| macOS | `lib/libarchive.13.dylib`, `../Frameworks/libarchive.13.dylib` | Homebrew paths, `libarchive.13.dylib`, `/usr/lib/libarchive.2.dylib` |
| Windows | `libarchive-13.dll`, `archive.dll` | the same names |

The package paths are relative to the folder of the Nexus program. The library must also
support RAR 5, or Nexus treats it as missing and tries the next place.

If no libarchive loads, the unpack fails with `ArchiveError::Missing` and a hint from
`install_hint`: `brew install libarchive` on macOS, "a Nexus package, which ships it" on
Windows, and `libarchive13` on Debian and Ubuntu or `libarchive` on Fedora on Linux. The
packages ship a copy. See [third-party-notices.md](../third-party-notices.md).

### unar

A `Rar` set or a `Script` archive that libarchive cannot read comes back as
`ArchiveError::Unsupported`. `unpack` in `crates/nexus-app/src/archives.rs` then tries `unar`
(The Unarchiver), for example for a RAR 5 with a dictionary bigger than libarchive takes. It
only does this for those two kinds. It does not do it for a password, a path or a lack of
room.

Nexus needs both `lsar` and `unar` on the PATH. If either is missing, the error says
"Install unar to unpack it".

The flow is the same: `lsar -j -jss -nr` lists, `plan_unpack` checks, then `unar` writes.
These are the arguments for `unar`, from `Unar::unpack`:

```
-f -D -nr -o <folder> <archive>
```

`unar` reads only a whole file. For a `Script` archive, Nexus copies the archive out of the
script to a file beside the work folder, and removes the copy at the end. The copy needs
room on the same disk, and the plan counts it.

Nexus reads the progress from the lines `unar` prints. After `unar` ends, Nexus sets the Unix
mode that a RAR 5 made on Unix keeps, because `unar` leaves the program flag off.

### Errors

`ArchiveError` says why an unpack failed. The core shows its text. An adapter picks a fallback
by the kind, never by the text.

| Kind | Meaning |
| --- | --- |
| `Password` | An entry needs a password. Nexus cannot ask for one. |
| `Escapes` | A path or link gets out of the folder. |
| `NoRoom` | The disk is too small. The text gives both sizes in GB. |
| `Unsupported` | The reader cannot read this. Another reader may. |
| `Missing` | A library or program is not installed. The text says which. |
| `Corrupt` | The archive is damaged, cut short, or changed during the unpack. |
| `Io` | Reading the parts or writing the folder failed. |
| `Cancelled` | The player cancelled. |

## Unpacking a plugin's build

A second unpack exists for emulator builds, in `crates/nexus-core/src/unpack.rs`. It does not
use the `Archives` port. A file in a plugin's download plan can carry an `Unpack` with an `ArchiveFormat` (`TarGz`, `Zip`
or `AppImageInZip`) and a folder to unpack into. The core unpacks after the file passes its
checks. It unpacks into a sibling folder named `<folder>.unpacking` first and replaces the old
folder at the end, so a failure leaves the old contents.

It applies the same path rules (`inside`, `link_stays_inside`, `stays_inside`).

For `AppImageInZip`, the core takes the one AppImage out of the zip and runs it with
`--appimage-extract`. This is allowed on Linux only, and only when the plugin's
`permissions.process` allows that file. The run is killed after `EXTRACT_TIMEOUT`, 600
seconds. The result must be a `squashfs-root` folder.

## Testing an adapter

`contract::archives` in `crates/nexus-testkit/src/contract.rs` checks the refusals every
`Archives` adapter must make. An adapter gives a fixture that builds archives. The contract
checks that:

- A plain archive unpacks, and progress ends at the files' bytes.
- A `..` path is refused, and nothing is written.
- An absolute path is refused.
- A link to `../../../etc/passwd` or `/etc/passwd` is refused.
- An archive that needs a password is refused. If the fixture cannot make one, the check is
  skipped and says so.
- An archive bigger than the free space is refused.

The app runs this contract on `HostArchives`, with real zip files. The test kit runs it on
`FakeArchives`, which calls the same `plan_unpack`. The other seam tests are
`tests/tests/seams/release_archives.rs` for the whole flow and `tests/tests/seams/unpack.rs`
for plugin builds.

## PC games

The PC plugin is `emu-pc` (`plugins/emulators/pc/plugin.toml`). It runs on Linux, macOS and
Windows. It gives the capabilities `catalog`, `launcher`, `slots`, `game_installer` and
`configurator`.

PC games install from downloads only. The plugin's `candidates` returns nothing, so no game is
offered from a folder the player picks. The manifest's install note says so for the install
sheet: "Nothing in it runs until you press Play."

### Install

`begin` in `plugins/emulators/pc/src/install.rs` installs from a download folder. The core
has unpacked the archives by then. The plugin works in these steps:

1. It scans the download (`Download::scan` in the emulator kit). A download deeper than
   `SCAN_DEPTH` (64 folders), or with more than `SCAN_FILES` (1,000,000) files, is refused.
2. It decides which archive sets are the release's packaging. It replaces those with the
   files the core unpacked from them. The rules:
   - A script installer is always packaging.
   - A set with nothing beside it is packaging.
   - A set beside a start script is packaging only when what is inside holds a program.
   - A set beside a program is not packaging. It is the game's own data, and it is copied
     as it is.
3. It picks the game's name. GOG's `data/noarch/gameinfo` gives it first (`GOG_INFO`). Else
   the installer's own folder. Else the torrent's top folder. Else the name of the set. Else
   "Game". `display_name` cleans tags and versions off the name.
4. It reads the first 4096 bytes (`probe::HEAD`) of up to 4000 files (`PROBED`), nearest
   the game's folder first, and ranks them. A portable Wine wrapper (Kron4ek's) names its
   game on the sixth line of `game_info/game_info.txt` beside its start script.
5. It refuses a download with nothing that starts a game: "Nothing in this download starts a
   game".
6. It writes into `PC/<game>.part` and renames it to `PC/<game>` at the end. If
   `PC/<game>` exists, the install fails with "is already installed".
7. When one unpacked folder alone is the game, and the unpacked copy is the host's own, the
   plugin asks the host to move the whole folder instead of copying. The host copies when it
   cannot move. A bundled Proton needs the links and empty folders that a move keeps.
8. Off Windows it sets the program flag on every file that should have it, because downloads
   drop the flag.
9. It writes the manifest file.

The game goes to the folder the player picked for the install. Else it goes to `PC/` in the
first games folder. If there is no games folder, the install fails with "add a game folder in
Settings first".

### What a file is

Nexus reads a file's first bytes, not its name. `probe` in `probe.rs` tells:

| First bytes | Probe |
| --- | --- |
| `#!` | A script |
| ELF, x86 or x86-64, executable or position-independent with an interpreter | A Linux program |
| ELF with the AppImage marker | An AppImage |
| `MZ` with a `PE` header | A Windows program |
| Mach-O, or a universal binary | A Mac program |

An ELF for ARM or a big-endian ELF is `Other`. So is a library.

Each file that can start the game has a kind (`Kind`): `script`, `appimage`, `native`,
`macapp` or `windows`.

### What starts the game

`rank` puts the files in order. Helpers come last. These names mark a helper: `setup`,
`install`, `uninstall`, `redist`, `crashhandler`, `update`, `easyanticheat` and others in
`helper`. Then the order is:

1. A start script by its name. The names are in `START_NAMES`: `start`, `start.sh`,
   `start-game.sh`, `start_game.sh`, `play.sh`, `run.sh`, `launch.sh`, `game.sh`.
2. An AppImage.
3. A Mac program inside a `.app`.
4. A Linux program.
5. Another Mac program.
6. Another script.
7. A Windows program.

Within one group, a file nearer the game's folder comes first, a 64-bit program comes before
a 32-bit one, and then the path decides.

`rank` leaves out libraries (`.so`, `.dll`) and programs in the folders of bundled runtimes
(`RUNTIMES`: `lib/`, `wine/bin`, `runtime/`, `jre/` and others), except start scripts.

### The manifest file

The install writes `.nexus-game.json` in the game's folder (`GAME_FILE` in `manifest.rs`). It
holds the game's name and every file that can start it, best first. It is at most 64 KiB
(`MAX_GAME_FILE`). Its `version` is 2 (`GameManifest::VERSION`). The plugin still reads
version 1 and drops any path that is not a safe path inside the folder.

The default start on a system is the first start script that runs there. Else it is the first
file that runs there. Else it is the first file.

| Kind | Runs on |
| --- | --- |
| `script`, `appimage`, `native` | Linux |
| `macapp` | macOS |
| `windows` | Every system. Off Windows it needs Proton or Wine. |

### The library

The plugin lists the folders in `PC/` of each games folder. A folder is a game when it has a
manifest file. A folder named with a leading `.` or an ending `.part` is never a game.

A player can copy a game into `PC/` by hand. The first time the library sees the folder, the
plugin probes its files as an install does and writes its manifest. It also sets the program
flag off Windows. A folder with nothing that starts a game is left alone and is not probed
again until the next run.

A `cover.jpg`, `cover.png` or `cover.webp` in the game's folder is its cover, in that order.

### Settings

A game has these settings in its Game Hub (`settings.rs`):

- **Starts.** The file Play starts. It must be one of the manifest's files.
- **Resolution.** On Linux only. The choices (`resolution::CHOICES`) are `screen` and sizes from
  640 × 480 to 1920 × 1080. Anything but `screen` runs the game inside gamescope.

### Play

`spec` in `launch.rs` builds the launch. The file's kind decides how it runs:

| Kind | How it runs |
| --- | --- |
| `script` | As it is. If it does not start with `#!`, through `sh`. |
| `appimage` | With `APPIMAGE_EXTRACT_AND_RUN=1`, so it needs no FUSE. |
| `native`, `macapp` | As it is. |
| `windows` on Windows | As it is. |
| `windows` elsewhere | Through umu-launcher (Proton) or Wine. |

The game's folder is the working folder, which is the folder of the file that starts it. A
file that this system cannot run fails with a message such as "This is a Linux game: it can't
run on Windows".

On Linux, two more wrappers can apply:

- With a Resolution other than `screen`, the game runs inside `gamescope -f -w <width> -h
  <height> --`. If gamescope is not installed, Play fails and says to install it or reset the
  Resolution.
- If `gamemoderun` is installed, the game runs through it.

### Windows games off Windows

`runner` in `runner.rs` picks what runs a Windows game:

1. On Linux, the umu-launcher this plugin installed.
2. On Linux, `umu-run` on the system.
3. `wine`, then `wine64`.

On macOS only Wine is tried. If nothing is found, Play fails and says what to install.

Each game gets its own Wine prefix in the plugin's data folder, `prefixes/<game id>`. With
umu-launcher the plugin sets `GAMEID=umu-default` and `PROTONPATH=GE-Proton`. umu-launcher
downloads and updates Proton by itself. If the host does not run 32-bit programs, the plugin
also sets `PROTON_USE_WOW64=1`.

The plugin page has a "Windows games" section. On Linux it has a button, Install umu-launcher
or Update umu-launcher. The button does this (`install_umu`):

1. It asks GitHub's API for the latest release of `Open-Wine-Components/umu-launcher`.
2. It picks the asset named `umu-launcher-*-zipapp.tar` and the SHA-256 that GitHub lists for
   it. If GitHub lists none, the install stops.
3. It downloads the tar and checks the SHA-256. A mismatch stops the install.
4. It takes `umu/umu-run` from the tar, writes it beside the old one as `umu/umu-run.part`,
   sets the program flag, and renames it into place. A failed install leaves the old one.
5. It saves the release tag in `umu/version`.

The zipapp needs only Python 3.

### Permissions

The plugin declares these in `plugins/emulators/pc/plugin.toml`:

```toml
process = ["folder:games", "umu/umu-run", "system:umu-run", "system:wine", "system:wine64", "system:sh", "system:gamescope", "system:gamemoderun"]
network = ["api.github.com", "github.com", "*.githubusercontent.com"]
```

A PC game runs with the player's access to their files. Install only releases you trust.

## Limitations

- **Archives with a password do not unpack.** Nexus cannot ask for one. The unpack fails with
  `Password`.
- **Only RAR and 7z can be cut into parts.** A split zip or tar is not recognized as one set.
- **Sets deeper than one folder are ignored.** `scan` reads the top of the download and one
  folder down.
- **RAR 5 with a big dictionary needs `unar`.** libarchive cannot read it, and no other
  reader tries. Without `unar` and `lsar` the unpack fails with `Missing`.
- **Archives need libarchive for most formats.** RAR, tar.xz, tar.bz2, tar.zst, ISO and cab
  all go through it. The packages ship a copy. A source build uses the system's, and it must
  support RAR 5.
- **Links are not made on Windows.** The adapter writes symbolic links on Unix only.
- **Script installers only unpack.** Nexus does not run them. A script installer that does not
  state its archive's size, and is not a GOG installer, is not recognized.
- **PC games do not install from a folder the player picks.** They install from downloads.
  A folder copied by hand into `PC/` works as an installed game.
- **The PC plugin gives no description, developer or art.** It reports a cover only from
  `cover.jpg`, `cover.png` or `cover.webp` in the game's folder. Other art comes from the
  catalog.
- **Resolution needs gamescope and works on Linux only.**
- **No test here runs a real game.** The seam tests in `tests/tests/seams/pc.rs` run the plugin
  against a fake host for each system. The Windows and macOS paths are not tried on those
  systems by these tests.
