import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./studio-preview.css";

const text = {
  zh: {
    title: "工作区预览",
    empty: "选择并启用启动动画",
    unsupported: "此动画不支持当前主题",
    loading: "正在加载启动动画…",
    themeLoading: "正在同步客户端主题…",
    failed: "无法加载启动动画",
    hidden: "预览已暂停",
    ended: "播放完毕",
  },
  en: {
    title: "Workspace preview",
    empty: "Choose and enable a startup animation",
    unsupported: "This animation does not support the selected theme",
    loading: "Loading startup animation…",
    themeLoading: "Syncing client appearance…",
    failed: "Could not load the startup animation",
    hidden: "Preview paused",
    ended: "Preview complete",
  },
};

const PARAMETER_DELAY_MS = 200;
const READY_TIMEOUT_MS = 10_000;
const MIN_PREVIEW_CYCLE_MS = 500;

function usePreviewVisibility() {
  const [documentVisible, setDocumentVisible] = useState(() => !document.hidden);
  const [windowVisible, setWindowVisible] = useState(true);
  useEffect(() => {
    const sync = () => setDocumentVisible(!document.hidden);
    document.addEventListener("visibilitychange", sync);
    sync();
    const subscription = listen("prism-manager-visibility", ({ payload }) => {
      setWindowVisible(payload === true);
      sync();
    });
    void subscription.catch(() => {});
    return () => {
      document.removeEventListener("visibilitychange", sync);
      void subscription.then((unlisten) => unlisten(), () => {});
    };
  }, []);
  return documentVisible && windowVisible;
}

function PreviewMessage({ message }) {
  return (
    <div className="studio-preview-message" role="status">
      <strong>ChatGPT</strong>
      <p>{message}</p>
    </div>
  );
}

function PreviewFrame({ workId, theme, parameters, audioParameters, locale, mode, words, onEnd }) {
  const host = useRef(null);
  const contextPatch = useRef({ theme, audio: JSON.parse(audioParameters) });
  const sendContext = useRef(null);
  const [status, setStatus] = useState("loading");

  useLayoutEffect(() => {
    contextPatch.current = { theme, audio: JSON.parse(audioParameters) };
    sendContext.current?.();
  }, [theme, audioParameters]);

  useEffect(() => {
    const container = host.current;
    let frame;
    let html;
    let context;
    let timer;
    let startedAt;
    let initialized = false;
    let closed = false;

    const latestContext = (value) => ({
      ...value,
      theme: contextPatch.current.theme,
      parameters: { ...value.parameters, ...contextPatch.current.audio },
    });
    const post = (type) => frame?.contentWindow?.postMessage({ type, context }, "*");
    const receive = (event) => {
      if (!frame || event.source !== frame.contentWindow) return;
      if (event.data?.type === "prism:ready" && !initialized) {
        clearTimeout(timer);
        initialized = true;
        startedAt = performance.now();
        context = latestContext(context);
        frame.hidden = false;
        setStatus("playing");
        post("prism:init");
      } else if (initialized && event.data?.type === "prism:done") {
        if (event.data.reason === "skip") {
          close();
          onEnd();
        } else {
          removeFrame();
          timer = setTimeout(mountFrame, Math.max(0, MIN_PREVIEW_CYCLE_MS - (performance.now() - startedAt)));
        }
      }
    };
    const removeFrame = () => {
      if (initialized) post("prism:dispose");
      frame?.remove();
      frame = null;
      initialized = false;
    };
    const close = () => {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      window.removeEventListener("message", receive);
      removeFrame();
      sendContext.current = null;
    };
    const fail = () => {
      close();
      setStatus("failed");
    };
    const mountFrame = () => {
      clearTimeout(timer);
      timer = setTimeout(fail, READY_TIMEOUT_MS);
      frame = document.createElement("iframe");
      frame.setAttribute("aria-label", words.title);
      frame.hidden = true;
      frame.setAttribute("sandbox", "allow-scripts");
      frame.setAttribute("allow", "autoplay");
      frame.referrerPolicy = "no-referrer";
      frame.srcdoc = html;
      container.append(frame);
    };
    timer = setTimeout(fail, READY_TIMEOUT_MS);

    sendContext.current = () => {
      if (!context) return;
      const next = latestContext(context);
      if (context.theme === next.theme && Object.entries(contextPatch.current.audio)
        .every(([key, value]) => Object.is(context.parameters[key], value))) return;
      context = next;
      if (initialized) post("prism:update");
    };
    window.addEventListener("message", receive);

    const command = mode === "market" ? "get_market_preview" : "get_studio_preview";
    void invoke(command, {
      workId,
      theme: contextPatch.current.theme,
      parameters: { ...JSON.parse(parameters), ...contextPatch.current.audio },
      locale,
    }).then((payload) => {
      if (closed) return;
      if (!payload?.html || !payload.context) {
        fail();
        return;
      }
      context = latestContext(payload.context);
      html = payload.html;
      mountFrame();
    }).catch(() => {
      if (!closed) fail();
    });

    return close;
  }, [workId, parameters, locale, mode, words, onEnd]);

  return (
    <>
      <div className="studio-preview-frame" ref={host} />
      {status !== "playing" && <PreviewMessage message={words[status]} />}
    </>
  );
}

function PreviewRun({ workId, theme, parameters, audioParameters, locale, mode, words, visible, supported }) {
  const [ended, setEnded] = useState(false);
  const complete = useCallback(() => setEnded(true), []);
  if (!supported) return <PreviewMessage message={words.unsupported} />;
  if (ended) return <PreviewMessage message={words.ended} />;
  if (!visible) return <PreviewMessage message={words.hidden} />;
  return (
    <PreviewFrame
      workId={workId}
      theme={theme}
      parameters={parameters}
      audioParameters={audioParameters}
      locale={locale}
      mode={mode}
      words={words}
      onEnd={complete}
    />
  );
}

export function StudioPreview({
  workId,
  theme,
  parameters,
  locale,
  enabled = true,
  active = true,
  supported = true,
  replay = 0,
  mode = "studio",
  themeReady = true,
}) {
  const words = text[locale] || text.zh;
  const visible = usePreviewVisibility();
  const audio = {};
  const restartParameters = { ...parameters };
  for (const key of ["sound", "volume"]) {
    if (Object.hasOwn(restartParameters, key)) {
      audio[key] = restartParameters[key];
      delete restartParameters[key];
    }
  }
  const audioParameters = JSON.stringify(audio);
  const parametersKey = JSON.stringify(restartParameters, Object.keys(restartParameters).sort());
  const [settled, setSettled] = useState(() => ({ workId, parametersKey }));
  useEffect(() => {
    if (settled.workId === workId && settled.parametersKey === parametersKey) return;
    const timer = setTimeout(() => setSettled({ workId, parametersKey }), PARAMETER_DELAY_MS);
    return () => clearTimeout(timer);
  }, [settled, workId, parametersKey]);

  const activeParameters = settled.workId === workId ? settled.parametersKey : parametersKey;
  const run = JSON.stringify([workId, locale, activeParameters, mode, replay]);

  return (
    <div className="studio-preview" data-theme={themeReady ? theme : "pending"} aria-label={words.title}>
      {!enabled || !workId
        ? <PreviewMessage message={words.empty} />
        : !themeReady ? <PreviewMessage message={words.themeLoading} /> : (
          <PreviewRun
            key={run}
            workId={workId}
            theme={theme}
            parameters={activeParameters}
            audioParameters={audioParameters}
            locale={locale}
            mode={mode}
            words={words}
            visible={active && visible}
            supported={supported}
          />
        )}
    </div>
  );
}
