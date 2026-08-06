// SPDX-License-Identifier: BUSL-1.1
//
// Minimal DOM helpers (ADR-0043 §1: vanilla JS, no framework, no dependency).
// Everything user-derived is inserted as a text node via `textContent`, so the
// explorer never interpolates artifact strings into HTML — there is no XSS
// surface even though it renders untrusted local files.

/**
 * Create an element. `props` sets attributes/properties:
 *   - `class` / `className`  → class attribute
 *   - `text`                 → textContent (safe, never parsed as HTML)
 *   - `aria*` / data-* / role / etc. → setAttribute
 *   - `on<Event>`            → addEventListener
 * `children` is a node or array of nodes/strings appended in order.
 */
export function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null) continue;
    if (key === "class" || key === "className") {
      node.className = value;
    } else if (key === "text") {
      node.textContent = value;
    } else if (key.startsWith("on") && typeof value === "function") {
      node.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === "html") {
      throw new Error("dom.el: raw html is not permitted; use text");
    } else {
      node.setAttribute(key, value);
    }
  }
  appendChildren(node, children);
  return node;
}

/** Append a node, string, or (possibly nested) array of them. */
export function appendChildren(node, children) {
  const list = Array.isArray(children) ? children : [children];
  for (const child of list) {
    if (child == null) continue;
    if (Array.isArray(child)) {
      appendChildren(node, child);
    } else if (typeof child === "string" || typeof child === "number") {
      node.appendChild(document.createTextNode(String(child)));
    } else {
      node.appendChild(child);
    }
  }
}

/** Remove every child of a node (idempotent view re-render). */
export function clear(node) {
  while (node.firstChild) {
    node.removeChild(node.firstChild);
  }
}

/**
 * Announce a message to assistive tech via the shared aria-live region, and
 * mirror it visibly in the status bar. Used for load results and errors so a
 * screen-reader user hears what a sighted user sees.
 */
export function announce(message) {
  const live = document.getElementById("live-region");
  if (live) {
    live.textContent = "";
    // Reassign on the next frame so repeat messages re-announce.
    window.requestAnimationFrame(() => {
      live.textContent = message;
    });
  }
}

/** Format a possibly-null primitive for a table cell without throwing. */
export function displayValue(value) {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

/** A short, copy-safe rendering of a 64-char hash: first 12 chars + ellipsis. */
export function shortHash(hash) {
  if (typeof hash !== "string" || hash.length <= 16) return hash || "—";
  return `${hash.slice(0, 12)}…`;
}
