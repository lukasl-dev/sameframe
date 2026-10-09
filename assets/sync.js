export const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

export function parseVideoId(input) {
  const text = String(input ?? '').trim();
  if (VIDEO_ID.test(text)) return text;

  let url;
  try { url = new URL(text); } catch { return null; }
  if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.port) return null;
  let id;
  if (url.hostname === 'youtu.be') {
    const parts = url.pathname.split('/').filter(Boolean);
    if (parts.length !== 1) return null;
    [id] = parts;
  } else if (['youtube.com', 'www.youtube.com', 'm.youtube.com', 'music.youtube.com'].includes(url.hostname)) {
    const parts = url.pathname.split('/').filter(Boolean);
    if (url.pathname === '/watch') id = url.searchParams.get('v');
    else if (parts.length === 2 && ['shorts', 'embed'].includes(parts[0])) id = parts[1];
  }
  return VIDEO_ID.test(id ?? '') ? id : null;
}

export function parseRoomLink(input, origin) {
  const text = String(input ?? '').trim();
  if (/^[A-Za-z0-9_-]+$/.test(text)) return `/room/${text}`;
  try {
    const url = new URL(text, origin);
    if (url.origin !== origin || url.username || url.password) return null;
    const match = /^\/room\/([A-Za-z0-9_-]+)\/?$/.exec(url.pathname);
    return match ? `/room/${match[1]}` : null;
  } catch { return null; }
}

export function validSnapshot(s) {
  return !!s && typeof s.incarnation === 'string' && s.incarnation.length > 0
    && typeof s.room_name === 'string' && s.room_name.length > 0 && s.room_name.length <= 100
    && Number.isSafeInteger(s.revision) && s.revision >= 0
    && Number.isSafeInteger(s.media_revision) && s.media_revision >= 0 && s.media_revision <= s.revision
    && (s.video_id === null || (typeof s.video_id === 'string' && VIDEO_ID.test(s.video_id)))
    && Number.isFinite(s.playback_rate) && s.playback_rate >= 0.25 && s.playback_rate <= 4
    && typeof s.playing === 'boolean' && Number.isFinite(s.position_secs) && s.position_secs >= 0
    && Number.isFinite(s.anchor_ms) && s.anchor_ms >= 0
    && Number.isSafeInteger(s.members) && s.members >= 0 && s.members <= 32
    && Array.isArray(s.participants) && s.participants.length === s.members
    && s.participants.every((member) => member && Number.isSafeInteger(member.id) && member.id > 0
      && ['host', 'moderator', 'guest'].includes(member.role) && typeof member.name === 'string'
      && member.name.length > 0 && member.name.length <= 80 && typeof member.avatar === 'string'
      && /^[0-9a-f]{16}$/.test(member.avatar))
    && new Set(s.participants.map((member) => member.id)).size === s.members
    && Array.isArray(s.events) && s.events.length <= 100
    && s.events.every((entry, index) => entry && Number.isSafeInteger(entry.id) && entry.id > 0
      && (index === 0 || entry.id > s.events[index - 1].id)
      && Number.isFinite(entry.at_ms) && entry.at_ms >= 0
      && Number.isSafeInteger(entry.member_id) && entry.member_id > 0
      && typeof entry.name === 'string' && entry.name.length > 0 && entry.name.length <= 80
      && typeof entry.avatar === 'string' && /^[0-9a-f]{16}$/.test(entry.avatar)
      && ['joined', 'left', 'message', 'proposal', 'set_video', 'play', 'pause', 'seek', 'set_rate', 'role_changed'].includes(entry.kind)
      && (entry.text === null || (typeof entry.text === 'string' && entry.text.length <= 1000))
      && (entry.video_id === null || (typeof entry.video_id === 'string' && VIDEO_ID.test(entry.video_id)))
      && (entry.position_secs === null || (Number.isFinite(entry.position_secs) && entry.position_secs >= 0))
      && (entry.kind !== 'message' || (typeof entry.text === 'string' && entry.text.trim().length > 0))
      && (!['proposal', 'set_video'].includes(entry.kind) || VIDEO_ID.test(entry.video_id ?? '')));
}

export function avatarPattern(seed) {
  const colors = ['#cba6f7', '#89b4fa', '#a6e3a1', '#f5c2e7', '#fab387', '#94e2d5'];
  const bits = parseInt(seed.slice(2, 6), 16) | (1 << 8);
  const cells = [];
  for (let row = 0; row < 5; row++) {
    for (let column = 0; column < 5; column++) {
      if (bits & (1 << (row * 3 + Math.min(column, 4 - column)))) cells.push([column, row]);
    }
  }
  return { color: colors[parseInt(seed.slice(0, 2), 16) % colors.length], cells };
}

export class SnapshotOrder {
  constructor() { this.reset(); }

  reset() {
    this.snapshot = null;
    this.serverMs = -Infinity;
  }

  accept(snapshot, serverMs, welcome = false) {
    if (!validSnapshot(snapshot) || !Number.isFinite(serverMs) || serverMs < 0) return false;
    const previous = welcome ? null : this.snapshot;
    if (previous && (snapshot.incarnation !== previous.incarnation
      || snapshot.revision < previous.revision
      || (snapshot.revision === previous.revision && serverMs < this.serverMs))) return false;

    this.snapshot = snapshot;
    this.serverMs = serverMs;
    return true;
  }
}

export class ClockFilter {
  constructor() { this.reset(); }

  reset() {
    this.samples = [];
    this.offset = null;
  }

  seed(serverMs, receivedMs) {
    if (this.offset === null && Number.isFinite(serverMs) && Number.isFinite(receivedMs)) {
      this.offset = serverMs - receivedMs;
    }
  }

  sample(sentMs, receivedMs, serverMs) {
    const rtt = receivedMs - sentMs;
    if (![sentMs, receivedMs, serverMs].every(Number.isFinite) || rtt < 0 || rtt > 10000) return false;
    this.samples.push({ rtt, offset: serverMs - (sentMs + receivedMs) / 2 });
    if (this.samples.length > 12) this.samples.shift();
    this.offset = this.samples.reduce((best, sample) => sample.rtt <= best.rtt ? sample : best).offset;
    return true;
  }

  serverNow(localMs) { return localMs + (this.offset ?? 0); }
}

export function targetPosition(snapshot, serverMs, duration = Infinity) {
  let position = snapshot.position_secs;
  if (snapshot.playing) position += Math.max(0, serverMs - snapshot.anchor_ms) / 1000 * snapshot.playback_rate;
  const end = Number.isFinite(duration) && duration > 0 ? duration : Infinity;
  return Math.min(end, Math.max(0, position));
}

export function playbackUpdate(previous, next) {
  const newRevision = !previous || previous.incarnation !== next.incarnation || previous.revision !== next.revision;
  const newVideo = !previous || previous.incarnation !== next.incarnation || previous.video_id !== next.video_id;
  const resetVideo = newVideo || previous.media_revision !== next.media_revision;
  return { forceSeek: newRevision || newVideo, resetVideo };
}

export function shouldSeek(current, target, nowMs, lastSeekMs, force = false, buffering = false, recovery = false) {
  if (![current, target, nowMs].every(Number.isFinite) || (buffering && !force)) return false;
  const threshold = force ? 0.05 : 1.2;
  return (force || recovery || nowMs - lastSeekMs >= 3000) && Math.abs(current - target) > threshold;
}

export class NativePlaybackObserver {
  constructor() { this.reset(); }

  reset() {
    this.last = null;
    this.playing = null;
    this.pauseCandidate = null;
    this.operation = null;
    this.eventState = null;
    this.state = null;
  }

  suppress(playing, position, at, rate = 1) {
    this.last = null;
    this.playing = playing;
    this.pauseCandidate = null;
    this.operation = { playing, position, rate, at, until: at + 4000, settledAt: null, nativePlayAt: null };
  }

  get settling() { return this.operation !== null; }
  get waiting() { return this.pauseCandidate !== null; }

  observe(position, state, at, event = false, rate = 1) {
    if (![position, at].every(Number.isFinite) || position < 0) return null;

    if (event) this.eventState = { state, at };
    else if (this.eventState) {
      if (state === this.eventState.state || at - this.eventState.at >= 750) this.eventState = null;
      else state = this.eventState.state;
    }
    this.state = state;
    const current = { position, state, at, rate };

    if (this.operation) {
      const op = this.operation;
      const expectedState = op.playing ? state === 1 : [2, 5].includes(state);
      const expectedPosition = op.position === null || (position >= op.position - 0.75
        && position <= op.position + 0.75 + (state === 1 ? Math.max(0, at - op.at) / 1000 * op.rate : 0));

      if (!op.playing && state === 1) {
        op.nativePlayAt ??= at;
        if (at - op.nativePlayAt >= 250) {
          this.operation = null;
          this.last = current;
          this.playing = true;
          return { playing: true, position, at, kind: 'state' };
        }
        if (op.settledAt === null) return null;
      } else op.nativePlayAt = null;

      if (expectedState && expectedPosition) {
        op.settledAt ??= at;
        this.last = current;
      }
      if (op.settledAt !== null && at - op.settledAt >= 250) {
        this.operation = null;
      } else if (at >= op.until) {
        this.operation = null;
        this.last = current;
        this.playing = state === 1 ? true : state === 2 ? false : this.playing;
        return null;
      } else return null;
    }

    const previous = this.last;
    this.last = current;
    const elapsed = previous ? at - previous.at : Infinity;
    const advance = previous?.state === 1 && state === 1 ? elapsed / 1000 * previous.rate : 0;
    const seekThreshold = previous?.state === 2 && state === 2 ? 0.15 : 1.25;
    const seek = previous && elapsed >= 0 && elapsed <= 1500
      && Math.abs(position - previous.position - advance) > seekThreshold;

    if (state === 0) {
      this.playing = false;
      this.pauseCandidate = null;
      return null;
    }
    if (state === 1) {
      const changed = this.playing === false;
      this.playing = true;
      this.pauseCandidate = null;
      if (seek || changed) return { playing: true, position, at, kind: seek ? 'seek' : 'state' };
    } else if (state === 2) {
      if (seek) {
        this.playing = false;
        this.pauseCandidate = null;
        return { playing: false, position, at, kind: 'seek' };
      }
      if (this.playing === true) {
        this.pauseCandidate ??= { at };
        if (at - this.pauseCandidate.at >= 300) {
          this.playing = false;
          this.pauseCandidate = null;
          return { playing: false, position, at, kind: 'state' };
        }
      } else this.playing ??= false;
    } else {
      this.pauseCandidate = null;
      if (seek && this.playing !== null) return { playing: this.playing, position, at, kind: 'seek' };
    }
    return null;
  }
}

export function supportedRate(requested, available) {
  if (!Array.isArray(available)) return null;
  const rates = available.filter((rate) => Number.isFinite(rate) && rate >= 0.25 && rate <= 4);
  if (!rates.length) return null;
  return rates.reduce((best, rate) => Math.abs(rate - requested) < Math.abs(best - requested) ? rate : best);
}

export function reconnectDelay(attempt, random = Math.random) {
  const base = Math.min(30000, 500 * 2 ** Math.min(Math.max(0, attempt), 16));
  return Math.min(30000, Math.round(base * (0.75 + random() * 0.5)));
}

export function visibleProblem(roomError, playerError) {
  return roomError || playerError || '';
}
