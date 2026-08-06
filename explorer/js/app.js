// SPDX-License-Identifier: BUSL-1.1
//
// Application bootstrap (ADR-0043): wires the file controls to the three views,
// manages tab navigation (keyboard-accessible), the light/dark theme toggle,
// and the honest error region. No network, no backend, no framework.

import { wireFileControls } from "./loader.js";
import { KIND } from "./schema.js";
import { renderModel } from "./model_view.js";
import { renderResolution } from "./resolution_view.js";
import { renderExplain } from "./explain_view.js";
import { clear, announce, el } from "./dom.js";

const TABS = ["model", "resolution", "explain"];
const KIND_TO_TAB = {
  [KIND.MODEL]: "model",
  [KIND.FACETS]: "model",
  [KIND.RESOLVE]: "resolution",
  [KIND.EXPLAIN]: "explain",
};

function panel(tab) {
  return document.getElementById(`panel-${tab}`);
}
function tabButton(tab) {
  return document.getElementById(`tab-${tab}`);
}

/** Show one tab, update ARIA state, and (optionally) move focus to its button. */
function selectTab(tab, focus = false) {
  for (const t of TABS) {
    const isActive = t === tab;
    const btn = tabButton(t);
    const pnl = panel(t);
    if (btn) {
      btn.setAttribute("aria-selected", String(isActive));
      btn.setAttribute("tabindex", isActive ? "0" : "-1");
    }
    if (pnl) pnl.hidden = !isActive;
  }
  if (focus) tabButton(tab)?.focus();
}

/** Arrow-key roving tabindex for the tablist (WAI-ARIA tabs pattern). */
function wireTabs() {
  for (const t of TABS) {
    const btn = tabButton(t);
    if (!btn) continue;
    btn.addEventListener("click", () => selectTab(t));
    btn.addEventListener("keydown", (ev) => {
      const i = TABS.indexOf(t);
      let next = null;
      if (ev.key === "ArrowRight" || ev.key === "ArrowDown") next = TABS[(i + 1) % TABS.length];
      else if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") next = TABS[(i - 1 + TABS.length) % TABS.length];
      else if (ev.key === "Home") next = TABS[0];
      else if (ev.key === "End") next = TABS[TABS.length - 1];
      if (next) {
        ev.preventDefault();
        selectTab(next, true);
      }
    });
  }
}

/** Render a loaded artifact into its panel and reveal that tab. */
function routeResult(result) {
  const errorRegion = document.getElementById("error-region");
  clear(errorRegion);
  errorRegion.hidden = true;

  if (!result.ok) {
    errorRegion.hidden = false;
    errorRegion.appendChild(
      el("div", { class: "error-card", role: "alert" }, [
        el("strong", { text: "Could not load that file. " }),
        el("span", { text: result.error }),
      ])
    );
    announce(`Error: ${result.error}`);
    return;
  }

  const tab = KIND_TO_TAB[result.kind];
  const host = panel(tab);
  try {
    if (result.kind === KIND.MODEL || result.kind === KIND.FACETS) {
      renderModel(host, result.data, result.kind);
    } else if (result.kind === KIND.RESOLVE) {
      renderResolution(host, result.data);
    } else if (result.kind === KIND.EXPLAIN) {
      renderExplain(host, result.data);
    }
    setLoadedName(result.name);
    selectTab(tab);
  } catch (err) {
    errorRegion.hidden = false;
    errorRegion.appendChild(
      el("div", { class: "error-card", role: "alert" }, [
        el("strong", { text: "That artifact could not be rendered. " }),
        el("span", { text: err.message }),
      ])
    );
    announce(`Render error: ${err.message}`);
  }
}

function setLoadedName(name) {
  const badge = document.getElementById("loaded-name");
  if (badge) badge.textContent = name ? `loaded: ${name}` : "";
}

// --- theme (dark mode) -------------------------------------------------------

const THEME_KEY = "configflux-explorer-theme";

function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === "light" || theme === "dark") {
    root.setAttribute("data-theme", theme);
  } else {
    root.removeAttribute("data-theme"); // fall back to prefers-color-scheme
  }
  const btn = document.getElementById("theme-toggle");
  if (btn) {
    const effective = theme || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    btn.setAttribute("aria-pressed", String(effective === "dark"));
    btn.textContent = effective === "dark" ? "Light mode" : "Dark mode";
  }
}

function storedTheme() {
  try {
    return window.localStorage.getItem(THEME_KEY) || "";
  } catch {
    return "";
  }
}

function wireTheme() {
  applyTheme(storedTheme());
  const btn = document.getElementById("theme-toggle");
  if (!btn) return;
  btn.addEventListener("click", () => {
    const current =
      document.documentElement.getAttribute("data-theme") ||
      (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    const next = current === "dark" ? "light" : "dark";
    try {
      window.localStorage.setItem(THEME_KEY, next);
    } catch {
      /* storage unavailable (private mode / file://): theme still applies for this session */
    }
    applyTheme(next);
    announce(`${next} mode`);
  });
}

function init() {
  wireTabs();
  wireTheme();
  wireFileControls({
    inputEl: document.getElementById("file-input"),
    dropEl: document.getElementById("drop-zone"),
    onResult: routeResult,
  });
  selectTab("model");
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
