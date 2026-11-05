import React, { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "./tooltip.css";

const anchorFor = (target) => target instanceof Element ? target.closest("[data-tooltip]") : null;

export function Tooltip() {
  const [anchor, setAnchor] = useState(null);
  const popup = useRef(null);
  const id = useId();

  useEffect(() => {
    let hovered;
    let focused;
    let visible;
    let timer;

    function show(next, delay) {
      clearTimeout(timer);
      if (next === visible) return;
      visible = null;
      setAnchor(null);
      if (!next?.dataset.tooltip) return;
      timer = setTimeout(() => {
        if (!next.isConnected) return;
        visible = next;
        setAnchor(next);
      }, delay);
    }
    function dismiss() {
      hovered = null;
      focused = null;
      show(null);
    }
    function onPointerOver(event) {
      if (event.pointerType === "touch") return;
      const target = anchorFor(event.target);
      if (!target || target === hovered) return;
      hovered = target;
      show(target, 360);
    }
    function onPointerOut(event) {
      const from = event.target;
      const to = event.relatedTarget;
      if (!hovered || (!hovered.contains(from) && !popup.current?.contains(from))) return;
      if (to instanceof Node && (hovered.contains(to) || popup.current?.contains(to))) return;
      hovered = null;
      show(focused, 0);
    }
    function onFocusIn(event) {
      focused = anchorFor(event.target);
      if (focused?.matches(":focus-visible")) show(focused, 0);
      else focused = null;
    }
    function onFocusOut() {
      focused = null;
      show(hovered, 360);
    }
    function onKeyDown(event) {
      if (event.key !== "Escape" || !visible) return;
      event.preventDefault();
      event.stopPropagation();
      dismiss();
    }

    document.addEventListener("pointerover", onPointerOver);
    document.addEventListener("pointerout", onPointerOut);
    document.addEventListener("pointerdown", dismiss, true);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusOut);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", dismiss);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("pointerover", onPointerOver);
      document.removeEventListener("pointerout", onPointerOut);
      document.removeEventListener("pointerdown", dismiss, true);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("focusout", onFocusOut);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", dismiss);
    };
  }, []);

  const hint = anchor?.dataset.tooltip;
  useLayoutEffect(() => {
    if (!anchor || !hint) return;
    const tip = popup.current;
    const describedBy = anchor.getAttribute("aria-describedby");
    const describes = anchor.getAttribute("aria-label") !== hint;
    if (describes) anchor.setAttribute("aria-describedby", describedBy ? `${describedBy} ${id}` : id);
    function place() {
      if (!anchor.isConnected) {
        setAnchor(null);
        return;
      }
      const gap = 7;
      const margin = 8;
      const topMargin = anchor.closest(".topbar-actions") ? 0 : margin;
      const bounds = anchor.getBoundingClientRect();
      const { width, height } = tip.getBoundingClientRect();
      const middle = (bounds.left + bounds.right - width) / 2;
      tip.style.left = `${Math.max(margin, Math.min(middle, window.innerWidth - width - margin))}px`;
      const top = bounds.top - height - gap;
      tip.style.top = `${Math.max(topMargin, Math.min(top, window.innerHeight - height - margin))}px`;
      tip.style.pointerEvents = top < topMargin ? "none" : "";
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      if (describes) {
        if (describedBy) anchor.setAttribute("aria-describedby", describedBy);
        else anchor.removeAttribute("aria-describedby");
      }
    };
  }, [anchor, hint, id]);

  if (!anchor || !hint) return null;
  return createPortal(
    <span className={anchor.closest(".topbar-actions") ? "ui-tooltip topbar-tooltip" : "ui-tooltip"} id={id} ref={popup} role="tooltip">{hint}</span>,
    anchor.closest("dialog[open]") || document.body,
  );
}
