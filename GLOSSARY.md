# Glossary

Terms used in the code and the docs, in alphabetical order.

**App shell** (`AppShell`, nexus-ui-host): what the launcher does on the UI thread around the
theme host: it carries out the theme's requests, coalesces renders, loads images and runs the
tick. It reaches the system only through its platform. The app and the seam tests run the same
shell.
_Avoid_: event loop, main loop

**Archive error** (`ArchiveError`): why an archive didn't unpack, typed: `Password`, `Escapes`,
`NoRoom`, `Unsupported`, `Missing`, `Corrupt`, `Io` or `Cancelled`. The core shows its text;
an adapter picks a fallback reader by its kind, never by its text.

**Archive plan** (`Plan`): what an unpack may write, decided from an archive's listing before
anything is written. `plan_unpack` in nexus-core makes it, and every `Archives` adapter writes
only what it holds.

**Archives contract** (`contract::archives`): the refusals every `Archives` adapter must make,
checked by the test kit against the app's `HostArchives` and the fake `FakeArchives`.

**Area**: A part of the core's state that a change names: the torrents, the catalogs, the installs, the
library and so on.
_Avoid_: section, domain

**Base copy**: the copy of a console's catalog without a refresh: the copy fetched from the
catalog address, else the bundled one. A refresh is always of one base copy.

**Build manager** (`BuildManager`, emulator-kit): runs an emulator's builds over its flavor. It
plans downloads, installs, updates, rolls back and removes builds, and picks the program a game
starts.

**Capability**: one thing a plugin can do for the host, such as list games, offer releases or install builds.
A plugin's manifest declares its capabilities.
_Avoid_: feature, extension point

**Catalog sources**: the list of forums each console's catalog comes from:
`assets/catalog-sources.json`, or the file at the Game Catalog's Sources address. It is also the
one place that says what a forum's topics are for: the console, the system and Wine packing.

**Change**: the core's note that its state in one area changed. The core and its tests read changes; the
app renders on any of them.
_Avoid_: update, state event

**Clock**: the time the core runs on: what time it is, and its waits. The app's clock is real time.
_Avoid_: timer, time source

**Code plugin** (`CodePlugin`): a plugin with code and an `init`: an emulator, a service or a
system plugin. One trait for all three; the kind decides only the privileges.

**Console emulator** (`ConsoleEmulator`, emulator-kit): a console emulator's games, compatibility
list, settings and page, served from a console spec and a flavor.

**Console spec** (`ConsoleSpec`, emulator-kit): what sets one console emulator apart besides its
builds: its games, package formats, compatibility list, keyboard keys and game page section.

**Copy store** (`CopyStore`): one folder of copies that the Game Catalog keeps: `fetched/`,
`refreshed/` or `sources/`. Each copy has a meta file that names it. The copy is written first,
then its meta file.

**Current build**: the downloaded build games start. The build before it stays for a rollback.
Both are saved in the plugin's `versions.json`.

**Fact** (`Fact`): one thing a catalog says about a release: a kind, a label, a value and an
optional page. Its `FactKind` tells the host what it is about, such as the publisher, the
release date or a compatibility tier.
_Avoid_: attribute, property

**Fake clock**: the clock in tests. Its time moves only when a test moves it, or when every task waits for a
timer.
_Avoid_: mock time, test time

**Flavor** (`Flavor`, emulator-kit): one emulator's description of its builds: release source,
matching asset, archive format, program, test run, own-build search paths, and the hooks that
run when the current build changes.

**Flow**: work that crosses the core's areas, as a free function over the core in its own
module: a plugin read whose answer goes to a subsystem, an install that downloads and then
posts a notice. The core routes a command to a flow when no one subsystem owns it.
_Avoid_: handler, service

**Format reader**: the code that lists and streams one archive format's entries: `zip`, `tar`,
`sevenz-rust2`, libarchive, or `unar`. It decides nothing about safety; the archive plan does.

**Forum list**: a forum's list of topics with a torrent, from the tracker's API: the hash, size,
peers and upload time of each topic, with no titles.

**Game install**: a plugin's `GameInstaller` decoding a package or a finished download into a
game folder, in the background. The game installs subsystem owns the job; the plugin reports
through an `InstallJob`.

**Game record** (`GameRecord`): everything the core knows about one game, by its game key: the
game as its plugin lists it, its badges, its details, a store's metadata, and for an installed
game, the catalog's offer it is and that offer's cover. The library subsystem holds one per game.

**Hand-off** (`HandOff`): the fields of a download that another area writes: the unpack of its
archives and the install from it. They change only through the torrents subsystem's
`hand_off` call.

**Hold**: the wait of a fetch or a refresh that failed or waits for the VPN. The work doesn't
try again before the hold ends. Each hold is twice as long as the one before, at most the
fresh time.

**Host facades** (`Host`, emulator-kit): the narrow parts of a `PluginContext` the kit takes:
storage, folders, programs, HTTP, settings and the log.

**Install event** (`InstallEvent`): what the game installs subsystem asks of the core: refresh
the library for a new game, tell the player of a failure, or start the install of a download
whose archives just unpacked. The subsystem waits until the core is done with each.

**Install warning**: why a release may not install, shown beside it in the Game Hub. The
plugin that lists the release sets it; the release stays listed.

**Kind**: what sort of plugin a manifest says it is: emulator, service, theme or system. The kind decides
a plugin's privileges only: the system capabilities, long-running programs and polkit are for
system plugins. Any code plugin may provide the data capabilities.
_Avoid_: type, category

**Library subsystem** (`LibrarySubsystem`): the subsystem that holds the game records, the
catalogs' copies and their offers, and the library's saved choices (hidden games, kept covers,
the player's results). It asks no plugin; the core hands it the plugins' answers.

**Listing** (`ArchiveEntry`): an archive's entries as a format reader gives them, unchecked: each
name as the archive spells it, its size, its kind, and whether it needs a password.

**Login session** (`LoginSession`): the port for the player's login session: starting the
launcher at login, and installing the launcher's own package with the administrator prompt.
Not a play session.

**Notice**: a short message for the player, such as "Download complete", until they close it.
The notices subsystem keeps the newest three, each with its own id.

**Offer**: one game a catalog offers, grouped from its releases by title id or by title
without its edition words. An installed game can be an offer too: its record names the offer.

**Opener** (`Opener`): the port that opens a link or a file with the system's opener, as a click
on it would. The core opens plugins' store links with it; the app shell's platform opens pages
and folders with it.

**Own build**: an emulator build the player built or installed, used instead of a downloaded
one while the `own_build` setting is on. The host never updates it.

**Phase rules** (`torrents::phase::next`): what the BitTorrent engine's stats mean for a
download, and what the core does next: unpause, stop, finished or stopped. They exist once, in
the core; the adapter and the fake decide nothing.

**Platform** (`Platform`, nexus-ui-host): the app shell's port: running work off the UI thread,
waking the shell, opening links and paths, quitting, the lines the Nexus session reads, sounds,
trailers, the local time, and the window's monitors and focus. The test fake records each call.

**Plugin box** (`PluginBox`): a plugin as the host registers it: code or a theme.

**Port**: what the core needs from the outside world, as one interface. The app plugs in real ones and
the tests plug in fakes.
_Avoid_: adapter (that is the thing plugged in), driver

**Ports bundle** (`Ports`): every port the app plugged in, built once at boot. It makes the VPN
rule. The kernel hands it to the core, which splits it per subsystem.

**Precondition**: what must happen before a plugin call can work, such as installing a build.
A plugin says it in words, or with a typed precondition (`OwnBuildMissing`) that the host
words for the player.

**Pref** (`Pref`): one of the app's own one-value settings, such as sounds, the size or the
monitor, saved as `app.<key>`. Its spec says what values it takes and its default. The
preferences subsystem checks each value the player sets, and the core reads them typed.
_Avoid_: option (a theme's options are something else)

**Refreshed copy**: a base copy brought up to date from its forums' feeds and forum lists. The
Game Catalog keeps it in `refreshed/`.

**Release metadata**: a release's `GameMetadata`: its game's words and art from a store, such as
Steam's. The host fills in from it what the `metadata` plugins leave out.

**Schedule**: the Game Catalog's rules for when a fetch or a refresh runs and how long a hold
is (`schedule.rs`). They take the time as an input, so the tests need no clock.

**Seed-only start**: a start of a finished download after a restart. The torrent starts paused,
checks its files, and seeds only when every piece is there. It never downloads.

**Session**: one run of a game, from its spawn to the end of its last process. It has a number
(`SessionId`), and the game's log and play time come from it.
_Avoid_: process, run

**Sessions subsystem** (`SessionsSubsystem`): the subsystem of the running games. It starts the
game the core resolved, watches it, carries out Stop and the window focus calls, and saves play
time.

**Snapshot**: a subsystem's state read whole, in one lock. Reading it changes nothing. The
core's `Snapshot` holds one read of each subsystem, for the view.

**Subsystem**: one area of the core's state with its own lock, saved records and ports. The core
sends it commands, reads its snapshot and turns its events into work. The torrents, the game
installs, the library, the preferences, the running games, the notices, the VPN setting and
the launcher's update are subsystems.

**Test run**: a new build runs once with `--help` before it becomes current. A build that fails
it is removed.

**Topic**: one release thread of a forum, as a catalog file lists it: the title, magnet link,
size, peers and game info. `topic.rs` has its typed model, which the refresh and the reader
share.

**Torrent stats** (`TorrentStats`): what the BitTorrent engine reports for a torrent: its
state (checking, live, paused or error), whether every piece is in, bytes, rates, peers and
the error. Raw numbers, with no decision.

**VPN rule** (`VpnRule`): "Download only through a VPN" as every area reads it: the setting,
the picked VPN, the VPNs' source and whether a plugin waits for the VPN. Plugins get it as
their VPN gate.

**Windows** (`Windows`): the port for the desktop's windows: which window is in front, bringing
a game's window or the launcher's to the front, and whether a game has a window.
