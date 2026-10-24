import assert from 'node:assert/strict';
import { test } from 'node:test';
import { evaluateThroughNavigation, findChrome } from './chrome.mjs';

const none = () => false;

test('Chrome: CHROME_PATH first, when the file is there', () => {
  const exists = path => path === '/opt/chrome/chrome';
  assert.equal(findChrome({ env: { CHROME_PATH: '/opt/chrome/chrome' }, platform: 'linux', exists, onPath: () => true }),
    '/opt/chrome/chrome');
  assert.equal(findChrome({ env: { CHROME_PATH: '/missing' }, platform: 'linux', exists, onPath: none }), null);
});

test('Chrome: a name on the PATH, as on Linux', () => {
  const onPath = name => name === 'chromium';
  assert.equal(findChrome({ env: {}, platform: 'linux', exists: none, onPath }), 'chromium');
});

test('Chrome: the app in Applications on macOS, which is not on the PATH', () => {
  const app = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  assert.equal(findChrome({ env: {}, platform: 'darwin', home: '/Users/me', exists: path => path === app, onPath: none }), app);
  const own = '/Users/me/Applications/Chromium.app/Contents/MacOS/Chromium';
  assert.equal(findChrome({ env: {}, platform: 'darwin', home: '/Users/me', exists: path => path === own, onPath: none }), own);
});

test('Chrome: Program Files and the user\'s own install on Windows', () => {
  const env = { PROGRAMFILES: 'C:\\Program Files', LOCALAPPDATA: 'C:\\Users\\me\\AppData\\Local' };
  const user = 'C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\Application\\chrome.exe';
  assert.equal(findChrome({ env, platform: 'win32', exists: path => path === user, onPath: none }), user);
});

test('Chrome: none found', () => {
  assert.equal(findChrome({ env: {}, platform: 'darwin', home: '/Users/me', exists: none, onPath: none }), null);
});

/** A page whose first `moves` evaluations die because the page navigates. */
function movingPage(moves) {
  const calls = { evaluate: 0, settle: 0 };
  return {
    calls,
    async evaluate(fn, arg) {
      calls.evaluate += 1;
      if (calls.evaluate <= moves) {
        throw new Error('page.evaluate: Execution context was destroyed, most likely because of a navigation.');
      }
      return fn(arg);
    },
  };
}

test('A batch the page moved under runs again once the page settles', async () => {
  const page = movingPage(2);
  const settled = [];
  const result = await evaluateThroughNavigation(page, n => n * 2, 21, { settle: async attempt => settled.push(attempt) });
  assert.equal(result, 42);
  assert.equal(page.calls.evaluate, 3);
  assert.deepEqual(settled, [1, 2]);
});

test('A page that keeps moving fails after the last try', async () => {
  const page = movingPage(10);
  await assert.rejects(evaluateThroughNavigation(page, n => n, 1, { attempts: 3, settle: async () => {} }),
    /Execution context was destroyed/);
  assert.equal(page.calls.evaluate, 3);
});

test('Other errors fail at once', async () => {
  const page = { async evaluate() { throw new Error('Target page, context or browser has been closed'); } };
  let settles = 0;
  await assert.rejects(evaluateThroughNavigation(page, n => n, 1, { settle: async () => { settles += 1; } }), /closed/);
  assert.equal(settles, 0);
});
