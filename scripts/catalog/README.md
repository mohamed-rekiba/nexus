# Catalog collector

These scripts make the catalog files that Nexus's Game Catalog plugin
(`plugins/services/release-catalog`) reads, one for each console: `ps5.json`, `ps4.json` and
`pc.json`. Each file lists the release topics of one or more RuTracker forums, in English, with
their magnet links and game facts.

## How the catalogs reach Nexus

- `assets/catalog-sources.json` names the forums each catalog comes from. The collector reads it,
  and the plugin's `build.rs` compiles the same file into Nexus.
- `assets/catalog/ps5.json`, `ps4.json` and `pc.json` are the bundled catalogs. The plugin's
  `build.rs` compiles these three into Nexus (the list is in `bundled-names.rs`); a build fails
  when one is missing. Git holds a full set; `make catalog` and `make catalog-refresh` replace
  them, and the next build bundles the new ones.
- In Nexus, the plugin uses the bundled catalog while its **Catalog address** setting is empty.
  With an address, it fetches `<address>/<console>.json`, keeps it in the plugin's `fetched/`
  folder, fetches again after 6 hours, and falls back to the copy fetched before, then to the
  bundled one. **Sources address** does the same for `catalog-sources.json`.
- Both addresses must be `https://`, on a host in the `network` list of
  `plugins/services/release-catalog/plugin.toml`. That list is compiled in, so a new host needs
  a new build. Its catalog host is a placeholder, `xyz.dev`.
- Nexus reads a catalog file only when it has `"complete": true`,
  `"translation": { "language": "en" }` and a `topic_count` that matches its topics. It leaves
  out topics without a magnet link, apps and system software.

The core gives each console's catalog to the first emulator plugin that plays that console.

## Sources

`assets/catalog-sources.json` has one entry for each catalog, by its file name (`ps5` is
`ps5.json`). Each entry has the console its releases are for (`platform`) and its `forums`. Each
forum has:

- `key`: the name the scripts use for it (`--platform <key>`);
- `url`: its first listing page, `https://<host>/forum/viewforum.php?f=<number>`;
- `tag`: for a console forum, the tag its titles start with (`PS5`);
- `system`: for a PC forum, the system its games are for (`linux`);
- `wine`: `true` for a PC forum whose games are packed with Wine.

| Key | Forum | Catalog |
|---|---|---|
| `ps5` | f=546, PS5 games | `ps5.json` |
| `ps4` | f=973, PS4 games | `ps4.json` |
| `linux` | f=1992, native Linux games | `pc.json` |
| `wine` | f=2059, Linux games with Wine | `pc.json` |

Each script's `--platform` takes one of these keys; the default is the first one, `ps5`. The
browser step opens each forum's `url`, and reads pages only on that host. Nexus never opens a
forum's `url`: it takes only the `f=` number. A listing keeps the address it came from in
`source`, and `publish.mjs` publishes a listing only when that address is the one in the file.

## Requirements

- Node 20 or later, with npm: `sudo dnf install nodejs24-bin nodejs24-npm-bin` (Fedora),
  `sudo apt-get install nodejs npm` (Debian).
- `jq`, for `update-catalogs.sh` and `publish-catalog.sh`.
- For the browser step, Google Chrome or Chromium. The collector looks at `CHROME_PATH` first,
  then the `PATH`, then the usual install places (Applications on macOS, Program Files or
  AppData on Windows).

```sh
cd scripts/catalog
npm install      # playwright-core, which drives your own Chrome
npm test
```

## Update the bundled catalogs

From the repo root:

```sh
make catalog                  # the full run: Chrome opens for each forum
STEAM=1 make catalog          # the same, with Steam art and ProtonDB ratings (slow)
make catalog-refresh          # no browser: each forum's feed and API update the listings
make catalog CONSOLE=pc       # one catalog: only its forums are read, only pc.json changes
make catalog-refresh CONSOLE=ps4
```

Both targets run `scripts/catalog/update-catalogs.sh`, with `--refresh` for the second and
`--console <name>` for `CONSOLE`. The script checks for `jq`, Node 20+ and, for the full run,
Chrome, and runs `npm install` when `node_modules` is missing.

- **Full run**: for each forum, `list.mjs --include-magnets`. Do the Cloudflare check in the
  Chrome window when it shows. The new listing is copied over the forum's Russian source, so
  `steam.mjs` (with `STEAM=1`) and `translate.mjs --from-original` work from it. A full run
  without `STEAM=1` drops the Steam data of earlier runs, since that data lives in the Russian
  source; the backup keeps a copy.
- **Refresh**: for each forum, `refresh.mjs`, then `translate.mjs --from-original`. A forum needs
  a Russian source from an earlier full run. The script skips a forum without one and says so.

Then `publish.mjs` writes the catalogs to a new temporary folder, and the script copies them to
`dist/catalog/publish/` and `assets/catalog/`. Build Nexus again to bundle them.

The script guards the current files:

- First it copies the current `assets/catalog/<name>.json` and the forums' Russian sources to
  `dist/catalog/backup/<UTC time>/`, and prints that folder. To go back, copy the files from
  there. `make clean` removes `dist/`, and the backups with it.
- It stops at the first forum whose listing is partial (`list.mjs` exit 2), before it copies
  anything into `assets/catalog/`.
- It checks every new file before the first copy, and copies only complete, English files that
  this run wrote.

`scripts/catalog/update-catalogs.sh --dry-run` (also with `--refresh` or `--console`) prints each
command and copy without running it. `--help` prints the usage.

## The collector's files

The scripts keep their files in `dist/catalog/`:

| File | What it is |
|---|---|
| `<key>-topics.json` | The English listing of one forum |
| `<key>-topics.json.ru-backup.json` | Its Russian source, which `translate.mjs --from-original`, `steam.mjs` and `refresh.mjs` work on |
| `<key>-topics.json.partial.json` | A listing that stopped early; it never replaces a complete one |
| `english-translations.json` | The translation cache |
| `publish/<catalog>.json` | The files for Nexus |
| `backup/<UTC time>/` | The copies `update-catalogs.sh` makes before it changes anything |

`dist/` isn't in `.gitignore`, and the repo tracks the listings, Russian sources, translation
cache and `publish/` files under `dist/catalog/`. Check `git status` after a run, and leave
`backup/` out of commits.

The browser profile is `~/.cache/nexus/rutracker-list-browser`. It can hold your session cookies:
keep it private, and don't commit or share it. One collector at a time can use it.

## Run the steps by hand

From `scripts/catalog/`:

```sh
# 1. Read each forum. A Chrome window opens: do the Cloudflare check yourself when it shows.
for p in ps5 ps4 linux wine; do node list.mjs --platform "$p" --include-magnets; done

# 2. Make each listing English. The first run keeps the Russian listing as its source.
for p in ps5 ps4 linux wine; do node translate.mjs --platform "$p"; done

# 3. Optional: Steam art and ProtonDB ratings, added to the Russian sources, then English again.
for p in ps5 ps4 linux wine; do node steam.mjs --platform "$p"; done
for p in ps5 ps4 linux wine; do node translate.mjs --platform "$p" --from-original; done

# 4. Write dist/catalog/publish/ps5.json, ps4.json and pc.json.
node publish.mjs
```

- `translate.mjs` writes the Russian source only when there is none. After an earlier run,
  `--from-original` works from that old source: copy the new `<key>-topics.json` over
  `<key>-topics.json.ru-backup.json` first, as `update-catalogs.sh` does.
- `publish.mjs` refuses a listing that is partial, not English, has duplicate topics, or came
  from another address than `catalog-sources.json` names. A catalog with no listing yet is
  skipped. `pc.json` holds the `linux` and `wine` listings together. Each topic gets its console
  in `platform` and its forum in `source`; a PC topic also gets `system` and `wine_packed`.
  `--output <folder>` writes to another folder.

## What `list.mjs` reads

It follows the forum's listing pages and reads each topic's title, id, address, size, author
and peer counts, and removes topics listed twice. In a console forum a release is a topic whose
title starts with the console's tag (`[PS5] …`), and releases the emulators can't play are left
out: VR, PS1, PS2, PSP and other re-releases, and homebrew. The PC forums have no tag: a release
is a topic with a torrent.

`--include-magnets` also reads each topic's first post for its magnet link and release facts:
genre, developer, publisher, release date, title id, region, edition, version, image format,
firmware, languages and age rating. Nexus needs the title id to show a PS5 or PS4 game's
official art and details. To add these to an existing listing without reading the listing pages
again:

```sh
node list.mjs --platform ps5 --enrich-existing ../../dist/catalog/ps5-topics.json
```

It reads one page at a time, 1.5 seconds apart. It never opens a magnet link, fetches a torrent,
contacts a tracker or a peer, or downloads a game. It stops at verification, login or an HTTP
failure and doesn't retry. There is no challenge solver, stealth plugin, credential collection or
cookie export. An unknown magnet link is `null`, not a guess. Image addresses are kept as text,
not downloaded. Long descriptions, replies and install steps aren't copied.

Other options: `--output <file>`, `--max-pages <1-200>` (default 200),
`--verification-timeout <seconds>` (default 180), `--concurrency <1-4>`, `--headless`. Use
`--headless` only to look for faults: the check page stops a run without a window. Exit code 0
means every page was read; 2 means the listing is partial and went to
`<key>-topics.json.partial.json`.

## Translation

`translate.mjs` sends only the listing's text, without duplicates, to Google's translator at
`translate.googleapis.com`: no credentials, cookies, addresses or magnet links. It keeps a
glossary for game terms, protects versions and dates, and caches each translation in
`english-translations.json`. Steam data is in English already and isn't translated. The
uploaders' claims are translated, not checked.

## Refresh without a browser

`refresh.mjs --platform <key>` updates a Russian source from two public endpoints, whose base
addresses are fixed in `platforms.mjs` and in the plugin's `refresh.rs`:

- the forum's Atom feed, `https://feed.rutracker.cc/atom/f/<number>.atom`: new and renamed
  releases from the last one to four weeks;
- the API, `https://api.rutracker.cc/v1/static/pvc/f/<number>`: each release's hash, size and
  peers.

A topic the API no longer lists is removed, unless the API answered for less than 80% of the
listing. A new topic has no facts, since its page isn't read: it takes the game's facts (title
id, genre, developer, cover) from another release of the same game and is marked `borrowed`. The
next full run reads its own facts. After a refresh, run `translate.mjs --from-original` and
`publish.mjs`.

`api.mjs --platform <key>` applies only the API to the English listing: magnets, sizes and peers.

## Refresh in Nexus

With **Refresh from the forums** on (it is off by default), the Game Catalog plugin does what
`refresh.mjs` does, inside Nexus. For each console's catalog it reads the feed and the API of
that catalog's forums, adds new releases, drops removed ones, and updates sizes, peers and magnet
links. It refreshes a copy older than 6 hours, and the plugin's page has **Refresh now**. A
failed refresh tries again after 5 minutes, then after twice as long each time, at most 6 hours.
The result goes to the plugin's `refreshed/` folder; the bundled and fetched files stay as they
are.

The in-app refresh isn't the collector:

- It can't build a catalog from nothing: the feed covers only the last weeks, and the API has no
  titles. It starts from the bundled or fetched catalog.
- It keeps the titles the catalog has. A new release has the feed's title, untranslated, until
  the collector's next run.
- It asks only the fixed feed and API addresses, with each forum's `f=` number. A forum whose
  `url` has no `f=` number is skipped, and the plugin's page names it.
- With "Download only through a VPN" on, it waits until the VPN connects. Its own requests don't
  go through the VPN: the VPN carries only the downloads.

## Steam data and ProtonDB

`steam.mjs --platform <key>` adds Steam art, a short description, genres and, for PC releases,
the ProtonDB rating to a Russian source, from Steam's public store API and ProtonDB. For `ps5`
and `ps4` it looks up only releases without a title id, which have no PlayStation art, and takes
only an exact name match. A release with no match is tried again after 30 days. `--limit <n>`
and `--minutes <n>` stop it early, and the next run goes on from there; `--retry` looks up the
unmatched ones again now; `--catalog <file>` works on another file.

## Publish to a git branch

`publish-catalog.sh` pushes `dist/catalog/publish/<catalog>.json` to the `catalog` branch of a
git remote, with the Russian sources and the translation cache, gzipped, in `sources/`. The
branch keeps one commit, replaced each time. From the repo root:

```sh
scripts/catalog/publish-catalog.sh [dist/catalog] [git remote URL]
```

The remote defaults to `origin`. To have Nexus read the branch, add the host that serves it to
the plugin's `network` list, build Nexus, and set **Catalog address** to the folder.

## Limits

- A topic is a release, not a game. One game can have topics for its regions, versions, formats
  and editions.
- `complete` means every page the run found in that forum was read. It isn't a snapshot of the
  whole forum at one moment: topics move between pages while the collector runs, and deleted,
  private and untagged topics aren't in it.
- A missing peer count is `null`, not zero. Peer counts change all the time.
- The forum can ask for verification or login, so an unattended full run can fail. Never give
  your credentials to a chat or a script.
