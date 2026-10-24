import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { nexusCatalog } from './publish.mjs';
import { API, FEED, platformFrom } from './platforms.mjs';

const forum = key => platformFrom(['--platform', key]).source;
const report = (key, ids, at = '2026-10-19T00:00:00Z') => ({
  source: forum(key), platform: key, complete: true, translation: { language: 'en' }, collected_at: at,
  topic_count: ids.length, topics: ids.map(id => ({ id, title: `Game ${id}`, magnet: `magnet:?xt=urn:btih:${id}` })),
});

test('catalog-sources.json: every forum has a key, an address, and a tag or a system', async () => {
  const sources = JSON.parse(await readFile(new URL('../../assets/catalog-sources.json', import.meta.url), 'utf8'));
  const names = Object.keys(sources);
  assert.deepEqual(names, ['ps5', 'ps4', 'pc'], 'the root is the catalogs, by name');
  const keys = new Set();
  for (const name of names) {
    const catalog = sources[name];
    assert.deepEqual(Object.keys(catalog).sort(), ['forums', 'platform'], `${name}: a platform and its forums`);
    assert.equal(catalog.platform.toLowerCase(), name, `${name}: the file is named by its console`);
    assert.ok(catalog.forums.length, name);
    for (const f of catalog.forums) {
      assert.match(f.key ?? '', /^[a-z\d]+$/, `${name}: a forum key`);
      assert.ok(!keys.has(f.key), `${f.key} is used once`);
      keys.add(f.key);
      assert.match(new URL(f.url).searchParams.get('f') ?? '', /^\d+$/, `${f.key}: a forum address with its f number`);
      assert.ok(new URL(f.url).protocol === 'https:', `${f.key}: https`);
      assert.ok(Boolean(f.tag) !== Boolean(f.system), `${f.key}: a console tag or a PC system`);
    }
  }
});

test('platforms: the API and the feed of a forum are at the fixed addresses with its f number', () => {
  const ps5 = platformFrom(['--platform', 'ps5']);
  assert.equal(ps5.forumId, '546');
  assert.equal(ps5.api, `${API}546`);
  assert.equal(ps5.feed, `${FEED}546.atom`);
  assert.equal(API, 'https://api.rutracker.cc/v1/static/pvc/f/');
  assert.equal(FEED, 'https://feed.rutracker.cc/atom/f/');
});

test('publish: a console forum is its catalog, and each topic says its console and forum', () => {
  const ps5 = report('ps5', ['1', '2']);
  const catalog = nexusCatalog('ps5', { ps5 });
  assert.deepEqual({ ...catalog, topics: undefined }, { ...ps5, topics: undefined }, 'the listing as it is');
  assert.deepEqual(catalog.topics, ps5.topics.map(t => ({ ...t, platform: 'PS5', source: forum('ps5') })));
});

test('publish: the PC forums make one catalog, and each topic says its forum, system and packing', () => {
  const pc = nexusCatalog('pc', {
    linux: report('linux', ['10'], '2026-10-18T00:00:00Z'),
    wine: report('wine', ['20', '21'], '2026-10-19T00:00:00Z'),
  });
  assert.equal(pc.complete, true);
  assert.equal(pc.translation.language, 'en');
  assert.equal(pc.topic_count, 3);
  assert.deepEqual(pc.topics.map(t => [t.id, t.source, t.platform, t.system, t.wine_packed]), [
    ['10', forum('linux'), 'PC', 'linux', false],
    ['20', forum('wine'), 'PC', 'linux', true],
    ['21', forum('wine'), 'PC', 'linux', true],
  ]);
  assert.equal(pc.collected_at, '2026-10-18T00:00:00Z', 'the oldest listing: no topic is newer than that');
  assert.equal(pc.source, undefined, 'no one forum for the whole file');
});

test('publish: one PC forum is enough', () => {
  const pc = nexusCatalog('pc', { wine: report('wine', ['20']) });
  assert.deepEqual(pc.topics.map(t => t.source), [forum('wine')]);
});

test('publish: only complete, English listings of the right forum', () => {
  const partial = { ...report('ps4', ['1']), complete: false };
  assert.throws(() => nexusCatalog('ps4', { ps4: partial }), /complete, English/);
  const russian = { ...report('ps4', ['1']), translation: { language: 'ru' } };
  assert.throws(() => nexusCatalog('ps4', { ps4: russian }), /complete, English/);
  assert.throws(() => nexusCatalog('ps4', { ps4: report('ps5', ['1']) }), /complete, English/);
  assert.throws(() => nexusCatalog('pc', {}), /no listing/);
  assert.throws(() => nexusCatalog('pc', { linux: report('linux', ['1']), wine: report('wine', ['1']) }), /twice/);
  assert.throws(() => nexusCatalog('emu-kyty', {}), /no Nexus catalog/);
  assert.throws(() => nexusCatalog('toString', {}), /no Nexus catalog/);
});
