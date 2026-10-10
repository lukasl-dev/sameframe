import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const helperSource = await readFile(new URL('../assets/sync.js', import.meta.url), 'utf8');
const sync = await import(`data:text/javascript;base64,${Buffer.from(helperSource).toString('base64')}`);
const roomSource = (await readFile(new URL('../assets/room.js', import.meta.url), 'utf8'))
  .replace('import(document.body.dataset.syncUrl)', 'Promise.resolve(sync)');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const id = 'dQw4w9WgXcQ';
const memberToken = 'a'.repeat(96);

const initial = (overrides = {}) => {
  const snapshot = { incarnation: 'a', room_name: 'The Sleepy Observatory', events: [], revision: 0, media_revision: 0, playback_rate: 1,
    video_id: null, playing: false, position_secs: 0, anchor_ms: 1000, members: 1, ...overrides };
  snapshot.participants ??= Array.from({ length: Math.max(0, Math.min(32, snapshot.members)) }, (_, index) => ({
    id: index + 1, name: index ? `Cosmic Goose ${index}` : 'Sleepy Waffle', role: index ? 'guest' : 'host', avatar: (index + 1).toString(16).padStart(16, '0'),
  }));
  return snapshot;
};

async function browser({ token = null, api = true, storageBlocked = false, importFailure = false,
  clipboardBlocked = false, query = '', pathname = '/room/test-room', rates = [0.5, 1, 1.5, 2],
  unavailableInitialCaches = false, memberStorage = new Map() } = {}) {
  let now = 1000;
  let timerId = 0;
  const timers = new Map();
  const intervals = [];

  const nodes = new Map();
  const writes = [];
  const documentEvents = new Map();
  const windowEvents = new Map();

  let copied = null;
  const stored = new Map();
  const historyCalls = [];
  const warnings = [];

  let commandId = 0;
  const sockets = [];
  const players = [];
  const get = (name) => {
    if (!nodes.has(name)) {
      const node = { tagName: 'DIV', parentNode: { append(node) { nodes.set(node.id, node); } },
        disabled: false, value: '', events: {}, dataset: {}, children: [],
        replaceChildren(...children) { this.children = children; this.replacements = (this.replacements ?? 0) + 1; },
        append(...children) { this.children.push(...children); },
        scrollTop: 0, scrollHeight: 300, clientHeight: 300,
        toggleAttribute(attr, value) { writes.push([name, attr, value]); this[attr] = value; },
        addEventListener(event, fn) { this.events[event] = fn; } };
      for (const [key, initial] of [['hidden', true], ['textContent', ''], ['title', '']]) {
        let value = initial;
        Object.defineProperty(node, key, { get: () => value,
          set(next) { writes.push([name, key, next]); value = next; } });
      }
      nodes.set(name, node);
    }
    return nodes.get(name);
  };

  const document = { body: { dataset: { roomId: 'test-room', syncUrl: '/assets/hash-sync.js' } }, hidden: false,
    getElementById: get, addEventListener(event, fn) { documentEvents.set(event, fn); },
    createElement(tag) { return { tagName: tag.toUpperCase(), dataset: {}, children: [],
      events: {}, addEventListener(event, fn) { this.events[event] = fn; },
      setAttribute(key, value) { this[key] = value; },
      append(...children) { this.children.push(...children); }, remove() { this.removed = true; } }; },
    createElementNS(_namespace, tag) { return { tagName: tag, children: [], attributes: {},
      classList: { add() {} }, setAttribute(key, value) { this.attributes[key] = value; },
      append(...children) { this.children.push(...children); } }; },
    head: { scripts: [], append(script) { this.scripts.push(script); } } };

  class WebSocket {
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    constructor(url) { this.url = url; this.readyState = 0; this.events = {}; this.sent = []; sockets.push(this); }
    addEventListener(event, fn) { this.events[event] = fn; }
    send(value) { this.sent.push(JSON.parse(value)); }
    open() { this.readyState = 1; this.events.open(); }
    receive(value) {
      if (value.snapshot) this.latestSnapshot = value.snapshot;
      this.events.message({ data: JSON.stringify(value) });
    }
    close(code = 1006) { this.readyState = 3; this.events.close?.({ code }); }
  }

  const YT = {
    PlayerState: { UNSTARTED: -1, ENDED: 0, PLAYING: 1, PAUSED: 2, BUFFERING: 3, CUED: 5 },
    Player: class {
      constructor(mount, options) {
        this.options = options;
        this.state = -1;
        this.time = 0;
        this.duration = 100;
        this.rate = 1;
        this.videoId = '';

        this.messages = [];
        this.seeks = [];
        this.plays = 0;
        this.pauses = 0;
        this.videos = [];
        this.cues = [];
        this.loads = [];
        this.rateWrites = [];
        players.push(this);

        assert.equal(get(mount).tagName, 'DIV');
        get(mount).tagName = 'IFRAME';
      }
      ready() { this.options.events.onReady(); }
      destroy() { this.destroyed = true; nodes.delete('player-mount'); }
      deliver() {
        let count = 0;
        while (this.messages.length) {
          assert.ok(count++ < 30, 'no correction write loop');
          this.messages.shift()();
        }
      }
      emit(state) { this.state = state; this.options.events.onStateChange({ data: state }); }

      cueVideoById(value) {
        this.videos.push(value);
        this.cues.push(value);
        this.messages.push(() => {
          this.videoId = value.videoId;
          this.time = value.startSeconds;
          this.emit(5);
        });
      }

      loadVideoById(value) {
        this.videos.push(value);
        this.loads.push(value);
        this.messages.push(() => {
          this.videoId = value.videoId;
          this.time = value.startSeconds;
          this.emit(3);
          this.emit(1);
        });
      }

      seekTo(value) {
        this.seeks.push(value);
        this.messages.push(() => {
          const wasPlaying = this.state === 1;
          this.time = value;
          this.emit(3);
          this.emit(wasPlaying ? 1 : 2);
        });
      }

      playVideo() {
        this.plays++;
        const missingVideo = !this.videoId;
        this.messages.push(() => {
          if (missingVideo) this.options.events.onError({ data: 2 });
          else this.emit(1);
        });
      }
      pauseVideo() { this.pauses++; this.messages.push(() => this.emit(2)); }

      getCurrentTime() { return this.time; }
      getDuration() { return this.duration; }
      getPlayerState() { return this.state; }
      getVideoData() { return unavailableInitialCaches && !this.videoId ? undefined : { video_id: this.videoId }; }
      getPlaybackRate() { return unavailableInitialCaches && !this.videoId ? undefined : this.rate; }
      getAvailablePlaybackRates() { return unavailableInitialCaches && !this.videoId ? undefined : rates; }

      setPlaybackRate(rate) {
        assert.ok(rates.includes(rate), 'only supported rates are written');
        this.rateWrites.push(rate);
        this.messages.push(() => { this.rate = rate; this.options.events.onPlaybackRateChange({ data: rate }); });
      }

      userState(state, lagCache = false) {
        if (lagCache) {
          this.messages.push(() => { this.state = state; });
          this.options.events.onStateChange({ data: state });
        } else this.emit(state);
      }
      userRate(rate, lagCache = false) {
        if (lagCache) this.messages.push(() => { this.rate = rate; }); else this.rate = rate;
        this.options.events.onPlaybackRateChange({ data: rate });
      }
    },
  };

  const globals = { YT: api ? YT : undefined, crypto: { randomUUID: () => `command-${++commandId}` } };
  const setTimeout = (fn, ms) => { const key = ++timerId; timers.set(key, { fn, ms }); return key; };
  const clearTimeout = (key) => timers.delete(key);
  const source = importFailure ? roomSource.replace('Promise.resolve(sync)', 'Promise.reject(new Error("missing"))') : roomSource;
  await new AsyncFunction('sync', 'document', 'window', 'location', 'history', 'sessionStorage', 'localStorage', 'performance',
    'WebSocket', 'YT', 'globalThis', 'setTimeout', 'clearTimeout', 'setInterval', 'navigator', 'console', source)(
    sync, document, { addEventListener(event, fn) { windowEvents.set(event, fn); } },
    { origin: 'https://sameframe.test', host: 'sameframe.test', protocol: 'https:', pathname, search: query, hash: token ? `#host=${token}` : '' },
    { replaceState(...args) { historyCalls.push(args); } },
    { getItem(key) { if (storageBlocked) throw Error('blocked'); return stored.get(key) ?? null; },
      setItem(key, value) { if (storageBlocked) throw Error('blocked'); stored.set(key, value); } },
    { getItem(key) { if (storageBlocked) throw Error('blocked'); return memberStorage.get(key) ?? null; },
      setItem(key, value) { if (storageBlocked) throw Error('blocked'); memberStorage.set(key, value); },
      removeItem(key) { if (storageBlocked) throw Error('blocked'); memberStorage.delete(key); } },
    { now: () => now }, WebSocket, YT, globals, setTimeout, clearTimeout,
    (fn, ms) => intervals.push({ fn, ms }), { clipboard: { async writeText(value) {
      if (clipboardBlocked) throw Error('denied'); copied = value;
    } } }, { warn(...args) { warnings.push(args); } });

  const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
  const tick = (ms = 300) => intervals.find((timer) => timer.ms === ms).fn();
  const frame = (ms = 300, advance = true) => {
    now += ms;
    if (advance) for (const p of players) if (!p.destroyed && p.state === 1) p.time += ms / 1000 * p.rate;
    tick();
  };
  const welcome = async (snapshot = initial(), role = 'host') => {
    const ws = sockets.at(-1);
    ws.testRole = role;
    snapshot.participants = snapshot.participants.map((member) => member.id === 1 ? { ...member, role } : member);
    ws.open();
    ws.receive({ type: 'welcome', role, member_id: 1, member_token: memberToken, snapshot, server_ms: now });
    await flush();
    return ws;
  };
  const ready = () => {
    const p = players.at(-1);
    p.ready();
    p.deliver();
    frame();
    frame();
    return p;
  };
  const state = (ws, snapshot) => {
    snapshot.participants = snapshot.participants.map((member) => member.id === 1 ? { ...member, role: ws.testRole } : member);
    ws.receive({ type: 'snapshot', snapshot, server_ms: now });
  };
  const accept = (ws, command = commands(ws).at(-1), overrides = {}, snapshotFirst = false) => {
    const revision = command.revision + 1;
    const action = command.action;
    const snapshot = { ...ws.latestSnapshot, revision, anchor_ms: now };
    if (action.type === 'set_video') Object.assign(snapshot, { video_id: action.video_id, media_revision: revision, playing: false, position_secs: 0, playback_rate: 1 });
    else if (action.type === 'set_rate') Object.assign(snapshot, { position_secs: sync.targetPosition(ws.latestSnapshot, now), playback_rate: action.playback_rate });
    else Object.assign(snapshot, { playing: action.type === 'play', position_secs: action.position_secs });
    Object.assign(snapshot, overrides);

    if (snapshotFirst) state(ws, snapshot);
    ws.receive({ type: 'ack', id: command.id, revision });
    if (!snapshotFirst) state(ws, snapshot);
    return snapshot;
  };

  return { get, document, documentEvents, windowEvents, sockets, players, timers, intervals, stored, memberStorage, historyCalls, writes, warnings,
    welcome, ready, frame, tick, accept, state, flush, get copied() { return copied; },
    time(value) { now = value; },
    click(name) { return get(name).events.click?.({ preventDefault() {} }); },
    submit(name) { return get(name).events.submit?.({ preventDefault() {} }); },
    fireTimer(ms) {
      const entry = [...timers].find(([, timer]) => timer.ms === ms);
      assert.ok(entry);
      timers.delete(entry[0]);
      entry[1].fn();
    } };
}

const commands = (ws) => ws.sent.filter((message) => message.type === 'command');
const socialCommands = (ws) => ws.sent.filter((message) => message.type === 'social');
const event = (id, kind, overrides = {}) => ({ id, kind, at_ms: 1000, member_id: 1,
  name: 'Sleepy Waffle', avatar: '0123456789abcdef', text: null, video_id: null, position_secs: null, ...overrides });

test('server-issued room identity survives reconnect and reload without granting Keeper access', async () => {
  const memberStorage = new Map();
  const b = await browser({ memberStorage });
  const ws = await b.welcome(initial(), 'guest');

  assert.equal(ws.sent[0].member_token, null);
  assert.equal(memberStorage.get('sameframe:member:test-room'), memberToken);

  ws.close();
  b.click('reconnect-button');
  const reconnected = await b.welcome(initial(), 'guest');
  assert.deepEqual(reconnected.sent[0], { type: 'join', host_token: null, member_token: memberToken });

  const reload = await browser({ memberStorage });
  const fresh = await reload.welcome(initial(), 'guest');
  assert.deepEqual(fresh.sent[0], { type: 'join', host_token: null, member_token: memberToken });
  assert.equal(reload.get('role-label').textContent, 'You’re a Companion');
  assert.equal(reload.historyCalls.length, 0);
  await reload.click('copy-link');
  assert.equal(reload.copied, 'https://sameframe.test/room/test-room');
});

test('blocked member storage keeps the current room identity in memory for reconnect', async () => {
  const b = await browser({ storageBlocked: true });
  const ws = await b.welcome(initial(), 'guest');
  assert.equal(b.memberStorage.size, 0);

  ws.close();
  b.click('reconnect-button');
  const resumed = await b.welcome(initial(), 'guest');
  assert.equal(resumed.sent[0].member_token, memberToken);
});

test('rejected saved identity retries without it, but invalid Keeper access still stops', async () => {
  const memberStorage = new Map([['sameframe:member:test-room', memberToken]]);
  const b = await browser({ token: 'wrong-host', memberStorage });
  const first = b.sockets[0];
  first.open();
  first.receive({ type: 'error', code: 'invalid_token', id: null });

  assert.equal(b.sockets.length, 2);
  assert.equal(memberStorage.size, 0);
  const retry = b.sockets[1];
  retry.open();
  assert.deepEqual(retry.sent[0], { type: 'join', host_token: 'wrong-host', member_token: null });

  retry.receive({ type: 'error', code: 'invalid_token', id: null });
  assert.equal(b.get('connection-status').textContent, 'Host access rejected');
  assert.equal(b.sockets.length, 2);
});

test('malformed identity storage and welcome tokens cannot become room credentials', async () => {
  const memberStorage = new Map([['sameframe:member:test-room', 'not-a-token']]);
  const b = await browser({ memberStorage });
  const ws = b.sockets[0];
  ws.open();
  assert.equal(ws.sent[0].member_token, null);

  for (const invalid of ['invalid', [memberToken], null, 123]) {
    ws.receive({ type: 'welcome', role: 'guest', member_id: 1, member_token: invalid,
      snapshot: initial(), server_ms: 1000 });
  }
  assert.equal(b.get('connection-status').textContent, 'Connecting…');
  assert.equal(memberStorage.get('sameframe:member:test-room'), 'not-a-token');

  ws.receive({ type: 'welcome', role: 'guest', member_id: 1, member_token: memberToken,
    snapshot: initial(), server_ms: 1000 });
  assert.equal(b.get('connection-status').textContent, 'Connected');
  assert.equal(memberStorage.get('sameframe:member:test-room'), memberToken);
});

test('same-room reconnect applies a paused snapshot without echoing delayed corrections as Play or Pause', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser();
    const old = await b.welcome(initial({ video_id: id, playing: true }), role);
    const p = b.ready();

    b.document.hidden = true;
    b.documentEvents.get('visibilitychange')();
    old.close();
    b.click('reconnect-button');
    const paused = initial({ video_id: id, playing: false, position_secs: 60, revision: 1 });
    const ws = await b.welcome(paused, role);
    b.document.hidden = false;
    b.documentEvents.get('visibilitychange')();
    b.frame(300, false);
    assert.deepEqual(commands(ws), []);

    b.state(ws, paused);
    b.frame(300, false);
    b.frame(300, false);
    assert.deepEqual(commands(ws), []);
    assert.equal(b.get('join-playback').hidden, true);

    while (p.messages.length) {
      p.messages.shift()();
      b.frame(300, false);
      assert.deepEqual(commands(ws), []);
    }
    b.frame(300, false);
    assert.equal(p.state, 2);
    assert.equal(p.time, 60);
    assert.equal(b.get('join-playback').hidden, true);

    p.userState(1);
    if (role === 'host') assert.equal(commands(ws)[0].action.type, 'play');
    else assert.equal(b.get('join-playback').hidden, false);
  }
});

test('cached offline rate changes are corrected on reconnect instead of changing or detaching from the room', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser();
    const old = await b.welcome(initial({ video_id: id }), role);
    const p = b.ready();

    b.document.hidden = true;
    b.documentEvents.get('visibilitychange')();
    old.close();
    p.rate = 2;
    p.time = 40;
    b.click('reconnect-button');
    const fresh = initial({ video_id: id, position_secs: 10, playback_rate: 1 });
    const ws = await b.welcome(fresh, role);
    b.document.hidden = false;
    b.documentEvents.get('visibilitychange')();
    b.state(ws, fresh);

    b.frame(300, false);
    b.frame(300, false);
    assert.deepEqual(commands(ws), []);
    p.deliver();
    b.frame(300, false);
    b.frame(300, false);
    assert.equal(p.rate, 1);
    assert.equal(p.time, 10);
    assert.equal(b.get('join-playback').hidden, true);

    p.userRate(1.5);
    b.frame(300, false);
    if (role === 'host') assert.deepEqual(commands(ws)[0].action, { type: 'set_rate', playback_rate: 1.5 });
    else assert.equal(b.get('join-playback').hidden, false);
  }
});

test('reconnecting to the same playing video reloads lost iframe media atomically', async () => {
  const b = await browser();
  const old = await b.welcome(initial({ video_id: id, playing: true }));
  const p = b.ready();
  const loads = p.loads.length;

  old.close();
  p.videoId = '';
  p.state = -1;
  b.click('reconnect-button');
  const ws = await b.welcome(initial({ video_id: id, playing: true, position_secs: 30 }));
  assert.equal(p.loads.length, loads + 1);
  assert.equal(p.seeks.length, 0);
  p.deliver();
  b.frame();
  b.frame();

  assert.equal(p.videoId, id);
  assert.equal(p.state, 1);
  assert.equal(b.get('room-error').hidden, true);
  assert.deepEqual(commands(ws), []);
});

test('visibility recovery without disconnect also waits for fresh state and ignores old cached playback', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, playing: true }));
  const p = b.ready();

  b.document.hidden = true;
  b.documentEvents.get('visibilitychange')();
  p.rate = 2;
  b.document.hidden = false;
  b.documentEvents.get('visibilitychange')();
  b.frame(300, false);
  assert.deepEqual(commands(ws), []);

  b.state(ws, initial({ video_id: id, playing: false, position_secs: 20, revision: 1 }));
  b.frame(300, false);
  b.frame(300, false);
  p.deliver();
  b.frame(300, false);
  b.frame(300, false);
  assert.deepEqual(commands(ws), []);
  assert.equal(p.time, 20);
  assert.equal(p.rate, 1);
  assert.equal(p.state, 2);
});

test('returning to a closed socket reconnects immediately instead of waiting on a suspended retry timer', async () => {
  const b = await browser();
  const ws = await b.welcome(initial(), 'guest');

  b.document.hidden = true;
  ws.close();
  assert.equal(b.sockets.length, 1);
  b.document.hidden = false;
  b.documentEvents.get('visibilitychange')();
  assert.equal(b.sockets.length, 2);

  const resumed = await b.welcome(initial(), 'guest');
  assert.equal(resumed.sent[0].member_token, memberToken);
});

test('temporary unavailable media metadata does not reload an otherwise retained iframe on reconnect', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();
  const cues = p.cues.length;
  p.getVideoData = () => undefined;

  ws.close();
  b.click('reconnect-button');
  const resumed = await b.welcome(initial({ video_id: id }));
  b.frame(300, false);
  assert.equal(p.cues.length, cues);
  assert.deepEqual(commands(resumed), []);

  p.getVideoData = () => ({ video_id: id });
  b.frame(300, false);
  p.userState(1);
  assert.equal(commands(resumed)[0].action.type, 'play');
});

test('visibility recovery preserves intentional local watching for a detached Companion', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, playing: true }), 'guest');
  const p = b.ready();
  p.userState(2);
  b.frame(300, false);
  assert.equal(b.get('join-playback').hidden, false);

  b.document.hidden = true;
  b.document.hidden = false;
  b.documentEvents.get('visibilitychange')();
  b.state(ws, initial({ video_id: id, playing: true, position_secs: 20 }));
  b.frame(300, false);
  assert.equal(p.state, 2);
  assert.equal(b.get('join-playback').hidden, false);

  b.click('join-playback');
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(p.state, 1);
  assert.deepEqual(commands(ws), []);
});

test('visibility before selecting a video does not suppress the first native Play on a new iframe', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser();
    const ws = await b.welcome(initial(), role);
    b.documentEvents.get('visibilitychange')();
    b.state(ws, initial());

    b.state(ws, initial({ video_id: id, revision: 1, media_revision: 1 }));
    await b.flush();
    const p = b.players[0];
    p.ready();
    p.messages = [];
    p.videoId = id;
    p.userState(3);
    b.frame(300, false);
    p.userState(1);
    b.frame(300, false);
    b.frame(300, false);

    if (role === 'host') assert.equal(commands(ws)[0].action.type, 'play');
    else assert.equal(b.get('join-playback').hidden, false);
  }
});

test('unavailable metadata is rechecked and later-confirmed lost media reloads without empty-video Play', async () => {
  for (const playing of [false, true]) {
    const b = await browser();
    const ws = await b.welcome(initial({ video_id: id, playing }));
    const p = b.ready();
    const videos = p.videos.length;
    p.videoId = '';
    p.state = -1;
    p.getVideoData = () => undefined;

    ws.close();
    b.click('reconnect-button');
    const resumed = await b.welcome(initial({ video_id: id, playing, position_secs: 30 }));
    b.frame(300, false);
    assert.equal(p.videos.length, videos);
    assert.equal(p.messages.length, 0);

    p.getVideoData = () => ({ video_id: p.videoId });
    b.frame(300, false);
    assert.equal(p.videos.length, videos + 1);
    p.deliver();
    b.frame(300, false);
    b.frame(300, false);

    assert.equal(p.state, playing ? 1 : 2);
    assert.equal(p.videoId, id);
    assert.equal(b.get('room-error').hidden, true);
    assert.deepEqual(commands(resumed), []);
  }
});

test('native autoplay-unlock Play during recovery rejoins locally instead of leaving stale blocked state', async () => {
  for (const role of ['host', 'guest']) {
    for (const delayedCache of [false, true]) {
      const b = await browser();
      const old = await b.welcome(initial({ video_id: id, playing: true }), role);
      const p = b.ready();
      old.close();
      b.click('reconnect-button');
      const ws = await b.welcome(initial({ video_id: id, playing: true, position_secs: 20 }), role);
      p.messages = [];
      p.time = 20;
      p.state = 2;
      p.options.events.onAutoplayBlocked();

      p.userState(1, delayedCache);
      p.deliver();
      b.frame();
      b.frame();
      assert.equal(p.state, 1);
      assert.equal(b.get('join-playback').hidden, true);
      assert.equal(b.get('room-notice').textContent, '');
      assert.deepEqual(commands(ws), []);
    }
  }
});

test('Companions suggest videos without playback commands and chat drafts clear only on their own ack', async () => {
  const b = await browser();
  const ws = await b.welcome(initial(), 'guest');

  assert.equal(b.get('load-video').textContent, 'Suggest');
  assert.equal(b.get('video-url').disabled, false);

  b.get('video-url').value = `https://youtu.be/${id}`;
  b.submit('video-form');
  const proposal = socialCommands(ws)[0];
  assert.deepEqual(proposal.action, { type: 'propose_video', video_id: id });
  assert.equal(commands(ws).length, 0);

  ws.receive({ type: 'social_ack', id: 'other', event_id: 1 });
  assert.equal(b.get('video-url').value, `https://youtu.be/${id}`);
  ws.receive({ type: 'social_ack', id: proposal.id, event_id: 1 });
  assert.equal(b.get('video-url').value, '');

  b.get('chat-message').value = '  Hello <script>no HTML</script>  ';
  b.submit('chat-form');
  const message = socialCommands(ws)[1];
  assert.deepEqual(message.action, { type: 'message', text: 'Hello <script>no HTML</script>' });
  ws.receive({ type: 'error', id: message.id, code: 'busy', message: 'Please try again.' });
  assert.equal(b.get('chat-error').textContent, 'Please try again.');
  assert.equal(b.get('chat-message').value, '  Hello <script>no HTML</script>  ');
  assert.equal(b.get('send-message').disabled, false);
});

test('chat YouTube links become proposals and disconnects preserve unsent text', async () => {
  const b = await browser();
  const ws = await b.welcome();

  b.get('chat-message').value = `https://youtu.be/${id}`;
  b.submit('chat-form');
  assert.deepEqual(socialCommands(ws)[0].action, { type: 'propose_video', video_id: id });

  ws.close();
  assert.equal(b.get('chat-message').value, `https://youtu.be/${id}`);
  assert.equal(b.get('send-message').disabled, true);
  assert.equal(commands(ws).length, 0);
});

test('retained feed is appended once on same-revision snapshots and changes only on a new room incarnation', async () => {
  const b = await browser();
  const entries = [event(1, 'joined'), event(2, 'message', { text: '<img src=x onerror=alert(1)>' })];
  const ws = await b.welcome(initial({ events: entries }));
  const feed = b.get('chat-feed');

  assert.equal(feed.children.length, 2);
  assert.equal(feed.children[1].children[1].children[1].textContent, '<img src=x onerror=alert(1)>');

  const first = feed.children[0];
  b.state(ws, initial({ events: entries }));
  assert.equal(feed.children.length, 2);
  assert.equal(feed.children[0], first);

  b.state(ws, initial({ events: [...entries, event(3, 'pause', { position_secs: 12 })] }));
  assert.equal(feed.children.length, 3);
  assert.equal(feed.children[0], first);
  assert.equal(commands(ws).length, 0);

  ws.close();
  b.click('reconnect-button');
  await b.welcome(initial({ incarnation: 'b', events: [event(1, 'left')] }));
  assert.equal(feed.children.length, 1);
  assert.notEqual(feed.children[0], first);
});

test('new activity does not scroll someone reading history; unread control returns to the bottom', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ events: [event(1, 'joined')] }));
  const feed = b.get('chat-feed');
  feed.scrollHeight = 1000;
  feed.clientHeight = 300;
  feed.scrollTop = 100;

  b.state(ws, initial({ events: [event(1, 'joined'), event(2, 'message', { text: 'Hello!' })] }));
  assert.equal(feed.scrollTop, 100);
  assert.equal(b.get('chat-unread').hidden, false);

  b.click('chat-unread');
  assert.equal(feed.scrollTop, 1000);
  assert.equal(b.get('chat-unread').hidden, true);
});

test('same-revision grants enable Co-keeper proposal approval and revocation discards pending playback', async () => {
  const b = await browser();
  const entries = [event(1, 'proposal', { video_id: id })];
  const ws = await b.welcome(initial({ events: entries }), 'guest');
  const proposal = b.get('chat-feed').children[0].proposal;
  assert.equal(proposal.button.hidden, true);

  ws.testRole = 'moderator';
  b.state(ws, initial({ events: entries }));
  assert.equal(b.get('role-label').textContent, 'You’re a Co-keeper');
  assert.equal(b.get('load-video').textContent, 'Watch');
  assert.equal(proposal.button.hidden, false);

  proposal.button.events.click();
  assert.deepEqual(commands(ws)[0].action, { type: 'set_video', video_id: id });
  assert.equal(proposal.button.disabled, true);

  ws.testRole = 'guest';
  b.state(ws, initial({ events: entries }));
  assert.equal(b.get('load-video').textContent, 'Suggest');
  assert.equal(proposal.button.hidden, true);
  assert.equal(b.get('video-url').disabled, false);
  ws.receive({ type: 'ack', id: commands(ws)[0].id, revision: 1 });
  assert.equal(commands(ws).length, 1);
});

test('Keepers grant roles through independent social actions; Co-keepers do not get role controls', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ members: 2 }));
  const button = b.get('members').children[1].children[2];

  button.events.click();
  assert.deepEqual(socialCommands(ws)[0].action, { type: 'set_moderator', member_id: 2, enabled: true });
  assert.equal(commands(ws).length, 0);

  const moderator = await browser();
  await moderator.welcome(initial({ members: 2 }), 'moderator');
  assert.equal(moderator.get('members').children[1].children.length, 2);
});

test('native Co-keeper controls share playback; buffering never produces a banner', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }), 'moderator');
  const p = b.ready();

  p.userState(3);
  assert.equal(b.get('room-notice').textContent, '');
  assert.equal(b.get('room-notice').hidden, true);
  p.userState(1);
  assert.equal(commands(ws)[0].action.type, 'play');

  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();

  p.userState(2);
  b.frame(300, false);
  assert.equal(commands(ws)[1].action.type, 'pause');
});

test('member list renders stable local avatars, roles and own identity without playback churn', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ members: 2 }));
  const list = b.get('members');

  assert.equal(list.children.length, 2);
  assert.equal(list.children[0].dataset.role, 'host');
  assert.equal(list.children[0].children[1].children[0].textContent, 'Sleepy Waffle (you)');
  assert.equal(list.children[1].children[1].children[0].textContent, 'Cosmic Goose 1');
  assert.equal(list.children[0].children[1].children[1].textContent, 'Keeper');
  assert.equal(list.children[0].children[0].tagName, 'svg');
  const firstAvatar = list.children[0].children[0];
  const replacements = list.replacements;

  b.state(ws, initial({ members: 2, revision: 1 }));
  assert.equal(list.replacements, replacements, 'playback changes do not rebuild the member list');
  assert.equal(list.children[0].children[0], firstAvatar);

  b.state(ws, initial({ members: 3, revision: 1 }));
  assert.equal(list.children.length, 3);
  assert.equal(commands(ws).length, 0, 'presence does not generate playback commands');

  b.state(ws, initial({ members: 1, revision: 1 }));
  assert.equal(list.children.length, 1);
});

test('credentials, clean canonical address and icon-only invite copy remain safe', async () => {
  const b = await browser({ token: 'secret', query: '?tracking=x&host=ignored', pathname: '/room/test-room/' });
  const ws = await b.welcome();

  assert.equal(b.stored.get('sameframe:host:test-room'), 'secret');
  assert.deepEqual(b.historyCalls[0], [null, '', '/room/test-room']);
  assert.deepEqual(ws.sent[0], { type: 'join', host_token: 'secret', member_token: null });

  await b.click('copy-link');
  assert.equal(b.copied, 'https://sameframe.test/room/test-room');
  assert.equal(b.get('copy-link').textContent, '');
  assert.equal(b.players.length, 0);
});

test('storage blocking and clipboard denial have safe visible fallbacks', async () => {
  const b = await browser({ token: 'secret', storageBlocked: true, clipboardBlocked: true });
  const ws = await b.welcome();
  assert.equal(ws.sent[0].host_token, 'secret');

  await b.click('copy-link');
  assert.equal(b.get('copy-feedback').textContent, 'Copy URL from address bar');
  assert.equal(b.get('copy-link').title, 'Copy URL from address bar');
  assert.equal(b.get('room-notice').textContent, '');
});

test('native controls enabled; selected video initially paused; no duplicate handlers', async () => {
  const b = await browser();
  const ws = await b.welcome();

  b.get('video-url').value = id;
  b.submit('video-form');
  b.submit('video-form');
  assert.equal(commands(ws).length, 1);
  assert.equal(b.get('load-video').disabled, true);

  b.accept(ws);
  await b.flush();
  const p = b.ready();
  assert.deepEqual(p.options.playerVars, { controls: 1, disablekb: 0, fs: 1, playsinline: 1, rel: 0, origin: 'https://sameframe.test' });
  assert.equal(p.state, 2);
  assert.equal(p.time, 0);
  for (const name of ['playback-toggle', 'seek-form', 'mute-toggle', 'fullscreen-toggle']) assert.deepEqual(b.get(name).events, {});
  assert.equal(b.get('load-video').disabled, false);
});

test('native host play and stable pause issue revisioned commands despite asynchronous state getters', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userState(1, true);
  assert.equal(p.getPlayerState(), 2, 'cached state still paused');
  assert.equal(commands(ws)[0].action.type, 'play');

  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();

  p.userState(2, true);
  assert.equal(commands(ws).length, 1, 'pause debounce avoids buffering transitions');
  b.frame(300, false);
  assert.equal(commands(ws)[1].action.type, 'pause');
  assert.equal(commands(ws)[1].revision, 1);
});

test('native paused seek and playing scrub are observed before room drift correction', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.time = 30;
  b.frame(300, false);
  assert.deepEqual(commands(ws)[0].action, { type: 'pause', position_secs: 30 });
  assert.equal(p.seeks.length, 0, 'native scrub not corrected back before its command');

  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();

  p.userState(1);
  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();

  p.time = 60;
  p.userState(3);
  assert.equal(commands(ws).at(-1).action.type, 'play');
  assert.equal(commands(ws).at(-1).action.position_secs, 60, 'buffering during scrub preserves playing');
});

test('programmatic cue, seek, pause, play and delayed cached state updates never echo', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, position_secs: 10 }));
  const p = b.ready();
  assert.equal(commands(ws).length, 0);

  b.state(ws, initial({ revision: 1, video_id: id, playing: true, position_secs: 40, anchor_ms: 1600 }));
  assert.equal(p.time, 10, 'programmatic seek getter still old');
  b.frame(300, false);
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 0);

  b.state(ws, initial({ revision: 2, video_id: id, position_secs: 20 }));
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 0);
  assert.equal(p.state, 2);
});

test('one command plus one latest intention coalesces rapid scrub/play/pause without older ack rollback', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userState(1);
  const first = commands(ws)[0];
  p.time = 25;
  b.frame(300, false);
  p.time = 42;
  p.userState(2);
  b.frame(300, false);
  assert.equal(commands(ws).length, 1);

  b.accept(ws, first, {}, true);
  assert.equal(commands(ws).length, 2);
  assert.deepEqual(commands(ws)[1].action, { type: 'pause', position_secs: 42 });
  assert.equal(p.time, 42, 'older play ack does not overwrite newest paused scrub');

  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 2);
});

test('media generation changes discard old pending/queued scrub and ignore stale video events', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userState(1);
  const old = commands(ws)[0];
  p.time = 42;
  b.frame(300, false);

  b.state(ws, initial({ revision: 2, media_revision: 2, video_id: 'abcdefghijk' }));
  p.userState(0);
  p.userState(1);
  p.time = 70;
  b.tick();
  ws.receive({ type: 'ack', id: old.id, revision: 1 });
  assert.equal(commands(ws).length, 1);

  p.deliver();
  b.frame();
  b.frame();
  assert.equal(p.videoId, 'abcdefghijk');
  assert.equal(commands(ws).length, 1);
  assert.equal(p.state, 2);
});

test('old-video scrubs during Load are discarded; new-media native input may coalesce before Load ack', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  b.get('video-url').value = 'abcdefghijk';
  b.submit('video-form');
  const load = commands(ws)[0];
  p.time = 42;
  p.userState(1);
  b.frame();
  assert.equal(commands(ws).length, 1);

  b.state(ws, initial({ revision: 1, media_revision: 1, video_id: 'abcdefghijk' }));
  p.deliver();
  b.frame();
  b.frame();
  p.time = 30;
  b.frame(300, false);
  assert.equal(commands(ws).length, 1);

  ws.receive({ type: 'ack', id: load.id, revision: 1 });
  assert.deepEqual(commands(ws)[1].action, { type: 'pause', position_secs: 30 });
  assert.equal(commands(ws)[1].revision, 1);
});

test('pending native rate plus queued scrub cannot cross a same-ID media generation', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userRate(2);
  const rate = commands(ws)[0];
  p.time = 42;
  b.frame(300, false);

  b.state(ws, initial({ revision: 2, media_revision: 2, video_id: id }));
  ws.receive({ type: 'ack', id: rate.id, revision: 1 });
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 1);
  assert.equal(p.rate, 1);
  assert.equal(p.time, 0);
});

test('same ID only re-cues on media_revision change, not pause or seek zero', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  b.state(ws, initial({ revision: 1, video_id: id }));
  p.deliver();
  b.frame();
  assert.equal(p.videos.length, 1);

  p.options.events.onError({ data: 100 });
  b.state(ws, initial({ revision: 2, media_revision: 2, video_id: id }));
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(p.videos.length, 2);
  assert.equal(b.get('room-error').hidden, true);
});

test('buffering, stalled local time and short paused buffer transitions never pause/seek room', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, playing: true }));
  const p = b.ready();

  p.userState(3);
  for (let i = 0; i < 20; i++) b.frame(300, false);
  assert.equal(commands(ws).length, 0);
  assert.equal(p.seeks.length, 0, 'unchanged buffering never hard-seeks');

  p.userState(2);
  b.frame(100, false);
  p.userState(1);
  assert.equal(commands(ws).length, 0, 'short buffer PAUSED observation is not a user pause');

  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 0, 'buffer-exit catch-up is programmatic');
});

test('guest native pause and seeks detach locally; native Play stays local; explicit Rejoin catches up', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, playing: true }), 'guest');
  const p = b.ready();

  p.userState(2);
  b.frame(300, false);
  assert.equal(b.get('join-playback').textContent, 'Rejoin playback');
  assert.equal(b.get('join-playback').hidden, false);
  assert.equal(b.get('playback-status').textContent, 'YouTube · watching locally');

  const plays = p.plays;
  for (let i = 0; i < 5; i++) b.frame(300, false);
  assert.equal(p.plays, plays, 'local pause is not force-resumed');

  p.time = 0;
  b.frame(300, false);
  p.userState(1);
  b.frame();
  assert.equal(p.seeks.length, 0, 'rewind and native Play remain local');

  b.click('join-playback');
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(b.get('join-playback').hidden, true);
  assert.ok(p.seeks.at(-1) > 2);
  assert.equal(b.get('playback-status').textContent, 'YouTube · following room');
  assert.equal(commands(ws).length, 0);
});

test('guest paused seek detaches, a new host media selection reattaches with a clear notice', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }), 'guest');
  const p = b.ready();

  p.time = 30;
  b.frame(300, false);
  assert.match(b.get('room-notice').textContent, /local on this device/);

  b.state(ws, initial({ revision: 1, media_revision: 1, video_id: id }));
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(b.get('join-playback').hidden, true);
  assert.match(b.get('room-notice').textContent, /following the room again/);
  assert.equal(commands(ws).length, 0);
});

test('native host rate is sent once; combined rate + scrub intentions are serialized without loss', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userRate(2, true);
  assert.equal(p.getPlaybackRate(), 1, 'rate cache still old when event arrives');
  assert.deepEqual(commands(ws)[0].action, { type: 'set_rate', playback_rate: 2 });

  p.deliver();
  p.time = 30;
  b.frame(300, false);
  p.userState(1);
  assert.equal(commands(ws).length, 1);

  b.accept(ws);
  assert.equal(commands(ws).length, 2);
  assert.equal(commands(ws)[1].action.type, 'play');
  assert.ok(commands(ws)[1].action.position_secs >= 30);

  b.accept(ws);
  p.deliver();
  b.frame();
  b.frame();

  const count = commands(ws).length;
  for (let i = 0; i < 8; i++) b.frame();
  assert.equal(commands(ws).length, count, 'natural 2× advancement is never classified as a scrub');
});

test('programmatic rate changes and supported fallback never echo host rate commands', async () => {
  const b = await browser({ rates: [1, 1.5] }); const ws = await b.welcome(initial({ video_id: id, playback_rate: 2 })); const p = b.ready();
  assert.deepEqual(p.rateWrites, [1.5]);
  assert.match(b.get('room-notice').textContent, /1.5×/);
  assert.equal(commands(ws).length, 0);
  b.state(ws, initial({ revision: 1, video_id: id, playback_rate: 1 })); p.deliver(); b.frame(); b.frame();
  assert.equal(p.rate, 1);
  assert.equal(commands(ws).length, 0);
});

test('host native Play at a supported fallback rate does not accidentally change the room rate', async () => {
  const b = await browser({ rates: [1, 1.5] });
  const ws = await b.welcome(initial({ video_id: id, playback_rate: 2 })); const p = b.ready();
  p.userState(1);
  assert.equal(commands(ws).length, 1);
  assert.equal(commands(ws)[0].action.type, 'play');
  assert.equal(commands(ws).some((command) => command.action.type === 'set_rate'), false);
});

test('guest rate changes detach locally and Rejoin restores authoritative rate without commands', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id, playback_rate: 1.5 }), 'guest'); const p = b.ready();
  p.userRate(2); b.frame();
  assert.equal(b.get('join-playback').hidden, false);
  assert.equal(p.rate, 2);
  b.click('join-playback'); p.deliver(); b.frame(); b.frame();
  assert.equal(p.rate, 1.5);
  assert.equal(commands(ws).length, 0);
});

test('natural end sends no automatic pause, native replay starts host room at zero', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id, playing: true, position_secs: 99 })); const p = b.ready();
  p.time = 100; p.userState(0);
  assert.equal(commands(ws).length, 0);
  p.time = 0; p.userState(1);
  assert.deepEqual(commands(ws)[0].action, { type: 'play', position_secs: 0 });
});

test('autoplay denial shows local Join and native Play rejoins without a host/guest room command', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser(); const ws = await b.welcome(initial({ video_id: id, playing: true }), role); const p = b.ready();
    p.state = 2; p.options.events.onAutoplayBlocked();
    assert.equal(b.get('join-playback').hidden, false);
    p.userState(1); p.deliver(); b.frame(); b.frame();
    assert.equal(b.get('join-playback').hidden, true);
    assert.equal(commands(ws).length, 0);
  }
});

test('reconnect discards pending and latest native intentions, even across process/rate resets', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userState(1);
  p.time = 42;
  b.frame(300, false);
  ws.close();

  p.time = 55;
  p.userState(2);
  b.frame(300, false);

  b.click('reconnect-button');
  const next = await b.welcome(initial({ incarnation: 'b', video_id: id }));
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(next).length, 0);
  assert.equal(p.state, 2);
  assert.equal(p.time, 0);
});

test('stale_revision discards coalesced intent and reconciles only the fresh snapshot', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id }));
  const p = b.ready();

  p.userState(1);
  p.time = 42;
  b.frame(300, false);
  ws.receive({ type: 'error', code: 'stale_revision', id: commands(ws)[0].id, message: 'Room changed' });
  assert.equal(b.get('load-video').disabled, true);

  b.state(ws, initial({ revision: 3, video_id: id, position_secs: 10 }));
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 1);
  assert.equal(p.time, 10);
});

test('online during pending welcome closes old lease and ignores its delayed callbacks', async () => {
  const b = await browser();
  const old = b.sockets[0];

  old.open();
  b.windowEvents.get('online')();
  assert.equal(old.readyState, 3);
  assert.equal(b.sockets.filter((ws) => ws.readyState < 2).length, 1);
  assert.equal([...b.timers.values()].filter((timer) => timer.ms === 10000).length, 1);

  await b.welcome();
  old.events.close({ code: 1006 });
  assert.equal(b.get('connection-status').textContent, 'Connected');
});

test('hidden-tab time gaps are not scrubs; visibility sync and silent-open sockets recover', async () => {
  const b = await browser();
  const ws = await b.welcome(initial({ video_id: id, playing: true }));
  const p = b.ready();

  b.document.hidden = true;
  b.documentEvents.get('visibilitychange')();
  p.time = 20;
  b.time(20000);
  b.tick();

  b.document.hidden = false;
  b.documentEvents.get('visibilitychange')();
  p.deliver();
  b.frame();
  b.frame();
  assert.equal(commands(ws).length, 0);

  b.time(60000);
  b.tick(10000);
  assert.equal(ws.readyState, 3);
});

test('API and iframe readiness failures expose functional generation-safe local retry', async () => {
  const api = await browser({ api: false }); await api.welcome(initial({ video_id: id }));
  api.document.head.scripts[0].onerror(); await api.flush(); await api.flush();
  assert.match(api.get('room-error').textContent, /YouTube could not load/);
  api.click('join-playback'); assert.equal(api.document.head.scripts.length, 2);
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id }), 'guest'); const old = b.players[0];
  b.fireTimer(15000);
  assert.equal(old.destroyed, true);
  assert.match(b.get('room-error').textContent, /did not become ready/);
  assert.equal(b.get('join-playback').disabled, false);
  b.click('join-playback'); await b.flush();
  old.ready(); old.options.events.onError({ data: 150 }); old.emit(3);
  assert.equal(b.get('room-error').textContent, '');
  const p = b.ready(); assert.equal(p.state, 2);
  assert.equal(b.get('join-playback').hidden, true);
  assert.equal(commands(ws).length, 0);
});

test('embed errors cannot mask expiry; unavailable and invalid token stop reconnect permanently', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id })); const p = b.ready();
  p.options.events.onError({ data: 150 }); assert.match(b.get('room-error').textContent, /does not allow/);
  ws.receive({ type: 'error', code: 'unavailable', id: null });
  assert.match(b.get('room-error').textContent, /expired/);
  assert.equal(b.timers.size, 0);
  assert.equal(b.get('reconnect-button').hidden, true);
  const rejected = await browser({ token: 'bad' }); rejected.sockets[0].open();
  rejected.sockets[0].receive({ type: 'error', code: 'invalid_token', id: null });
  assert.equal(rejected.get('connection-status').textContent, 'Host access rejected');
  assert.equal(rejected.timers.size, 0);
});

test('guest joins an already-playing room before onReady using a single atomic media load', async () => {
  const videoId = 'M7lc1UVf-VE';
  const b = await browser({ unavailableInitialCaches: true });
  const ws = await b.welcome(initial({ video_id: videoId, revision: 1 }), 'guest');
  const p = b.players[0];
  b.state(ws, initial({ video_id: videoId, revision: 2, playing: true, position_secs: 30 }));
  assert.equal(p.videoId, '', 'no video metadata in the initial iframe yet');
  p.ready();
  assert.deepEqual(p.loads, [{ videoId, startSeconds: 30 }]);
  assert.equal(p.cues.length, 0);
  assert.equal(p.plays, 0, 'no playVideo races with an unconfirmed media operation');
  assert.equal(p.seeks.length, 0);
  p.deliver(); b.frame(); b.frame();
  assert.equal(p.videoId, videoId);
  assert.equal(p.state, 1);
  assert.ok(p.time >= 30);
  assert.equal(b.get('room-error').textContent, '');
  assert.equal(b.get('join-playback').hidden, true);
  assert.equal(commands(ws).length, 0);
});

test('atomic startup load retains the local autoplay watchdog and functional Join recovery', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id, playing: true, position_secs: 30 }), 'guest');
  const p = b.players[0]; p.ready();
  assert.equal(p.loads.length, 1);
  p.messages = [];
  p.videoId = id;
  p.time = 30;
  p.state = 5;

  b.fireTimer(2500);
  assert.equal(b.get('join-playback').hidden, false);
  assert.match(b.get('room-notice').textContent, /tap to start audio/);
  b.click('join-playback'); p.deliver(); b.frame(); b.frame();
  assert.equal(p.state, 1);
  assert.equal(b.get('join-playback').hidden, true);
  assert.equal(b.get('room-error').textContent, '');
  assert.equal(commands(ws).length, 0);
});

test('the asynchronous iframe model reproduces error 2 for cue then immediate empty-video Play', async () => {
  const b = await browser(); await b.welcome(initial({ video_id: id })); const p = b.players[0];
  const errors = []; p.options.events.onError = ({ data }) => errors.push(data);
  p.cueVideoById({ videoId: id, startSeconds: 30 });
  p.playVideo(); p.deliver();
  assert.deepEqual(errors, [2]);
  assert.equal(p.videoId, id, 'cue metadata arrives, but cannot retroactively fix the Play');
  assert.equal(p.state, 5);
});

test('local retry of a playing room with unloaded media uses atomic load, not cue/seek/play', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id, playing: true, position_secs: 30 }), 'guest');
  const p = b.players[0]; p.ready(); p.deliver(); b.frame(); b.frame();
  p.options.events.onError({ data: 2 });
  const loads = p.loads.length; const plays = p.plays; const seeks = p.seeks.length;
  p.videoId = ''; p.state = -1;
  b.click('join-playback');
  assert.equal(p.loads.length, loads + 1);
  assert.equal(p.plays, plays);
  assert.equal(p.seeks.length, seeks);
  assert.equal(p.cues.length, 0);
  p.deliver(); b.frame(); b.frame();
  assert.equal(b.get('room-error').textContent, '');
  assert.equal(p.state, 1);
  assert.equal(commands(ws).length, 0);
});

test('onReady with undefined video and rate caches loads successfully and applies rates once available', async () => {
  for (const playing of [false, true]) {
    const b = await browser({ unavailableInitialCaches: true });
    const ws = await b.welcome(initial({ video_id: id, playback_rate: 1.5, playing }));
    const p = b.players[0]; p.ready();
    assert.equal(b.get('room-error').textContent, '');
    assert.equal(p.videos.length, 1, 'cue must not be blocked by absent startup caches');
    assert.equal(p.rateWrites.length, 0);
    b.frame(300, false); p.deliver(); b.frame(); b.frame();
    assert.equal(p.rate, 1.5);
    assert.deepEqual(p.rateWrites, [1.5]);
    assert.equal(commands(ws).length, 0);
    assert.deepEqual(b.warnings, []);
  }
});

test('native Play BUFFERING then interval then PLAYING is not canceled in a paused room', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser(); const ws = await b.welcome(initial({ revision: 1, video_id: id }), role);
    const p = b.ready(); const pauses = p.pauses;
    p.time = 0.09; p.userState(3); b.frame(300, false);
    assert.equal(p.pauses, pauses, 'no pauseVideo before native PLAYING');
    assert.equal(commands(ws).length, 0, 'buffering alone is not an intent');
    p.userState(1);
    assert.equal(p.pauses, pauses);
    if (role === 'host') {
      assert.deepEqual(commands(ws)[0].action, { type: 'play', position_secs: 0.09 });
      assert.equal(commands(ws)[0].revision, 1);
    } else {
      assert.equal(commands(ws).length, 0);
      assert.equal(b.get('playback-status').textContent, 'YouTube · watching locally');
      assert.equal(b.get('join-playback').hidden, false);
    }
  }
});

test('a fresh authoritative pause revision still stops a buffering transition', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id })); const p = b.ready();
  p.userState(3); const pauses = p.pauses;
  b.state(ws, initial({ revision: 1, video_id: id }));
  assert.equal(p.pauses, pauses + 1);
  p.deliver(); b.frame(); b.frame();
  assert.equal(commands(ws).length, 0);
});

test('native initial Play before CUED or any cue sample survives the unconfirmed paused operation', async () => {
  for (const role of ['host', 'guest']) {
    const b = await browser(); const ws = await b.welcome(initial({ video_id: id }), role);
    const p = b.players[0]; p.ready();
    p.messages = [];
    p.videoId = id;
    const pauses = p.pauses;
    p.userState(3); b.frame(300, false);
    p.userState(1, true); b.frame(300, false);
    assert.equal(p.pauses, pauses, 'unconfirmed cue cannot roll back native Play');
    if (role === 'host') assert.equal(commands(ws)[0].action.type, 'play');
    else {
      assert.equal(commands(ws).length, 0);
      assert.equal(b.get('join-playback').hidden, false);
    }
  }
});

test('initial Play callback with unavailable video cache is retained by later stable getter samples', async () => {
  const b = await browser({ unavailableInitialCaches: true }); const ws = await b.welcome(initial({ video_id: id }));
  const p = b.players[0]; p.ready(); p.messages = [];
  p.userState(1);
  assert.equal(commands(ws).length, 0);
  p.videoId = id; b.frame(300); b.frame(300);
  assert.equal(commands(ws)[0].action.type, 'play');
});

test('short late old PLAYING callback during an unconfirmed cue never echoes', async () => {
  const b = await browser(); const ws = await b.welcome(initial({ video_id: id })); const p = b.players[0];
  p.ready(); p.messages = []; p.videoId = id;
  p.userState(1); b.frame(100, false); p.userState(5); p.userState(2);
  b.frame(); b.frame();
  assert.equal(commands(ws).length, 0);
  assert.equal(p.state, 2);
});

test('synchronization warnings include the caught cause but redact host secrets', async () => {
  const b = await browser({ token: 'secret' }); await b.welcome(initial({ video_id: id })); const p = b.ready();
  p.getDuration = () => { throw Error('broken duration secret #host=another-secret'); };
  b.frame();
  assert.equal(b.warnings.length, 1);
  assert.match(b.warnings[0][1], /broken duration/);
  assert.equal(JSON.stringify(b.warnings).includes('secret'), false);
  assert.match(b.get('room-error').textContent, /could not synchronize/);
});

test('copy feedback only writes icons, title and live-region; rapid clicks restart its timer', async () => {
  const b = await browser(); await b.welcome();
  const allowed = new Set(['copy-icon', 'copy-success', 'copy-link', 'copy-feedback']);
  b.writes.length = 0;
  await b.click('copy-link');
  assert.equal(b.get('copy-icon').hidden, true);
  assert.equal(b.get('copy-success').hidden, false);
  assert.equal(b.get('copy-link').title, 'Copied');
  assert.equal(b.get('copy-feedback').textContent, 'Room link copied');
  assert.equal(b.get('room-notice').textContent, '');
  const firstTimer = [...b.timers.keys()][0];
  await b.click('copy-link');
  assert.equal(b.timers.has(firstTimer), false);
  assert.equal([...b.timers.values()].filter((timer) => timer.ms === 1800).length, 1);
  b.fireTimer(1800);
  assert.equal(b.get('copy-icon').hidden, false);
  assert.equal(b.get('copy-success').hidden, true);
  assert.equal(b.get('copy-link').title, 'Copy room link');
  assert.equal(b.get('copy-feedback').textContent, '');
  assert.ok(b.writes.every(([name]) => allowed.has(name)), JSON.stringify(b.writes));
});

test('failed copy has accessible inline feedback and never renders a room banner', async () => {
  const b = await browser({ clipboardBlocked: true }); await b.welcome(); b.writes.length = 0;
  await b.click('copy-link');
  assert.equal(b.get('copy-link').title, 'Copy URL from address bar');
  assert.equal(b.get('copy-feedback').textContent, 'Copy URL from address bar');
  assert.ok(b.writes.every(([name]) => name.startsWith('copy-')));
});

test('module import failure remains visible and strips credentials first', async () => {
  const b = await browser({ token: 'secret', importFailure: true });
  assert.equal(b.sockets.length, 0);
  assert.equal(b.historyCalls.length, 1);
  assert.equal(b.get('room-error').hidden, false);
  assert.equal(b.get('load-video').disabled, true);
});
