/* ConfigFlux landing page — vanilla interactivity (ported from the design
   export's component logic). No framework, no runtime. */
(function () {
  "use strict";

  // --- footer year ---
  var yearEl = document.getElementById("cf-year");
  if (yearEl) yearEl.textContent = String(new Date().getFullYear());

  // --- hover styles (port of the export's `style-hover` directive) ---
  document.querySelectorAll("[style-hover]").forEach(function (el) {
    var base = el.getAttribute("style") || "";
    var hover = el.getAttribute("style-hover") || "";
    el.addEventListener("mouseenter", function () { el.style.cssText = base + ";" + hover; });
    el.addEventListener("mouseleave", function () { el.style.cssText = base; });
  });

  // --- mobile menu ---
  function setMenu(open) {
    var m = document.getElementById("mobile-menu");
    if (m) m.hidden = !open;
    var h = document.querySelector('[data-action="toggle-menu"]');
    if (h) h.setAttribute("aria-expanded", open ? "true" : "false");
  }

  // --- quickstart tabs ---
  var TAB_ACTIVE = "flex:1 1 auto; padding:13px 18px; border:none; border-bottom:2px solid var(--accent); background:var(--surface); font-family:'IBM Plex Mono', monospace; font-size:13px; cursor:pointer; color:var(--fg); font-weight:500;";
  var TAB_BASE = "flex:1 1 auto; padding:13px 18px; border:none; border-bottom:2px solid transparent; background:transparent; font-family:'IBM Plex Mono', monospace; font-size:13px; cursor:pointer; color:var(--muted); font-weight:500;";
  function setTab(which) {
    document.querySelectorAll('[data-panel="source"]').forEach(function (p) { p.hidden = which !== "source"; });
    document.querySelectorAll('[data-panel="binary"]').forEach(function (p) { p.hidden = which !== "binary"; });
    var s = document.querySelector('[data-tab="source"]');
    var b = document.querySelector('[data-tab="binary"]');
    if (s) s.setAttribute("style", which === "source" ? TAB_ACTIVE : TAB_BASE);
    if (b) b.setAttribute("style", which === "binary" ? TAB_ACTIVE : TAB_BASE);
  }

  // --- copy-to-clipboard for the visible quickstart command ---
  function copyFrom(btn) {
    var panel = btn.closest("[data-panel]");
    var pre = panel && panel.querySelector("pre");
    if (!pre) return;
    try { navigator.clipboard.writeText(pre.innerText); } catch (e) {}
    btn.textContent = "Copied ✓";
    setTimeout(function () { btn.textContent = "Copy"; }, 1800);
  }

  // --- concepts accordion ---
  function toggleConcepts(btn) {
    var d = document.getElementById("concepts-detail");
    if (!d) return;
    var opening = d.hidden;
    d.hidden = !opening;
    btn.setAttribute("aria-expanded", opening ? "true" : "false");
    var icon = btn.querySelector("span");
    if (icon) icon.textContent = opening ? "−" : "+";
  }

  // --- event delegation ---
  document.addEventListener("click", function (ev) {
    var t = ev.target.closest("[data-action]");
    if (!t) return;
    var a = t.getAttribute("data-action");
    if (a === "toggle-menu") { var m = document.getElementById("mobile-menu"); setMenu(m ? m.hidden : true); }
    else if (a === "close-menu") setMenu(false);
    else if (a === "tab-source") setTab("source");
    else if (a === "tab-binary") setTab("binary");
    else if (a === "copy-source" || a === "copy-binary") copyFrom(t);
    else if (a === "toggle-concepts") toggleConcepts(t);
  });
})();
