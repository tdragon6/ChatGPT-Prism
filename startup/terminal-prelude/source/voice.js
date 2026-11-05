/* Announcement playback controller: MIT — ChatGPT Prism contributors. */
(() => {
  "use strict";
  if (window.__PRISM_VOICE__) return;
  const names = ["permission", "identity", "verified", "initialize", "granted", "online"];
  const clips = new Map(names.map((name) => [name, document.getElementById(`voice-${name}`)]));
  const spoken = new Set();
  let enabled = true, volume = 0.5, speed = 1, active = null, wanted = null;
  let generation = 0, blocked = false, duck = () => {};

  function stopCurrent() {
    generation++;
    const previous = active;
    active = null;
    if (previous) {
      previous.onended = previous.onerror = null;
      previous.pause();
      previous.currentTime = 0;
    }
    duck(false);
  }
  function stop() {
    wanted = null;
    stopCurrent();
  }
  async function play(name) {
    if (!names.includes(name)) return false;
    wanted = name;
    if (!enabled || spoken.has(name)) return false;
    const clip = clips.get(name);
    if (!clip || (active === clip && !clip.paused)) return false;
    stopCurrent();
    active = clip;
    const token = generation;
    clip.volume = Math.max(0, Math.min(1, volume * 0.94));
    clip.playbackRate = speed;
    clip.preservesPitch = true;
    clip.onended = clip.onerror = () => {
      if (token !== generation) return;
      active = null;
      duck(false);
    };
    duck(true);
    try {
      await clip.play();
      if (token !== generation || !enabled) return false;
      spoken.add(name);
      blocked = false;
      return true;
    } catch {
      if (token === generation) {
        active = null;
        blocked = true;
        duck(false);
      }
      return false;
    }
  }
  function configure(options = {}) {
    enabled = options.enabled !== false;
    volume = Number.isFinite(options.volume) ? Math.max(0, Math.min(1, options.volume)) : 0.5;
    speed = Number.isFinite(options.speed) ? Math.max(0.75, Math.min(2, options.speed)) : 1;
    if (typeof options.duck === "function") duck = options.duck;
    if (!enabled) stopCurrent();
    else if (active) { active.volume = volume * 0.94; active.playbackRate = speed; }
  }
  function reset() { stop(); spoken.clear(); blocked = false; }
  function dispose() { reset(); enabled = false; }
  addEventListener("pagehide", dispose);
  addEventListener("message", (event) => {
    if (event.source === parent && event.data?.type === "prism:dispose") dispose();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) stop();
  });
  window.__PRISM_VOICE__ = Object.freeze({
    configure, play, stop, reset,
    retry: () => wanted && enabled && blocked ? play(wanted) : Promise.resolve(false),
  });
})();
