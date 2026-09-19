// SPDX-License-Identifier: BUSL-1.1

//! Human-readable rendering of an unsat core (ADR-0031 D5, configflux-9d28).
//!
//! `render_unsat_core` turns the machine-form `UnsatCore` JSON envelope
//! (ADR-0031 D3) into a deterministic block of human text. It is a strict
//! **presentation layer**:
//!
//! - **Pure.** No I/O, no solver calls, no global state. The output is a total
//!   function of the borrowed `&UnsatCore` and nothing else.
//! - **Adds no facts.** Every line is derived solely from fields already present
//!   in the JSON (`rejected`, `conflicting_constraints`, `note`). The renderer
//!   never invents a facet, option, or constraint the core did not name.
//! - **Layered, never baked in.** The machine envelope (ADR-0031 D2/D3 —
//!   `schema_version`, `status`, `model_hash`, the `unsat_core` JSON) does NOT
//!   depend on this text. The runtime `ExplainRejectionResult` serializes
//!   identically whether or not anyone ever calls this function. Changing the
//!   wording here changes no contract field. This is the ADR-0031 D5/D6
//!   layerability guarantee: a future front-end swaps the renderer without
//!   touching the solver decision content or the JSON contract.
//!
//! The wording sketch in ADR-0031 D5 is an illustrative style guide, not a spec;
//! the ADR explicitly does not standardize the rendered text.
//!
//! ADR-0003 §2 keeps this renderer out of the compiler: it consumes the
//! compiler-side `UnsatCore` type but is itself only visible from the runtime
//! (and a sibling copy in the interpreter). Per configflux-9d28, a small
//! per-crate renderer is preferred over a shared dependency that would cross the
//! ADR-0003 boundary; the two copies are intentionally duplicated.
//!
//! The renderer ships as a pure, independently-tested presentation helper and is
//! deliberately *not* wired into any envelope-producing path: the machine
//! contract (ADR-0031 D2/D3) must not depend on the rendered text, so the safest
//! embedding is none — the human view is the JSON-vs-human split D2 places
//! "outside the envelope". A future ergonomic front-end (ADR-0031 D6) calls
//! `render_unsat_core` over the same `unsat_core` JSON. Until that caller lands,
//! the binary does not invoke it; `allow(dead_code)` covers that intentional gap
//! the same way `c_abi_lib.rs` covers its intentionally-unused exports.
#![allow(dead_code)]

use compiler::loader_api::{ConflictingConstraint, ConstraintFacet, ConstraintKind, UnsatCore};
use compiler::lowering::{describe_attribution, Attribution};

/// Render an unsat core as a deterministic human-readable block (ADR-0031 D5).
///
/// Shape (illustrative — wording is non-normative):
///
/// ```text
/// cannot select engine.v8:
///   blocked by your earlier choice: engine.v6
///   blocked by constraint one_engine_only: engine != 'v6' || engine != 'v8'
/// (one minimal explanation; other equivalent explanations may exist)
/// ```
///
/// When `conflicting_constraints` is empty the function returns a fallback
/// message instead of panicking (the core named a rejection but carried no
/// minimized constraints — still presentable, never a crash).
///
/// The output never has a trailing newline; callers compose it into whatever
/// surface they like.
pub fn render_unsat_core(core: &UnsatCore) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "cannot select {}:",
        facet_option(&core.rejected)
    ));

    if core.conflicting_constraints.is_empty() {
        // Fallback for an empty core: presentable, not a panic (configflux-9d28
        // acceptance criterion). The rejection stands but no minimal explanation
        // was attached.
        lines.push("  no minimal explanation is available".to_string());
    } else {
        for constraint in &core.conflicting_constraints {
            lines.push(render_constraint(constraint));
        }
    }

    // Advisory note, derived verbatim from the JSON `note` field (so the line
    // still adds no facts the core lacks). Acknowledges non-uniqueness of the
    // MUS witness (ADR-0031 D3/D5).
    lines.push(format!("({})", core.note));

    lines.join("\n")
}

/// Render one conflicting-constraint entry (ADR-0031 D3 `conflicting_constraints[]`).
///
/// - `Selection` — a prior choice already in the parameter state that blocks the
///   rejected option.
/// - `ModelRule` — a `requires`/`excludes`-style rule baked into the model.
///
/// A `ModelRule` that carries a `constraint_id` is an **authored**
/// `constraints:` declaration (ADR-0054 §5.4): the line names it, with the
/// entry's `summary` (the constraint's condition text) after the colon.
/// Without one, no declared constraint accounts for the clause and the generic
/// model-rule wording stands — §5.4 forbids naming a synthesized cardinality
/// conjunct as if it were authored policy.
///
/// Two constraint ids are not authored at all: the reserved `derive:` and
/// `accepts:` prefixes ADR-0057 §D4 lowers a binding's `derive` table and a
/// requirement's `accepts` list to. Printing those as
/// `blocked by constraint derive:line_container:site=factory_a: site !=
/// 'factory_a' || line_container == 'c1'` would be truthful and useless — it
/// names a rule no one wrote, in a form no one authored. So they are rendered
/// as the AUTHORING construct they came from, decoded by
/// `compiler::lowering::describe_attribution` (which lives beside the code that
/// emitted the text, so the two cannot drift). A shape that decoder cannot read
/// falls through to the generic line rather than being guessed at.
///
/// `ModelRule` uses the entry's advisory `summary` gloss (itself a JSON field);
/// `Selection` names the labeled `{facet}.{option}` pairs directly so the line
/// echoes the prior choice without relying on the gloss. The "blocked by ..."
/// label belongs to the renderer, never to the gloss — a gloss that carried its
/// own label printed it twice (configflux-hdgn).
fn render_constraint(constraint: &ConflictingConstraint) -> String {
    match constraint.kind {
        ConstraintKind::Selection => {
            format!("  blocked by your earlier choice: {}", facets_joined(&constraint.facets))
        }
        ConstraintKind::ModelRule => match constraint.constraint_id.as_deref() {
            Some(id) => render_named_rule(id, &constraint.summary),
            None => format!("  blocked by model rule: {}", constraint.summary),
        },
    }
}

/// Render a `ModelRule` clause that carries a constraint id.
///
/// The two ADR-0057 §D4 lowerings read as what the author wrote; everything
/// else — every authored `constraints:` entry, and any lowered conjunct whose
/// text the decoder does not recognise — keeps the ADR-0054 §5.4 wording.
fn render_named_rule(id: &str, summary: &str) -> String {
    match describe_attribution(id, summary) {
        Some(Attribution::Derive {
            binding,
            source,
            source_value,
            entry,
        }) => format!(
            "  blocked by binding {binding}, derived from {source}: \
             {source} == '{source_value}' -> '{entry}'"
        ),
        Some(Attribution::Accepts {
            component,
            slot,
            entries,
        }) => format!(
            "  blocked by requirement {component}.{slot}: accepts {}",
            entries.join(", ")
        ),
        None => format!("  blocked by constraint {id}: {summary}"),
    }
}

/// `"{facet}.{option}"` for a single labeled pair (ADR-0005 §3 naming).
fn facet_option(facet: &ConstraintFacet) -> String {
    format!("{}.{}", facet.facet, facet.option)
}

/// Join a constraint's labeled facets as `"a.x, b.y"`. Returns a stable
/// placeholder for an empty list so a `Selection` entry that (defensively)
/// carries no facets renders without a dangling colon — never a panic and never
/// an invented fact.
fn facets_joined(facets: &[ConstraintFacet]) -> String {
    if facets.is_empty() {
        return "(unspecified)".to_string();
    }
    facets
        .iter()
        .map(facet_option)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use compiler::loader_api::{ConflictingConstraint, ConstraintFacet, ConstraintKind, UnsatCore};

    fn facet(f: &str, o: &str) -> ConstraintFacet {
        ConstraintFacet {
            facet: f.to_string(),
            option: o.to_string(),
        }
    }

    const NOTE: &str = "one minimal explanation; other minimal cores may exist";

    /// The minimal 2-option / 1-constraint fixture from the solver MUS unit test
    /// (configflux-kv5d, `solver/tests/explain_rejection_mus.rs`): after pinning
    /// `engine.v6` under exactly-one, `engine.v8` is rejected; the labeled core
    /// names the prior selection `engine.v6` and the exclusion model rule over
    /// `{engine.v6, engine.v8}`. Mapped onto the compiler-side `UnsatCore`.
    fn minimal_two_option_core() -> UnsatCore {
        UnsatCore {
            rejected: facet("engine", "v8"),
            conflicting_constraints: vec![
                ConflictingConstraint {
                    kind: ConstraintKind::Selection,
                    facets: vec![facet("engine", "v6")],
                    summary: "blocked by your earlier choice: engine.v6".to_string(),
                    constraint_id: None,
                },
                ConflictingConstraint {
                    kind: ConstraintKind::ModelRule,
                    facets: vec![facet("engine", "v6"), facet("engine", "v8")],
                    // ADR-0054 §5.4: an authored constraint names itself; the
                    // summary is its condition text, not a re-labelled gloss.
                    summary: "engine != 'v6' || engine != 'v8'".to_string(),
                    constraint_id: Some("one_engine_only".to_string()),
                },
            ],
            minimal: true,
            note: NOTE.to_string(),
        }
    }

    #[test]
    fn renders_selection_entry() {
        let core = UnsatCore {
            rejected: facet("database", "postgres"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::Selection,
                facets: vec![facet("storage", "local")],
                summary: "ignored gloss".to_string(),
                constraint_id: None,
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains("blocked by your earlier choice: storage.local"),
            "selection entry must name the labeled prior choice; got:\n{text}"
        );
        // A Selection line is driven by the labeled facet, not the summary gloss.
        assert!(
            !text.contains("ignored gloss"),
            "selection rendering must not leak the summary gloss; got:\n{text}"
        );
    }

    #[test]
    fn renders_declared_constraint_entry_by_id() {
        // ADR-0054 §5.4: a model clause attributed to an authored constraint
        // names that constraint and quotes its condition.
        let core = UnsatCore {
            rejected: facet("database", "postgres"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::ModelRule,
                facets: vec![facet("database", "postgres"), facet("storage", "remote")],
                summary: "database != 'postgres' || storage == 'remote'".to_string(),
                constraint_id: Some("postgres_needs_remote".to_string()),
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains(
                "blocked by constraint postgres_needs_remote: \
                 database != 'postgres' || storage == 'remote'"
            ),
            "a declared constraint must be named by id; got:\n{text}"
        );
        // The renderer owns the label; it must appear exactly once
        // (configflux-hdgn).
        assert_eq!(
            text.matches("blocked by").count(),
            1,
            "the 'blocked by' label must not be duplicated; got:\n{text}"
        );
    }

    #[test]
    fn renders_unattributed_model_rule_without_naming_a_constraint() {
        // ADR-0054 §5.4 hard rule: a clause no declared constraint accounts
        // for is reported as the model being over-constrained. It must NOT
        // borrow a constraint id.
        let core = UnsatCore {
            rejected: facet("engine", "v8"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::ModelRule,
                facets: vec![facet("engine", "v6"), facet("engine", "v8")],
                summary: "the model is over-constrained here; \
                          no declared constraint accounts for this conflict"
                    .to_string(),
                constraint_id: None,
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains("blocked by model rule: the model is over-constrained here"),
            "an unattributed model clause keeps the generic wording; got:\n{text}"
        );
        assert!(
            !text.contains("blocked by constraint"),
            "an unattributed model clause must name no constraint; got:\n{text}"
        );
    }

    #[test]
    fn empty_conflicting_constraints_produces_fallback_not_panic() {
        let core = UnsatCore {
            rejected: facet("engine", "v8"),
            conflicting_constraints: vec![],
            minimal: true,
            note: NOTE.to_string(),
        };
        // The call itself must not panic on an empty core.
        let text = render_unsat_core(&core);
        assert!(
            text.contains("cannot select engine.v8"),
            "fallback still names the rejected option; got:\n{text}"
        );
        assert!(
            text.contains("no minimal explanation is available"),
            "empty core must produce a fallback line; got:\n{text}"
        );
        // The advisory note is still appended.
        assert!(text.contains(NOTE), "fallback still carries the note; got:\n{text}");
    }

    #[test]
    fn render_is_deterministic_for_the_same_input() {
        let core = minimal_two_option_core();
        let first = render_unsat_core(&core);
        let second = render_unsat_core(&core);
        assert_eq!(first, second, "the renderer must be a pure, deterministic function");
    }

    #[test]
    fn golden_minimal_two_option_fixture() {
        // Golden: pin the exact rendered output for the minimal 2-option fixture
        // from configflux-kv5d's MUS unit test. If the wording changes, this test
        // changes deliberately — it is the human-format contract for this crate.
        let core = minimal_two_option_core();
        let expected = "\
cannot select engine.v8:
  blocked by your earlier choice: engine.v6
  blocked by constraint one_engine_only: engine != 'v6' || engine != 'v8'
(one minimal explanation; other minimal cores may exist)";
        assert_eq!(render_unsat_core(&core), expected);
    }

    #[test]
    fn selection_with_multiple_facets_joins_without_panicking() {
        let core = UnsatCore {
            rejected: facet("database", "postgres"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::Selection,
                facets: vec![facet("storage", "local"), facet("cache", "off")],
                summary: String::new(),
                constraint_id: None,
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains("blocked by your earlier choice: storage.local, cache.off"),
            "multiple selection facets must join cleanly; got:\n{text}"
        );
    }

    #[test]
    fn selection_with_no_facets_renders_placeholder_not_dangling_colon() {
        let core = UnsatCore {
            rejected: facet("database", "postgres"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::Selection,
                facets: vec![],
                summary: String::new(),
                constraint_id: None,
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains("blocked by your earlier choice: (unspecified)"),
            "an empty Selection facet list must render a stable placeholder; got:\n{text}"
        );
    }
}

#[cfg(test)]
mod adr_0057_attribution_tests {
    use super::*;

    fn model_rule(id: &str, summary: &str) -> ConflictingConstraint {
        ConflictingConstraint {
            kind: ConstraintKind::ModelRule,
            facets: Vec::new(),
            summary: summary.to_string(),
            constraint_id: Some(id.to_string()),
        }
    }

    /// ADR-0057 §D4: a derive attribution names the binding, the source, and
    /// the entry the rule forces — not the implication the model asserts.
    #[test]
    fn a_derive_attribution_reads_as_the_derive_rule() {
        assert_eq!(
            render_constraint(&model_rule(
                "derive:line_container:site=factory_a",
                "site != 'factory_a' || line_container == 'c1'",
            )),
            "  blocked by binding line_container, derived from site: \
             site == 'factory_a' -> 'c1'"
        );
    }

    /// ADR-0057 §D4: an accepts attribution names the requirement and the list.
    #[test]
    fn an_accepts_attribution_reads_as_the_requirement() {
        assert_eq!(
            render_constraint(&model_rule(
                "accepts:compute_service.container",
                "any_of(line_container == 'c1', line_container == 'c2')",
            )),
            "  blocked by requirement compute_service.container: accepts c1, c2"
        );
    }

    /// The guard a conditional component adds is not part of what the component
    /// accepts, and its own literals must not leak into the list.
    #[test]
    fn a_guarded_accepts_attribution_reports_only_the_accepted_entries() {
        assert_eq!(
            render_constraint(&model_rule(
                "accepts:edge_service.container",
                "!(mode == 'x') || line_container == 'c1'",
            )),
            "  blocked by requirement edge_service.container: accepts c1"
        );
    }

    /// An authored constraint id keeps the ADR-0054 §5.4 wording verbatim —
    /// the explorer's fixture test reads that exact template out of this file.
    #[test]
    fn an_authored_constraint_keeps_the_existing_wording() {
        assert_eq!(
            render_constraint(&model_rule(
                "prod_forbids_debug",
                "environment != 'prod' || log_level != 'debug'",
            )),
            "  blocked by constraint prod_forbids_debug: \
             environment != 'prod' || log_level != 'debug'"
        );
    }

    /// Honest degradation: a lowered id whose text does not have the emitted
    /// shape falls back rather than inventing an entry list.
    #[test]
    fn an_unreadable_lowered_conjunct_falls_back_to_the_generic_line() {
        assert_eq!(
            render_constraint(&model_rule("accepts:broken", "nothing quotable")),
            "  blocked by constraint accepts:broken: nothing quotable"
        );
    }
}
