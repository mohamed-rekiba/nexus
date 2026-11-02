import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { win32 } from 'node:path';

const NAMES = ['google-chrome-stable', 'google-chrome', 'chromium', 'chromium-browser'];

/** Whether `name` is a program on the PATH. */
function onPathOf(platform) {
  return name => {
    try {
      execFileSync(platform === 'win32' ? 'where' : 'which', [name], { stdio: 'ignore' });
      return true;
    } catch {
      return false;
    }
  };
}

/**
 * The Chrome or Chromium program to start, or null when there is none. CHROME_PATH comes
 * first, then a name on the PATH (Linux), then the usual install places: macOS keeps the
 * program inside the app, and Windows in Program Files or the user's own AppData.
 */
export function findChrome({
  env = process.env,
  platform = process.platform,
  home = homedir(),
  exists = existsSync,
  onPath = onPathOf(platform),
} = {}) {
  if (env.CHROME_PATH) return exists(env.CHROME_PATH) ? env.CHROME_PATH : null;
  const named = NAMES.find(name => onPath(name));
  if (named) return named;
  const places = [];
  if (platform === 'darwin') {
    for (const root of ['/Applications', `${home}/Applications`]) {
      places.push(`${root}/Google Chrome.app/Contents/MacOS/Google Chrome`,
        `${root}/Chromium.app/Contents/MacOS/Chromium`);
    }
  } else if (platform === 'win32') {
    for (const root of [env.PROGRAMFILES, env['PROGRAMFILES(X86)'], env.LOCALAPPDATA].filter(Boolean)) {
      places.push(win32.join(root, 'Google', 'Chrome', 'Application', 'chrome.exe'));
    }
  }
  return places.find(path => exists(path)) ?? null;
}

/**
 * Runs `fn` in the page. When the page navigates while it runs (a Cloudflare check reloads it,
 * for example), the run dies with "Execution context was destroyed". Then `settle(attempt)`
 * waits for the page, and the run starts again, at most `attempts` times in all. Other errors
 * fail at once.
 */
export async function evaluateThroughNavigation(page, fn, arg, { attempts = 3, settle }) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await page.evaluate(fn, arg);
    } catch (error) {
      const moved = /Execution context was destroyed|navigat/i.test(error.message);
      if (!moved || attempt >= attempts) throw error;
      await settle(attempt);
    }
  }
}
