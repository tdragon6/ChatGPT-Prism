(() => {
  const root = document.documentElement;
  const controls = document.getElementById("controls");
  const title = document.getElementById("title");
  const secondary = document.getElementById("secondary");
  const status = document.getElementById("status");
  const identity = document.getElementById("identity");
  const progressBar = document.getElementById("progress");
  const counter = document.getElementById("counter");
  const stageNumber = document.getElementById("stage-number");
  const keyLabel = document.getElementById("key-label");
  const keyCount = document.getElementById("key-count");
  const operatorLabel = document.getElementById("operator-label");
  const sessionLabel = document.getElementById("session-label");
  const instrumentCode = document.getElementById("instrument-code");
  const segments = [...document.querySelectorAll("#segments span")];
  const sound = document.getElementById("sound");
  const skip = document.getElementById("skip");
  const chime = document.getElementById("chime");
  const abort = new AbortController();
  const parented = window.parent !== window;
  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");

  const words = {
    zh: {
      name: "密钥协议",
      stages: ["捕获密钥", "分段解密", "核验身份", "通道已解锁"],
      statuses: ["截获加密载波", "读取分段密钥", "核对授权指纹", "安全通道已建立"],
      key: "密钥分段",
      operator: "操作员",
      session: "会话",
      controls: "动画控制",
      mute: "关闭音乐",
      unmute: "开启音乐",
      skip: "跳过启动动画",
    },
    en: {
      name: "Key Protocol",
      stages: ["Acquire key", "Decrypting", "Verify identity", "Channel open"],
      statuses: ["INTERCEPTING CARRIER", "READING KEY FRAGMENTS", "CHECKING AUTHORIZATION", "SECURE CHANNEL ESTABLISHED"],
      key: "KEY FRAGMENTS",
      operator: "OPERATOR",
      session: "SESSION",
      controls: "Animation controls",
      mute: "Mute music",
      unmute: "Play music",
      skip: "Skip startup animation",
    },
  };
  const stageTags = ["SIGNAL ACQUISITION", "CIPHER RECOVERY", "ACCESS CHALLENGE", "AUTHORIZATION ACCEPTED"];
  const instrumentTags = ["SYNC 01", "SCAN 02", "AUTH 03", "OPEN 04"];
  const keyParts = ["7D", "44", "C7", "0F", "A2", "91"];
  const releaseTimes = [1060, 1710, 2370, 3040, 3730, 4370];
  const duration = 7550;

  let started = false;
  let finished = false;
  let frame = 0;
  let previous = 0;
  let elapsed = 0;
  let stage = -1;
  let lastKeyFrame = -1;
  let reduced = motion.matches;
  let speed = 1;
  let configuredSound = true;
  let soundOn = true;
  let hostSkip = false;
  let copy = words.zh;
  let currentContext = {};

  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
  const fraction = (value, from, to) => clamp((value - from) / (to - from), 0, 1);
  const ease = (value) => value * value * (3 - 2 * value);

  function applySound() {
    sound.textContent = soundOn ? "AUDIO ON" : "AUDIO OFF";
    sound.setAttribute("aria-pressed", String(soundOn));
    sound.setAttribute("aria-label", soundOn ? copy.mute : copy.unmute);
    chime.muted = !soundOn;
  }

  function applyContext(nextContext = {}) {
    const parameters = { ...currentContext.parameters, ...nextContext.parameters };
    const context = { ...currentContext, ...nextContext, parameters };
    currentContext = context;
    const locale = context.locale || (navigator.language.startsWith("zh") ? "zh" : "en");
    const nextCopy = locale === "zh" ? words.zh : words.en;
    if (copy !== nextCopy) stage = -1;
    copy = nextCopy;
    root.lang = locale === "zh" ? "zh" : "en";
    root.dataset.theme = context.theme || (systemTheme.matches ? "dark" : "light");
    document.title = copy.name;
    keyLabel.textContent = copy.key;
    operatorLabel.textContent = copy.operator;
    sessionLabel.textContent = copy.session;
    controls.setAttribute("aria-label", copy.controls);
    skip.setAttribute("aria-label", copy.skip);
    hostSkip = Boolean(context.hostControls?.skip);
    skip.toggleAttribute("data-host-owned", hostSkip);
    skip.tabIndex = hostSkip ? -1 : 0;
    if (hostSkip) skip.setAttribute("aria-hidden", "true");
    else skip.removeAttribute("aria-hidden");

    reduced = context.reducedMotion || motion.matches;
    speed = clamp(Number(parameters.speed) || 1, 0.5, 2);
    const nextSound = parameters.sound !== false;
    if (!started || configuredSound !== nextSound) soundOn = nextSound;
    configuredSound = nextSound;
    chime.volume = typeof parameters.volume === "number" ? clamp(parameters.volume, 0, 1) : 0.5;
    chime.playbackRate = speed;
    identity.textContent = parameters.identity || "tdragon6";
    applySound();
  }

  function draw(value) {
    const time = value * duration;
    const scan = ease(fraction(time, 0, 4600));
    const unlock = reduced ? Number(time >= 5610) : ease(fraction(time, 5610, 6050));
    root.style.setProperty("--entry", reduced ? 1 : ease(fraction(time, 0, 530)));
    root.style.setProperty("--energy", reduced ? 0.45 : scan);
    root.style.setProperty("--turn", `${reduced ? 36 : -52 + 334 * scan + 42 * unlock}deg`);
    root.style.setProperty("--inner-turn", `${reduced ? 0 : 18 - 160 * scan}deg`);
    root.style.setProperty("--lift", `${-11 * unlock}px`);
    root.style.setProperty("--tilt", `${-27 * unlock}deg`);
    root.style.setProperty("--scan", `${reduced ? 50 : 13 + (time % 1550) / 1550 * 72}%`);
    root.style.setProperty("--exit", parented && !reduced ? 1 - ease(fraction(time, 7250, duration)) : 1);

    const next = time < 1020 ? 0 : time < 4370 ? 1 : time < 5610 ? 2 : 3;
    if (next !== stage) {
      stage = next;
      root.dataset.stage = String(stage);
      stageNumber.textContent = `0${stage + 1}`;
      title.textContent = copy.stages[stage];
      status.textContent = copy.statuses[stage];
      secondary.textContent = stageTags[stage];
      instrumentCode.textContent = instrumentTags[stage];
    }

    const keyFrame = Math.floor(time / (reduced ? 390 : 72));
    if (keyFrame !== lastKeyFrame) {
      lastKeyFrame = keyFrame;
      const solved = releaseTimes.filter((point) => time >= point).length;
      keyCount.textContent = `0${solved} / 06`;
      segments.forEach((segment, index) => {
        const current = index === solved && time >= 1020;
        segment.dataset.state = index < solved ? "ready" : current ? "current" : "pending";
        segment.textContent = index < solved ? keyParts[index] : current && !reduced
          ? ((keyFrame * 157 + index * 37) % 256).toString(16).toUpperCase().padStart(2, "0")
          : "– –";
      });
    }
    progressBar.style.transform = `scaleX(${value})`;
    counter.textContent = `${String(Math.floor(value * 100)).padStart(2, "0")}%`;
  }

  function stop(reason) {
    if (finished) return;
    finished = true;
    cancelAnimationFrame(frame);
    chime.pause();
    abort.abort();
    if (parented && reason) parent.postMessage({ type: "prism:done", reason }, "*");
  }

  function advance(now) {
    if (previous) elapsed += (now - previous) * speed;
    previous = now;
    const value = clamp(elapsed / (reduced ? 2400 : duration), 0, 1);
    draw(value);
    if (value === 1) stop("complete");
    else frame = requestAnimationFrame(advance);
  }

  function start(context) {
    if (started) return;
    applyContext(context);
    started = true;
    draw(0);
    if (soundOn) chime.play().catch(() => {});
    frame = requestAnimationFrame(advance);
  }

  const options = { signal: abort.signal };
  sound.addEventListener("click", () => {
    soundOn = !soundOn;
    applySound();
    if (soundOn && chime.paused && !chime.ended) {
      if (Number.isFinite(chime.duration)) chime.currentTime = Math.min(elapsed / 1000, chime.duration);
      chime.play().catch(() => {});
    }
  }, options);
  skip.addEventListener("click", () => stop("skip"), options);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && started) stop("skip");
  }, options);
  window.addEventListener("pagehide", () => stop(), options);
  window.addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const { type, context } = event.data || {};
    if (type === "prism:init") start(context);
    if (type === "prism:update" && started) {
      const wasSoundOn = soundOn;
      applyContext(context);
      if (wasSoundOn !== soundOn && soundOn && chime.paused && !chime.ended) chime.play().catch(() => {});
    }
    if (type === "prism:dispose") stop();
  }, options);

  if (window.__PRISM_CONTEXT__ || !parented) start(window.__PRISM_CONTEXT__);
  else parent.postMessage({ type: "prism:ready" }, "*");
})();
