/* Procedural score. MIT — tdragon6. */
(() => {
  "use strict";
  if (window.__PRISM_AUDIO__) return;

  const BPM = 112;
  const BEAT = 60 / BPM;
  const GRID = BEAT / 4;
  // Individual voices are deliberately quiet; this bus gain brings the full
  // arrangement to a comfortable level before the gentle peak compressor.
  const LEVEL = 2.5;
  // D minor -> Bbmaj7 -> Gm9 -> Am(add11) -> Cmaj9 -> Fmaj9.
  const HARMONY = [
    [50, 57, 60, 64, 65],
    [46, 53, 57, 60, 62],
    [43, 50, 53, 57, 58],
    [45, 52, 55, 60, 62],
    [48, 55, 59, 62, 64],
    [53, 57, 60, 64, 67],
  ];
  const ARP = [0, 2, 1, 3, 0, 4, 2, 1, 0, 3, 1, 4, 2, 3, 1, 4];
  const midi = (note) => 440 * 2 ** ((note - 69) / 12);
  const clampStage = (value) =>
    Math.max(0, Math.min(5, Math.floor(Number(value) || 0)));

  function random(seed) {
    let value = seed >>> 0;
    return () => {
      value += 0x6d2b79f5;
      let number = value;
      number = Math.imul(number ^ (number >>> 15), number | 1);
      number ^= number + Math.imul(number ^ (number >>> 7), number | 61);
      return ((number ^ (number >>> 14)) >>> 0) / 4294967296;
    };
  }

  function makeGraph(context) {
    const sources = new Set();
    const nodes = new Set();
    const pads = new Set();
    const own = (node) => {
      nodes.add(node);
      return node;
    };
    const master = own(context.createGain());
    const compressor = own(context.createDynamicsCompressor());
    compressor.threshold.value = -18;
    compressor.knee.value = 15;
    compressor.ratio.value = 2.6;
    compressor.attack.value = 0.012;
    compressor.release.value = 0.22;
    master.gain.value = 0;
    master.connect(compressor);
    const output = own(context.createGain());
    output.gain.value = volume;
    const ceiling = own(context.createWaveShaper());
    ceiling.curve = Float32Array.from(
      { length: 4096 },
      (_, index) =>
        (Math.tanh(((index / 4095) * 2 - 1) * 1.4) / Math.tanh(1.4)) * 0.92,
    );
    ceiling.oversample = "none";
    compressor.connect(ceiling);
    ceiling.connect(output);
    output.connect(context.destination);
    const dry = own(context.createGain());
    dry.gain.value = 0.9;
    dry.connect(master);
    const reverb = own(context.createConvolver());
    const wetFilter = own(context.createBiquadFilter());
    wetFilter.type = "lowpass";
    wetFilter.frequency.value = 4200;
    const wet = own(context.createGain());
    wet.gain.value = 0.26;
    reverb.connect(wetFilter);
    wetFilter.connect(wet);
    wet.connect(master);

    const impulseLength = Math.floor(context.sampleRate * 2.3);
    const impulse = context.createBuffer(2, impulseLength, context.sampleRate);
    for (let channel = 0; channel < 2; channel++) {
      const sample = impulse.getChannelData(channel);
      const rng = random(19477 + channel * 7207);
      let smooth = 0;
      for (let i = 0; i < sample.length; i++) {
        const time = i / context.sampleRate;
        smooth = smooth * 0.42 + (rng() * 2 - 1) * 0.58;
        sample[i] = smooth * Math.exp(-time * 3.5) * Math.min(1, time / 0.018);
      }
    }
    reverb.buffer = impulse;
    const noise = context.createBuffer(
      1,
      context.sampleRate * 2,
      context.sampleRate,
    );
    const noiseData = noise.getChannelData(0),
      noiseRandom = random(31847);
    for (let i = 0; i < noiseData.length; i++)
      noiseData[i] = noiseRandom() * 2 - 1;

    function track(source) {
      sources.add(source);
      source.onended = () => {
        sources.delete(source);
        try {
          source.disconnect();
        } catch {}
      };
      return source;
    }
    function route(gain, pan = 0, ambience = true) {
      const panner = own(context.createStereoPanner());
      panner.pan.value = pan;
      gain.connect(panner);
      panner.connect(dry);
      if (ambience) panner.connect(reverb);
      return panner;
    }
    function oscillator(
      type,
      frequency,
      at,
      duration,
      destination,
      detune = 0,
    ) {
      const source = track(context.createOscillator());
      source.type = type;
      source.frequency.value = frequency;
      source.detune.value = detune;
      source.connect(destination);
      source.start(at);
      source.stop(at + duration);
      return source;
    }
    function envelope(at, attack, sustain, release, level) {
      const gain = own(context.createGain());
      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(level, at + attack);
      gain.gain.setValueAtTime(level * 0.83, at + attack + sustain);
      gain.gain.exponentialRampToValueAtTime(
        0.00001,
        at + attack + sustain + release,
      );
      return gain;
    }
    function chord(at, stage, duration = BEAT * 4, resolving = false) {
      const notes = HARMONY[clampStage(stage)];
      const attack = resolving ? 0.12 : 0.8;
      const release = resolving ? 1.05 : 1.35;
      const sustain = Math.max(0.1, duration - attack);
      const group = [];
      for (let index = 0; index < notes.length; index++) {
        const note = notes[index];
        const filter = own(context.createBiquadFilter());
        filter.type = "lowpass";
        filter.Q.value = 0.45;
        filter.frequency.setValueAtTime(
          resolving ? 1900 : 700 + stage * 80,
          at,
        );
        filter.frequency.linearRampToValueAtTime(
          resolving ? 1100 : 1500 + stage * 100,
          at + duration,
        );
        const gain = envelope(
          at,
          attack,
          sustain,
          release,
          index === 0 ? 0.033 : 0.026,
        );
        filter.connect(gain);
        route(gain, (index - 2) * 0.27);
        const lifetime = attack + sustain + release + 0.05;
        const primary = oscillator(
          "sawtooth",
          midi(note),
          at,
          lifetime,
          filter,
          index % 2 ? 4 : -4,
        );
        const layerGain = own(context.createGain());
        layerGain.gain.value = 0.24;
        layerGain.connect(filter);
        const secondary = oscillator(
          "sine",
          midi(note + 12),
          at,
          lifetime,
          layerGain,
          index % 2 ? -6 : 6,
        );
        group.push({ gain, sources: [primary, secondary] });
      }
      const item = { group, end: at + duration + release };
      pads.add(item);
      return item;
    }
    function softenPads(at) {
      for (const pad of pads) {
        if (pad.end < at) {
          pads.delete(pad);
          continue;
        }
        for (const voice of pad.group) {
          const param = voice.gain.gain;
          if (typeof param.cancelAndHoldAtTime === "function")
            param.cancelAndHoldAtTime(at);
          else {
            param.cancelScheduledValues(at);
            param.setValueAtTime(Math.max(0.00001, param.value), at);
          }
          param.exponentialRampToValueAtTime(0.00001, at + 0.8);
          for (const source of voice.sources)
            try {
              source.stop(at + 0.85);
            } catch {}
        }
      }
      pads.clear();
    }
    function bass(at, note, level = 0.09) {
      const gain = envelope(at, 0.008, 0.035, 0.21, level);
      route(gain, 0, false);
      const low = oscillator("sine", midi(note - 12), at, 0.28, gain);
      low.frequency.setValueAtTime(midi(note - 12) * 1.04, at);
      low.frequency.exponentialRampToValueAtTime(midi(note - 12), at + 0.045);
      const harmonic = own(context.createGain());
      harmonic.gain.value = 0.16;
      harmonic.connect(gain);
      oscillator("sawtooth", midi(note), at, 0.26, harmonic);
    }
    function pluck(at, note, pan, emphasis = 1) {
      const filter = own(context.createBiquadFilter());
      filter.type = "lowpass";
      filter.frequency.setValueAtTime(3200, at);
      filter.frequency.exponentialRampToValueAtTime(550, at + 0.5);
      const gain = envelope(at, 0.004, 0.008, 0.21, 0.035 * emphasis);
      filter.connect(gain);
      route(gain, pan);
      oscillator("triangle", midi(note), at, 0.24, filter);
      const bright = own(context.createGain());
      bright.gain.value = 0.19;
      bright.connect(filter);
      oscillator("sawtooth", midi(note + 12), at, 0.24, bright);
    }
    function kick(at, emphasis = 1) {
      const gain = envelope(at, 0.002, 0.018, 0.23, 0.16 * emphasis);
      route(gain, 0, false);
      const source = oscillator("sine", 138, at, 0.27, gain);
      source.frequency.exponentialRampToValueAtTime(43, at + 0.09);
      source.frequency.exponentialRampToValueAtTime(34, at + 0.25);
    }
    function noiseHit(at, kind, level, pan = 0) {
      const source = track(context.createBufferSource());
      source.buffer = noise;
      const filter = own(context.createBiquadFilter());
      filter.type = kind === "snare" ? "bandpass" : "highpass";
      filter.frequency.value = kind === "snare" ? 1850 : 6500;
      filter.Q.value = kind === "snare" ? 0.6 : 0.4;
      const length = kind === "snare" ? 0.14 : 0.045;
      const gain = envelope(at, 0.001, 0.002, length, level);
      source.connect(filter);
      filter.connect(gain);
      route(gain, pan, kind === "snare");
      source.start(at, (Math.floor(at * 997) % 700) / 1000);
      source.stop(at + length + 0.008);
      if (kind === "snare") oscillator("triangle", 178, at, 0.1, gain);
    }
    function metal(at, level = 0.012, pan = 0.35) {
      const gain = envelope(at, 0.002, 0.002, 0.065, level);
      route(gain, pan, false);
      [1830, 2765, 4193].forEach((hz, index) =>
        oscillator("sine", hz, at, 0.075, gain, index * 3),
      );
    }
    function sweep(at, duration = 0.9, level = 0.035) {
      const source = track(context.createBufferSource());
      source.buffer = noise;
      source.loop = true;
      const filter = own(context.createBiquadFilter());
      filter.type = "bandpass";
      filter.Q.value = 0.65;
      filter.frequency.setValueAtTime(250, at);
      filter.frequency.exponentialRampToValueAtTime(5300, at + duration * 0.88);
      const gain = own(context.createGain());
      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(level, at + duration * 0.72);
      gain.gain.exponentialRampToValueAtTime(0.00001, at + duration);
      source.connect(filter);
      filter.connect(gain);
      const pan = route(gain, -0.3);
      pan.pan.linearRampToValueAtTime(0.3, at + duration);
      source.start(at);
      source.stop(at + duration + 0.02);
    }
    function step(at, index, stage) {
      const harmony = HARMONY[clampStage(stage)];
      const position = index % 16;
      if ([0, 6, 8, 14].includes(position))
        kick(at, position === 0 || position === 8 ? 1 : 0.7);
      if (position === 4 || position === 12)
        noiseHit(at, "snare", stage < 2 ? 0.045 : 0.065);
      if (position % 2 === 0)
        noiseHit(
          at,
          "hat",
          position % 4 === 0 ? 0.012 : 0.022,
          position % 4 ? 0.45 : -0.45,
        );
      if ([0, 3, 6, 8, 11, 14].includes(position))
        bass(at, harmony[position === 11 ? 1 : 0], stage < 2 ? 0.08 : 0.1);
      if (stage > 1 || position % 2 === 0) {
        const note =
          harmony[ARP[position] % harmony.length] +
          12 +
          (position === 15 ? 12 : 0);
        pluck(
          at,
          note,
          position % 2 ? 0.42 : -0.42,
          position % 4 === 0 ? 1 : 0.72,
        );
      }
      if (position === 7 || position === 15)
        metal(at, 0.012, position === 7 ? -0.6 : 0.6);
    }
    function cue(kind, at, stage) {
      const harmony = HARMONY[clampStage(stage)];
      if (kind === "confirm") {
        pluck(at, harmony[2] + 12, -0.12, 0.8);
        pluck(at + 0.075, harmony[3] + 12, 0.12, 0.8);
      } else if (kind === "scan") {
        metal(at, 0.012, -0.35);
        metal(at + 0.08, 0.008, 0.35);
      } else if (kind === "reveal") {
        sweep(at, 0.85, 0.045);
        pluck(at + 0.5, 84, 0.1, 0.65);
      } else if (kind === "transition") sweep(at, 0.5, 0.018);
    }
    function dispose() {
      for (const source of sources) {
        try {
          source.stop();
        } catch {}
        try {
          source.disconnect();
        } catch {}
      }
      sources.clear();
      for (const node of nodes)
        try {
          node.disconnect();
        } catch {}
      nodes.clear();
      pads.clear();
    }
    return { master, output, chord, softenPads, step, cue, dispose };
  }

  let enabled = true,
    volume = 0.5,
    stage = 0,
    context = null,
    graph = null;
  let playing = false,
    finishing = false,
    scheduler = null,
    tailTimer = null;
  let startedAt = 0,
    nextStep = 0,
    stepIndex = 0,
    lastPad = -Infinity;
  let epoch = 0;
  function ensureContext() {
    if (context && context.state !== "closed") return context;
    const Context = window.AudioContext || window.webkitAudioContext;
    if (!Context) return null;
    context = new Context({ latencyHint: "interactive" });
    graph = makeGraph(context);
    return context;
  }
  function setMaster(value, duration = 0.08) {
    if (!context || !graph || context.state === "closed") return;
    const gain = graph.master.gain,
      now = context.currentTime;
    if (typeof gain.cancelAndHoldAtTime === "function")
      gain.cancelAndHoldAtTime(now);
    else {
      gain.cancelScheduledValues(now);
      gain.setValueAtTime(gain.value, now);
    }
    gain.linearRampToValueAtTime(value, now + duration);
  }
  function schedule() {
    if (!playing || finishing || !context || context.state !== "running")
      return;
    const now = context.currentTime;
    // Skip overdue notes when window timers are throttled.
    if (nextStep < now - 0.15) nextStep = now + 0.025;
    while (nextStep < now + 0.22) {
      if (nextStep - lastPad >= BEAT * 4 - 0.02) {
        graph.chord(nextStep, stage);
        lastPad = nextStep;
      }
      graph.step(nextStep, stepIndex++, stage);
      nextStep += GRID;
    }
  }
  async function unlock(options = {}) {
    if ("enabled" in options) enabled = options.enabled === true;
    if (!enabled) {
      setMaster(0);
      return false;
    }
    try {
      const current = ensureContext();
      if (!current) return false;
      // The standalone preview can need a real gesture. The trusted desktop
      // host may resume this same API when launching the selected animation.
      const resumed = current.resume();
      let timeout;
      try {
        // A denied resume can remain pending. Never let media permission stall
        // the visual gate; another real click can retry later.
        await Promise.race([
          resumed,
          new Promise((resolve) => {
            timeout = setTimeout(resolve, 700);
          }),
        ]);
      } finally {
        clearTimeout(timeout);
      }
      if (current !== context || current.state !== "running") return false;
      setMaster(LEVEL, 0.3);
      return true;
    } catch {
      return false;
    }
  }
  function start() {
    if (!enabled || finishing) return false;
    try {
      const current = ensureContext();
      // Already-permitted playback can start. Otherwise the real click calls unlock().
      if (!current || current.state !== "running") return false;
      if (playing) return true;
      playing = true;
      startedAt = current.currentTime;
      nextStep = startedAt + 0.025;
      lastPad = -Infinity;
      stepIndex = 0;
      setMaster(LEVEL, 0.65);
      schedule();
      scheduler = setInterval(schedule, 80);
      return true;
    } catch {
      return false;
    }
  }
  function setStage(value) {
    const next = clampStage(value);
    if (next === stage) return;
    stage = next;
    if (!playing || finishing || !context || context.state !== "running")
      return;
    const at = context.currentTime + 0.025;
    graph.softenPads(at);
    graph.chord(at, stage);
    lastPad = at;
  }
  function setEnabled(value) {
    enabled = value === true;
    if (!enabled) {
      stop();
      return false;
    }
    if (!finishing) setMaster(enabled ? LEVEL : 0, 0.08);
    return enabled;
  }
  function setVolume(value) {
    volume = Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0.5;
    if (context && graph)
      graph.output.gain.setTargetAtTime(volume, context.currentTime, 0.02);
    return volume;
  }
  function cue(kind) {
    if (
      !enabled ||
      !playing ||
      finishing ||
      !context ||
      context.state !== "running"
    )
      return;
    if (["confirm", "transition", "scan", "reveal"].includes(kind))
      graph.cue(kind, context.currentTime + 0.01, stage);
  }
  function stop() {
    epoch++;
    if (scheduler) clearInterval(scheduler);
    if (tailTimer) clearTimeout(tailTimer);
    scheduler = tailTimer = null;
    playing = finishing = false;
    if (graph) graph.dispose();
    graph = null;
    const old = context;
    context = null;
    if (old && old.state !== "closed") old.close().catch(() => {});
  }
  function finish() {
    if (!playing || !context || !graph) {
      stop();
      return;
    }
    if (finishing) return;
    finishing = true;
    if (scheduler) clearInterval(scheduler);
    scheduler = null;
    const token = epoch,
      now = context.currentTime;
    graph.softenPads(now);
    if (enabled) {
      graph.chord(now + 0.015, 5, 0.55, true);
      graph.cue("confirm", now + 0.08, 5);
      const gain = graph.master.gain;
      if (typeof gain.cancelAndHoldAtTime === "function")
        gain.cancelAndHoldAtTime(now);
      else {
        gain.cancelScheduledValues(now);
        gain.setValueAtTime(LEVEL, now);
      }
      gain.setValueAtTime(LEVEL, now + 0.3);
      gain.linearRampToValueAtTime(0, now + 1.25);
    }
    tailTimer = setTimeout(() => {
      if (token === epoch) stop();
    }, 1350);
  }
  window.addEventListener("pagehide", stop);
  window.addEventListener("beforeunload", stop);
  window.__PRISM_AUDIO__ = Object.freeze({
    unlock,
    start,
    setStage,
    setEnabled,
    setVolume,
    cue,
    finish,
    stop,
    isUnlocked: () => context?.state === "running",
  });
})();
