const element = (id) => document.getElementById(id);
const text = (id, value) => {
  const node = element(id);
  if (!node) return;
  node.textContent = value;
  if (['room-error', 'room-notice', 'chat-error'].includes(id)) node.hidden = !value;
};
const roomId = document.body.dataset.roomId;
const roomPath = `/room/${encodeURIComponent(roomId ?? '')}`;
const shareUrl = `${location.origin}${roomPath}`;
const tokenKey = `sameframe:host:${roomId}`;
let hostToken = null;
// Strip the secret and irrelevant query before loading any third-party resource.
const fragmentToken = new URLSearchParams(location.hash.slice(1)).get('host');
if (location.hash || location.search || location.pathname !== roomPath) history.replaceState(null, '', roomPath);
try {
  if (fragmentToken) sessionStorage.setItem(tokenKey, fragmentToken);
  hostToken = fragmentToken || sessionStorage.getItem(tokenKey);
} catch { hostToken = fragmentToken; }

const helpers = await import(document.body.dataset.syncUrl).catch(() => {
  text('room-error', 'The room client could not load. Reload this page to retry.');
  text('connection-status', 'Client unavailable');
  for (const id of ['video-url', 'load-video', 'chat-message', 'send-message', 'join-playback', 'reconnect-button']) {
    if (element(id)) element(id).disabled = true;
  }
  element('video-form')?.addEventListener('submit', (event) => event.preventDefault());
  element('chat-form')?.addEventListener('submit', (event) => event.preventDefault());
  return null;
});
if (helpers) {
  const {
    ClockFilter, SnapshotOrder, NativePlaybackObserver, targetPosition, shouldSeek,
    reconnectDelay, parseVideoId, visibleProblem, playbackUpdate, supportedRate, avatarPattern,
  } = helpers;
  const clock = new ClockFilter();
  const ordering = new SnapshotOrder();
  const native = new NativePlaybackObserver();
  const pings = new Set();
  let socket = null;
  let connected = false;
  let terminal = false;
  let connection = 'Connecting…';
  let reconnectAttempt = 0;
  let reconnectTimer = null;
  let joinTimer = null;
  let pending = null;
  let pendingTimer = null;
  let latestIntent = null; // At most ONE latest native intention while a command is in flight.
  let awaitingSync = false;
  let role = 'guest';
  let memberId = null;
  let membersSignature = '';
  let feedSignature = '';
  let feedIncarnation = null;
  const feedNodes = new Map();
  let socialPending = null;
  let socialTimer = null;
  let chatError = '';
  let snapshot = null;
  let lastResponseMs = 0;
  let roomError = '';
  let playerError = '';
  let notice = '';

  let player = null;
  let playerReady = false;
  let playerReadyTimer = null;
  let playerGeneration = 0;
  let playerMountParent = null;
  let loadingPlayer = false;
  let loadedVideo = null;
  let buffering = false;
  let detached = false;
  let autoplayBlocked = false;
  let autoplayTimer = null;
  let lastSeekMs = -Infinity;
  let forceSeek = false;
  let recoverySeek = false;
  let apiPromise = null;
  let localRate = 1;
  let rateGuard = null;
  let rateEvent = null;

  const roleName = (value) => ({ host: 'Keeper', moderator: 'Co-keeper', guest: 'Companion' })[value];
  const canControl = () => role === 'host' || role === 'moderator';

  function feedEntry(entry) {
    const item = document.createElement('div');
    item.dataset.eventId = String(entry.id);
    const activity = !['message', 'proposal'].includes(entry.kind);
    item.className = activity
      ? 'mb-3 flex items-baseline gap-2 text-[11px] leading-relaxed text-muted-foreground last:mb-0'
      : 'mb-4 flex gap-2.5 break-words last:mb-0';
    const content = document.createElement('div');
    content.className = 'min-w-0 flex-1';
    if (activity) {
      const mark = document.createElement('span');
      mark.className = 'w-7 shrink-0 text-center text-muted-foreground/50';
      mark.textContent = ({ joined: '↳', left: '↗', play: '▸', pause: 'Ⅱ', seek: '↝', set_video: '♫', set_rate: '»', role_changed: '✳' })[entry.kind] ?? '·';
      const body = document.createElement('p');
      body.className = 'text-muted-foreground';
      const position = Number.isFinite(entry.position_secs)
        ? `${Math.floor(entry.position_secs / 60)}:${String(Math.floor(entry.position_secs % 60)).padStart(2, '0')}` : '';
      const roleChange = (entry.text ?? 'changed a role').replace(/ is now a moderator\.$/, ' is now a Co-keeper.')
        .replace(/ is now a guest\.$/, ' is now a Companion.');
      const description = ({ joined: 'settled in', left: 'stepped out', play: 'pressed play', pause: 'paused the video',
        seek: `skipped to ${position}`, set_video: 'put on a new video', set_rate: `changed the speed to ${entry.text ?? '1×'}`,
        role_changed: `made a little change: ${roleChange}` })[entry.kind] ?? entry.kind;
      body.textContent = `${entry.name} ${description}`;
      content.append(body);
      item.append(mark, content);
    } else {
      const author = document.createElement('div');
      author.className = 'flex flex-wrap items-baseline gap-2 text-[11px] font-medium text-foreground/80';
      const name = document.createElement('span');
      name.textContent = entry.name;
      const time = document.createElement('time');
      time.className = 'text-[9px] font-normal text-muted-foreground';
      const date = new Date(Date.now() - Math.max(0, clock.serverNow(performance.now()) - entry.at_ms));
      time.textContent = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      time.dateTime = date.toISOString();
      author.append(name, time);
      content.append(author);
      if (entry.kind === 'message') {
        const body = document.createElement('p');
        body.className = 'mt-1 whitespace-pre-wrap text-[13px] leading-relaxed text-foreground [overflow-wrap:anywhere]';
        body.textContent = entry.text;
        content.append(body);
      } else {
        const card = document.createElement('div');
        card.className = 'mt-2 rounded-[10px] border border-primary/20 bg-primary/5 px-3 py-2.5';
        const label = document.createElement('span');
        label.className = 'mb-1 block text-[9px] tracking-wider text-primary';
        label.textContent = 'SOMETHING FOR THE ROOM';
        const link = document.createElement('a');
        link.className = 'block text-[13px] text-foreground/80 hover:underline';
        link.href = `https://www.youtube.com/watch?v=${entry.video_id}`;
        link.target = '_blank';
        link.rel = 'noopener noreferrer';
        link.textContent = `YouTube · ${entry.video_id}`;
        const button = document.createElement('button');
        button.className = 'mt-2 min-h-9 rounded-md border-0 bg-popover px-3 py-1.5 text-[11px] text-foreground hover:bg-foreground/10';
        button.type = 'button';
        button.textContent = 'Watch this together';
        button.addEventListener?.('click', () => command({ type: 'set_video', video_id: entry.video_id }));
        const hint = document.createElement('p');
        hint.className = 'mt-2 text-[10px] text-muted-foreground';
        hint.textContent = 'A Keeper or Co-keeper can put this on.';
        card.append(label, link, button, hint);
        content.append(card);
        item.proposal = { button, hint };
      }
      item.append(avatar(entry.avatar, 'mt-1 size-7 shrink-0 rounded-full'), content);
    }
    return item;
  }

  function renderFeed() {
    const feed = element('chat-feed');
    if (!feed || !snapshot) return;
    const events = snapshot.events ?? [];
    const signature = JSON.stringify([snapshot.incarnation, events]);
    const reset = feedIncarnation !== snapshot.incarnation;
    if (reset) {
      feedNodes.clear();
      feed.replaceChildren();
      feedIncarnation = snapshot.incarnation;
    }
    if (signature !== feedSignature) {
      const nearBottom = reset || feed.scrollHeight - feed.scrollTop - feed.clientHeight < 70;
      const previousHeight = feed.scrollHeight;
      const previousTop = feed.scrollTop;
      const retained = new Set(events.map((entry) => entry.id));
      for (const [id, node] of feedNodes) {
        if (!retained.has(id)) { node.remove?.(); feedNodes.delete(id); }
      }
      // Account for history pruned above the viewport without moving the reader.
      if (!nearBottom) feed.scrollTop = Math.max(0, previousTop - (previousHeight - feed.scrollHeight));
      for (const entry of events) {
        if (feedNodes.has(entry.id)) continue;
        const node = feedEntry(entry);
        feedNodes.set(entry.id, node);
        feed.append(node);
      }
      feedSignature = signature;
      if (nearBottom) { feed.scrollTop = feed.scrollHeight; if (element('chat-unread')) element('chat-unread').hidden = true; }
      else if (element('chat-unread')) element('chat-unread').hidden = false;
    }
    for (const node of feedNodes.values()) {
      if (!node.proposal) continue;
      node.proposal.button.hidden = !canControl();
      node.proposal.hint.hidden = canControl();
      node.proposal.button.disabled = !connected || !!pending || awaitingSync;
    }
  }

  function clearSocial() { socialPending = null; clearTimeout(socialTimer); }
  function social(action, inputId = null) {
    if (!connected || socialPending) return false;
    const id = globalThis.crypto?.randomUUID?.() ?? `${performance.now()}-${Math.random().toString(36).slice(2)}`;
    socialPending = { id, inputId, value: inputId ? element(inputId)?.value : null };
    chatError = '';
    if (!send({ type: 'social', id, action })) { clearSocial(); return false; }
    socialTimer = setTimeout(() => {
      chatError = 'Not confirmed. Your text is saved here; check the chat before sending again.';
      clearSocial();
      socket?.close();
      render();
    }, 8000);
    render();
    return true;
  }

  function avatar(seed, classes) {
    const pattern = avatarPattern(seed);
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', classes);
    svg.setAttribute('viewBox', '0 0 40 40');
    svg.setAttribute('aria-hidden', 'true');
    const background = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
    for (const [key, value] of Object.entries({ cx: 20, cy: 20, r: 20, fill: '#313244' })) background.setAttribute(key, value);
    svg.append(background);
    for (const [column, row] of pattern.cells) {
      const cell = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
      for (const [key, value] of Object.entries({ x: 5 + column * 6, y: 5 + row * 6, width: 5, height: 5, rx: 1, fill: pattern.color })) cell.setAttribute(key, value);
      svg.append(cell);
    }
    return svg;
  }

  function renderMembers() {
    const list = element('members');
    if (!list || !snapshot) return;
    const signature = JSON.stringify([connected, memberId, role, !!socialPending, snapshot.incarnation, snapshot.participants]);
    if (signature === membersSignature) return;
    membersSignature = signature;

    const nodes = snapshot.participants.map((member) => {
      const item = document.createElement('li');
      item.className = 'flex min-w-0 items-center gap-2 rounded-[10px] border border-border bg-card px-2.5 py-2';
      item.dataset.role = member.role;
      const name = member.name;
      item.title = `${name} · ${roleName(member.role)}`;
      const info = document.createElement('div');
      info.className = 'flex flex-col leading-normal';
      const label = document.createElement('span');
      label.className = 'text-xs text-foreground';
      label.textContent = connected && member.id === memberId ? `${name} (you)` : name;
      const badge = document.createElement('span');
      badge.className = member.role === 'host' ? 'text-[10px] text-primary'
        : member.role === 'moderator' ? 'text-[10px] text-[#94e2d5]' : 'text-[10px] text-muted-foreground';
      badge.textContent = roleName(member.role);
      info.append(label, badge);
      item.append(avatar(member.avatar, 'size-[30px] shrink-0 rounded-full'), info);
      if (role === 'host' && member.role !== 'host') {
        const manage = document.createElement('button');
        const enabled = member.role !== 'moderator';
        manage.type = 'button';
        manage.className = 'grid h-8 w-[30px] place-items-center rounded-md border-0 bg-transparent text-lg text-muted-foreground hover:bg-popover hover:text-primary';
        manage.textContent = enabled ? '+' : '−';
        manage.title = `${enabled ? 'Make' : 'Remove'} ${name} ${enabled ? 'a Co-keeper' : 'as Co-keeper'}`;
        manage.setAttribute?.('aria-label', manage.title);
        manage.disabled = !connected || !!socialPending;
        manage.addEventListener?.('click', () => social({ type: 'set_moderator', member_id: member.id, enabled }));
        item.append(manage);
      }
      return item;
    });
    list.replaceChildren(...nodes);
  }

  function render() {
    renderMembers();
    renderFeed();
    text('connection-status', connection);
    if (element('connection-status')) element('connection-status').dataset.state = connected ? 'connected' : 'disconnected';
    text('role-label', connected ? `You’re a ${roleName(role)}` : 'Not connected');
    text('room-name', snapshot?.room_name ?? 'Your viewing room');
    text('member-count', snapshot ? String(snapshot.members) : '—');
    text('room-code', roomId ?? '');
    text('playback-status', !snapshot?.video_id ? 'YouTube · waiting for a video'
      : !connected ? 'YouTube · reconnecting' : detached ? 'YouTube · watching locally' : 'YouTube · following room');
    text('room-error', visibleProblem(roomError, playerError));
    text('room-notice', detached
      ? 'Playback is local on this device. Rejoin playback to catch up with the room.' : notice);
    text('playback-help', snapshot?.video_id
      ? 'Keeper and Co-keeper player controls apply to everyone. Companions can suggest a video.'
      : 'Load a video to get started.');
    if (element('guest-note')) element('guest-note').hidden = !connected || role !== 'guest';
    for (const id of ['video-url', 'load-video']) {
      if (element(id)) element(id).disabled = !connected || !!pending || awaitingSync || !!socialPending;
    }
    text('load-video', canControl() ? 'Watch' : 'Suggest');
    text('chat-error', chatError);
    for (const id of ['chat-message', 'send-message']) {
      if (element(id)) element(id).disabled = !connected || !!socialPending;
    }
    if (element('join-playback')) {
      element('join-playback').hidden = terminal || (!detached && !autoplayBlocked && !playerError);
      element('join-playback').textContent = playerError ? 'Retry playback on this device'
        : detached ? 'Rejoin playback' : 'Join playback on this device';
      element('join-playback').disabled = !connected || loadingPlayer || !snapshot?.video_id;
    }
    if (element('reconnect-button')) {
      element('reconnect-button').hidden = connected || terminal;
      element('reconnect-button').disabled = terminal;
    }
    if (element('player-placeholder')) element('player-placeholder').hidden = !!snapshot?.video_id;
  }

  function send(message) {
    if (socket?.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(message));
    return true;
  }
  function clearPending() { pending = null; clearTimeout(pendingTimer); }
  function discardIntents() { clearPending(); latestIntent = null; native.reset(); rateGuard = null; rateEvent = null; }
  function ping() {
    if (!connected) return;
    const clientMs = performance.now();
    if (pings.size >= 8) pings.delete(pings.values().next().value);
    pings.add(clientMs);
    send({ type: 'ping', client_ms: clientMs });
  }
  function recover() {
    if (!connected) return;
    if (performance.now() - lastResponseMs > 30000) { socket?.close(); return; }
    ping();
    send({ type: 'sync' });
    updatePlayer();
  }
  function stopUnavailable(message) {
    terminal = true;
    connected = false;
    clearTimeout(reconnectTimer);
    clearTimeout(joinTimer);
    clearTimeout(autoplayTimer);
    clearTimeout(playerReadyTimer);
    ++playerGeneration;
    buffering = false;
    detached = false;
    autoplayBlocked = false;
    notice = '';
    discardIntents();
    clearSocial();
    connection = 'Room unavailable';
    roomError = message || 'This room has expired or is no longer available. Create a new room to keep watching.';
    socket?.close();
    if (playerReady) player.pauseVideo();
    render();
  }

  function connect() {
    if (terminal) return;
    clearTimeout(reconnectTimer);
    clearTimeout(joinTimer);
    const previous = socket;
    socket = null; // Invalidate old callbacks before closing its server lease.
    previous?.close();
    connected = false;
    awaitingSync = true;
    detached = false;
    discardIntents(); // No command or native intention crosses a connection.
    clearSocial();
    pings.clear();
    clock.reset();
    ordering.reset();
    connection = reconnectAttempt ? 'Reconnecting…' : 'Connecting…';
    render();
    const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/rooms/${encodeURIComponent(roomId)}/ws`);
    socket = ws;
    joinTimer = setTimeout(() => { if (socket === ws) ws.close(); }, 10000);
    ws.addEventListener('open', () => { if (socket === ws) send({ type: 'join', host_token: hostToken }); });
    ws.addEventListener('message', (event) => {
      if (socket !== ws || terminal) return;
      let message;
      try { message = JSON.parse(event.data); } catch { return; }
      if (!message || typeof message !== 'object') return;
      lastResponseMs = performance.now();
      if (message.type === 'welcome' || message.type === 'snapshot') {
        const welcome = message.type === 'welcome';
        if (welcome && (!['host', 'moderator', 'guest'].includes(message.role)
          || !Number.isSafeInteger(message.member_id) || message.member_id <= 0)) return;
        if (!welcome && !connected) return;
        if (!ordering.accept(message.snapshot, message.server_ms, welcome)) return;
        const previousSnapshot = snapshot;
        const update = playbackUpdate(snapshot, message.snapshot);
        const changedMedia = update.resetVideo;
        if (welcome) {
          clearTimeout(joinTimer);
          connected = true;
          reconnectAttempt = 0;
          connection = 'Connected';
          role = message.role;
          memberId = message.member_id;
          roomError = '';
          if (!autoplayBlocked) notice = '';
        }
        snapshot = message.snapshot;
        const previousRole = role;
        role = snapshot.participants.find((member) => member.id === memberId)?.role ?? role;
        if (previousRole !== role) {
          discardIntents();
          detached = false;
          forceSeek = true;
        }
        if (pending && !pending.native && pending.action.type === 'set_video') {
          pending.mediaAccepted = snapshot.incarnation === pending.incarnation
            && snapshot.media_revision > pending.mediaRevision && snapshot.video_id === pending.action.video_id;
        }
        forceSeek ||= update.forceSeek || welcome;
        recoverySeek ||= welcome;
        clock.seed(message.server_ms, performance.now());
        awaitingSync = false;
        if (changedMedia) {
          if (pending?.native) clearPending();
          latestIntent = null;
          native.reset();
          rateGuard = null;
          rateEvent = null;
          loadedVideo = null;
          playerError = '';
          autoplayBlocked = false;
          buffering = false;
          if (detached && previousSnapshot?.video_id) notice = 'The host selected a video. You are following the room again.';
          detached = false;
          lastSeekMs = -Infinity;
        }
        if (pending?.ackRevision != null && snapshot.revision >= pending.ackRevision) clearPending();
        if (welcome) ping();
        ensurePlayer();
        updatePlayer(); // Observe/coalesce native input BEFORE applying the room snapshot.
        render();
      } else if (message.type === 'social_ack') {
        if (message.id !== socialPending?.id) return;
        const input = socialPending.inputId ? element(socialPending.inputId) : null;
        if (input && input.value === socialPending.value) input.value = '';
        clearSocial();
        render();
        input?.focus?.();
      } else if (message.type === 'pong') {
        if (!pings.delete(message.client_ms)) return;
        clock.sample(message.client_ms, performance.now(), message.server_ms);
        updatePlayer();
      } else if (message.type === 'ack') {
        if (pending?.id !== message.id || !Number.isSafeInteger(message.revision)) return;
        pending.ackRevision = message.revision;
        if (snapshot && snapshot.revision >= message.revision) clearPending();
        else { awaitingSync = true; send({ type: 'sync' }); }
        updatePlayer();
        render();
      } else if (message.type === 'error') {
        if (message.code === 'unavailable') { stopUnavailable(); return; }
        if (message.id && message.id === socialPending?.id) {
          chatError = typeof message.message === 'string' ? message.message : 'That could not be sent.';
          clearSocial();
          render();
          return;
        }
        if (message.code === 'invalid_token') {
          stopUnavailable('Host access could not be verified. Open the original host link, or join using a shared guest link in another tab.');
          connection = 'Host access rejected';
          render();
          return;
        }
        if (message.id && message.id !== pending?.id) return;
        if (message.id) discardIntents();
        roomError = typeof message.message === 'string' ? message.message : 'The room could not accept that action.';
        // A denied action doesn't prove a role change; only snapshots do.
        if (message.code === 'stale_revision') {
          awaitingSync = true;
          send({ type: 'sync' });
        } else { forceSeek = true; updatePlayer(); }
        render();
      }
    });
    ws.addEventListener('close', (event) => {
      if (socket !== ws) return;
      clearTimeout(joinTimer);
      connected = false;
      discardIntents();
      clearSocial();
      if (event.code === 4004) { stopUnavailable(); return; }
      if (terminal) return;
      connection = 'Disconnected · retrying';
      notice = 'Reconnecting to the room. No offline controls will be replayed.';
      render();
      reconnectTimer = setTimeout(connect, reconnectDelay(reconnectAttempt++));
    });
  }

  function command(action, intention = null) {
    if (!connected || !canControl() || !snapshot || pending || awaitingSync) return false;
    const id = globalThis.crypto?.randomUUID?.() ?? `${performance.now()}-${Math.random().toString(36).slice(2)}`;
    pending = { id, ackRevision: null, native: intention !== null, action, intention, mediaAccepted: false,
      mediaRevision: snapshot.media_revision, incarnation: snapshot.incarnation };
    roomError = '';
    if (!send({ type: 'command', id, revision: snapshot.revision, action })) { clearPending(); return false; }
    pendingTimer = setTimeout(() => {
      roomError = 'The command was not confirmed. Reconnecting for fresh room state.';
      socket?.close();
    }, 8000);
    render();
    return true;
  }
  function flushIntent() {
    if (!latestIntent || pending || awaitingSync || !connected || !canControl() || playerError || !playerReady) return;
    const intent = latestIntent;
    if (intent.incarnation !== snapshot.incarnation || intent.videoId !== snapshot.video_id || intent.mediaRevision !== snapshot.media_revision) { latestIntent = null; return; }
    // Unified play/pause-with-position preserves the newest scrub AND playing intent.
    if (snapshot.playback_rate !== intent.rate) {
      command({ type: 'set_rate', playback_rate: intent.rate }, intent);
      return; // Keep the latest combined intention until its rate is authoritative.
    }
    if (!intent.needsPlayback) { latestIntent = null; return; }
    const advance = intent.playing && !buffering ? Math.max(0, performance.now() - intent.at) / 1000 * intent.observedRate : 0;
    const position = Math.min(player.getDuration() || Infinity, Math.max(0, intent.position + advance));
    if (command({ type: intent.playing ? 'play' : 'pause', position_secs: position }, intent)) latestIntent = null;
  }

  function loadYouTubeApi() {
    if (globalThis.YT?.Player) return Promise.resolve();
    if (apiPromise) return apiPromise;
    apiPromise = new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = 'https://www.youtube.com/iframe_api';
      script.async = true;
      const previous = globalThis.onYouTubeIframeAPIReady;
      const finish = (error) => {
        clearTimeout(timeout);
        globalThis.onYouTubeIframeAPIReady = previous;
        script.onerror = null;
        if (error) { script.remove(); reject(error); } else resolve();
      };
      globalThis.onYouTubeIframeAPIReady = () => { finish(); if (typeof previous === 'function') previous(); };
      script.onerror = () => finish(new Error('YouTube could not load. Check your connection or content blocker, then retry.'));
      const timeout = setTimeout(() => finish(new Error('YouTube took too long to load. Check your connection, then retry.')), 12000);
      document.head.append(script);
    }).catch((error) => { apiPromise = null; throw error; });
    return apiPromise;
  }
  function resetPlayer() {
    ++playerGeneration;
    clearTimeout(playerReadyTimer);
    clearTimeout(autoplayTimer);
    playerReadyTimer = null;
    autoplayTimer = null;
    const previous = player;
    player = null;
    playerReady = false;
    loadedVideo = null;
    buffering = false;
    autoplayBlocked = false;
    latestIntent = null;
    native.reset();
    rateGuard = null;
    rateEvent = null;
    forceSeek = false;
    recoverySeek = false;
    lastSeekMs = -Infinity;
    try { previous?.destroy(); } catch { /* The iframe may already have been removed. */ }
    const mount = element('player-mount');
    if (!mount || mount.tagName !== 'DIV') {
      const replacement = document.createElement('div');
      replacement.id = 'player-mount';
      if (mount) mount.replaceWith(replacement);
      else playerMountParent?.append(replacement);
    }
  }
  async function ensurePlayer() {
    if (!snapshot?.video_id || loadingPlayer || player || playerError) return;
    loadingPlayer = true;
    try {
      await loadYouTubeApi();
      if (!snapshot?.video_id || terminal) return;
      const generation = ++playerGeneration;
      playerMountParent = element('player-mount')?.parentNode ?? playerMountParent;
      playerReadyTimer = setTimeout(() => {
        if (generation !== playerGeneration || playerReady || terminal) return;
        resetPlayer();
        playerError = 'The YouTube player did not become ready. Check your connection or content blocker, then retry playback.';
        notice = '';
        render();
      }, 15000);
      player = new YT.Player('player-mount', {
        width: '100%', height: '100%',
        playerVars: { controls: 1, disablekb: 0, fs: 1, playsinline: 1, rel: 0, origin: location.origin },
        events: {
          onReady: () => {
            if (generation !== playerGeneration || terminal) return;
            clearTimeout(playerReadyTimer);
            playerReadyTimer = null;
            playerReady = true;
            recoverySeek = true;
            recover();
            render();
          },
          onStateChange: (event) => {
            if (generation !== playerGeneration || terminal) return;
            const wasBuffering = buffering;
            updatePlayer(event.data);
            if (wasBuffering && !buffering) {
              recoverySeek = true;
              ping();
              send({ type: 'sync' });
            }
            render();
          },
          onAutoplayBlocked: () => { if (generation === playerGeneration && !terminal) markAutoplayBlocked(); },
          onPlaybackRateChange: (event) => {
            if (generation === playerGeneration && !terminal) { updatePlayer(undefined, event.data); render(); }
          },
          onError: (event) => {
            if (generation !== playerGeneration || terminal) return;
            if (!playerReady) resetPlayer();
            clearTimeout(autoplayTimer);
            autoplayTimer = null;
            buffering = false;
            const errors = {
              2: 'YouTube rejected this video ID.',
              5: 'This browser could not play the video. Retry playback or try another browser.',
              100: 'This video is unavailable or private. The host can choose another video.',
              101: 'The owner does not allow this video to be embedded. Choose another video.',
              150: 'The owner does not allow this video to be embedded. Choose another video.',
              153: 'YouTube could not verify this embed. Check browser privacy settings or try another browser.',
            };
            playerError = errors[event.data] || 'YouTube could not play this video. The host can choose another video.';
            render();
          },
        },
      });
    } catch (error) {
      resetPlayer();
      playerError = error.message || 'YouTube could not load. Try again.';
      notice = '';
    } finally { loadingPlayer = false; render(); }
  }
  function applyRate() {
    const rate = supportedRate(snapshot.playback_rate, player.getAvailablePlaybackRates?.());
    if (rate === null) return; // The iframe has not reported its supported rates yet.
    if (rate !== snapshot.playback_rate) notice = `This video cannot use the room’s speed. Following at ${rate}× on this device.`;
    if (rateGuard?.rate === rate || (!rateGuard && player.getPlaybackRate?.() === rate)) return;
    rateGuard = { rate, until: performance.now() + 4000, settledAt: null };
    localRate = rate;
    player.setPlaybackRate(rate); // Supported values only, and guard BEFORE the async write.
  }
  function observeRate(eventRate) {
    const now = performance.now();
    let rate = eventRate ?? player.getPlaybackRate?.();
    if (!Number.isFinite(rate) || rate < 0.25 || rate > 4) return false;
    if (eventRate !== undefined) rateEvent = { rate, at: now };
    else if (rateEvent) {
      if (rate === rateEvent.rate || now - rateEvent.at >= 750) rateEvent = null;
      else rate = rateEvent.rate;
    }
    if (rateGuard) {
      if (rate === rateGuard.rate) rateGuard.settledAt ??= now;
      if (rateGuard.settledAt !== null && now - rateGuard.settledAt >= 250) rateGuard = null;
      else if (now >= rateGuard.until) { rateGuard = null; localRate = rate; return false; }
      else return false;
    }
    const changed = rate !== localRate;
    localRate = rate;
    return changed;
  }
  function recordIntent(intent) {
    if (canControl()) {
      const previous = latestIntent;
      const intendedRate = intent.kind === 'rate' ? localRate
        : previous?.rate ?? pending?.intention?.rate ?? snapshot.playback_rate;
      latestIntent = { ...intent, rate: intendedRate, observedRate: localRate,
        needsPlayback: intent.kind !== 'rate' || !!previous?.needsPlayback,
        incarnation: snapshot.incarnation, videoId: snapshot.video_id, mediaRevision: snapshot.media_revision };
    } else { detached = true; notice = ''; }
    autoplayBlocked = false;
    render();
  }
  function markAutoplayBlocked() {
    clearTimeout(autoplayTimer);
    autoplayTimer = null;
    autoplayBlocked = true;
    native.reset();
    // Establish a paused baseline so a later native Play is a local join gesture.
    if (playerReady) native.observe(player.getCurrentTime(), YT.PlayerState.PAUSED, performance.now(), false, localRate);
    notice = 'Your browser needs a tap to start audio. Join playback on this device.';
    render();
  }
  function watchAutoplay() {
    clearTimeout(autoplayTimer);
    autoplayTimer = setTimeout(() => {
      autoplayTimer = null;
      if (snapshot?.playing && !detached && !buffering && player.getPlayerState() !== YT.PlayerState.PLAYING) markAutoplayBlocked();
    }, 2500);
  }

  function observeNative(eventState, eventRate) {
    if (!connected || !playerReady || !snapshot?.video_id || playerError || document.hidden) return;
    // Do not attribute delayed old-video callbacks to the newly selected media.
    if (loadedVideo !== snapshot.video_id || player.getVideoData?.()?.video_id !== snapshot.video_id) return;
    if (pending && !pending.native && !pending.mediaAccepted) return;
    const blocked = autoplayBlocked;
    const changedRate = observeRate(eventRate);
    const intent = native.observe(player.getCurrentTime(), eventState ?? player.getPlayerState(), performance.now(), eventState !== undefined, localRate);
    buffering = native.state === YT.PlayerState.BUFFERING;
    if (native.state === YT.PlayerState.PLAYING) {
      clearTimeout(autoplayTimer);
      autoplayTimer = null;
    }
    if (intent) {
      if (blocked && intent.playing && intent.kind === 'state') { rejoin(); return; }
      recordIntent(intent);
    }
    if (changedRate) recordIntent({ playing: native.playing ?? snapshot.playing, position: player.getCurrentTime(),
      at: performance.now(), kind: 'rate' });
  }
  function updatePlayer(eventState, eventRate) {
    if (!connected || !playerReady || document.hidden) return;
    try {
      observeNative(eventState, eventRate); // MUST precede drift correction, even on snapshot/pong.
      flushIntent();
      reconcile();
    } catch (error) {
      // Never log the player/socket objects or a URL carrying host credentials.
      let cause = String(error?.message ?? error);
      if (hostToken) cause = cause.split(hostToken).join('[redacted]');
      cause = cause.replace(/([#?&]host=)[^\s&]+/g, '$1[redacted]');
      console.warn('YouTube synchronization failed:', cause);
      playerError = 'The YouTube player could not synchronize. Retry playback on this device.';
      render();
    }
  }
  function reconcile() {
    if (!snapshot?.video_id || !playerReady || playerError || awaitingSync || detached) return;
    if (pending?.native || latestIntent || native.waiting) return; // Keep newest native state until acknowledged.
    const now = performance.now();
    const newVideo = loadedVideo !== snapshot.video_id;
    if (!newVideo) applyRate();
    if (native.settling && !forceSeek && !newVideo) return;
    const duration = newVideo ? Infinity : player.getDuration();
    const target = targetPosition(snapshot, clock.serverNow(now), duration);
    const state = native.state ?? player.getPlayerState();
    const seek = !newVideo && shouldSeek(player.getCurrentTime(), target, now, lastSeekMs, forceSeek, buffering, recoverySeek);
    const play = snapshot.playing && !autoplayBlocked && !buffering && state !== YT.PlayerState.PLAYING
      && (state !== YT.PlayerState.ENDED || target < (player.getDuration() || Infinity) - 1);
    // Native Play goes BUFFERING -> PLAYING. An unchanged paused snapshot must
    // not cancel that transition; only a fresh authoritative revision may do so.
    const pause = !snapshot.playing && (state === YT.PlayerState.PLAYING
      || (state === YT.PlayerState.BUFFERING && forceSeek));
    forceSeek = false;
    recoverySeek = false;
    if (!newVideo && !seek && !play && !pause) return;
    // Register expected state/position BEFORE any async iframe operation.
    native.suppress(snapshot.playing, newVideo || seek ? target : null, now, localRate);
    if (newVideo) {
      loadedVideo = snapshot.video_id;
      // The empty bootstrap iframe cannot safely play before its async cue lands.
      // Load both the video ID and playing position in one YouTube operation.
      if (snapshot.playing && !autoplayBlocked) {
        player.loadVideoById({ videoId: loadedVideo, startSeconds: target });
        watchAutoplay();
      } else {
        player.cueVideoById({ videoId: loadedVideo, startSeconds: target });
        player.pauseVideo();
      }
      applyRate(); // onReady may have had no rates until media caches populate.
      lastSeekMs = now;
      return;
    }
    if (seek) { player.seekTo(target, true); lastSeekMs = now; }
    if (snapshot.playing && !autoplayBlocked) {
      if (seek || play) { player.playVideo(); watchAutoplay(); }
    } else if (seek || pause) player.pauseVideo();
  }
  function rejoin() {
    if (!connected || !snapshot?.video_id) return;
    detached = false;
    autoplayBlocked = false;
    notice = '';
    if (playerError) {
      playerError = '';
      loadedVideo = null;
      native.reset();
    }
    if (!playerReady) { ensurePlayer(); render(); return; }
    // Perform the local recovery within the native/button user gesture.
    const newVideo = loadedVideo !== snapshot.video_id;
    const target = targetPosition(snapshot, clock.serverNow(performance.now()), newVideo ? Infinity : player.getDuration());
    native.suppress(snapshot.playing, target, performance.now(), localRate);
    if (newVideo) {
      loadedVideo = snapshot.video_id;
      if (snapshot.playing) player.loadVideoById({ videoId: loadedVideo, startSeconds: target });
      else { player.cueVideoById({ videoId: loadedVideo, startSeconds: target }); player.pauseVideo(); }
    } else {
      player.seekTo(target, true);
      if (snapshot.playing) player.playVideo();
      else player.pauseVideo();
    }
    applyRate();
    lastSeekMs = performance.now();
    if (snapshot.playing) watchAutoplay();
    ping();
    send({ type: 'sync' });
    render();
  }

  element('video-form')?.addEventListener('submit', (event) => {
    event.preventDefault();
    const videoId = parseVideoId(element('video-url')?.value);
    if (!videoId) {
      roomError = 'Enter a valid YouTube watch, shorts, embed, or youtu.be link (or an 11-character video ID).';
      render();
      return;
    }
    if (canControl()) command({ type: 'set_video', video_id: videoId });
    else social({ type: 'propose_video', video_id: videoId }, 'video-url');
  });
  element('chat-form')?.addEventListener('submit', (event) => {
    event.preventDefault();
    const value = element('chat-message')?.value.trim();
    if (!value) return;
    const videoId = parseVideoId(value);
    social(videoId ? { type: 'propose_video', video_id: videoId } : { type: 'message', text: value }, 'chat-message');
  });
  element('chat-unread')?.addEventListener('click', () => {
    const feed = element('chat-feed');
    feed.scrollTop = feed.scrollHeight;
    element('chat-unread').hidden = true;
  });
  element('chat-feed')?.addEventListener('scroll', () => {
    const feed = element('chat-feed');
    if (feed.scrollHeight - feed.scrollTop - feed.clientHeight < 70) element('chat-unread').hidden = true;
  });
  element('join-playback')?.addEventListener('click', rejoin);
  element('reconnect-button')?.addEventListener('click', () => {
    if (terminal) return;
    if (!playerReady && snapshot?.video_id) { playerError = ''; ensurePlayer(); }
    connect();
  });
  let copyTimer = null;
  let copyAttempt = 0;
  function copyFeedback(copied, feedback, title) {
    element('copy-icon')?.toggleAttribute('hidden', copied);
    element('copy-success')?.toggleAttribute('hidden', !copied);
    if (element('copy-link')) element('copy-link').title = title;
    text('copy-feedback', feedback);
  }
  element('copy-link')?.addEventListener('click', async () => {
    const attempt = ++copyAttempt;
    clearTimeout(copyTimer);
    text('copy-feedback', ''); // Re-announce repeated clicks without a visible banner.
    try {
      await navigator.clipboard.writeText(shareUrl);
      if (attempt !== copyAttempt) return;
      copyFeedback(true, 'Room link copied', 'Copied');
      copyTimer = setTimeout(() => copyFeedback(false, '', 'Copy room link'), 1800);
    } catch {
      if (attempt !== copyAttempt) return;
      copyFeedback(false, 'Copy URL from address bar', 'Copy URL from address bar');
    }
  });
  document.addEventListener('visibilitychange', () => {
    native.reset(); // A suspended tab's sampling gap is not a native scrub.
    recoverySeek = true;
    recover();
  });
  window.addEventListener('online', () => {
    if (connected) recover();
    else if (!terminal && socket?.readyState !== WebSocket.CONNECTING) connect();
  });
  setInterval(() => { if (!document.hidden) recover(); }, 10000);
  setInterval(() => { if (!document.hidden) updatePlayer(); }, 300);
  render();
  if (/^[A-Za-z0-9_-]+$/.test(roomId ?? '')) connect();
  else stopUnavailable();
}
