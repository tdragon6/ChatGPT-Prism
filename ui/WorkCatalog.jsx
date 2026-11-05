import React, { useEffect, useId, useRef, useState } from "react";
import { Icon, PrismMark } from "./icons.jsx";
import { Parameter } from "./Parameter.jsx";
import { StudioPreview } from "./StudioPreview.jsx";
import "./work-catalog.css";

const themes = [
  ["", "全部主题", "All themes", null],
  ["dark", "黑曜", "Obsidian", "moon"],
  ["light", "极昼", "Polar Day", "sun"],
];

function workName(work, locale) {
  return work.i18n?.[locale]?.name || work.name;
}

function WorkTags({ work, t }) {
  const languages = work.languages || [];
  const custom = t("支持自定义参数", "Supports custom parameters");
  return (
    <div className="work-tags">
      {themes.filter(([id]) => work.themes.includes(id)).map(([id, zh, en, icon]) => (
        <span key={id}><Icon name={icon} size={11} />{t(zh, en)}</span>
      ))}
      {languages.length > 0 && <span>{languages.length > 1 ? t("中 / EN", "EN / 中文") : languages[0] === "en" ? "EN" : "中文"}</span>}
      {work.parameters?.length > 0 && <span data-tooltip={custom} tabIndex={0} aria-label={custom} role="img"><Icon name="sliders" size={12} /></span>}
    </div>
  );
}

function ThemeTabs({ label, theme, onChange, panel, tabId, t }) {
  const tabList = useRef(null);
  function move(event, index) {
    const last = themes.length - 1;
    const next = {
      ArrowLeft: index === 0 ? last : index - 1,
      ArrowRight: index === last ? 0 : index + 1,
      Home: 0,
      End: last,
    }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    tabList.current.children[next].focus();
    onChange(themes[next][0]);
  }
  return (
    <div ref={tabList} className="catalog-tabs" role="tablist" aria-label={label}>
      {themes.map(([id, zh, en, icon], index) => (
        <button key={id} id={tabId(id)} type="button" role="tab" aria-controls={panel}
          aria-selected={theme === id} tabIndex={theme === id ? 0 : -1}
          onClick={() => onChange(id)} onKeyDown={(event) => move(event, index)}>
          {icon && <Icon name={icon} size={14} />}{t(zh, en)}
        </button>
      ))}
    </div>
  );
}

function CatalogDialog({ title, author, className = "", t, onClose, children }) {
  const dialog = useRef(null);
  const titleElement = useRef(null);
  const heading = useId();
  useEffect(() => {
    dialog.current.showModal();
    titleElement.current.focus({ preventScroll: true });
  }, []);
  return (
    <dialog ref={dialog} className={`close-dialog ${className}`} aria-labelledby={heading} onClose={onClose}>
      <div className="dialog-header">
        <div className={author ? "catalog-dialog-heading work-title-line" : undefined}>
          <h2 id={heading} ref={titleElement} tabIndex={-1} data-dialog-title>{title}</h2>
          {author && <small>{author}</small>}
        </div>
        <button className="icon-button" type="button" aria-label={t("关闭", "Close")} onClick={() => dialog.current.close()}>
          <Icon name="close" size={17} />
        </button>
      </div>
      {children}
    </dialog>
  );
}

function AnimationPreview({ work, values, installed, clientTheme, locale, active, t, onConfigure, onClose }) {
  const [theme, setTheme] = useState(() => work.themes.includes(clientTheme) ? clientTheme : work.themes[0]);
  const [replay, setReplay] = useState(0);
  const [parameters, setParameters] = useState(() => ({
    ...Object.fromEntries((work.parameters || []).map((parameter) => [parameter.key, parameter.default])),
    ...values,
  }));
  const [saveError, setSaveError] = useState("");
  const latest = useRef(parameters);
  const pending = useRef(null);
  const timer = useRef(null);
  const save = useRef(onConfigure);
  save.current = onConfigure;

  function commit() {
    clearTimeout(timer.current);
    const changes = pending.current;
    if (!changes) return;
    const snapshot = latest.current;
    pending.current = null;
    void Promise.resolve().then(() => save.current?.(work.id, changes)).then((saved) => {
      if (saved === false && latest.current === snapshot) setSaveError(t("请检查参数值", "Check the parameter values"));
    }).catch((error) => {
      if (latest.current === snapshot) setSaveError((error?.message || String(error)).trimEnd().replace(/[.。]+$/, ""));
    });
  }

  function change(key, value, delay = 0) {
    if (latest.current[key] === value) return;
    const next = { ...latest.current, [key]: value };
    latest.current = next;
    setParameters(next);
    if (!installed) return;
    setSaveError("");
    pending.current = { ...pending.current, [key]: value };
    clearTimeout(timer.current);
    if (delay) timer.current = setTimeout(commit, delay);
    else commit();
  }

  useEffect(() => () => commit(), []);
  const translated = work.i18n?.[locale]?.parameters;
  const configurable = Boolean(work.parameters?.length);

  return (
    <CatalogDialog title={workName(work, locale)} author={work.author} className={`catalog-preview${configurable ? " has-parameters" : ""}`} t={t} onClose={() => { commit(); onClose(); }}>
      <div className="catalog-preview-content">
        <div className="catalog-preview-stage">
          <div className="catalog-preview-tools">
            <div className="theme-switch" role="group" aria-label={t("预览配色", "Preview theme")}>
              {themes.filter(([id]) => work.themes.includes(id)).map(([id, zh, en, icon]) => (
                <button key={id} type="button" aria-pressed={theme === id} onClick={() => setTheme(id)}>
                  <Icon name={icon} size={14} />{t(zh, en)}
                </button>
              ))}
            </div>
            <button className="icon-button" type="button" data-tooltip={t("重新播放", "Replay")}
              aria-label={t("重新播放", "Replay")} onClick={() => setReplay((count) => count + 1)}>
              <Icon name="refresh" size={16} />
            </button>
          </div>
          <StudioPreview workId={work.id} mode={installed ? "studio" : "market"} theme={theme}
            parameters={parameters} locale={locale} active={active} supported={Boolean(theme)} replay={replay} />
        </div>
        {configurable && (
          <aside className="catalog-parameters" aria-label={t("动画设置", "Animation settings")}>
            <h3>{t("动画设置", "Animation settings")}</h3>
            {work.parameters.map((parameter) => (
              <Parameter key={parameter.key} parameter={parameter} value={parameters[parameter.key]}
                label={parameter.key === "sound" ? t("音乐", "Music") : translated?.[parameter.key]?.label || parameter.label}
                onChange={(value, delay) => change(parameter.key, value, delay)} onCommit={commit} />
            ))}
            {saveError && <p className="catalog-config-error" role="alert" data-tooltip={saveError} tabIndex={0}>{t("无法应用动画设置", "Couldn’t apply animation settings")}</p>}
          </aside>
        )}
      </div>
    </CatalogDialog>
  );
}

export function WorkCatalog({
  entries, market, settings, installedWorks, getParameters, clientTheme, locale, loading, busy, error, active,
  onBrowse, onConfigure, onInstall, onUse, onUnload, onDelete,
}) {
  const [query, setQuery] = useState("");
  const [theme, setTheme] = useState("");
  const [preview, setPreview] = useState(null);
  const [deleting, setDeleting] = useState(null);
  const id = useId();
  const results = `${id}-results`;
  const tabId = (value) => `${id}-${value || "all"}`;
  const t = (zh, en) => locale === "en" ? en : zh;
  const search = query.trim().toLowerCase();
  const selected = (work) => settings.enabled && work.id === settings.workId;
  const installedById = new Map(installedWorks.map((work) => [work.id, work]));
  const catalog = entries.map(({ work, installedVersion, updateAvailable }) => {
    const local = installedById.get(work.id);
    return {
      work: market ? work : local || work,
      previewWork: local || work,
      installed: Boolean(local),
      updateAvailable: updateAvailable && (!market || local?.version === installedVersion),
    };
  });
  const visible = catalog.filter(({ work }) =>
    (!theme || work.themes.includes(theme)) &&
    [workName(work, locale), work.name, work.author, ...(work.tags || [])]
      .some((value) => value?.toLowerCase().includes(search)),
  );
  const group = ({ work, installed }) => selected(work) ? 0 : installed ? 1 : 2;
  visible.sort((a, b) => group(a) - group(b));
  const previewEntry = preview ? catalog.find(({ work }) => work.id === preview) : null;
  return (
    <section aria-label={market ? t("社区市场", "Marketplace") : t("启动动画库", "Startup animation library")}>
      <div className="catalog-controls">
        <label className="market-toolbar">
          <Icon name="search" size={16} />
          <input type="search" aria-label={t("搜索作品", "Search animations")}
            placeholder={t("搜索作品或作者", "Search by name or author")} value={query} onChange={(event) => setQuery(event.target.value)} />
        </label>
        {!loading && <span className="catalog-count">{t(`${visible.length} 个作品`, `${visible.length} animations`)}</span>}
      </div>
      {market && <ThemeTabs label={t("主题筛选", "Filter by theme")} theme={theme} panel={results} tabId={tabId} t={t} onChange={setTheme} />}
      {error && <p className="market-notice" role="alert" data-tooltip={error.detail || undefined} tabIndex={error.detail ? 0 : undefined}>{error.text}</p>}
      <div id={results} role={market ? "tabpanel" : undefined} aria-labelledby={market ? tabId(theme) : undefined}>
        {loading && <p className="market-empty" role="status">{t("正在读取市场…", "Loading marketplace…")}</p>}
        {!loading && !visible.length && (
          <div className="market-empty">
            <p>{search || theme ? t("没有匹配的作品", "No matching animations")
              : market ? t("市场暂无作品", "No animations available") : t("动画库暂无作品", "The library is empty")}</p>
            {!market && !search && <button className="button" type="button" onClick={onBrowse}>{t("浏览市场", "Browse marketplace")}</button>}
          </div>
        )}
        <div className="market-grid">
          {visible.map(({ work, installed, updateAvailable }) => (
            <article className={`market-card${selected(work) ? " active-work" : ""}`} key={work.id}>
              <div className="market-cover">
                <PrismMark size={48} />
                {work.previewUrl && <img src={work.previewUrl} alt="" loading="lazy"
                  onLoad={(event) => { event.currentTarget.hidden = false; }} onError={(event) => { event.currentTarget.hidden = true; }} />}
                {(selected(work) || market && installed) && <span className="active-work-label">
                  <Icon name="check" size={13} />{selected(work) ? t("使用中", "In use") : t("已安装", "Installed")}
                </span>}
              </div>
              <div className="market-card-body">
                <div className="work-title-line">
                  <h2>{workName(work, locale)}</h2>
                  {work.author && <small>{work.author}</small>}
                </div>
                <WorkTags work={work} t={t} />
                <div className="work-card-actions">
                  <button className="button work-preview-button" type="button" disabled={busy} onClick={() => setPreview(work.id)}>
                    <Icon name="sliders" size={14} />{t("预览", "Preview")}
                  </button>
                  {installed
                    ? <>
                      {selected(work)
                        ? <button className="button" type="button" disabled={busy} onClick={() => onUnload(work.id)}>
                          <Icon name="power" size={14} />{t("卸载", "Unload")}
                        </button>
                        : <button className="button primary" type="button" disabled={busy} onClick={() => onUse(work.id)}>
                          {t("使用", "Use")}
                        </button>}
                      {updateAvailable && <button className="icon-button" type="button" disabled={busy} data-tooltip={t("更新", "Update")}
                        aria-label={t(`更新 ${workName(work, locale)}`, `Update ${workName(work, locale)}`)} onClick={() => onInstall(work.id)}>
                        <Icon name="refresh" size={15} />
                      </button>}
                      <button className="icon-button library-delete" type="button" disabled={busy}
                        data-tooltip={t("删除作品", "Delete animation")} aria-label={t(`删除 ${workName(work, locale)}`, `Delete ${workName(work, locale)}`)}
                        onClick={() => setDeleting(work)}><Icon name="trash" size={16} /></button>
                    </>
                    : <button className="button primary" type="button" disabled={busy} onClick={() => onInstall(work.id)}>
                      <Icon name="download" size={15} />{t("下载安装", "Download and install")}
                    </button>}
                </div>
              </div>
            </article>
          ))}
        </div>
      </div>
      {previewEntry && <AnimationPreview key={`${preview}:${previewEntry.installed}`} work={previewEntry.previewWork} locale={locale}
        clientTheme={clientTheme} t={t} active={active} installed={previewEntry.installed} onConfigure={onConfigure}
        values={previewEntry.installed ? getParameters?.(preview) ?? (settings.workId === preview ? settings.parameters : {}) : {}}
        onClose={() => setPreview(null)} />}
      {deleting && <CatalogDialog title={t(`删除「${workName(deleting, locale)}」？`, `Delete “${workName(deleting, locale)}”?`)}
        className="catalog-delete" t={t} onClose={() => setDeleting(null)}>
        <p>{t("将删除此作品的本地文件", "This deletes the animation’s local files")}</p>
        <div className="dialog-actions">
          <button className="button" type="button" onClick={() => setDeleting(null)}>{t("取消", "Cancel")}</button>
          <button className="button danger" type="button" disabled={busy} onClick={() => {
            onDelete(deleting.id);
            setDeleting(null);
          }}>{t("删除作品", "Delete animation")}</button>
        </div>
      </CatalogDialog>}
    </section>
  );
}
