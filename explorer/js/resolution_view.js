// SPDX-License-Identifier: BUSL-1.1
//
// Resolution view (ADR-0043 §2.2): render a resolved snapshot (`cfx resolve`
// ResolveResult). The hash lineage model_hash → selection_state_hash →
// resolve_hash is shown prominently; below it a searchable parameters table
// flattens resolved_output (scope → component → parameter), and between the two
// a requirements table shows what each component's `requires` block resolved to
// (ADR-0057 §D7).

import { el, clear, announce, displayValue, shortHash } from "./dom.js";

/** Render a ResolveResult into `container`. */
export function renderResolution(container, data) {
  clear(container);
  container.appendChild(lineage(data));
  container.appendChild(choices(data));

  const requirementRows = flattenRequirements(data.resolved_output);
  if (requirementRows.length > 0) {
    container.appendChild(requirements(requirementRows));
  }

  const rows = flattenParams(data.resolved_output);
  const search = el("input", {
    type: "search",
    id: "resolve-search",
    class: "search-box",
    placeholder: "Filter parameters…",
    "aria-label": "Filter parameters by substring",
    autocomplete: "off",
    spellcheck: "false",
  });
  const tableHost = el("div", { class: "table-host" });
  const empty = el("p", { class: "muted hidden", role: "status", text: "No matching parameters." });

  const paint = () => {
    const q = search.value.trim().toLowerCase();
    const shown = q === "" ? rows : rows.filter((r) => rowMatches(r, q));
    clear(tableHost);
    tableHost.appendChild(paramsTable(shown));
    empty.classList.toggle("hidden", shown.length > 0);
  };
  search.addEventListener("input", paint);

  container.appendChild(
    el("section", { class: "params", "aria-label": "Resolved parameters" }, [
      el("h2", { text: `Resolved parameters (${rows.length})` }),
      el("div", { class: "toolbar" }, search),
      tableHost,
      empty,
    ])
  );
  paint();
  announce(`Loaded resolved snapshot: ${rows.length} parameters in scope ${data.scope ?? "?"}`);
}

/** The three-stage hash lineage with explicit arrows. */
function lineage(data) {
  const stages = [
    { key: "model_hash", value: data.model_hash },
    { key: "selection_state_hash", value: data.selection_state_hash },
    { key: "resolve_hash", value: data.resolve_hash },
  ];
  const chain = [];
  stages.forEach((s, i) => {
    chain.push(
      el("div", { class: "lineage-stage" }, [
        el("span", { class: "hash-key", text: s.key }),
        el("code", { class: "hash-val", title: s.value || "—", text: shortHash(s.value) }),
      ])
    );
    if (i < stages.length - 1) {
      chain.push(el("span", { class: "lineage-arrow", "aria-hidden": "true", text: "→" }));
    }
  });
  return el("section", { class: "lineage", "aria-label": "Hash lineage" }, [
    el("h2", { text: "Hash lineage" }),
    el("div", { class: "lineage-chain" }, chain),
  ]);
}

/** The selection that produced this snapshot (choices + context tags). */
function choices(data) {
  const pairs = [
    ...Object.entries(data.choices || {}).map(([k, v]) => ({ k, v, kind: "choice" })),
    ...Object.entries(data.context_tags || {}).map(([k, v]) => ({ k, v, kind: "context" })),
  ];
  if (pairs.length === 0) {
    return el("section", { class: "choices" }, el("p", { class: "muted", text: "Empty selection (defaults only)." }));
  }
  return el("section", { class: "choices", "aria-label": "Selection" }, [
    el("h2", { text: "Selection" }),
    el(
      "ul",
      { class: "chip-list" },
      pairs.map((p) =>
        el("li", { class: `chip chip-${p.kind}` }, [
          el("span", { class: "chip-key", text: p.k }),
          el("span", { class: "chip-eq", "aria-hidden": "true", text: "=" }),
          el("span", { class: "chip-val", text: displayValue(p.v) }),
        ])
      )
    ),
  ]);
}

/**
 * Flatten resolved_output (scope → { components: { id → { requires } } }) into
 * one row per requirement FIELD (ADR-0057 §D7). Sorted at every level for the
 * same reason the parameters table is: the rendered order is a property of the
 * data, never of object insertion order.
 *
 * `requires` is skip-if-empty in the snapshot, so a model that declares no
 * requirement yields no rows and the section is not rendered at all.
 */
function flattenRequirements(resolvedOutput) {
  const rows = [];
  if (!resolvedOutput || typeof resolvedOutput !== "object") return rows;
  for (const scope of Object.keys(resolvedOutput).sort()) {
    const components = resolvedOutput[scope]?.components || {};
    for (const compId of Object.keys(components).sort()) {
      const requires = components[compId]?.requires || {};
      for (const slot of Object.keys(requires).sort()) {
        const req = requires[slot] || {};
        const fields = req.fields || {};
        for (const field of Object.keys(fields).sort()) {
          rows.push({
            scope,
            component: compId,
            slot,
            binding: req.binding ?? "",
            entry: req.entry ?? "",
            field,
            value: fields[field],
          });
        }
      }
    }
  }
  return rows;
}

const REQUIREMENT_COLUMNS = ["scope", "component", "slot", "binding", "entry", "field", "value"];

/** The catalogue entries this snapshot delivered, one row per field. */
function requirements(rows) {
  const head = el(
    "tr",
    {},
    REQUIREMENT_COLUMNS.map((c) => el("th", { scope: "col", text: c }))
  );
  const body = rows.map((r) =>
    el("tr", {}, [
      el("td", { text: r.scope }),
      el("td", { text: r.component }),
      el("td", { class: "cell-param", text: r.slot }),
      el("td", { text: r.binding }),
      el("td", { class: "cell-entry", text: r.entry }),
      el("td", { class: "cell-param", text: r.field }),
      el("td", { class: "cell-value", text: displayValue(r.value) }),
    ])
  );
  return el("section", { class: "requires", "aria-label": "Resolved requirements" }, [
    el("h2", { text: `Requirements (${rows.length})` }),
    el("table", { class: "params-table" }, [el("thead", {}, head), el("tbody", {}, body)]),
  ]);
}

/**
 * Flatten resolved_output (scope → { components: { id → { params } } }) into a
 * flat, sorted list of table rows. Determinism-safe: scopes, components and
 * parameters are each sorted so the table order never depends on object
 * insertion order.
 */
function flattenParams(resolvedOutput) {
  const rows = [];
  if (!resolvedOutput || typeof resolvedOutput !== "object") return rows;
  for (const scope of Object.keys(resolvedOutput).sort()) {
    const components = resolvedOutput[scope]?.components || {};
    for (const compId of Object.keys(components).sort()) {
      const params = components[compId]?.params || {};
      const keys = Object.keys(params).sort();
      if (keys.length === 0) {
        rows.push({ scope, component: compId, param: "", value: "", type: "", unit: "", lifecycle: "", safety: "", access: "", empty: true });
        continue;
      }
      for (const key of keys) {
        const p = params[key] || {};
        rows.push({
          scope,
          component: compId,
          param: key,
          value: p.value,
          type: p.type ?? "",
          unit: p.unit ?? "",
          lifecycle: p.lifecycle ?? "",
          safety: p.safety ?? "",
          access: p.access ?? "",
        });
      }
    }
  }
  return rows;
}

function rowMatches(r, q) {
  return [r.scope, r.component, r.param, displayValue(r.value), r.type, r.unit, r.lifecycle, r.safety, r.access]
    .some((cell) => String(cell).toLowerCase().includes(q));
}

const COLUMNS = ["scope", "component", "parameter", "value", "type", "unit", "lifecycle", "safety", "access"];

function paramsTable(rows) {
  const head = el(
    "tr",
    {},
    COLUMNS.map((c) => el("th", { scope: "col", text: c }))
  );
  const body = rows.map((r) => {
    if (r.empty) {
      return el("tr", { class: "row-empty" }, [
        el("td", { text: r.scope }),
        el("td", { text: r.component }),
        el("td", { class: "muted", colspan: "7", text: "no parameters" }),
      ]);
    }
    return el("tr", {}, [
      el("td", { text: r.scope }),
      el("td", { text: r.component }),
      el("td", { class: "cell-param", text: r.param }),
      el("td", { class: "cell-value", text: displayValue(r.value) }),
      el("td", { text: r.type }),
      el("td", { text: r.unit || "—" }),
      el("td", { text: r.lifecycle || "—" }),
      el("td", { text: r.safety || "—" }),
      el("td", { text: r.access || "—" }),
    ]);
  });
  return el("table", { class: "params-table" }, [
    el("thead", {}, head),
    el("tbody", {}, body),
  ]);
}
