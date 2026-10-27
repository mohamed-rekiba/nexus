# Glossary

Terms used across the code and the docs, in alphabetical order.

**Base copy.** The copy of a console's catalog without a refresh: the copy fetched from the
catalog address, else the bundled one. A refresh is always of one base copy.

**Catalog sources.** The list of forums each console's catalog comes from:
`assets/catalog-sources.json`, or the file at the Game Catalog's Sources address. It is also the
one place that says what a forum's topics are for: the console, the system and Wine packing.

**Copy store** (`CopyStore`). One folder of copies that the Game Catalog keeps: `fetched/`,
`refreshed/` or `sources/`. Each copy has a meta file that names it. The copy is written first,
then its meta file.

**Forum list.** A forum's list of topics with a torrent, from the tracker's API: the hash, size,
peers and upload time of each topic, with no titles.

**Hold.** The wait of a fetch or a refresh that failed or waits for the VPN. The work doesn't
try again before the hold ends. Each hold is twice as long as the one before, at most the
fresh time.

**Refreshed copy.** A base copy brought up to date from its forums' feeds and forum lists. The
Game Catalog keeps it in `refreshed/`.

**Schedule.** The Game Catalog's rules for when a fetch or a refresh runs and how long a hold
is (`schedule.rs`). They take the time as an input, so the tests need no clock.

**Topic.** One release thread of a forum, as a catalog file lists it: the title, magnet link,
size, peers and game info. `topic.rs` has its typed model, which the refresh and the reader
share.
