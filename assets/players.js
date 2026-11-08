export const PlayerState = Object.freeze({ UNSTARTED: -1, ENDED: 0, PLAYING: 1, PAUSED: 2, BUFFERING: 3, CUED: 5 });

export function youtubePlayer(api) {
  return {
    getCurrentTime: () => api.getCurrentTime(),
    getDuration: () => api.getDuration(),
    getPlayerState: () => api.getPlayerState(),
    getSourceKey: () => api.getVideoData?.()?.video_id,
    getPlaybackRate: () => api.getPlaybackRate?.(),
    getAvailablePlaybackRates: () => api.getAvailablePlaybackRates?.(),
    setPlaybackRate: rate => api.setPlaybackRate(rate),
    seekTo: position => api.seekTo(position, true),
    playVideo: () => api.playVideo(),
    pauseVideo: () => api.pauseVideo(),
    load(source, position, playing) {
      const video = { videoId: source.id, startSeconds: position };
      if (playing) api.loadVideoById(video);
      else { api.cueVideoById(video); api.pauseVideo(); }
    },
    destroy: () => api.destroy?.(),
  };
}

export function directPlayer(mount, source, events) {
  const video = mount.ownerDocument.createElement('video');
  video.id = 'player-mount';
  video.className = `${mount.className} object-contain`;
  video.controls = true;
  video.playsInline = true;
  video.preload = 'metadata';
  video.setAttribute('aria-label', 'Shared video');
  let waiting = false;
  let destroyed = false;
  let ready = false;
  // Resource selection is asynchronous; old metadata can outlive a src assignment.
  let metadataPending = true;
  let playAttempt = 0;
  let pendingLoad = null;
  let requestedRate = 1;

  function play() {
    const attempt = ++playAttempt;
    if (pendingLoad && metadataPending) { pendingLoad.playing = true; return; }
    video.play()?.catch(error => {
      if (destroyed || attempt !== playAttempt || error.name === 'AbortError') return;
      if (error.name === 'NotAllowedError') events.onAutoplayBlocked();
      else events.onError({ data: video.error?.code ?? 4 });
    });
  }

  function pause() {
    ++playAttempt;
    if (pendingLoad) pendingLoad.playing = false;
    video.pause();
  }

  function applyLoad() {
    if (!pendingLoad || metadataPending || video.readyState < 1) return;
    const { position, playing } = pendingLoad;
    pendingLoad = null;
    video.currentTime = Math.min(position, Number.isFinite(video.duration) ? video.duration : Infinity);
    if (playing) play(); else pause();
  }

  const listeners = {
    loadedmetadata() {
      if (video.readyState < 1 || video.currentSrc !== video.src) return;
      video.playbackRate = requestedRate;
      metadataPending = false;
      applyLoad();
      if (!ready) { ready = true; events.onReady(); }
    },
    playing() { waiting = false; events.onStateChange({ data: PlayerState.PLAYING }); },
    pause() { events.onStateChange({ data: video.ended ? PlayerState.ENDED : PlayerState.PAUSED }); },
    waiting() { waiting = true; events.onStateChange({ data: PlayerState.BUFFERING }); },
    seeked() { waiting = false; events.onStateChange({ data: video.paused ? PlayerState.PAUSED : PlayerState.PLAYING }); },
    ended() { events.onStateChange({ data: PlayerState.ENDED }); },
    ratechange() { events.onPlaybackRateChange({ data: video.playbackRate }); },
    error() { if (video.error) events.onError({ data: video.error.code }); },
  };
  for (const [name, listener] of Object.entries(listeners)) video.addEventListener(name, listener);
  mount.replaceWith(video);
  video.src = source.url;

  return {
    getCurrentTime: () => video.currentTime,
    getDuration: () => metadataPending ? NaN : video.duration,
    getPlayerState() {
      if (metadataPending || video.readyState < 1) return pendingLoad ? PlayerState.BUFFERING : PlayerState.UNSTARTED;
      if (video.ended) return PlayerState.ENDED;
      if (waiting || video.seeking) return PlayerState.BUFFERING;
      return video.paused ? PlayerState.PAUSED : PlayerState.PLAYING;
    },
    getSourceKey: () => metadataPending || video.readyState < 1 ? '' : video.currentSrc,
    getPlaybackRate: () => metadataPending ? requestedRate : video.playbackRate,
    getAvailablePlaybackRates: requested => [requested],
    setPlaybackRate(rate) {
      requestedRate = rate;
      if (!metadataPending) video.playbackRate = rate;
    },
    seekTo: position => {
      if (pendingLoad) pendingLoad.position = position;
      else video.currentTime = position;
    },
    playVideo: play,
    pauseVideo: pause,
    load(next, position, playing) {
      pendingLoad = { position, playing };
      if (video.src !== next.url || !video.currentSrc || video.error) {
        ++playAttempt;
        metadataPending = true;
        video.src = next.url;
        waiting = false;
      }
      applyLoad();
    },
    destroy() {
      destroyed = true;
      for (const [name, listener] of Object.entries(listeners)) video.removeEventListener(name, listener);
      pause();
      video.removeAttribute('src');
      video.load();
      video.remove();
    },
  };
}
