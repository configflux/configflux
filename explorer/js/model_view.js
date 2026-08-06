// SPDX-License-Identifier: BUSL-1.1
//
// Model view (ADR-0043 §2.1): browse a compiled model's component / facet /
// option structure as a collapsible tree with substring search, above a hash
// header (a hash + schema version + counts). The two artifacts carry different
// hashes and label them so: `inspect summary` reports a source_digest over
// content alone (ADR-0056 §9), `cfx options` a model_hash from the compiled
// package. It renders two real pipeline
// artifacts — the `inspect summary` inventory (components/definitions/artifacts)
// and the `cfx options` facet→option listing — auto-detected by shape, so the
// user can browse either half of the model's static structure.

import { el, clear, announce, shortHash } from "./dom.js";
import { KIND } from "./schema.js";

/** Render a model artifact into `container`. `kind` is KIND.MODEL or KIND.FACETS. */
export function renderModel(container, data, kind) {
  clear(container);
  const groups = kind === KIND.FACETS ? facetGroups(data) : inventoryGroups(data);
  const header = kind === KIND.FACETS ? facetHeader(data) : inventoryHeader(data);

  container.appendChild(hashHeader(header));

  const search = el("input", {
    type: "search",
    id: "model-search",
    class: "search-box",
    placeholder: "Filter the tree…",
    "aria-label": "Filter model tree by substring",
    autocomplete: "off",
    spellcheck: "false",
  });
  const treeHost = el("div", { class: "tree-host" });
  const empty = el("p", { class: "muted hidden", role: "status", text: "No matches." });

  const paint = () => {
    const q = search.value.trim().toLowerCase();
    clear(treeHost);
    const visible = buildTree(treeHost, groups, q);
    empty.classList.toggle("hidden", visible > 0 || q === "");
  };
  search.addEventListener("input", paint);

  container.appendChild(el("div", { class: "toolbar" }, search));
  container.appendChild(treeHost);
  container.appendChild(empty);
  paint();
  announce(`Loaded ${header.title}: ${header.summaryLine}`);
}

/** The prominent hash + counts header shared by both model shapes. */
function hashHeader(header) {
  const rows = header.facts.map((f) =>
    el("div", { class: "hash-row" }, [
      el("span", { class: "hash-key", text: f.key }),
      el("span", { class: "hash-val", title: f.full || f.value, text: f.value }),
    ])
  );
  return el("section", { class: "hash-header", "aria-label": "Model identity" }, [
    el("h2", { text: header.title }),
    el("div", { class: "hash-grid" }, rows),
  ]);
}

function inventoryHeader(data) {
  const s = data.summary || {};
  return {
    title: "Model summary",
    summaryLine: `${s.component_count ?? 0} components, ${s.definition_count ?? 0} definitions, ${s.artifact_count ?? 0} artifacts`,
    facts: [
      { key: "source_digest", value: shortHash(data.source_digest), full: data.source_digest },
      { key: "schema_version", value: String(data.schema_version) },
      { key: "sources", value: String(s.source_count ?? 0) },
      { key: "components", value: String(s.component_count ?? 0) },
      { key: "definitions", value: String(s.definition_count ?? 0) },
      { key: "artifacts", value: String(s.artifact_count ?? 0) },
    ],
  };
}

function facetHeader(data) {
  const first = data[0] || {};
  const totalOptions = data.reduce((n, f) => n + (f.valid_options?.length || 0), 0);
  return {
    title: "Facet options",
    summaryLine: `${data.length} facets, ${totalOptions} options`,
    facts: [
      { key: "model_hash", value: shortHash(first.model_hash), full: first.model_hash },
      { key: "schema_version", value: String(first.schema_version) },
      { key: "facets", value: String(data.length) },
      { key: "options", value: String(totalOptions) },
    ],
  };
}

/** Inventory groups from an `inspect summary` artifact. */
function inventoryGroups(data) {
  const s = data.summary || {};
  return [
    { label: "Components", items: (s.component_ids || []).slice() },
    { label: "Facets", items: [], note: "Load cfx options JSON to browse facet options" },
    { label: "Definitions", items: (s.definition_ids || []).slice() },
    { label: "Artifacts", items: (s.artifact_ids || []).slice() },
  ];
}

/** Facet→option groups from a `cfx options` artifact (array of per-facet results). */
function facetGroups(data) {
  return data
    .slice()
    .sort((a, b) => String(a.facet).localeCompare(String(b.facet)))
    .map((f) => ({ label: `facet ${f.facet}`, items: (f.valid_options || []).slice() }));
}

/**
 * Build the tree into `host`, filtered by lowercase substring `q`. A group is
 * shown when its label matches or any of its items match; matching groups are
 * expanded. Returns the number of visible leaf items (0 ⇒ no matches).
 * Uses ARIA tree roles so the structure is navigable with assistive tech.
 */
function buildTree(host, groups, q) {
  const tree = el("ul", { class: "tree", role: "tree", "aria-label": "Model structure" });
  let visibleLeaves = 0;

  for (const group of groups) {
    const labelMatch = q === "" || group.label.toLowerCase().includes(q);
    const items = group.items.filter((it) => q === "" || String(it).toLowerCase().includes(q));
    const show = q === "" ? true : labelMatch || items.length > 0;
    if (!show) continue;

    const shownItems = labelMatch ? group.items : items;
    visibleLeaves += shownItems.length;
    const expanded = q !== "" || shownItems.length <= 12;

    const leafList = el(
      "ul",
      { class: "tree-group", role: "group" },
      shownItems.length
        ? shownItems.map((it) =>
            el("li", { class: "tree-leaf", role: "treeitem", tabindex: "-1", text: String(it) })
          )
        : el("li", {
            class: "tree-leaf muted",
            role: "treeitem",
            tabindex: "-1",
            text: group.note || "(none)",
          })
    );
    leafList.classList.toggle("collapsed", !expanded);

    const twisty = el("button", {
      type: "button",
      class: "twisty",
      "aria-expanded": String(expanded),
      text: `${group.label} (${group.items.length})`,
      onClick: (ev) => {
        const btn = ev.currentTarget;
        const open = btn.getAttribute("aria-expanded") === "true";
        btn.setAttribute("aria-expanded", String(!open));
        leafList.classList.toggle("collapsed", open);
      },
    });

    tree.appendChild(el("li", { class: "tree-branch", role: "treeitem", "aria-expanded": String(expanded) }, [twisty, leafList]));
  }

  host.appendChild(tree);
  return visibleLeaves;
}
