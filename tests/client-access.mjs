import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../assets/access.js', import.meta.url), 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const token = 'a'.repeat(64); // Synthetic fixture, not an instance credential.
const key = 'sameframe:access-token';

async function gate({ url = 'https://sameframe.test/room/abc', saved = null,
  blockedStorage = false, response = { ok: true, status: 204 } } = {}) {
  const nodes = new Map();
  const requests = [];
  const redirects = [];
  const replaced = [];
  const operations = [];
  const storage = new Map(saved ? [[key, saved]] : []);
  const timers = new Map();
  let timerId = 0;
  let nextResponse = response;
  let currentUrl = url;
  const get = (id) => {
    if (!nodes.has(id)) nodes.set(id, { value: '', disabled: true, hidden: true, type: 'password', events: {},
      textContent: '', addEventListener(name, fn) { this.events[name] = fn; },
      setAttribute(name, value) { this[name] = value; }, focus() { this.focused = true; } });
    return nodes.get(id);
  };

  await new AsyncFunction('document', 'location', 'history', 'localStorage', 'fetch', 'setTimeout', 'clearTimeout', source)(
    { getElementById: get },
    { href: url, reload() { redirects.push(currentUrl); operations.push('redirect'); } },
    { replaceState(_state, _title, path) { currentUrl = path; replaced.push(path); operations.push('strip'); } },
    { getItem(name) { if (blockedStorage) throw Error('blocked'); return storage.get(name) ?? null; },
      setItem(name, value) { if (blockedStorage) throw Error('blocked'); storage.set(name, value); operations.push('save'); },
      removeItem(name) { storage.delete(name); } },
    async (path, options) => {
      requests.push([path, options]); operations.push('fetch');
      if (typeof nextResponse === 'function') return nextResponse(options);
      if (nextResponse instanceof Error) throw nextResponse;
      return nextResponse;
    },
    (fn, ms) => { const id = ++timerId; timers.set(id, { fn, ms }); return id; },
    (id) => timers.delete(id),
  );
  return { get, requests, redirects, replaced, operations, storage, timers,
    setResponse(value) { nextResponse = value; },
    submit(value) { get('access-token').value = value; get('access-form').events.submit({ preventDefault() {} }); },
    async flush() { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); } };
}

test('access links strip keys before requests and preserve the requested page, query and Keeper fragment', async () => {
  const g = await gate({ url: `https://sameframe.test/room/abc?access_token=${token}&theme=dark#host=keeper` });

  assert.deepEqual(g.replaced, ['https://sameframe.test/room/abc?theme=dark#host=keeper']);
  assert.deepEqual(g.operations, ['strip', 'fetch', 'fetch', 'save', 'redirect']);
  assert.deepEqual(g.redirects, ['https://sameframe.test/room/abc?theme=dark#host=keeper']);
  assert.equal(g.storage.get(key), token);
  assert.equal(g.requests[0][0], '/api/access');
  assert.equal(g.requests[0][1].credentials, 'same-origin');
  assert.equal(g.requests[0][1].cache, 'no-store');
  assert.deepEqual(JSON.parse(g.requests[0][1].body), { token });
  assert.equal(g.get('access-token').value, '');
});

test('saved access restores after a browser restart without another access URL', async () => {
  const g = await gate({ saved: token, url: 'https://sameframe.test/any/page' });

  assert.equal(g.requests.length, 2);
  assert.deepEqual(g.redirects, ['https://sameframe.test/any/page']);
  assert.deepEqual(g.replaced, []);
  assert.equal(g.storage.get(key), token);
});

test('an explicit invalid key takes precedence over stored access and never echoes it', async () => {
  const g = await gate({ saved: token, url: 'https://sameframe.test/?access_token=wrong&access_token=other' });

  assert.deepEqual(g.replaced, ['https://sameframe.test/']);
  assert.equal(g.requests.length, 0);
  assert.deepEqual(g.redirects, []);
  assert.match(g.get('access-error').textContent, /full access token/);
  assert.doesNotMatch(g.get('access-error').textContent, /wrong|other/);
  assert.equal(g.get('unlock-access').disabled, false);
});

test('revoked saved access is cleared, with a retryable token form', async () => {
  const g = await gate({ saved: token, response: { ok: false, status: 403 } });

  assert.equal(g.storage.has(key), false);
  assert.deepEqual(g.redirects, []);
  assert.match(g.get('access-error').textContent, /current access token/);
  assert.equal(g.get('unlock-access').disabled, false);

  g.setResponse({ ok: true, status: 204 });
  g.submit(` ${token} `); await g.flush();
  assert.deepEqual(g.redirects, ['https://sameframe.test/room/abc']);
  assert.equal(g.storage.get(key), token);
});

test('first visit prompts for a key; visibility toggling and failed submissions preserve the draft', async () => {
  const g = await gate({ response: { ok: false, status: 429 } });

  assert.equal(g.requests.length, 0);
  assert.equal(g.get('access-token').disabled, false);
  g.get('access-visibility').events.click();
  assert.equal(g.get('access-token').type, 'text');
  assert.equal(g.get('access-visibility')['aria-pressed'], 'true');
  g.get('access-visibility').events.click();
  assert.equal(g.get('access-token').type, 'password');

  g.submit(token); await g.flush();
  assert.match(g.get('access-error').textContent, /too many knocks/);
  assert.equal(g.get('access-token').value, token);
  assert.equal(g.storage.has(key), false);
  assert.equal(g.get('unlock-access').disabled, false);
});

test('blocked localStorage still unlocks with the server cookie and network errors remain actionable', async () => {
  const g = await gate({ blockedStorage: true, url: `https://sameframe.test/?access_token=${token}` });

  assert.deepEqual(g.redirects, ['https://sameframe.test/']);
  assert.equal(g.get('access-error').hidden, true);

  const offline = await gate({ saved: token, response: new TypeError('network failed with secret') });
  assert.deepEqual(offline.redirects, []);
  assert.equal(offline.storage.get(key), token);
  assert.match(offline.get('access-error').textContent, /connection/);
  assert.doesNotMatch(offline.get('access-error').textContent, /secret/);
});

test('unlock is single-flight and an unresponsive request aborts then allows retry', async () => {
  const g = await gate({ response: (options) => new Promise((_resolve, reject) => {
    options.signal.addEventListener('abort', () => reject(new Error('aborted')));
  }) });

  g.submit(token); g.submit(token);
  assert.equal(g.requests.length, 1);
  assert.equal(g.get('unlock-access').disabled, true);
  const timer = [...g.timers.values()][0];
  assert.equal(timer.ms, 10000);
  timer.fn(); await g.flush();

  assert.equal(g.get('unlock-access').disabled, false);
  assert.match(g.get('access-error').textContent, /connection/);
  assert.deepEqual(g.redirects, []);
});

test('blocked cookies do not cause an unlock redirect loop or save unusable access', async () => {
  const g = await gate({ response: (options) => options.method === 'POST'
    ? { ok: true, status: 204 } : { ok: false, status: 401 } });

  g.submit(token); await g.flush(); await g.flush();
  assert.equal(g.requests.length, 2);
  assert.deepEqual(g.redirects, []);
  assert.equal(g.storage.has(key), false);
  assert.equal(g.get('unlock-access').disabled, false);
  assert.match(g.get('access-error').textContent, /Allow cookies/);
});

test('double-slash page paths never become external redirects', async () => {
  const g = await gate({ url: `https://sameframe.test//other.example/path?access_token=${token}` });

  assert.deepEqual(g.replaced, ['https://sameframe.test//other.example/path']);
  assert.deepEqual(g.redirects, ['https://sameframe.test//other.example/path']);
});
