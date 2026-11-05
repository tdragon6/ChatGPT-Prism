/* MIT — ChatGPT Prism contributors. Identity data is local display text. */
(() => {
  "use strict";
  const translations = {
    zh: {
      stages: [
        [
          "需要访问许可",
          "ACCESS PERMISSION REQUIRED",
          "AUTHORIZATION",
          "等待用户授权…",
          "AWAITING OPERATOR CONSENT",
        ],
        [
          "身份资料确认",
          "IDENTITY VERIFICATION",
          "IDENTITY",
          "核对身份资料，确认访问",
          "CONFIRM YOUR LOCAL IDENTITY PROFILE",
        ],
        [
          "接收访问需求",
          "ACCESS REQUEST RECEIVED",
          "REQUEST",
          "访问需求已接收，正在建立会话…",
          "ACCESS REQUEST ACCEPTED · PREPARING SESSION",
        ],
        [
          "开始读取权限",
          "PERMISSION READ STARTED",
          "READ",
          "开始读取权限，索引工作台界面…",
          "READING THE INTERFACE ACCESS MATRIX",
        ],
        [
          "读取完毕 · 核验通过",
          "READ COMPLETE · VERIFIED",
          "VERIFY",
          "读取完毕，核验通过，权限已就绪",
          "VERIFICATION COMPLETE · ACCESS READY",
        ],
        [
          "访问权限已就绪",
          "SESSION ACCESS AUTHORIZED",
          "WELCOME",
          "身份验证完成，请进入工作台",
          "IDENTITY VERIFIED · ENTERING WORKSPACE",
        ],
      ],
      gateLabels: [
        ["请求解析", "许可核验", "通道就绪"],
        ["资料读取", "编号匹配", "身份确认"],
      ],
      gateAria: ["访问许可核验进度", "身份认证进度"],
      gateReady: ["访问许可已就绪", "身份认证已完成"],
      gateWaiting: "等待确认",
      gateChecking: "核验中",
      soundOn: "开启配乐与音效",
      soundOff: "关闭配乐与音效",
      welcome: (name) => `欢迎 ${name} 访问`,
      verifyStatus: ["正在核验访问许可…", "正在认证身份资料…"],
      action: ["授予访问权限 ", "确认身份 "],
      actionKey: ["GRANT ACCESS", "CONFIRM IDENTITY"],
      soundHint: "Enter / Space · 点击确认并启用配乐",
      actionHint: "Enter / Space · 或点击按钮继续",
      verifying: ["访问许可核验中…", "身份认证中…"],
      completing: "正在完成核验",
      verified: ["✓ 访问许可已授予", "✓ 身份认证已完成"],
      verifiedStatus: ["访问许可已授予", "身份认证已完成"],
    },
    en: {
      stages: [
        [
          "ACCESS REQUIRED",
          "AUTHORIZE YOUR CREATIVE SESSION",
          "AUTHORIZATION",
          "Awaiting your confirmation…",
          "PRESS ENTER OR SELECT GRANT ACCESS",
        ],
        [
          "VERIFY IDENTITY",
          "CONFIRM YOUR LOCAL PROFILE",
          "IDENTITY",
          "Review your identity record",
          "OPERATOR NAME · IDENTITY ID · CLEARANCE",
        ],
        [
          "REQUEST ACCEPTED",
          "ESTABLISHING A CREATIVE SESSION",
          "REQUEST",
          "Your access request has been received",
          "PREPARING THE LOCAL WORKSPACE",
        ],
        [
          "READING ACCESS",
          "INDEXING THE WORKSPACE INTERFACE",
          "READ",
          "Reading the workspace access profile…",
          "EDITOR · CONTEXT · TERMINAL · VERSION CONTROL",
        ],
        [
          "VERIFIED",
          "ALL CHECKS COMPLETE",
          "VERIFY",
          "Access checks are complete",
          "YOUR CREATIVE SESSION IS READY",
        ],
        [
          "SESSION READY",
          "YOUR WORKSPACE AWAITS",
          "WELCOME",
          "Welcome to your workspace",
          "ACCESS AUTHORIZED · HANDING OVER CONTROL",
        ],
      ],
      gateLabels: [
        ["REQUEST", "PERMISSION", "READY"],
        ["PROFILE", "ID MATCH", "VERIFIED"],
      ],
      gateAria: ["Access permission verification progress", "Identity verification progress"],
      gateReady: ["Permission granted", "Identity verified"],
      gateWaiting: "AWAITING CONFIRMATION",
      gateChecking: "CHECKING",
      soundOn: "Enable music and sound effects",
      soundOff: "Mute music and sound effects",
      welcome: (name) => `WELCOME, ${name}`,
      verifyStatus: ["Verifying access permission…", "Verifying the identity record…"],
      action: ["Grant access ", "Confirm identity "],
      actionKey: ["ENTER ↵", "ENTER ↵"],
      soundHint: "Enter / Space · Confirm to enable music",
      actionHint: "Enter / Space · Or select the button",
      verifying: ["Verifying permission…", "Verifying identity…"],
      completing: "Completing verification",
      verified: ["✓ Permission granted", "✓ Identity verified"],
      verifiedStatus: ["Permission has been granted", "Your identity is verified"],
    },
  };

  function init() {
    const copy = translations[document.documentElement.lang.startsWith("zh") ? "zh" : "en"];
    const context = window.__PRISM_CONTEXT__ || {};
    const source = context.parameters || {};
    const previewTheme = window.parent === window
      ? new URLSearchParams(location.search).get("theme")
      : null;
    const identity =
      typeof source.identity === "string"
        ? source.identity.trim().normalize("NFC")
        : "";
    const config = {
      name: /^[\p{L}\p{N} ._-]{1,32}$/u.test(identity) ? identity : "tdragon6",
      identityId: "0001",
      clearance: "03",
      theme: (context.theme ?? previewTheme) === "dark" ? "dark" : "light",
      speed:
        typeof source.speed === "number" &&
        source.speed >= 0.75 &&
        source.speed <= 2
          ? source.speed
          : 1,
      sound: source.sound !== false,
      volume: Number.isFinite(source.volume)
        ? Math.max(0, Math.min(1, source.volume))
        : 0.5,
      preview: context.preview === true || window.parent === window,
    };
    let configuredSound = config.sound;
    let voiceActive = false;
    const $ = (id) => document.getElementById(id),
      root = $("startup"),
      sound = window.__PRISM_AUDIO__,
      voice = window.__PRISM_VOICE__;
    if (!root) return;
    if (window.__PRISM_CONTEXT__?.hostControls?.skip) {
      $("skip-button").style.visibility = "hidden";
      $("skip-button").style.pointerEvents = "none";
      $("skip-button").tabIndex = -1;
      $("skip-button").setAttribute("aria-hidden", "true");
    }
    const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const timers = new Set();
    let generation = 0,
      step = 0,
      done = false,
      waiting = null,
      raf = 0;
    let progress = 0,
      progressFrom = 0,
      progressTarget = 0,
      progressAt = 0,
      progressDuration = 1,
      lastInteger = -1;
    let lastTelemetry = 0,
      packets = 0,
      seed = 7193;
    let gateState = { stage: "permission", progress: 0, state: "hidden" },
      gateFrom = 0,
      gateTarget = 0,
      gateAt = 0,
      gateDuration = 1;
    const delay = (ms) =>
      new Promise((resolve) => {
        const timer = setTimeout(() => {
          timers.delete(timer);
          resolve();
        }, ms / config.speed);
        timers.add(timer);
      });
    function setPhase(value) {
      root.dataset.phase = value;
    }
    function moveProgress(target, ms) {
      progressFrom = progress;
      progressTarget = target;
      progressAt = performance.now();
      progressDuration = Math.max(1, ms / config.speed);
    }
    function updateProgress(now) {
      const elapsed = Math.min(1, (now - progressAt) / progressDuration);
      progress = progressFrom + (progressTarget - progressFrom) * elapsed;
      $("progress").style.width = `${progress.toFixed(3)}%`;
      root.style.setProperty("--progress", `${progress.toFixed(3)}%`);
      const integer = Math.floor(progress);
      if (integer !== lastInteger) {
        lastInteger = integer;
        $("progress-percent").replaceChildren(
          document.createTextNode(String(integer).padStart(3, "0")),
        );
        const span = document.createElement("span");
        span.textContent = "%";
        $("progress-percent").append(span);
        root
          .querySelector(".ruler")
          .setAttribute("aria-valuenow", String(integer));
      }
    }
    function resetGate(index) {
      gateState = {
        stage: index === 0 ? "permission" : "identity",
        progress: 0,
        state: "waiting",
      };
      gateFrom = gateTarget = 0;
      gateAt = performance.now();
      gateDuration = 1;
      const panel = $("gate-progress");
      panel.hidden = false;
      panel.dataset.stage = gateState.stage;
      panel.dataset.state = "waiting";
      panel.setAttribute(
        "aria-label",
        copy.gateAria[index],
      );
      root.dataset.gateState = "waiting";
      $("gate-title").textContent =
        index === 0 ? "ACCESS PERMISSION" : "IDENTITY AUTHENTICATION";
      panel.querySelectorAll(".gate-checks b").forEach((item, i) => {
        item.textContent = copy.gateLabels[index][i];
      });
      updateGate(performance.now());
    }
    function moveGate(target, ms) {
      gateFrom = gateState.progress;
      gateTarget = target;
      gateAt = performance.now();
      gateDuration = Math.max(1, ms / config.speed);
    }
    function gateCurve(t) {
      // Brief scan holds separate the three visible checks without reversing progress.
      const stops = [
        [0, 0],
        [0.23, 0.32],
        [0.29, 0.32],
        [0.66, 0.79],
        [0.72, 0.79],
        [1, 1],
      ];
      for (let i = 1; i < stops.length; i++)
        if (t <= stops[i][0]) {
          const [a, b] = stops[i - 1],
            [c, d] = stops[i];
          return b + ((d - b) * (t - a)) / (c - a);
        }
      return 1;
    }
    function updateGate(now) {
      if (gateState.state === "hidden") return;
      const fraction = gateCurve(
        Math.min(1, Math.max(0, (now - gateAt) / gateDuration)),
      );
      gateState.progress = gateFrom + (gateTarget - gateFrom) * fraction;
      const value =
        gateState.state === "complete"
          ? 100
          : Math.min(99, Math.floor(gateState.progress));
      const panel = $("gate-progress");
      panel.setAttribute("aria-valuenow", String(value));
      $("gate-progress-fill").style.width = `${gateState.progress.toFixed(3)}%`;
      $("gate-front").style.left = `${gateState.progress.toFixed(3)}%`;
      $("gate-percent").textContent = String(value).padStart(3, "0");
      const index = gateState.stage === "identity" ? 1 : 0,
        active = value < 32 ? 0 : value < 79 ? 1 : 2;
      $("gate-status").textContent =
        gateState.state === "complete"
          ? copy.gateReady[index]
          : `${copy.gateLabels[index][active]} · ${gateState.state === "waiting" && value >= 70 ? copy.gateWaiting : copy.gateChecking}`;
      panel.querySelectorAll(".gate-checks>span").forEach((item, i) => {
        item.classList.toggle("checked", value >= [32, 79, 100][i]);
        item.classList.toggle("checking", i === active && value < 100);
      });
    }
    function randomByte() {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return ((seed >>> 18) & 255).toString(16).padStart(2, "0").toUpperCase();
    }
    function telemetry(now) {
      if (now - lastTelemetry < 150 / config.speed) return;
      lastTelemetry = now;
      $("clock").textContent = new Date().toLocaleTimeString("en-GB", {
        hour12: false,
      });
      if (step >= 3 && step <= 5) {
        $("hex-stream").textContent = Array.from({ length: 6 }, () =>
          Array.from({ length: 8 }, randomByte).join(" "),
        ).join("\n");
        packets += step === 4 ? 23 : 7;
        $("packet-counter").textContent =
          `PACKET / ${String(packets).padStart(6, "0")}`;
        $("sequence-code").textContent =
          `${config.identityId}.${String(packets % 100).padStart(2, "0")}`;
      }
      if (step === 3 || step === 4) {
        const fraction = Math.min(1, Math.max(0, (progress - 31) / 43)),
          loaded = Math.floor(fraction * 8);
        $("workspace-count").textContent = String(Math.min(4, loaded)).padStart(
          2,
          "0",
        );
        $("system-count").textContent = String(
          Math.max(0, loaded - 4),
        ).padStart(2, "0");
        $("dossier-percent").textContent =
          `${String(Math.floor(fraction * 100)).padStart(3, "0")}%`;
        root
          .querySelectorAll(".dossier-columns li")
          .forEach((item, index) =>
            item.classList.toggle("loaded", index < loaded),
          );
      }
    }
    function frame(now) {
      if (done) return;
      updateProgress(now);
      updateGate(now);
      telemetry(now);
      raf = requestAnimationFrame(frame);
    }
    function updateSound() {
      const active = sound?.isUnlocked();
      $("sound-button").textContent = config.sound
        ? active
          ? "AUDIO ON"
          : "AUDIO READY"
        : "AUDIO OFF";
      $("sound-button").setAttribute("aria-pressed", String(config.sound));
      $("sound-button").setAttribute(
        "aria-label",
        config.sound ? copy.soundOff : copy.soundOn,
      );
    }
    function configureVoice() {
      voice?.configure({ enabled: config.sound, volume: config.volume, speed: config.speed,
        duck: (active) => {
          voiceActive = active;
          sound?.setVolume(config.volume * (active ? 0.34 : 1));
        } });
    }
    function unlockSound(cue = true) {
      const token = generation;
      // Try the selected soundtrack immediately; a gesture or trusted host can
      // retry if the browser initially suspends media playback.
      const attempt = sound?.unlock({ enabled: config.sound });
      return Promise.resolve(attempt).then((allowed) => {
        if (done || token !== generation) return false;
        if (allowed) {
          sound.start();
          sound.setStage(Math.max(0, step - 1));
          if (cue) sound.cue("confirm");
          configureVoice();
          void voice?.retry();
        }
        updateSound();
        return !!allowed;
      });
    }
    function setHeading(text) {
      const heading = $("stage-title");
      heading.replaceChildren();
      [...text].forEach((character, index) => {
        const span = document.createElement("span");
        span.className = "title-glyph";
        span.textContent = character;
        span.style.setProperty(
          "--delay",
          `${reduced ? 0 : (index * 34) / config.speed}ms`,
        );
        heading.append(span);
      });
      const cursor = document.createElement("span");
      cursor.className = "cursor";
      cursor.setAttribute("aria-hidden", "true");
      heading.append(cursor);
    }
    function setSubtitle(text) {
      const subtitle = $("stage-subtitle");
      subtitle.replaceChildren();
      [...text].forEach((character, index) => {
        const span = document.createElement("span");
        span.className = "letter";
        span.textContent = character;
        span.style.setProperty(
          "--delay",
          `${reduced ? 0 : (index * 18) / config.speed}ms`,
        );
        subtitle.append(span);
      });
    }
    function showStage(index) {
      step = index + 1;
      if (index < 5) void voice?.play(["permission", "identity", "verified", "initialize", "granted"][index]);
      root.dataset.step = String(step);
      const [title, subtitle, code, status, statusEn] = copy.stages[index];
      setHeading(title);
      setSubtitle(subtitle);
      $("stage-kicker").textContent =
        `${String(step).padStart(2, "0")} / ${code}`;
      $("step-label").textContent =
        `[ ${String(step).padStart(2, "0")} / 06 ] ${code}`;
      $("status-zh").textContent = status;
      $("status-en").textContent = statusEn;
      $("progress-state").textContent = `ACCESS SEQUENCE / ${code}`;
      $("ledger-status").textContent =
        code === "VERIFY" ? "ACCESS GRANTED" : code;
      $("buffer-state").textContent =
        step === 5 ? "VERIFIED" : step >= 3 ? "READING" : "STANDBY";
      $("identity-card").hidden = index !== 1;
      const identityName = $("identity-name");
      identityName.textContent = config.name;
      identityName.tabIndex = 0;
      identityName.parentElement.dataset.tooltip = config.name;
      $("identity-id").textContent = config.identityId;
      $("identity-level").textContent = config.clearance;
      $("session-label").textContent = `· ID CONFIRMED : ${config.identityId}`;
      $("module-dossier").hidden = !(index === 2 || index === 3);
      $("readout-panel").hidden = !(index === 2 || index === 3);
      $("continue-button").hidden = index > 1;
      $("interaction-hint").hidden = index > 1;
      $("welcome-name").textContent = copy.welcome(config.name);
      $("gate-progress").hidden = index > 1;
      if (index > 1) {
        gateState.state = "hidden";
        root.dataset.gateState = "hidden";
      }
      root
        .querySelectorAll(".segments i")
        .forEach((item, i) => item.classList.toggle("active", i <= index));
      sound?.setStage(index);
      sound?.cue(index === 3 ? "scan" : "transition");
      root.querySelector(".center").classList.remove("changing");
    }
    function confirmGate() {
      if (!waiting || done || gateState.state !== "waiting") return;
      unlockSound();
      $("continue-button").disabled = true;
      $("status-zh").textContent =
        copy.verifyStatus[step - 1];
      $("status-en").textContent =
        step === 1
          ? "VERIFYING ACCESS PERMISSION"
          : "AUTHENTICATING IDENTITY RECORD";
      const resolve = waiting;
      waiting = null;
      resolve();
    }
    async function gate(index, token) {
      resetGate(index);
      const button = $("continue-button");
      button.disabled = false;
      button.hidden = true;
      $("interaction-hint").hidden = true;
      button.replaceChildren(
        document.createTextNode(copy.action[index]),
      );
      const sub = document.createElement("span");
      sub.textContent = copy.actionKey[index];
      button.append(sub);
      $("interaction-hint").textContent = config.sound ? copy.soundHint : copy.actionHint;
      root.dataset.awaiting = "auto";
      const maximum = 400;
      moveProgress(index === 0 ? 14 : 28, maximum);
      moveGate(70, maximum);
      await Promise.race([
        new Promise((resolve) => {
          waiting = resolve;
        }),
        delay(maximum),
      ]);
      waiting = null;
      root.dataset.awaiting = "none";
      if (token !== generation || done) return;
      button.disabled = true;
      button.textContent = copy.verifying[index];
      $("interaction-hint").textContent = copy.completing;
      gateState.state = "running";
      $("gate-progress").dataset.state = "running";
      root.dataset.gateState = "running";
      moveGate(100, 1600);
      moveProgress(index === 0 ? 17 : 31, 1600);
      await delay(1600);
      if (token !== generation || done) return;
      gateState.state = "complete";
      gateState.progress = gateFrom = gateTarget = 100;
      $("gate-progress").dataset.state = "complete";
      root.dataset.gateState = "complete";
      updateGate(performance.now());
      button.textContent =
        copy.verified[index];
      $("interaction-hint").textContent = "VERIFICATION COMPLETE";
      $("status-zh").textContent =
        copy.verifiedStatus[index];
      $("status-en").textContent =
        index === 0
          ? "ACCESS PERMISSION GRANTED"
          : "IDENTITY AUTHENTICATION COMPLETE";
      await delay(450);
      if (token !== generation || done) return;
      button.hidden = true;
      $("interaction-hint").hidden = true;
    }
    function cancelWork() {
      cancelAnimationFrame(raf);
      timers.forEach(clearTimeout);
      timers.clear();
      if (waiting) {
        waiting();
        waiting = null;
      }
    }
    function finish(reason = "complete") {
      if (done) return;
      done = true;
      generation++;
      cancelWork();
      sound?.stop();
      voice?.stop();
      setPhase("done");
      root.style.display = "none";
      if (config.preview) {
        $("preview-end").dataset.theme = config.theme;
        $("preview-end").hidden = false;
      }
      const message = { type: "prism:done", reason };
      if (window.parent !== window) window.parent.postMessage(message, "*");
    }
    async function run() {
      generation++;
      const token = generation;
      cancelWork();
      sound?.stop();
      voice?.stop();
      done = false;
      voice?.reset();
      configureVoice();
      step = 0;
      progress = progressFrom = progressTarget = 0;
      lastInteger = -1;
      packets = 0;
      lastTelemetry = 0;
      root.style.display = "";
      root.dataset.theme = config.theme;
      root.removeAttribute("data-step");
      root.dataset.awaiting = "none";
      root.style.setProperty("--speed", String(config.speed));
      gateState = { stage: "permission", progress: 0, state: "hidden" };
      root.dataset.gateState = "hidden";
      $("gate-progress").hidden = true;
      $("preview-end").hidden = true;
      $("identity-card").hidden = true;
      $("module-dossier").hidden = true;
      $("continue-button").hidden = true;
      $("interaction-hint").hidden = true;
      $("readout-panel").hidden = true;
      $("hex-stream").textContent = "";
      $("progress").style.width = "0%";
      sound?.setVolume(config.volume);
      sound?.setEnabled(config.sound);
      configureVoice();
      if (config.sound) void unlockSound(false);
      updateSound();
      setPhase("intro");
      moveProgress(4, reduced ? 300 : 2400);
      raf = requestAnimationFrame(frame);
      await delay(reduced ? 300 : 2400);
      if (token !== generation || done) return;
      setPhase("wash");
      await delay(reduced ? 100 : 600);
      if (token !== generation || done) return;
      setPhase("sequence");
      for (let index = 0; index < 6; index++) {
        if (token !== generation || done) return;
        showStage(index);
        if (index < 2) await gate(index, token);
        else {
          const ms = [0, 0, 2300, 3600, 2900, 750][index];
          moveProgress([0, 0, 46, 76, 93, 97][index], ms);
          await delay(ms);
        }
        if (token !== generation || done) return;
        root.querySelector(".center").classList.add("changing");
        if (index < 5) await delay(230);
      }
      if (token !== generation || done) return;
      setPhase("transition");
      sound?.cue("reveal");
      moveProgress(99, 1100);
      await delay(reduced ? 100 : 1200);
      if (token !== generation || done) return;
      setPhase("welcome");
      void voice?.play("online");
      moveProgress(100, 700);
      await delay(1400);
      if (token !== generation || done) return;
      setPhase("reveal");
      sound?.finish();
      // Preserve the musical cadence even at faster visual speeds.
      await new Promise((resolve) => {
        const timer = setTimeout(
          () => {
            timers.delete(timer);
            resolve();
          },
          Math.max(1400, 1700 / config.speed),
        );
        timers.add(timer);
      });
      if (token !== generation || done) return;
      finish();
    }
    $("skip-button").addEventListener("click", () => finish("skip"));
    $("continue-button").addEventListener("click", confirmGate);
    $("sound-button").addEventListener("click", () => {
      if (config.sound && sound?.isUnlocked()) config.sound = false;
      else config.sound = true;
      sound?.setEnabled(config.sound);
      configureVoice();
      if (config.sound) unlockSound();
      updateSound();
    });
    $("replay-button").addEventListener("click", () => run());
    $("theme-button").addEventListener("click", () => {
      config.theme = config.theme === "light" ? "dark" : "light";
      run();
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        finish("skip");
      } else if (
        (event.key === "Enter" || event.key === " ") &&
        waiting &&
        event.target.tagName !== "BUTTON"
      ) {
        event.preventDefault();
        confirmGate();
      }
    });
    window.addEventListener("pagehide", () => {
      cancelWork();
      sound?.stop();
      voice?.stop();
    });
    window.addEventListener("message", (event) => {
      if (event.source !== parent) return;
      if (event.data?.type === "prism:update") {
        const theme = event.data.context?.theme;
        if (theme === "dark" || theme === "light") {
          config.theme = theme;
          root.dataset.theme = theme;
          $("preview-end").dataset.theme = theme;
        }
        const parameters = event.data.context?.parameters;
        let soundChanged = false;
        if (typeof parameters?.sound === "boolean" && parameters.sound !== configuredSound) {
          configuredSound = parameters.sound;
          soundChanged = config.sound !== configuredSound;
          config.sound = configuredSound;
          if (soundChanged) sound?.setEnabled(config.sound);
        }
        const volume = Number.isFinite(parameters?.volume)
          ? Math.max(0, Math.min(1, parameters.volume))
          : config.volume;
        const volumeChanged = volume !== config.volume;
        if (volumeChanged) {
          config.volume = volume;
          sound?.setVolume(config.volume * (voiceActive ? 0.34 : 1));
        }
        if (soundChanged || volumeChanged) {
          configureVoice();
          if (soundChanged && config.sound && !done) void unlockSound(false);
          updateSound();
        }
      }
      if (event.data?.type === "prism:dispose") {
        done = true;
        generation++;
        cancelWork();
        sound?.stop();
        voice?.stop();
      }
    });
    run();
  }
  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", init, { once: true });
  else init();
})();
