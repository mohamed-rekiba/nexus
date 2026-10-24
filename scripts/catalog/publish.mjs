#!/usr/bin/env node
// Writes the catalog files Nexus reads, one for each console: `ps5.json`, `ps4.json` and `pc.json`
// (the native Linux and Wine forums together), as catalog-sources.json lists them, from the
// complete, English listings in DIST. Nexus fetches them from `<catalog address>/<name>`. Nexus bundles no
// catalog: the files go to DIST/publish/, not into assets/.
import { access, mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { NEXUS_CATALOGS, distFile, platformFrom } from './platforms.mjs';

/** Checks that `report` is a complete, English, deduplicated listing of the `key` forum. */
function check(key, report) {
  const platform = platformFrom(['--platform', key]);
  if (!report.complete || report.translation?.language !== 'en'
      || !Array.isArray(report.topics) || !report.topics.length
      || report.topic_count !== report.topics.length
      || new Set(report.topics.map(topic => topic.id)).size !== report.topics.length
      || report.source !== platform.source) {
    const forum = report.source === platform.source ? ''
      : `: the listing is of ${report.source}, but catalog-sources.json says ${platform.source}`;
    throw new Error(`Only a complete, English, deduplicated ${platform.label} forum listing can be published${forum}`);
  }
}

/** What a topic of the `key` forum is for, as catalog-sources.json says: its console
 * (`platform`), its forum (`source`) and, for a PC forum, its system and whether it is packed
 * with Wine. */
function about(key) {
  const platform = platformFrom(['--platform', key]);
  return {
    platform: platform.platform,
    source: platform.source,
    ...(platform.system ? { system: platform.system, wine_packed: platform.wine } : {}),
  };
}

/**
 * The Nexus catalog file `name` (`ps5`, `ps4`, `pc`), from its forums' listings (`{ forum key:
 * report }`; a PC forum not collected yet is left out). Every topic says what it is for: see
 * `about`. One forum's listing is otherwise the file as it is. Several forums' listings make one
 * file.
 */
export function nexusCatalog(name, reports) {
  const forums = Object.hasOwn(NEXUS_CATALOGS, name) ? NEXUS_CATALOGS[name] : null;
  if (!forums) throw new Error(`no Nexus catalog is called ${name}`);
  const present = forums.filter(key => reports[key]);
  if (!present.length) throw new Error(`${name}: no listing of ${forums.join(' or ')}`);
  for (const key of present) check(key, reports[key]);
  const topics = present.flatMap(key => reports[key].topics.map(topic => ({ ...topic, ...about(key) })));
  if (forums.length === 1) return { ...reports[present[0]], topics };
  if (new Set(topics.map(topic => topic.id)).size !== topics.length) {
    throw new Error(`${name}: a topic is listed twice`);
  }
  const dates = key => present.map(forum => reports[forum][key]).filter(Boolean).sort();
  return {
    complete: true,
    translation: { language: 'en' },
    collected_at: dates('collected_at')[0],
    ...(dates('refreshed_at').length ? { refreshed_at: dates('refreshed_at').at(-1) } : {}),
    forums: present.map(key => reports[key].source),
    topic_count: topics.length,
    topics,
  };
}

/** The listing at `path`, or undefined when there is none. */
async function listing(path) {
  try {
    await access(path);
  } catch {
    return undefined;
  }
  return JSON.parse(await readFile(path, 'utf8'));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const index = args.indexOf('--output');
  const folder = resolve(index >= 0 ? args[index + 1] : distFile('publish'));
  await mkdir(folder, { recursive: true });
  let written = 0;
  for (const [name, forums] of Object.entries(NEXUS_CATALOGS)) {
    const reports = {};
    for (const key of forums) reports[key] = await listing(distFile(`${key}-topics.json`));
    if (!forums.some(key => reports[key])) {
      console.log(`${name}: skipped, no listing of ${forums.join(' or ')} in ${distFile('')}`);
      continue;
    }
    const catalog = nexusCatalog(name, reports);
    const output = `${folder}/${name}.json`;
    await writeFile(`${output}.tmp`, JSON.stringify(catalog, null, 2) + '\n');
    await rename(`${output}.tmp`, output);
    console.log(`${name}: ${catalog.topic_count} release topics in ${output}`);
    written++;
  }
  if (!written) process.exitCode = 1;
}
