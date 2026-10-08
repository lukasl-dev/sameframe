import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const helperSource = await readFile(new URL('../assets/sync.js', import.meta.url), 'utf8');
const sync = await import(`data:text/javascript;base64,${Buffer.from(helperSource).toString('base64')}`);
const homeSource = (await readFile(new URL('../assets/home.js', import.meta.url), 'utf8'))
  .replace('import(document.body.dataset.syncUrl)', 'Promise.resolve(sync)');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

async function home(fetch, importFailure = false, unavailable = false) {
  const nodes = new Map();
  const navigations = [];
  const get = (id) => {
    if (unavailable && ['join-link', 'join-form'].includes(id)) return null;
    if (!nodes.has(id)) nodes.set(id, { value: '', hidden: true, disabled: false, textContent: '',
      events: {}, addEventListener(name, fn) { this.events[name] = fn; } });
    return nodes.get(id);
  };
  const source = importFailure ? homeSource.replace('Promise.resolve(sync)', 'Promise.reject(new Error("missing module"))') : homeSource;
  await new AsyncFunction('sync', 'document', 'location', 'fetch', source)(sync,
    { body: { dataset: { syncUrl: '/assets/hash-sync.js' } }, getElementById: get },
    { origin: 'https://sameframe.test', assign(path) { navigations.push(path); } }, fetch);
  return { get, navigations,
    click() { return get('create-room').events.click?.({ preventDefault() {} }); },
    join(value) { get('join-link').value = value; return get('join-form').events.submit?.({ preventDefault() {} }); } };
}

test('create POST has no body and navigates with fragment-only host credential', async () => {
  const requests = [];
  const h = await home(async (...args) => {
    requests.push(args);
    return { ok: true, json: async () => ({ room_id: 'abc', host_token: 'secret/token' }) };
  });
  await h.click();
  assert.equal(requests.length, 1);
  assert.equal(requests[0][0], '/api/rooms');
  assert.equal(requests[0][1].method, 'POST');
  assert.equal(requests[0][1].body, undefined);
  assert.deepEqual(h.navigations, ['/room/abc#host=secret%2Ftoken']);
});

test('create is single-flight, retryable, and reports capacity errors visibly', async () => {
  let resolve;
  let calls = 0;
  const h = await home(() => { calls++; return new Promise((done) => { resolve = done; }); });
  const first = h.click();
  await h.click();
  assert.equal(calls, 1);
  assert.equal(h.get('create-room').disabled, true);
  resolve({ ok: false, status: 503 });
  await first;
  assert.equal(h.get('create-room').disabled, false);
  assert.equal(h.get('home-error').hidden, false);
  assert.match(h.get('home-error').textContent, /busy/);
});

test('network and invalid response errors are actionable and never navigate', async () => {
  const network = await home(async () => { throw new TypeError('Failed to fetch'); });
  await network.click();
  assert.match(network.get('home-error').textContent, /connection/);
  const invalid = await home(async () => ({ ok: true, json: async () => ({ room_id: '../bad', host_token: 'secret' }) }));
  await invalid.click();
  assert.equal(invalid.navigations.length, 0);
  assert.match(invalid.get('home-error').textContent, /invalid room/);
});

test('join accepts room link/code, never passes host fragment, and rejects another origin', async () => {
  const h = await home();
  h.join('https://sameframe.test/room/abc#host=secret');
  h.join('test-room');
  h.join('https://evil.test/room/abc');
  assert.deepEqual(h.navigations, ['/room/abc', '/room/test-room']);
  assert.equal(h.get('home-error').hidden, false);
});

test('module-load failure is visible rather than a dead create button', async () => {
  const h = await home(undefined, true);
  assert.equal(h.get('create-room').disabled, true);
  assert.equal(h.get('home-error').hidden, false);
  assert.match(h.get('home-error').textContent, /could not load/);
});

test('unavailable page creates a fresh room without join fields and allows retries after failure', async () => {
  let attempts = 0;
  const h = await home(async () => {
    attempts++;
    if (attempts === 1) return { ok: false, status: 503 };
    return { ok: true, json: async () => ({ room_id: 'fresh-room', host_token: 'keeper-token' }) };
  }, false, true);

  await h.click();
  assert.equal(h.get('home-error').hidden, false);
  assert.equal(h.get('create-room').disabled, false);
  assert.deepEqual(h.navigations, []);

  await h.click();
  assert.equal(h.get('home-error').hidden, true);
  assert.deepEqual(h.navigations, ['/room/fresh-room#host=keeper-token']);
});

test('unavailable page also exposes module failures without assuming a join form exists', async () => {
  const h = await home(undefined, true, true);

  assert.equal(h.get('create-room').disabled, true);
  assert.equal(h.get('home-error').hidden, false);
  assert.match(h.get('home-error').textContent, /could not load/);
});
