import React, { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./icons.jsx";

export function Select({ label, options, value, disabled, onChange, className = "", selectedContent }) {
  const listId = useId();
  const button = useRef(null);
  const menu = useRef(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [position, setPosition] = useState(null);
  const selected = Math.max(0, options.findIndex((option) => option.value === value));
  const expanded = open && !disabled && options.length > 0;
  function choose(index) {
    if (options[index] && options[index].value !== value) onChange(options[index].value);
    setOpen(false);
  }
  function show(index = selected) {
    button.current?.focus({ preventScroll: true });
    setActive(index);
    setOpen(true);
  }
  useLayoutEffect(() => {
    if (!expanded) return;
    const update = () => {
      const bounds = button.current.getBoundingClientRect();
      const below = window.innerHeight - bounds.bottom - 12;
      const above = bounds.top - 12;
      const dropDown = below >= Math.min(190, options.length * 35) || below >= above;
      setPosition({
        top: dropDown ? bounds.bottom + 5 : undefined,
        bottom: dropDown ? undefined : window.innerHeight - bounds.top + 5,
        left: Math.max(8, Math.min(bounds.left, window.innerWidth - Math.max(130, bounds.width) - 8)),
        width: Math.max(130, bounds.width),
        maxHeight: Math.min(245, Math.max(42, dropDown ? below : above)),
      });
    };
    const outside = (event) => {
      if (!button.current?.contains(event.target) && !menu.current?.contains(event.target)) setOpen(false);
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    window.addEventListener("pointerdown", outside, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("pointerdown", outside, true);
    };
  }, [expanded, options.length]);
  useLayoutEffect(() => {
    if (expanded) menu.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [expanded, active, position?.width]);
  function keyDown(event) {
    const last = options.length - 1;
    if (event.key === "Tab") {
      setOpen(false);
    } else if (event.key === "Escape" && expanded) {
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (expanded) choose(active);
      else show();
    } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const next = event.key === "Home" ? 0 : event.key === "End" ? last :
        Math.max(0, Math.min(last, (expanded ? active : selected) + (event.key === "ArrowDown" ? 1 : -1)));
      show(next);
    } else if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
      const match = options.findIndex(({ label }) => label.toLowerCase().startsWith(event.key.toLowerCase()));
      if (match >= 0) {
        event.preventDefault();
        show(match);
      }
    }
  }
  return (
    <>
      <button ref={button} className={`select-trigger ${className}`} type="button" role="combobox" aria-label={label} aria-haspopup="listbox"
        aria-expanded={expanded} aria-controls={expanded ? listId : undefined}
        aria-activedescendant={expanded ? `${listId}-${active}` : undefined}
        disabled={disabled || !options.length} onClick={() => expanded ? setOpen(false) : show()}
        onKeyDown={keyDown} onBlur={() => setOpen(false)}>
        {selectedContent || <span>{options[selected]?.label || "—"}</span>}<Icon name="down" size={14} />
      </button>
      {expanded && position && createPortal(
        <div ref={menu} id={listId} className="select-menu" role="listbox" aria-label={label} style={position}>
          {options.map((option, index) => (
            <div key={option.value} id={`${listId}-${index}`} role="option" aria-selected={option.value === value}
              className={active === index ? "highlighted" : undefined} onMouseEnter={() => setActive(index)}
              onMouseDown={(event) => event.preventDefault()} onClick={() => choose(index)}>
              <span>{option.label}</span>{option.value === value && <Icon name="check" size={14} />}
            </div>
          ))}
        </div>,
        button.current?.closest("dialog") || document.body,
      )}
    </>
  );
}

export function Switch({ label, compact, checked, disabled, onChange }) {
  return (
    <label className={compact ? "switch compact" : "switch"}>
      {!compact && <span>{label}</span>}
      <input type="checkbox" role="switch" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
      <span className="switch-track" aria-hidden="true" />
    </label>
  );
}

function RangeControl({ parameter, label, value, disabled, onChange, onCommit }) {
  const [numberDraft, setNumberDraft] = useState(null);
  const progress = `${100 * (value - parameter.min) / (parameter.max - parameter.min)}%`;
  const display = String(Number(value.toFixed(parameter.key === "speed" || parameter.key === "volume" ? 2 : 6)));
  const step = parameter.key === "speed" || parameter.key === "volume" ? "any" : parameter.step || "any";
  return (
    <div className="parameter-range">
      <input type="range" aria-label={label} style={{ "--progress": progress }}
        disabled={disabled} min={parameter.min} max={parameter.max} step={step} value={value}
        onChange={(event) => onChange(event.target.valueAsNumber, 140)}
        onPointerUp={onCommit} onKeyUp={onCommit} onBlur={onCommit} />
      <input type="number" aria-label={label}
        min={parameter.min} max={parameter.max} step={step} disabled={disabled} value={numberDraft ?? display}
        onChange={(event) => {
          const next = event.target.valueAsNumber;
          setNumberDraft(event.target.value);
          if (Number.isFinite(next) && next >= parameter.min && next <= parameter.max) onChange(next, 140);
        }}
        onBlur={() => {
          setNumberDraft(null);
          onCommit?.();
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
          }
        }} />
    </div>
  );
}

export function Parameter({ parameter, label, value, disabled, onChange, onCommit }) {
  return (
    <div className="parameter-line">
      <span>{label}</span>
      <div className="parameter-control">
        {parameter.type === "boolean" ? <Switch compact label={label} checked={Boolean(value)} disabled={disabled} onChange={onChange} />
          : parameter.type === "number" ? <RangeControl parameter={parameter} label={label} value={value} disabled={disabled} onChange={onChange} onCommit={onCommit} />
          : parameter.type === "select" ? <Select label={label} value={value} disabled={disabled} options={parameter.options} onChange={onChange} />
          : <input type={parameter.type === "color" ? "color" : "text"} aria-label={label}
              disabled={disabled} value={value} maxLength={parameter.maxLength}
              onChange={(event) => onChange(event.target.value, 240)} onBlur={onCommit}
              onKeyDown={(event) => { if (event.key === "Enter") onCommit?.(); }} />}
      </div>
    </div>
  );
}
