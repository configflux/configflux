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
//!   depend on this text. The `ExplainRejectionResult` serializes identically
//!   whether or not anyone ever calls this function. Changing the wording here
//!   changes no contract field. This is the ADR-0031 D5/D6 layerability
//!   guarantee: a future front-end swaps the renderer without touching the
//!   solver decision content or the JSON contract.
//!
//! The wording sketch in ADR-0031 D5 is an illustrative style guide, not a spec;
//! the ADR explicitly does not standardize the rendered text.
//!
//! ADR-0003 §2 keeps this renderer out of the compiler: it consumes the
//! compiler-side `UnsatCore` type but is itself only visible from the
//! interpreter (and a sibling copy in the runtime). Per configflux-9d28, a small
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

/// Render an unsat core as a deterministic human-readable block (ADR-0031 D5).
///
/// Shape (illustrative — wording is non-normative):
///
/// ```text
/// cannot select engine.v8:
///   blocked by your earlier choice: engine.v6
///   blocked by model rule: engine.v6 excludes engine.v8
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
/// - `Selection` — a prior choice already in the selection state that blocks the
///   rejected option.
/// - `ModelRule` — a `requires`/`excludes`-style rule baked into the model.
///
/// `ModelRule` uses the entry's advisory `summary` gloss (itself a JSON field);
/// `Selection` names the labeled `{facet}.{option}` pairs directly so the line
/// echoes the prior choice without relying on the gloss.
fn render_constraint(constraint: &ConflictingConstraint) -> String {
    match constraint.kind {
        ConstraintKind::Selection => {
            format!("  blocked by your earlier choice: {}", facets_joined(&constraint.facets))
        }
        ConstraintKind::ModelRule => {
            format!("  blocked by model rule: {}", constraint.summary)
        }
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
                },
                ConflictingConstraint {
                    kind: ConstraintKind::ModelRule,
                    facets: vec![facet("engine", "v6"), facet("engine", "v8")],
                    summary: "engine.v6 excludes engine.v8".to_string(),
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
    fn renders_model_rule_entry() {
        let core = UnsatCore {
            rejected: facet("database", "postgres"),
            conflicting_constraints: vec![ConflictingConstraint {
                kind: ConstraintKind::ModelRule,
                facets: vec![facet("database", "postgres"), facet("storage", "remote")],
                summary: "database.postgres requires storage.remote".to_string(),
            }],
            minimal: true,
            note: NOTE.to_string(),
        };
        let text = render_unsat_core(&core);
        assert!(
            text.contains("blocked by model rule: database.postgres requires storage.remote"),
            "model-rule entry must render its summary gloss; got:\n{text}"
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
  blocked by model rule: engine.v6 excludes engine.v8
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
