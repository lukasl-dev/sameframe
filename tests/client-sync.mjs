import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../assets/sync.js', import.meta.url), 'utf8');
const {
  parseVideoId, parseRoomLink, validSnapshot, SnapshotOrder, ClockFilter,
  targetPosition, shouldSeek, reconnectDelay, visibleProblem, playbackUpdate, NativePlaybackObserver, supportedRate, avatarPattern,
} = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

const state = (overrides = {}) => {
  const snapshot = { incarnation: 'process-a', room_name: 'The Sleepy Observatory', events: [], revision: 3, media_revision: 0, playback_rate: 1, video_id: 'dQw4w9WgXcQ',
    playing: true, position_secs: 20, anchor_ms: 10000, members: 2, ...overrides };
  snapshot.participants ??= Array.from({ length: Math.max(0, Math.min(32, snapshot.members)) }, (_, index) => ({
    id: index + 1, name: `Sleepy Goose ${index}`, role: index ? 'guest' : 'host', avatar: (index + 1).toString(16).padStart(16, '0'),
  }));
  return snapshot;
};

test('avatars are deterministic, symmetric, non-empty, and use the Mocha palette', () => {
  const seed = '89abcdef01234567';
  const avatar = avatarPattern(seed);
  assert.deepEqual(avatarPattern(seed), avatar);
  assert.match(avatar.color, /^#[0-9a-f]{6}$/);
  assert.ok(avatar.cells.length > 0);

  for (const [column, row] of avatar.cells) {
    assert.ok(avatar.cells.some(([otherColumn, otherRow]) => otherColumn === 4 - column && otherRow === row));
  }

  assert.ok(avatarPattern('0000000000000000').cells.length > 0);
  assert.notDeepEqual(avatarPattern('ffffffffffffffff'), avatar);
});

test('participant snapshots reject duplicate identities, malformed avatars and inconsistent counts', () => {
  assert.equal(validSnapshot(state()), true);

  for (const participants of [undefined, [], [{ id: 1, role: 'guest', avatar: 'bad' }],
    [{ id: 1, role: 'admin', avatar: '0123456789abcdef' }],
    [{ id: 1, role: 'guest', avatar: '0123456789abcdef' }, { id: 1, role: 'host', avatar: '0123456789abcdef' }]]) {
    const snapshot = state();
    snapshot.participants = participants;
    assert.equal(validSnapshot(snapshot), false);
  }

  assert.equal(validSnapshot(state({ members: 33 })), false);
});

test('room names, participant names, moderator roles and retained event payloads validate before rendering', () => {
  const entry = { id: 1, at_ms: 1000, member_id: 2, name: 'Cosmic Goose', avatar: '0123456789abcdef',
    kind: 'proposal', text: null, video_id: 'dQw4w9WgXcQ', position_secs: null };
  const snapshot = state({ events: [entry] });
  snapshot.participants[1].role = 'moderator';
  assert.equal(validSnapshot(snapshot), true);

  for (const overrides of [{ room_name: '' }, { events: undefined }, { events: [entry, entry] },
    { events: [{ ...entry, video_id: 'javascript:alert(1)' }] }, { events: [{ ...entry, kind: 'unknown' }] },
    { events: [{ ...entry, at_ms: NaN }] }, { events: [{ ...entry, kind: 'message', text: null }] },
    { events: [{ ...entry, name: '' }] }, { events: Array(101).fill(entry) }]) {
    assert.equal(validSnapshot({ ...snapshot, ...overrides }), false);
  }

  snapshot.participants[1].name = '';
  assert.equal(validSnapshot(snapshot), false);
});

test('video URLs accept only supported YouTube hosts and paths, or a raw ID', () => {
  for (const value of [
    'dQw4w9WgXcQ', ' https://youtu.be/dQw4w9WgXcQ?t=2 ',
    'https://youtube.com/watch?v=dQw4w9WgXcQ&list=no-playlists',
    'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
    'https://m.youtube.com/shorts/dQw4w9WgXcQ',
    'https://music.youtube.com/watch?v=dQw4w9WgXcQ',
    'https://www.youtube.com/embed/dQw4w9WgXcQ',
  ]) assert.equal(parseVideoId(value), 'dQw4w9WgXcQ', value);
});

test('invalid, spoofed and arbitrary embed URLs never become video IDs', () => {
  for (const value of [null, '', 'short', 'dQw4w9WgXcQx',
    'https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ',
    'https://evil.test/embed/dQw4w9WgXcQ',
    'https://youtube-nocookie.com/embed/dQw4w9WgXcQ',
    'https://www.youtu.be/dQw4w9WgXcQ',
    'https://youtube.com@evil.test/watch?v=dQw4w9WgXcQ',
    'https://user@youtube.com/watch?v=dQw4w9WgXcQ',
    'https://youtube.com:123/watch?v=dQw4w9WgXcQ',
    'javascript:alert(1)', 'ftp://youtube.com/watch?v=dQw4w9WgXcQ',
    'https://youtube.com/watch?v=bad', 'https://youtube.com/watch?v=',
    'https://youtube.com/playlist?list=dQw4w9WgXcQ',
    'https://youtube.com/not-embed/dQw4w9WgXcQ',
    'https://youtu.be/dQw4w9WgXcQ/extra',
    'https://youtube.com/embed/dQw4w9WgXcQ/extra',
    '<iframe src="https://youtube.com/embed/dQw4w9WgXcQ"></iframe>',
  ]) assert.equal(parseVideoId(value), null, String(value));
});

test('room joining stays same-origin and discards secrets and queries', () => {
  const origin = 'https://sameframe.test';
  assert.equal(parseRoomLink('https://sameframe.test/room/abc#host=secret', origin), '/room/abc');
  assert.equal(parseRoomLink('/room/abc?host=secret', origin), '/room/abc');
  assert.equal(parseRoomLink('abc_123', origin), '/room/abc_123');
  for (const url of ['', 'https://evil.test/room/abc', '//evil.test/room/abc', '/room/', '/room/a/b',
    'javascript:alert(1)', 'https://user@sameframe.test/room/abc']) assert.equal(parseRoomLink(url, origin), null);
});

test('snapshot boundary validates all fields before affecting state', () => {
  assert.equal(validSnapshot(state()), true);
  assert.equal(validSnapshot(state({ video_id: null, playing: false })), true);
  for (const invalid of [null, {}, state({ revision: -1 }), state({ revision: 1.5 }),
    state({ revision: Number.MAX_SAFE_INTEGER + 1 }), state({ incarnation: '' }),
    state({ media_revision: -1 }), state({ media_revision: 4 }), state({ media_revision: undefined }),
    state({ playback_rate: 0 }), state({ playback_rate: NaN }), state({ playback_rate: 4.1 }),
    state({ video_id: 'bad' }), state({ playing: 1 }), state({ position_secs: NaN }),
    state({ position_secs: -1 }), state({ anchor_ms: Infinity }), state({ members: -1 })]) {
    assert.equal(validSnapshot(invalid), false);
  }
});

test('snapshot ordering rejects rollback but permits membership-only updates', () => {
  const order = new SnapshotOrder();
  assert.equal(order.accept(state(), 11000, true), true);
  assert.equal(order.accept(state({ members: 3 }), 11500), true);
  assert.equal(order.snapshot.members, 3);

  assert.equal(order.accept(state({ revision: 2 }), 12000), false);
  assert.equal(order.accept(state({ members: 1 }), 10000), false);
  assert.equal(order.accept(state({ revision: 4 }), 12000), true);
  assert.equal(order.accept(state({ revision: 5, incarnation: 'process-b' }), 13000), false);
  assert.equal(order.snapshot.revision, 4);
});

test('fresh welcome accepts a restarted process, revision and clock reset', () => {
  const order = new SnapshotOrder();
  order.accept(state({ revision: 100 }), 50000, true);

  const restarted = state({ incarnation: 'process-b', revision: 0, anchor_ms: 0 });
  assert.equal(order.accept(restarted, 100, true), true);
  assert.equal(order.accept(state({ revision: 101 }), 51000), false);

  order.reset();
  assert.equal(order.snapshot, null);
});

test('invalid welcome cannot erase current ordering', () => {
  const order = new SnapshotOrder();
  order.accept(state(), 11000, true);
  assert.equal(order.accept({}, 100, true), false);
  assert.equal(order.snapshot.revision, 3);
});

test('clock uses the best recent RTT sample, not wall-clock time', () => {
  const clock = new ClockFilter();
  clock.seed(10000, 1000);
  assert.equal(clock.serverNow(1100), 10100);

  clock.sample(1000, 1020, 11010);
  assert.equal(clock.serverNow(1100), 11100);

  clock.sample(1100, 1900, 11800);
  assert.equal(clock.serverNow(2000), 12000);

  clock.sample(2100, 2110, 12105);
  assert.equal(clock.serverNow(2200), 12200);
});

test('old clock samples eventually expire and reconnect resets mapping', () => {
  const clock = new ClockFilter();
  clock.sample(0, 2, 1001);
  for (let i = 0; i < 12; i++) clock.sample(100 + i * 100, 120 + i * 100, 2110 + i * 100);
  assert.equal(clock.offset, 2000);

  clock.reset();
  assert.equal(clock.offset, null);
  clock.seed(5, 1000);
  assert.equal(clock.serverNow(1010), 15);
});

test('malformed and suspended-tab RTT samples do not move the clock', () => {
  const clock = new ClockFilter();
  clock.sample(1, 3, 102);
  for (const sample of [[10, 9, 100], [0, 20000, 100], [NaN, 20, 100], [1, 20, Infinity]]) {
    assert.equal(clock.sample(...sample), false);
    assert.equal(clock.offset, 100);
  }
});

test('playing position follows authoritative anchor despite a buffering player', () => {
  assert.equal(targetPosition(state(), 15000), 25);
  assert.equal(targetPosition(state(), 5000), 20);
  assert.equal(targetPosition(state(), 15000, 23), 23);
  assert.equal(targetPosition(state(), 15000, 0), 25);
});

test('paused state never advances and needs position correction too', () => {
  const paused = state({ playing: false, position_secs: 42 });
  assert.equal(targetPosition(paused, 100000), 42);
  assert.equal(shouldSeek(10, targetPosition(paused, 100000), 10000, 0), true);
});

test('drift policy requires meaningful error and respects three-second cooldown', () => {
  assert.equal(shouldSeek(20, 21.1, 5000, 0), false);
  assert.equal(shouldSeek(20, 22, 2999, 0), false);
  assert.equal(shouldSeek(20, 22, 3000, 0), true);
  assert.equal(shouldSeek(25, 20, 5000, 0), true);
  assert.equal(shouldSeek(20, 22, 1, 0, true), true);
  assert.equal(shouldSeek(NaN, 22, 5000, 0), false);
});

test('reconnect grows exponentially with jitter but never exceeds cap', () => {
  assert.equal(reconnectDelay(0, () => 0), 375);
  assert.equal(reconnectDelay(1, () => 0.5), 1000);
  assert.equal(reconnectDelay(2, () => 1), 2500);
  assert.equal(reconnectDelay(100, () => 1), 30000);
  assert.equal(reconnectDelay(100, () => 0), 22500);
});

test('room errors take precedence over local embed errors', () => {
  assert.equal(visibleProblem('Room expired', 'Embed blocked'), 'Room expired');
  assert.equal(visibleProblem('', 'Embed blocked'), 'Embed blocked');
  assert.equal(visibleProblem('', ''), '');
});

test('authoritative revision changes bypass drift cooldown, even for small explicit seeks', () => {
  const previous = state();
  const next = state({ revision: 4, position_secs: 21 });
  const update = playbackUpdate(previous, next);
  assert.equal(update.forceSeek, true);
  assert.equal(shouldSeek(20, 21, 100, 0, update.forceSeek), true);
  assert.equal(playbackUpdate(previous, state({ members: 3 })).forceSeek, false);
});

test('unchanged buffering does not hard-seek; explicit host revisions still apply', () => {
  assert.equal(shouldSeek(0, 20, 10000, 0, false, true), false);
  assert.equal(shouldSeek(0, 20, 100, 0, true, true), true);
  assert.equal(shouldSeek(0, 20, 100, 0, true, false), true);
  assert.equal(shouldSeek(0, 20, 100, 0, false, false, true), true, 'recovery bypasses cooldown');
  assert.equal(shouldSeek(0, 0.3, 100, 0, false, false, true), false, 'recovery still needs meaningful drift');
});

test('playing position uses authoritative playback rate; paused anchors remain fixed', () => {
  assert.equal(targetPosition(state({ playback_rate: 2 }), 15000), 30);
  assert.equal(targetPosition(state({ playback_rate: 0.5 }), 15000), 22.5);
  assert.equal(targetPosition(state({ playback_rate: 4, playing: false }), 15000), 20);
});

test('rate fallback chooses a supported nearest value, never an arbitrary rate', () => {
  assert.equal(supportedRate(2, [1, 1.5]), 1.5);
  assert.equal(supportedRate(1.5, [0.5, 1, 1.5, 2]), 1.5);
  assert.equal(supportedRate(2, []), null);
  for (const unavailable of [undefined, null, {}, 1]) assert.equal(supportedRate(2, unavailable), null);
});

test('observer settles asynchronous programmatic cue/seek/play without native echoes', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(true, 30, 0);
  assert.equal(observer.observe(0, 2, 0), null, 'stale paused cache');
  assert.equal(observer.observe(30, 3, 100, true), null, 'seek buffering');
  assert.equal(observer.observe(30, 1, 200, true), null);
  assert.equal(observer.observe(30.3, 1, 500), null);
  assert.equal(observer.settling, false);

  const intent = observer.observe(50, 1, 800);
  assert.equal(intent.kind, 'seek');
  assert.equal(intent.position, 50);
});

test('observer keeps contrary native input during the short operation drain window', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(false, 0, 0);
  observer.observe(0, 2, 100, true);
  assert.equal(observer.observe(0, 1, 200, true), null);
  assert.equal(observer.observe(0.2, 1, 400).playing, true, 'quick user play is not lost');
});

test('authoritative recovery pause drains pre-existing Play until the corrective operation settles', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(false, 60, 0, 1, false);

  assert.equal(observer.observe(0, 1, 0), null);
  assert.equal(observer.observe(0.3, 1, 300), null);
  assert.equal(observer.observe(0.6, 1, 600), null);
  assert.equal(observer.settling, true);

  assert.equal(observer.observe(60, 3, 900, true), null);
  assert.equal(observer.observe(60, 2, 1000, true), null);
  assert.equal(observer.observe(60, 2, 1300), null);
  assert.equal(observer.settling, false);

  assert.equal(observer.observe(60, 1, 1400, true).playing, true);
});

test('unconfirmed paused cue admits a stable opposite Play without waiting four seconds', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(false, 0, 0);
  assert.equal(observer.observe(0.09, 3, 100, true), null);
  assert.equal(observer.observe(0.09, 1, 200, true), null);
  assert.equal(observer.observe(0.1, 2, 500).playing, true, 'callback wins over stale paused cache');
  assert.equal(observer.settling, false);
});

test('native Play just before an unconfirmed cue expires is not reset into a false baseline', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(false, 0, 0);
  assert.equal(observer.observe(0, 1, 3900, true), null);
  assert.equal(observer.observe(0.1, 1, 4000), null);
  assert.equal(observer.observe(0.3, 1, 4200).playing, true);
});

test('short contrary callbacks before an unconfirmed cue are drained, not native input', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(false, 0, 0);
  assert.equal(observer.observe(0, -1, 100), null);
  assert.equal(observer.observe(0, -1, 400), null);
  assert.equal(observer.settling, true, 'UNSTARTED is not confirmation of a paused cue');
  assert.equal(observer.observe(0, 1, 500, true), null);
  assert.equal(observer.observe(0, 5, 600, true), null);
  assert.equal(observer.observe(0, 2, 900), null);
  assert.equal(observer.playing, false);
});

test('state callbacks override stale getter caches; steady native pause is debounced', () => {
  const observer = new NativePlaybackObserver();
  observer.observe(10, 1, 0);
  assert.equal(observer.observe(10, 2, 100, true), null);
  assert.equal(observer.waiting, true);

  const pause = observer.observe(10, 1, 400);
  assert.equal(pause.playing, false);
  assert.equal(pause.position, 10);
});

test('buffering uses frozen LOCAL samples, not the room clock or wall elapsed', () => {
  const observer = new NativePlaybackObserver();
  observer.observe(10, 1, 0);
  assert.equal(observer.observe(10.1, 3, 100, true), null);
  for (let at = 400; at <= 10000; at += 300) assert.equal(observer.observe(10.1, 3, at), null);
  assert.equal(observer.observe(10.2, 1, 10300, true), null);
  assert.equal(observer.observe(20, 3, 10600, true).playing, true, 'native scrub during buffering preserves playing');
});

test('short paused buffer transitions do not issue pause; suspended sample gaps are not seeks', () => {
  const observer = new NativePlaybackObserver();
  observer.observe(10, 1, 0);
  observer.observe(10.1, 3, 100, true);
  observer.observe(10.1, 2, 200, true);
  assert.equal(observer.observe(10.1, 1, 300, true), null);
  assert.equal(observer.observe(30, 1, 20000), null);
});

test('small paused native seeks are unambiguous and do not require playing-drift tolerance', () => {
  const observer = new NativePlaybackObserver();
  observer.observe(10, 2, 0);
  assert.equal(observer.observe(10.5, 2, 300).kind, 'seek');
});

test('natural playback at actual local rate never looks like a seek', () => {
  const observer = new NativePlaybackObserver();
  observer.observe(0, 1, 0, false, 4);
  for (let at = 300; at <= 3000; at += 300) assert.equal(observer.observe(at / 1000 * 4, 1, at, false, 4), null);
});

test('programmatic operation expiry resets baseline without inventing a pause', () => {
  const observer = new NativePlaybackObserver();
  observer.suppress(true, 30, 0);
  assert.equal(observer.observe(0, 2, 4000), null);
  assert.equal(observer.settling, false);
  assert.equal(observer.observe(0, 2, 4300), null);
});

test('only a changed media revision resets the same ID, not native pause or seek at zero', () => {
  const previous = state({ playing: false, position_secs: 0 });
  assert.equal(playbackUpdate(previous, state({ revision: 4, media_revision: 4, playing: false, position_secs: 0 })).resetVideo, true);
  assert.equal(playbackUpdate(previous, state({ revision: 4, playing: false, position_secs: 0 })).resetVideo, false);
  assert.equal(playbackUpdate(previous, state({ playing: false, position_secs: 0, members: 3 })).resetVideo, false);
  assert.equal(playbackUpdate(previous, state({ video_id: 'abcdefghijk' })).resetVideo, true);
});
