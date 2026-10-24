import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

/** Where the collector keeps its files: `nexus/dist/catalog`, which git ignores. Each forum has
 * `<key>-topics.json` (English), `<key>-topics.json.ru-backup.json` (its Russian source) and
 * the shared `english-translations.json`. */
export const DIST = fileURLToPath(new URL('../../dist/catalog/', import.meta.url));

/** A file in DIST. */
export function distFile(name) {
  return `${DIST}${name}`;
}

/** The forums the collector reads and the Nexus catalogs they make, from the project's
 * assets/catalog-sources.json: each catalog by its name. The Game Catalog plugin bundles the same
 * file. Its forum hosts are placeholders: the browser step opens each forum's `url`, so set the
 * real host there before you collect. */
export const SOURCES = JSON.parse(readFileSync(new URL('../../assets/catalog-sources.json', import.meta.url),
  'utf8'));

/** The base address of the forum list API. The forum's number goes after it. The Game Catalog
 * plugin uses the same address (`refresh::API`). */
export const API = 'https://api.rutracker.cc/v1/static/pvc/f/';

/** The base address of the Atom feed. The forum's number and `.atom` go after it. The Game
 * Catalog plugin uses the same address (`refresh::FEED`). */
export const FEED = 'https://feed.rutracker.cc/atom/f/';

/** The forums of each Nexus catalog, by the catalog's name: `ps5` is the file `ps5.json`. */
export const NEXUS_CATALOGS = Object.fromEntries(Object.entries(SOURCES)
  .map(([name, catalog]) => [name, catalog.forums.map(forum => forum.key)]));

/** A label for log lines: the console's tag, else the system and how its games run. */
function label(forum) {
  if (forum.tag) return forum.tag;
  const system = forum.system ? forum.system[0].toUpperCase() + forum.system.slice(1) : 'PC';
  return forum.wine ? `${system} (Wine)` : `native ${system}`;
}

// The forums the collector can read, by key. A release is a topic with a torrent; in the console
// forums most titles start with the console's tag ("[PS4] …", "[PS4 Exclusive] …"). Each forum
// also has the Nexus catalog it goes into (`catalog`), that catalog's console (`platform`) and,
// for a PC forum, the system its games are for and whether they are packed with Wine.
export const PLATFORMS = Object.fromEntries(Object.entries(SOURCES).flatMap(([name, catalog]) =>
  catalog.forums.map(forum => {
    const url = new URL(forum.url);
    const forumId = url.searchParams.get('f');
    return [forum.key, {
      forumId, tag: forum.tag ?? null, label: label(forum), source: forum.url, origin: url.origin,
      api: `${API}${forumId}`, feed: `${FEED}${forumId}.atom`,
      catalog: name, platform: catalog.platform, system: forum.system ?? null, wine: Boolean(forum.wine),
    }];
  })));

/** Console releases the launcher can't play, by their title's tag: games that need a VR headset
 * ("[PS VR Only]", "[PS4 VR]", "[PS VR+PS Move]"), PS2/PS1/PSP/retro re-releases ("[PS4 PS2
 * Classics]", "[PSP-PS4]"; shadPS4's compatibility list has none of them), and homebrew apps.
 * Mirrored in listing.mjs, which runs in the browser on its own. */
export const UNPLAYABLE = /^\s*\[[^\]]*\b(?:VR|Move|homebrew|classics?|classis|PSP|PSX|PS2|PS1|SNES|Saturn)\b[^\]]*\]/i;

/** `--platform <forum key>` from command-line arguments. The default is the first forum in
 * catalog-sources.json. */
export function platformFrom(args) {
  const index = args.indexOf('--platform');
  const key = index < 0 ? Object.keys(PLATFORMS)[0] : (args[index + 1] || '').toLowerCase();
  const platform = Object.hasOwn(PLATFORMS, key) ? PLATFORMS[key] : null;
  if (!platform) throw new Error(`--platform must be one of: ${Object.keys(PLATFORMS).join(', ')}`);
  return { key, ...platform };
}
