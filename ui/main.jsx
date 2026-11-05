import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Icon, PrismMark } from "./icons.jsx";
import { StudioPreview } from "./StudioPreview.jsx";
import { Parameter, Select, Switch } from "./Parameter.jsx";
import { WorkCatalog } from "./WorkCatalog.jsx";
import { Tooltip } from "./Tooltip.jsx";
import authorAvatar from "./author-avatar.jpg";
import "./styles.css";

const pages = [
  ["studio", "工作台", "Studio"],
  ["library", "启动动画", "Startup animations"],
  ["market", "社区市场", "Marketplace"],
  ["settings", "偏好设置", "Preferences"],
  ["about", "关于", "About"],
];
const themeLabel = {
  dark: ["黑曜", "Obsidian"],
  light: ["极昼", "Polar Day"],
};
const platform = navigator.userAgent.includes("Windows") ? "windows" : "macos";

const repository = "https://github.com/tdragon6/ChatGPT-Prism";
const parametersFor = (settings, workId) =>
  settings?.workId === workId ? settings.parameters : settings?.workParameters?.[workId] || {};
const diagnostic = (error) => (error?.message || String(error)).trimEnd().replace(/[.。]+$/, "");

function SettingsInput({ value, onCommit, ...props }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <input {...props} type="text" value={text} onChange={(event) => setText(event.target.value)}
      onBlur={() => { if (text !== value) onCommit(text); }}
      onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }} />
  );
}

function App() {
  const [data, setData] = useState(null);
  const [draft, setDraft] = useState(null);
  const draftRef = useRef(null);
  const savedSettings = useRef(null);
  const pendingSettings = useRef(null);
  const enabledEdit = useRef(0);
  const settingsQueue = useRef(Promise.resolve());
  const settingsTimer = useRef(null);
  const [page, setPage] = useState("studio");
  const [previewTheme, setPreviewTheme] = useState(null);
  const [replay, setReplay] = useState(0);
  const [busy, setBusy] = useState(false);
  const [systemLight, setSystemLight] = useState(() => matchMedia("(prefers-color-scheme: light)").matches);
  const processing = useRef(false);
  const [message, setMessage] = useState(null);
  const [catalog, setCatalog] = useState(null);
  const [catalogBusy, setCatalogBusy] = useState(false);
  const [catalogError, setCatalogError] = useState(null);
  const catalogRequest = useRef(0);
  const manualCatalogRefresh = useRef(false);
  const [appUpdate, setAppUpdate] = useState(null);
  const [updatePrompt, setUpdatePrompt] = useState(null);
  const [ignoreUpdate, setIgnoreUpdate] = useState(false);
  const [updateError, setUpdateError] = useState("");
  const updateDialog = useRef(null);
  const startupNotified = useRef(null);
  const pageRef = useRef(page);
  pageRef.current = page;
  const [closePrompt, setClosePrompt] = useState(false);
  const [rememberClose, setRememberClose] = useState(false);
  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState("");
  const closeDialog = useRef(null);
  const locale = draft?.locale || (navigator.language.startsWith("zh") ? "zh" : "en");
  const t = (zh, en) => locale === "en" ? en : zh;
  const read = () => run(async () => {
    const next = await invoke("get_state");
    accept(next);
    showStartupUpdate(next.startupUpdate);
  });
  const work = data?.works.find(({ id }) => id === draft?.workId);
  const translated = work?.i18n?.[locale];
  const workName = (entry) => entry?.i18n?.[locale]?.name || entry?.name;

  function replaceDraft(next) {
    draftRef.current = next;
    setDraft(next);
  }
  function accept(next) {
    if (!next) return;
    savedSettings.current = next.settings;
    setData((current) => current ? { ...next, runtime: current.runtime } : next);
    replaceDraft(next.settings);
    if (next.closeRequested) setClosePrompt(true);
  }
  function change(values) {
    if (draftRef.current) replaceDraft({ ...draftRef.current, ...values });
  }
  function flushStudio() {
    clearTimeout(settingsTimer.current);
    const pending = pendingSettings.current;
    if (pending) {
      pendingSettings.current = null;
      settingsQueue.current = settingsQueue.current.then(async () => {
        let saved = false;
        try {
          const next = await invoke("save_settings", {
            settings: { ...savedSettings.current, ...pending },
            updateEnabled: Object.hasOwn(pending, "enabled"),
          });
          savedSettings.current = next.settings;
          setData((current) => current && { ...current, settings: next.settings });
          saved = true;
        } catch (error) {
          setMessage({ text: diagnostic(error), error: true });
        }
        const settled = Object.fromEntries(Object.keys(pending)
          .filter((key) => Object.is(draftRef.current[key], pending[key]))
          .map((key) => [key, savedSettings.current[key]]));
        change(settled);
        return saved;
      });
    }
    return settingsQueue.current;
  }
  function changeStudio(values, delay = 0) {
    setMessage(null);
    const next = typeof values === "function" ? values(draftRef.current) : values;
    if (Object.hasOwn(next, "marketUrl") && next.marketUrl !== draftRef.current.marketUrl) {
      catalogRequest.current += 1;
      setCatalog(null);
      setCatalogError(null);
    }
    if (Object.hasOwn(next, "enabled")) enabledEdit.current += 1;
    change(next);
    pendingSettings.current = { ...pendingSettings.current, ...next };
    clearTimeout(settingsTimer.current);
    if (delay) settingsTimer.current = setTimeout(flushStudio, delay);
    else return flushStudio();
  }
  function selectWork(workId, enable = false) {
    return changeStudio((current) => {
      const activation = enable ? { enabled: true } : {};
      if (workId === current.workId) return activation;
      const workParameters = { ...current.workParameters };
      if (current.workId) workParameters[current.workId] = current.parameters;
      const parameters = workParameters[workId] || {};
      delete workParameters[workId];
      return { workId, workParameters, parameters, ...activation };
    });
  }
  async function configureWork(workId, changes) {
    const saved = await changeStudio((current) => {
      const parameters = { ...parametersFor(current, workId), ...changes };
      return workId === current.workId
        ? { parameters }
        : { workParameters: { ...current.workParameters, [workId]: parameters } };
    });
    if (!saved) throw new Error(t("无法保存动画参数", "Couldn’t save animation settings"));
  }
  async function run(action) {
    if (processing.current) return;
    processing.current = true;
    setBusy(true);
    setMessage(null);
    try {
      await action();
    } catch (error) {
      setMessage({ text: diagnostic(error), error: true });
    } finally {
      processing.current = false;
      setBusy(false);
    }
  }
  function launch() {
    return run(async () => {
      await flushStudio();
      await invoke("launch_chatgpt");
    });
  }
  function importWork() {
    return run(async () => {
      await flushStudio();
      accept(await invoke("import_work"));
    });
  }
  function unloadWork(workId) {
    return run(async () => {
      await flushStudio();
      accept(await invoke("unload_work", { workId }));
    });
  }
  function removeWork(workId) {
    return run(async () => {
      await flushStudio();
      accept(await invoke("remove_work", { workId }));
      await loadCatalog(false);
    });
  }
  async function loadCatalog(refresh) {
    if (refresh) manualCatalogRefresh.current = true;
    const request = ++catalogRequest.current;
    setCatalogBusy(true);
    setCatalog(null);
    setCatalogError(null);
    try {
      await flushStudio();
      if (request !== catalogRequest.current) return;
      const source = savedSettings.current?.marketUrl;
      const result = await invoke(refresh ? "refresh_catalog" : "get_catalog");
      if (request === catalogRequest.current && source === savedSettings.current?.marketUrl) setCatalog(result);
    } catch (error) {
      if (request === catalogRequest.current) {
        setCatalog(null);
        setCatalogError({ refresh, detail: diagnostic(error) });
      }
    } finally {
      if (refresh) manualCatalogRefresh.current = false;
      if (request === catalogRequest.current) setCatalogBusy(false);
    }
  }
  function installCatalogWork(workId) {
    return run(async () => {
      await flushStudio();
      accept(await invoke("install_catalog_work", { workId }));
      await loadCatalog(false);
    });
  }
  function showStartupUpdate(update) {
    if (!update?.available || startupNotified.current === update.latestVersion ||
        draftRef.current?.ignoredUpdateVersion === update.latestVersion) return;
    startupNotified.current = update.latestVersion;
    setIgnoreUpdate(false);
    setUpdateError("");
    setUpdatePrompt(update);
  }
  function checkUpdate() {
    return run(async () => {
      setAppUpdate(null);
      try {
        const update = await invoke("check_app_update");
        setAppUpdate(update);
        if (update.available) {
          setIgnoreUpdate(false);
          setUpdateError("");
          setUpdatePrompt(update);
        }
      } catch (error) {
        setAppUpdate({ error: diagnostic(error) });
      }
    });
  }
  function respondToUpdate(download) {
    return run(async () => {
      setUpdateError("");
      try {
        if (ignoreUpdate && !await changeStudio({ ignoredUpdateVersion: updatePrompt.latestVersion })) {
          throw new Error(t("无法保存更新提醒设置", "Couldn’t save update reminder preferences"));
        }
        if (download) await invoke("open_releases", { version: updatePrompt.latestVersion });
        setUpdatePrompt(null);
      } catch (error) {
        setUpdateError(diagnostic(error));
      }
    });
  }
  async function respondClose(action) {
    if (closing) return;
    setClosing(true);
    setCloseError("");
    try {
      const remember = rememberClose && action !== "cancel";
      await flushStudio();
      await invoke("respond_close", { action, remember });
      if (remember && savedSettings.current) {
        savedSettings.current = { ...savedSettings.current, closeBehavior: action };
        setData((current) => current && ({ ...current, settings: savedSettings.current }));
        change({ closeBehavior: action });
      }
      setClosePrompt(false);
    } catch (error) {
      setCloseError(diagnostic(error));
    } finally {
      setClosing(false);
    }
  }
  useEffect(() => {
    let disposed = false;
    const remove = [];
    const subscribe = (event, handler) => listen(event, handler).then((unlisten) => {
      if (disposed) unlisten();
      else remove.push(unlisten);
    }).catch(() => {});
    const closeSubscription = subscribe("prism-close-request", () => {
      setRememberClose(false);
      setCloseError("");
      setClosePrompt(true);
    });
    const runtimeSubscription = subscribe("prism-runtime", ({ payload }) => {
      setData((current) => current && { ...current, runtime: payload });
    });
    const marketSubscription = subscribe("prism-market-refreshed", ({ payload }) => {
      if (payload?.source && payload.source !== draftRef.current?.marketUrl) return;
      if (!manualCatalogRefresh.current && ["market", "library"].includes(pageRef.current)) {
        void loadCatalog(false);
      }
    });
    const updateSubscription = subscribe("prism-startup-update", ({ payload }) => showStartupUpdate(payload));
    const enabledSubscription = subscribe("prism-startup-enabled", () => {
      const edit = enabledEdit.current;
      settingsQueue.current = settingsQueue.current.then(async () => {
        const next = await invoke("get_state").catch(() => null);
        if (!next || disposed || !savedSettings.current) return false;
        const { enabled } = next.settings;
        savedSettings.current = { ...savedSettings.current, enabled };
        setData((current) => current && { ...current, settings: { ...current.settings, enabled } });
        if (edit === enabledEdit.current && !Object.hasOwn(pendingSettings.current || {}, "enabled")) {
          change({ enabled });
        }
        return true;
      });
    });
    void Promise.all([closeSubscription, runtimeSubscription, marketSubscription, updateSubscription, enabledSubscription])
      .then(() => { if (!disposed) void read(); });
    return () => {
      disposed = true;
      remove.forEach((unlisten) => unlisten());
    };
  }, []);
  useEffect(() => {
    const dialog = updateDialog.current;
    if (updatePrompt && !closePrompt && !dialog.open) {
      dialog.showModal();
      dialog.querySelector("h2").focus({ preventScroll: true });
    } else if ((!updatePrompt || closePrompt) && dialog.open) {
      dialog.close();
    }
  }, [updatePrompt, closePrompt]);
  useEffect(() => {
    const dialog = closeDialog.current;
    if (!dialog) return;
    if (closePrompt && !dialog.open) {
      dialog.showModal();
      dialog.querySelector("[data-dialog-title]")?.focus({ preventScroll: true });
    }
    if (!closePrompt && dialog.open) dialog.close();
  }, [closePrompt]);
  useEffect(() => {
    if ((page === "market" || page === "library") && data) void loadCatalog(false);
  }, [page, draft?.marketUrl, Boolean(data)]);
  useEffect(() => {
    if (!data) return;
    let stopped = false;
    const update = async () => {
      if (document.hidden) return;
      try {
        const runtime = await invoke("get_runtime");
        if (!stopped) setData((value) => ({ ...value, runtime }));
      } catch {
        // Status refresh is independent of saved settings and form actions.
      }
    };
    void update();
    window.addEventListener("focus", update);
    return () => {
      stopped = true;
      window.removeEventListener("focus", update);
    };
  }, [Boolean(data)]);
  useEffect(() => {
    const preference = matchMedia("(prefers-color-scheme: light)");
    const update = () => setSystemLight(preference.matches);
    preference.addEventListener("change", update);
    return () => preference.removeEventListener("change", update);
  }, []);
  const isLight = draft?.appearance === "light" || (draft?.appearance === "system" && systemLight);
  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dataset.theme = isLight ? "light" : "dark";
  }, [locale, isLight]);
  const runtime = data?.runtime || {};
  useEffect(() => setPreviewTheme(null), [work?.id, runtime.theme]);
  const themeSource = {
    "client-preference": t("ChatGPT 设置", "ChatGPT preference"),
    "client-system": t("ChatGPT 跟随系统", "ChatGPT follows system"),
    "system-fallback": t("系统外观", "System appearance"),
  }[runtime.themeSource];
  const title = t(...pages.find(([id]) => id === page).slice(1));
  const languageShortcut = locale === "en" ? "切换到简体中文" : "Switch to English";
  const themeShortcut = isLight ? t("切换为黑曜界面", "Switch to Obsidian") : t("切换为极昼界面", "Switch to Polar Day");
  const preference = changeStudio;
  const chooseApp = () => run(async () => {
    const selected = await invoke("select_application");
    if (selected) await preference({ applicationPath: selected });
  });
  const manualTheme = previewTheme && previewTheme.workId === work?.id && previewTheme.clientTheme === runtime.theme
    && work?.themes.includes(previewTheme.theme) ? previewTheme.theme : null;
  const stageTheme = manualTheme || runtime.theme;
  const themeReady = Boolean(themeLabel[stageTheme]);
  const previewParameters = {
    ...Object.fromEntries((work?.parameters || []).map((parameter) => [parameter.key, parameter.default])),
    ...draft?.parameters,
  };
  const runtimeLabel = runtime.connected ? (runtime.running ? t("已运行", "Running") : t("等待启动", "Waiting for launch")) : t("未连接", "Disconnected");
  const runtimeError = runtime.error && diagnostic(runtime.error);
  const runtimeHint = [themeSource, themeLabel[runtime.theme] && t(...themeLabel[runtime.theme]), runtimeError].filter(Boolean).join(" · ");
  const workOptions = data?.works.map((entry) => ({ value: entry.id, label: workName(entry) })) || [];
  const closeOptions = [
    { value: "ask", label: t("询问", "Ask") },
    { value: "background", label: t("后台运行", "Run in background") },
    { value: "quit", label: t("退出程序", "Quit") },
  ];
  const marketUpdates = new Set(catalog?.entries.filter(({ updateAvailable }) => updateAvailable).map(({ work }) => work.id));
  const libraryEntries = data?.works.map((work) => ({ work, updateAvailable: marketUpdates.has(work.id) }));
  const missingMarketIndex = catalogError && /\b404\b/.test(catalogError.detail);
  const removedMarketIndex = catalogError && /\b410\b/.test(catalogError.detail);
  const marketError = catalogError && {
    detail: catalogError.detail,
    text: missingMarketIndex
      ? t("市场索引不存在（404）", "Marketplace index not found (404)")
      : removedMarketIndex
        ? t("市场索引已移除（410）", "Marketplace index removed (410)")
      : catalogError.refresh
        ? t("市场更新失败", "Couldn’t update the marketplace")
        : t("无法读取市场索引", "Couldn’t read the marketplace index"),
  };
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <button className="brand" type="button" onClick={() => setPage("studio")} aria-label="ChatGPT Prism">
          <PrismMark size={48} centered />
          <span>Prism<small>For ChatGPT</small></span>
        </button>
        {data && (
          <>
            <nav aria-label={t("主要导航", "Main navigation")}>
              <button className="nav-link" type="button" aria-label={t("工作台", "Studio")} aria-current={page === "studio" ? "page" : undefined} onClick={() => setPage("studio")}>
                <Icon name="grid" /><span>{t("工作台", "Studio")}</span>
              </button>
              <button className="nav-link" type="button" aria-label={t("启动动画", "Startup animations")} aria-current={page === "library" ? "page" : undefined} onClick={() => setPage("library")}>
                <Icon name="film" /><span>{t("启动动画", "Startup animations")}</span>
              </button>
              <button className="nav-link" type="button" aria-label={t("社区市场", "Marketplace")} aria-current={page === "market" ? "page" : undefined} onClick={() => setPage("market")}>
                <Icon name="globe" /><span>{t("社区市场", "Marketplace")}</span>
              </button>
            </nav>
            <div className="sidebar-bottom">
              <button className="nav-link" type="button" aria-label={t("偏好设置", "Preferences")} aria-current={page === "settings" ? "page" : undefined} onClick={() => setPage("settings")}>
                <Icon name="sliders" /><span>{t("偏好设置", "Preferences")}</span>
              </button>
              <button className="author-link" type="button" aria-label={t("关于 Prism", "About Prism")} aria-current={page === "about" ? "page" : undefined} onClick={() => setPage("about")}>
                <img className="author-avatar" src={authorAvatar} alt="" />
                <span>tdragon6<small>Prism</small></span>
                <Icon name="chevron" size={14} />
              </button>
            </div>
          </>
        )}
      </aside>
      <div className="main">
        <header className="topbar">
          <div className="breadcrumb">
            <span>ChatGPT Prism</span><Icon name="chevron" size={12} /><span>{title}</span>
          </div>
          {data && (
            <div className="topbar-actions">
              <button className="icon-button" type="button" aria-label={languageShortcut} data-tooltip={languageShortcut} disabled={busy} onClick={() => preference({ locale: locale === "en" ? "zh" : "en" })}>
                <Icon name="language" />
              </button>
              <button className="icon-button" type="button" aria-label={themeShortcut} data-tooltip={themeShortcut} disabled={busy} onClick={() => preference({ appearance: isLight ? "dark" : "light" })}>
                <Icon name={isLight ? "sun" : "moon"} />
              </button>
            </div>
          )}
        </header>
        <div className="page" data-page={page} aria-busy={busy}>
          <div className="page-heading">
            <div className="page-title">
              <h1>{title}</h1>
              {data && page === "studio" && (
                <div className="runtime-badge" data-tooltip={runtimeHint || undefined} tabIndex={runtimeHint ? 0 : undefined} aria-label={t("客户端状态", "Client status")}>
                  <span className={`status-dot${runtime.connected ? " ready" : ""}`} aria-hidden="true" />
                  <strong>{runtime.clientName || "ChatGPT"}</strong><span>{runtimeLabel}</span>
                </div>
              )}
            </div>
            {data && page === "studio" && (
              <div className="heading-actions">
                <button className="button" type="button" disabled={busy} onClick={importWork}><Icon name="folder" size={15} />{t("导入作品", "Import animation")}</button>
                <button className="button primary" type="button" disabled={busy} onClick={launch}><Icon name="play" size={15} />{t("打开 ChatGPT", "Open ChatGPT")}</button>
              </div>
            )}
            {data && page === "market" && (
              <button className="button" type="button" disabled={catalogBusy || busy} onClick={() => loadCatalog(true)}><Icon name="refresh" size={15} />{catalogBusy && catalog ? t("更新中…", "Updating…") : t("更新市场", "Update marketplace")}</button>
            )}
            {data && page === "library" && (
              <div className="heading-actions">
                <button className="button" type="button" disabled={busy} onClick={importWork}><Icon name="folder" size={15} />{t("导入作品", "Import animation")}</button>
                <button className="button primary" type="button" onClick={() => setPage("market")}><Icon name="globe" size={15} />{t("浏览市场", "Browse marketplace")}</button>
              </div>
            )}
          </div>
          {message && <p className={`message${message.error ? " error" : ""}`} role={message.error ? "alert" : "status"}>{message.text}</p>}
          {!data && (
            <section className="panel loading">
              {message ? <button className="button" type="button" disabled={busy} onClick={read}>{t("重试", "Retry")}</button> : <p role="status">{t("载入中…", "Loading…")}</p>}
            </section>
          )}
          {data && page === "studio" && (
            <>
              {runtimeError && <p className="message error">{runtimeError}</p>}
              <div className="studio-layout">
                <section className="preview-panel" aria-label={t("动画预览", "Animation preview")}>
                  <div className="preview-toolbar">
                    <h2><span className="status-dot ready" aria-hidden="true" />{t("动画预览", "Animation preview")}</h2>
                    {work && (
                      <div className="preview-actions">
                        <div className="preview-theme-status" role="status">
                          <Icon name="monitor" size={13} />
                          <span>{t("默认跟随客户端", "Defaults to client")}</span>
                        </div>
                        <div className="theme-switch" role="group" aria-label={t("预览配色", "Preview theme")}>
                          {["dark", "light"].filter((theme) => work.themes.includes(theme)).map((theme) => (
                            <button type="button" key={theme} disabled={busy} aria-pressed={stageTheme === theme}
                              onClick={() => setPreviewTheme({ workId: work.id, clientTheme: runtime.theme, theme })}>
                              <Icon name={theme === "dark" ? "moon" : "sun"} size={13} />
                              <span>{t(...themeLabel[theme])}</span>
                            </button>
                          ))}
                        </div>
                        <button className="icon-button replay-button" type="button" disabled={busy || !draft.enabled || !themeReady || !work.themes.includes(stageTheme)}
                          aria-label={t("重新播放", "Replay")} data-tooltip={t("重新播放", "Replay")} onClick={() => setReplay((count) => count + 1)}>
                          <Icon name="refresh" size={15} />
                        </button>
                      </div>
                    )}
                  </div>
                  <StudioPreview workId={work?.id || ""} theme={stageTheme} parameters={previewParameters} locale={locale}
                    enabled={draft.enabled} active={!closePrompt && !updatePrompt} replay={replay} themeReady={themeReady}
                    supported={Boolean(work?.themes.includes(stageTheme))} />
                  {work && (
                    <div className="work-caption work-title-line">
                      <h3>{workName(work)}</h3>
                      {work.author && <small>{work.author}</small>}
                    </div>
                  )}
                </section>
                <section className="panel selection" aria-label={t("启动动画", "Startup animation")}>
                  <div className="selection-heading">
                    <h2><Icon name="film" size={16} />{t("启动动画", "Startup animation")}</h2>
                    <Switch label={t("启用启动动画", "Enable startup animation")} compact checked={draft.enabled} disabled={busy || !work} onChange={(enabled) => changeStudio({ enabled })} />
                  </div>
                  {workOptions.length === 0 ? (
                    <div className="studio-library-empty">
                      <p>{t("尚未安装启动动画", "No startup animations installed")}</p>
                      <button className="button" type="button" onClick={() => setPage("market")}>
                        <Icon name="globe" size={15} />{t("浏览市场", "Browse marketplace")}
                      </button>
                    </div>
                  ) : <Select label={t("选择启动动画作品", "Choose a startup animation")}
                    className="selected-work" value={draft.workId || ""} disabled={busy} options={workOptions}
                    selectedContent={work ? (
                      <span className="selected-work-content">
                        <span className="selected-work-cover">
                          {work.previewUrl ? <img src={work.previewUrl} alt="" /> : <PrismMark size={26} />}
                        </span>
                        <span className="selected-work-meta work-title-line">
                          <strong>{workName(work)}</strong>
                          {work.author && <small>{work.author}</small>}
                        </span>
                      </span>
                    ) : <span>{t("未选择作品", "No animation selected")}</span>}
                    onChange={selectWork} />}
                  {work && (
                    <div className="parameters">
                      {work.parameters?.map((parameter) => (
                        <Parameter
                          key={parameter.key}
                          parameter={parameter}
                          disabled={busy}
                          label={parameter.key === "sound" ? t("音乐", "Music") : translated?.parameters?.[parameter.key]?.label || parameter.label}
                          value={draft.parameters?.[parameter.key] ?? parameter.default}
                          onChange={(value, delay) => changeStudio(({ parameters }) => ({ parameters: { ...parameters, [parameter.key]: value } }), delay)}
                          onCommit={flushStudio}
                        />
                      ))}
                    </div>
                  )}
                </section>
              </div>
            </>
          )}
          {data && page === "market" && (
            <WorkCatalog key="market" market entries={catalog?.entries || []} settings={draft} locale={locale}
              installedWorks={data.works} getParameters={(workId) => parametersFor(draft, workId)} onConfigure={configureWork}
              clientTheme={runtime.theme} active={!closePrompt && !updatePrompt} onUnload={unloadWork} onDelete={removeWork}
              onUse={(workId) => selectWork(workId, true)}
              loading={!catalog && !catalogError} busy={busy || catalogBusy} error={marketError} onInstall={installCatalogWork} />
          )}
          {data && page === "library" && (
            <WorkCatalog key="library" entries={libraryEntries} settings={draft} clientTheme={runtime.theme}
              installedWorks={data.works} getParameters={(workId) => parametersFor(draft, workId)} onConfigure={configureWork}
              locale={locale} busy={busy} active={!closePrompt && !updatePrompt} onBrowse={() => setPage("market")}
              onInstall={installCatalogWork} onUnload={unloadWork} onDelete={removeWork}
              onUse={(workId) => selectWork(workId, true)} />
          )}
          {data && page === "settings" && (
            <div className="settings-grid">
              <section className="panel settings-section">
                <h2>{t("常规", "General")}</h2>
                <div className="field settings-row">
                  <span>{t("关闭主窗口时", "When closing the main window")}</span>
                  <Select label={t("关闭主窗口时", "When closing the main window")} value={draft.closeBehavior || "ask"}
                    disabled={busy} options={closeOptions} onChange={(closeBehavior) => preference({ closeBehavior })} />
                </div>
                {typeof draft.launchAtLogin === "boolean" && (
                  <div className="settings-row"><Switch label={t("登录时启动 Prism", "Start Prism at login")} checked={draft.launchAtLogin} disabled={busy} onChange={(launchAtLogin) => preference({ launchAtLogin })} /></div>
                )}
                <div className="settings-row"><Switch label={t("启动 Prism 时检查版本更新", "Check for app updates on startup")}
                  checked={draft.checkUpdatesOnStartup ?? true} disabled={busy}
                  onChange={(checkUpdatesOnStartup) => preference({ checkUpdatesOnStartup })} /></div>
              </section>
              <section className="panel settings-section">
                <h2>ChatGPT</h2>
                <div className="field settings-row stacked">
                  <label htmlFor="application-path">{t("ChatGPT 路径", "ChatGPT path")}</label>
                  <div className="path-control">
                    <SettingsInput id="application-path" disabled={busy} value={draft.applicationPath || ""} placeholder={t("自动检测", "Detect automatically")}
                      onCommit={(applicationPath) => preference({ applicationPath: applicationPath.trim() })} />
                    <button className="button" type="button" disabled={busy} onClick={chooseApp}><Icon name="folder" size={15} />{t("选择…", "Browse…")}</button>
                  </div>
                  <small>{platform === "windows" ? t("选择 ChatGPT 的可执行文件", "Select the ChatGPT executable") : t("选择 ChatGPT.app", "Select ChatGPT.app")}</small>
                  {draft.applicationPath && <button className="text-button" type="button" disabled={busy} onClick={() => preference({ applicationPath: "" })}>{t("自动检测", "Auto-detect")}</button>}
                </div>
              </section>
              <section className="panel settings-section market-settings">
                <h2>{t("社区市场", "Marketplace")}</h2>
                <div className="settings-row"><Switch label={t("启动 Prism 时自动更新市场", "Update the marketplace on startup")}
                  checked={draft.refreshMarketOnStartup ?? true} disabled={busy}
                  onChange={(refreshMarketOnStartup) => preference({ refreshMarketOnStartup })} /></div>
                <div className="field settings-row stacked">
                  <label htmlFor="market-url">{t("市场索引地址", "Marketplace index URL")}</label>
                  <div className="path-control">
                    <SettingsInput id="market-url" disabled={busy} value={draft.marketUrl || data.defaultMarketUrl || ""}
                      onCommit={(marketUrl) => preference({ marketUrl: marketUrl.trim() || data.defaultMarketUrl })} />
                    <button className="button" type="button" disabled={busy} onClick={() => preference({ marketUrl: data.defaultMarketUrl })}>{t("恢复默认", "Reset")}</button>
                  </div>
                </div>
              </section>
            </div>
          )}
          {data && page === "about" && (
            <section className="panel about-card">
              <div className="about-brand">
                <PrismMark size={62} centered />
                <h2>ChatGPT Prism</h2>
              </div>
              <div className="about-author">
                <img className="author-avatar large" src={authorAvatar} alt="" />
                <div><span>{t("作者", "Author")}</span><strong>tdragon6</strong></div>
              </div>
              <div className="about-row">
                <span>{t("项目地址", "Repository")}</span>
                <button className="repo-link" type="button" disabled={busy} onClick={() => run(() => invoke("open_repository"))}>{repository}<Icon name="external" size={14} /></button>
              </div>
              <div className="about-row about-actions">
                <button className="button" type="button" disabled={busy} onClick={checkUpdate}><Icon name="refresh" size={15} />{t("检查更新", "Check for updates")}</button>
                {appUpdate && !appUpdate.available && (appUpdate.error
                  ? <span className="update-status" role="status" data-tooltip={appUpdate.error} tabIndex={0}>{t("暂时无法检查更新", "Couldn’t check for updates")}</span>
                  : <span className="update-status" role="status">{t("已是最新版本", "You’re up to date")}</span>)}
              </div>
            </section>
          )}
        </div>
      </div>
      <dialog ref={updateDialog} className="close-dialog" aria-labelledby="update-title"
        onCancel={(event) => { event.preventDefault(); void respondToUpdate(false); }}>
        <div className="dialog-header">
          <h2 id="update-title" tabIndex={-1} data-dialog-title>{t("发现新版本", "Update available")}</h2>
          <button className="icon-button" type="button" disabled={busy} aria-label={t("关闭", "Close")}
            onClick={() => respondToUpdate(false)}><Icon name="close" size={17} /></button>
        </div>
        <label className="remember-close">
          <input type="checkbox" checked={ignoreUpdate} disabled={busy} onChange={(event) => setIgnoreUpdate(event.target.checked)} />
          {t("不再提醒此版本", "Don’t remind me about this version")}
        </label>
        {updateError && <p className="message error" role="alert">{updateError}</p>}
        <div className="dialog-actions">
          <button className="button" type="button" disabled={busy} onClick={() => respondToUpdate(false)}>{t("稍后", "Later")}</button>
          <button className="button primary" type="button" disabled={busy} onClick={() => respondToUpdate(true)}>
            <Icon name="external" size={15} />{t("前往下载", "Open download")}
          </button>
        </div>
      </dialog>
      <dialog ref={closeDialog} className="close-dialog" aria-labelledby="close-title"
        onCancel={(event) => { event.preventDefault(); void respondClose("cancel"); }}>
        <div className="dialog-header">
          <h2 id="close-title" tabIndex={-1} data-dialog-title>{t("关闭主窗口", "Close the main window")}</h2>
          <button className="icon-button" type="button" disabled={closing} aria-label={t("取消", "Cancel")} onClick={() => respondClose("cancel")}><Icon name="close" size={17} /></button>
        </div>
        <p>{t("让 Prism 在后台运行，或退出程序", "Keep Prism running in the background, or quit")}</p>
        <label className="remember-close">
          <input type="checkbox" checked={rememberClose} disabled={closing} onChange={(event) => setRememberClose(event.target.checked)} />
          {t("记住我的选择", "Remember my choice")}
        </label>
        {closeError && <p className="message error" role="alert">{closeError}</p>}
        <div className="dialog-actions">
          <button className="button" type="button" disabled={closing} onClick={() => respondClose("cancel")}>{t("取消", "Cancel")}</button>
          <button className="button" type="button" disabled={closing} onClick={() => respondClose("quit")}>{t("退出程序", "Quit")}</button>
          <button className="button primary" type="button" disabled={closing} onClick={() => respondClose("background")}>{t("后台运行", "Run in background")}</button>
        </div>
      </dialog>
    </div>
  );
}

createRoot(document.getElementById("app")).render(<><App /><Tooltip /></>);
