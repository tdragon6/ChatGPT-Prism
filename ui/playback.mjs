import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./playback.css";

const scene = document.getElementById("scene");
const skip = document.getElementById("skip");
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
let frame;
let payload;
let prepared = false;
let initialized = false;
let playing = false;
let disposed = false;
let geometry;
let themePatch;

function post(type) {
  frame?.contentWindow?.postMessage({ type, context: payload.context }, "*");
}

function clearFrame() {
  if (initialized) post("prism:dispose");
  frame?.remove();
  frame = undefined;
  prepared = false;
  initialized = false;
}

function finish(reason) {
  if (disposed) return;
  disposed = true;
  clearFrame();
  void invoke("finish_playback", { reason }).catch(() => {});
}

function present() {
  playing = true;
  if (!prepared || initialized || disposed) return;
  initialized = true;
  skip.hidden = false;
  post("prism:init");
}

function updateTheme(context) {
  if (!payload || disposed) return;
  payload.context = { ...payload.context, ...context, reducedMotion: reducedMotion.matches };
  document.documentElement.dataset.theme = payload.context.theme;
  if (initialized) post("prism:update");
}

function clip(next) {
  const { clipRects, clipped = Array.isArray(clipRects) } = next;
  if (!clipped) {
    scene.style.clipPath = "";
    return;
  }
  const rects = (Array.isArray(clipRects) ? clipRects : [])
    .filter(({ x, y, width, height }) => [x, y, width, height].every(Number.isFinite) && width > 0 && height > 0);
  const path = rects.map(({ x, y, width, height }) => `M${x} ${y}h${width}v${height}h${-width}Z`).join(" ");
  scene.style.clipPath = path ? `path("${path}")` : "inset(100%)";
}

function mount(next) {
  if (!next?.html || disposed) return;
  if (initialized) {
    updateTheme({ ...next.context, ...themePatch });
    return;
  }
  clearFrame();
  payload = next;
  updateTheme({ ...next.context, ...themePatch });
  clip(geometry || next);
  document.documentElement.lang = payload.context.locale === "en" ? "en" : "zh";
  skip.setAttribute("aria-label", document.documentElement.lang === "en" ? "Skip startup animation" : "跳过启动动画");
  frame = document.createElement("iframe");
  frame.setAttribute("aria-label", document.documentElement.lang === "en" ? "Startup animation" : "启动动画");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("allow", "autoplay");
  frame.referrerPolicy = "no-referrer";
  frame.srcdoc = payload.html;
  scene.prepend(frame);
  if (next.playing) present();
}

window.addEventListener("message", (event) => {
  if (event.source !== frame?.contentWindow || disposed) return;
  if (event.data?.type === "prism:ready" && !prepared) {
    prepared = true;
    void invoke("playback_ready").catch(() => finish("failed"));
    if (playing) present();
  } else if (initialized && event.data?.type === "prism:done") {
    finish(event.data.reason === "skip" ? "skip" : "complete");
  }
});
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && playing) finish("skip");
});
window.addEventListener("pagehide", clearFrame);
skip.addEventListener("click", () => finish("skip"));

async function start() {
  await Promise.all([
    listen("prism-present", present),
    listen("prism-theme", ({ payload: next }) => {
      themePatch = next.context || { theme: next.theme };
      updateTheme(themePatch);
    }),
    listen("prism-geometry", ({ payload: next }) => {
      geometry = next;
      clip(next);
    }),
  ]);
  mount(await invoke("get_playback"));
}
start().catch(() => finish("failed"));
