# Glossary

Terms used in the code and the docs, in alphabetical order.

**Archive error** (`ArchiveError`): why an archive didn't unpack, typed: `Password`, `Escapes`,
`NoRoom`, `Unsupported`, `Missing`, `Corrupt`, `Io` or `Cancelled`. The core shows its text;
an adapter picks a fallback reader by its kind, never by its text.

**Archive plan** (`Plan`): what an unpack may write, decided from an archive's listing before
anything is written. `plan_unpack` in nexus-core makes it, and every `Archives` adapter writes
only what it holds.

**Archives contract** (`contract::archives`): the refusals every `Archives` adapter must make,
checked by the test kit against the app's `HostArchives` and the fake `FakeArchives`.

**Base copy**: the copy of a console's catalog without a refresh: the copy fetched from the
catalog address, else the bundled one. A refresh is always of one base copy.

**Build manager** (`BuildManager`, emulator-kit): runs an emulator's builds over its flavor. It
plans downloads, installs, updates, rolls back and removes builds, and picks the program a game
starts.

**Catalog sources**: the list of forums each console's catalog comes from:
`assets/catalog-sources.json`, or the file at the Game Catalog's Sources address. It is also the
one place that says what a forum's topics are for: the console, the system and Wine packing.

**Console emulator** (`ConsoleEmulator`, emulator-kit): a console emulator's games, compatibility
list, settings and page, served from a console spec and a flavor.

**Console spec** (`ConsoleSpec`, emulator-kit): what sets one console emulator apart besides its
builds: its games, package formats, compatibility list, keyboard keys and game page section.

**Copy store** (`CopyStore`): one folder of copies that the Game Catalog keeps: `fetched/`,
`refreshed/` or `sources/`. Each copy has a meta file that names it. The copy is written first,
then its meta file.

**Current build**: the downloaded build games start. The build before it stays for a rollback.
Both are saved in the plugin's `versions.json`.

**Flavor** (`Flavor`, emulator-kit): one emulator's description of its builds: release source,
matching asset, archive format, program, test run, own-build search paths, and the hooks that
run when the current build changes.

**Format reader**: the code that lists and streams one archive format's entries: `zip`, `tar`,
`sevenz-rust2`, libarchive, or `unar`. It decides nothing about safety; the archive plan does.

**Forum list**: a forum's list of topics with a torrent, from the tracker's API: the hash, size,
peers and upload time of each topic, with no titles.

**Hold**: the wait of a fetch or a refresh that failed or waits for the VPN. The work doesn't
try again before the hold ends. Each hold is twice as long as the one before, at most the
fresh time.

**Host facades** (`Host`, emulator-kit): the narrow parts of a `PluginContext` the kit takes:
storage, folders, programs, HTTP, settings and the log.

**Listing** (`ArchiveEntry`): an archive's entries as a format reader gives them, unchecked: each
name as the archive spells it, its size, its kind, and whether it needs a password.

**Own build**: an emulator build the player built or installed, used instead of a downloaded
one while the `own_build` setting is on. The host never updates it.

**Refreshed copy**: a base copy brought up to date from its forums' feeds and forum lists. The
Game Catalog keeps it in `refreshed/`.

**Schedule**: the Game Catalog's rules for when a fetch or a refresh runs and how long a hold
is (`schedule.rs`). They take the time as an input, so the tests need no clock.

**Test run**: a new build runs once with `--help` before it becomes current. A build that fails
it is removed.

**Topic**: one release thread of a forum, as a catalog file lists it: the title, magnet link,
size, peers and game info. `topic.rs` has its typed model, which the refresh and the reader
share.
