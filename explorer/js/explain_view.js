// SPDX-License-Identifier: BUSL-1.1
//
// Explain view (ADR-0043 §2.3): render an ExplainRejectionResult's labeled
// unsat core (ADR-0031 D3). The wording mirrors the shared text renderer
// `render_unsat_core` (interpreter/src/explain_renderer.rs) EXACTLY — the same
// function the CLI carries — so the human explanation is defined in one place
// and this view stays byte-faithful to `cfx explain`'s text output. The
// renderer adds no facts: every line is derived solely from fields already in
// the JSON (rejected, conflicting_constraints, note).

import { el, clear, announce } from "./dom.js";

/** Render an ExplainRejectionResult into `container`. */
export function renderExplain(container, data) {
  clear(container);
  const rejection = data.rejection || {};
  const core = rejection.unsat_core || null;

  const facts = [
    el("div", { class: "hash-row" }, [
      el("span", { class: "hash-key", text: "rejected" }),
      el("span", { class: "hash-val", text: `${data.facet}.${data.option}` }),
    ]),
    el("div", { class: "hash-row" }, [
      el("span", { class: "hash-key", text: "code" }),
      el("span", { class: "hash-val", text: rejection.code || "—" }),
    ]),
  ];
  container.appendChild(
    el("section", { class: "hash-header", "aria-label": "Rejection identity" }, [
      el("h2", { text: "Why this selection was rejected" }),
      el("div", { class: "hash-grid" }, facts),
    ])
  );

  if (core) {
    container.appendChild(structuredCore(core));
    container.appendChild(
      el("section", { class: "explain-verbatim", "aria-label": "Explanation as text" }, [
        el("h2", { text: "As cfx explain prints it" }),
        el("pre", { class: "explain-pre", tabindex: "0", text: renderUnsatCore(core) }),
      ])
    );
  } else {
    // Division-of-labor rejection with no solver core: mirror render_explain_text,
    // which shows the canonical rejection message instead.
    container.appendChild(
      el("section", { class: "explain-verbatim" }, [
        el("h2", { text: "Explanation" }),
        el("pre", { class: "explain-pre", tabindex: "0", text: rejection.message || "(no message)" }),
      ])
    );
  }

  if (rejection.hint) {
    container.appendChild(
      el("section", { class: "explain-hint", role: "note" }, [
        el("span", { class: "hint-label", text: "hint" }),
        el("span", { text: rejection.hint }),
      ])
    );
  }

  announce(`Loaded explanation: cannot select ${data.facet}.${data.option}`);
}

/** The friendly, structured view of the conflicting-constraint set. */
function structuredCore(core) {
  const items = (core.conflicting_constraints || []).map((c) => {
    if (c.kind === "selection") {
      return el("li", { class: "constraint constraint-selection" }, [
        el("span", { class: "constraint-kind", text: "earlier choice" }),
        el("span", { class: "constraint-body", text: facetsJoined(c.facets) }),
      ]);
    }
    return el("li", { class: "constraint constraint-rule" }, [
      el("span", { class: "constraint-kind", text: "model rule" }),
      el("span", { class: "constraint-body", text: c.summary || "(rule)" }),
    ]);
  });

  const body =
    items.length > 0
      ? el("ul", { class: "constraint-list" }, items)
      : el("p", { class: "muted", text: "No minimal explanation is available." });

  return el("section", { class: "explain-core", "aria-label": "Conflicting constraints" }, [
    el("h2", { text: `Cannot select ${facetOption(core.rejected)}` }),
    body,
    el("p", { class: "muted core-note", text: `(${core.note})` }),
  ]);
}

// --- verbatim text renderer, mirroring render_unsat_core ---------------------

/** `"{facet}.{option}"` for a single labeled pair (ADR-0005 §3 naming). */
function facetOption(facet) {
  return `${facet.facet}.${facet.option}`;
}

/** Join labeled facets as "a.x, b.y"; stable placeholder for an empty list. */
function facetsJoined(facets) {
  if (!facets || facets.length === 0) return "(unspecified)";
  return facets.map(facetOption).join(", ");
}

/**
 * One conflicting-constraint line (selection names the pairs; rule uses gloss).
 *
 * A model rule carrying a `constraint_id` is an AUTHORED `constraints:`
 * declaration (ADR-0054 §5.4): the line names it, with the entry's `summary`
 * (the constraint's condition text) after the colon. Without one, no declared
 * constraint accounts for the clause and the generic model-rule wording stands
 * — §5.4 forbids naming a synthesized cardinality conjunct as if it were
 * authored policy.
 *
 * The `!= null` test mirrors the Rust `Option` match it must stay byte-faithful
 * to (`constraint_id.as_deref()`: absent -> None, present -> Some). A
 * truthiness test would instead route a present-but-empty id to the generic
 * wording while `cfx explain` still printed the attributed line, which is the
 * one input where the two surfaces could disagree.
 */
function renderConstraint(constraint) {
  if (constraint.kind === "selection") {
    return `  blocked by your earlier choice: ${facetsJoined(constraint.facets)}`;
  }
  if (constraint.constraint_id != null) {
    return `  blocked by constraint ${constraint.constraint_id}: ${constraint.summary}`;
  }
  return `  blocked by model rule: ${constraint.summary}`;
}

/**
 * Deterministic human block, byte-faithful to interpreter render_unsat_core:
 *   cannot select {facet}.{option}:
 *     blocked by your earlier choice: …
 *     blocked by constraint {id}: …
 *     blocked by model rule: …
 *   ({note})
 * Empty constraint list → the "no minimal explanation is available" fallback.
 */
function renderUnsatCore(core) {
  const lines = [`cannot select ${facetOption(core.rejected)}:`];
  const constraints = core.conflicting_constraints || [];
  if (constraints.length === 0) {
    lines.push("  no minimal explanation is available");
  } else {
    for (const c of constraints) {
      lines.push(renderConstraint(c));
    }
  }
  lines.push(`(${core.note})`);
  return lines.join("\n");
}
