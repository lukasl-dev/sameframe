import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../assets/players.js', import.meta.url), 'utf8');
const { directPlayer, PlayerState } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('native media waits for metadata, applies the latest load intent, reports rates and cancels stale play failures', async () => {
  const listeners = new Map();
  const received = [];
  let readyCount = 0;
  let blocked = 0;
  let failPlay;
  const video = {
    readyState: 0, currentSrc: '', duration: NaN, currentTime: 0, paused: true, ended: false, playbackRate: 1,
    addEventListener(name, listener) { listeners.set(name, listener); },
    removeEventListener(name) { listeners.delete(name); },
    setAttribute() {}, removeAttribute() {}, remove() { this.removed = true; }, load() {},
    get src() { return this.url; },
    set src(url) { this.url = url; },
    play() { this.paused = false; return new Promise((resolve, reject) => { failPlay = reject; }); },
    pause() { this.paused = true; },
  };
  const mount = { className: 'absolute inset-0 size-full', ownerDocument: { createElement: () => video }, replaceWith(node) { assert.equal(node, video); } };
  const first = { type: 'direct', url: 'https://example.org/a.mp4' };
  const player = directPlayer(mount, first, {
    onReady() { readyCount++; },
    onStateChange(event) { received.push(event.data); },
    onPlaybackRateChange(event) { received.push(event.data); },
    onAutoplayBlocked() { blocked++; }, onError() { assert.fail('unexpected media error'); },
  });
  assert.equal(video.controls, true);
  assert.equal(video.playsInline, true);
  assert.equal(player.getSourceKey(), '');
  player.load(first, 20, true);
  player.seekTo(9);
  player.pauseVideo();
  assert.equal(player.getPlayerState(), PlayerState.BUFFERING);
  Object.assign(video, { readyState: 1, currentSrc: first.url, duration: 30 });
  listeners.get('loadedmetadata')();
  assert.equal(readyCount, 1);
  assert.equal(video.currentTime, 9);
  assert.equal(video.paused, true);
  assert.equal(player.getSourceKey(), first.url);
  assert.deepEqual(player.getAvailablePlaybackRates(1.3), [1.3]);
  player.setPlaybackRate(1.3);
  listeners.get('ratechange')();
  assert.equal(received.at(-1), 1.3);

  player.playVideo();
  player.pauseVideo();
  failPlay(Object.assign(new Error('superseded play'), { name: 'NotAllowedError' }));
  await Promise.resolve();
  assert.equal(blocked, 0);
  player.playVideo();
  failPlay(Object.assign(new Error('gesture required'), { name: 'NotAllowedError' }));
  await Promise.resolve();
  assert.equal(blocked, 1);

  const second = { type: 'direct', url: 'https://example.org/b.webm' };
  player.load(second, 50, true);
  assert.equal(player.getSourceKey(), '');
  assert.equal(player.getPlayerState(), PlayerState.BUFFERING);
  assert.equal(Number.isNaN(player.getDuration()), true);
  assert.equal(video.currentTime, 9, 'old readyState and currentSrc cannot apply a new load before resource selection');
  listeners.get('loadedmetadata')();
  assert.equal(video.currentTime, 9, 'queued metadata for the previous URL is ignored');
  player.setPlaybackRate(2);
  assert.equal(video.playbackRate, 1.3, 'queued rate changes cannot touch the previous resource');
  assert.equal(player.getPlaybackRate(), 2);
  Object.assign(video, { readyState: 1, currentSrc: second.url, duration: 25 });
  listeners.get('loadedmetadata')();
  assert.equal(video.currentTime, 25);
  assert.equal(video.playbackRate, 2);
  assert.equal(video.paused, false);
  assert.equal(readyCount, 1);
  player.destroy();
  failPlay(Object.assign(new Error('destroyed'), { name: 'NotAllowedError' }));
  await Promise.resolve();
  assert.equal(blocked, 1);
  assert.equal(listeners.size, 0);
  assert.equal(video.removed, true);
});
