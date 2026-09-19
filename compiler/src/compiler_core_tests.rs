// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for `compiler_core`'s `.ccm` clause synthesis.
//!
//! Two concerns live here, and the split between them is the point:
//!
//! * **Declared-facet symbol synthesis** (ADR-0047 §4 Amendment 1) — every
//!   declared value, including a default arm no condition names, reaches the
//!   symbol universe as a tautology that asserts nothing.
//! * **Branch-selector lowering** (configflux-9xxq / ADR-0054 §5.1) — a
//!   parameter-override `condition` contributes its `(facet, value)` symbols
//!   through that same tautology instead of being AND-folded into the BDD
//!   root, where it would force its variables true model-wide.

use super::*;
use crate::scenario_test_support::unique_temp_path;

fn facet(values: &[&str], open: bool) -> schema::Facet {
    schema::Facet {
        values: values.iter().map(|v| v.to_string()).collect(),
        default: None,
        open,
        doc: None,
    }
}

/// Owned rather than borrowed since ADR-0057 §D3: a binding's facet is DERIVED
/// from its catalogue, so `declared_facets` cannot hand out references into the
/// authored chunks any more.
fn declared(pairs: &[(&str, &schema::Facet)]) -> BTreeMap<String, schema::Facet> {
    pairs
        .iter()
        .map(|(n, f)| (n.to_string(), (*f).clone()))
        .collect()
}

#[test]
fn closed_facet_introduces_each_declared_value_in_declared_order() {
    // Amendment 1: symbol-introduction only. Declared order (eu, us, apac)
    // is preserved verbatim — NOT sorted — so the emitted symbols are a pure
    // function of the declaration. No `exactly_one_of`, no constraint.
    let region = facet(&["eu", "us", "apac"], false);
    let clauses = synthesize_facet_clauses(&declared(&[("region", &region)]));
    assert_eq!(
        clauses,
        vec![
            "region == 'eu' || region != 'eu'".to_string(),
            "region == 'us' || region != 'us'".to_string(),
            "region == 'apac' || region != 'apac'".to_string(),
        ]
    );
}

#[test]
fn single_value_closed_facet_introduces_its_lone_value() {
    let mode = facet(&["only"], false);
    let clauses = synthesize_facet_clauses(&declared(&[("mode", &mode)]));
    assert_eq!(clauses, vec!["mode == 'only' || mode != 'only'".to_string()]);
}

#[test]
fn open_facet_introduces_declared_values_without_at_most_one() {
    // Amendment 1: open facets are handled identically to closed — symbol
    // introduction only, no AMO. Condition-referenced-but-undeclared values
    // enter the symbol universe via the authored clauses, not here, so the
    // synthesized output depends only on the declared values.
    let env = facet(&["prod", "staging"], true);
    let clauses = synthesize_facet_clauses(&declared(&[("env", &env)]));
    assert_eq!(
        clauses,
        vec![
            "env == 'prod' || env != 'prod'".to_string(),
            "env == 'staging' || env != 'staging'".to_string(),
        ]
    );
}

#[test]
fn single_value_open_facet_emits_symbol_introducing_tautology() {
    let flag = facet(&["on"], true);
    let clauses = synthesize_facet_clauses(&declared(&[("flag", &flag)]));
    assert_eq!(clauses, vec!["flag == 'on' || flag != 'on'".to_string()]);
}

#[test]
fn multiple_facets_emit_in_facet_name_ascending_order() {
    let region = facet(&["eu", "us"], false);
    let tier = facet(&["free", "paid"], false);
    // Insertion order (tier, region) is irrelevant: the BTreeMap key order
    // (region < tier) is the emission order; values keep declared order.
    let clauses =
        synthesize_facet_clauses(&declared(&[("tier", &tier), ("region", &region)]));
    assert_eq!(
        clauses,
        vec![
            "region == 'eu' || region != 'eu'".to_string(),
            "region == 'us' || region != 'us'".to_string(),
            "tier == 'free' || tier != 'free'".to_string(),
            "tier == 'paid' || tier != 'paid'".to_string(),
        ]
    );
}

#[test]
fn value_with_single_quote_falls_back_to_double_quotes() {
    let odd = facet(&["a'b", "c"], false);
    let clauses = synthesize_facet_clauses(&declared(&[("odd", &odd)]));
    assert_eq!(
        clauses,
        vec![
            "odd == \"a'b\" || odd != \"a'b\"".to_string(),
            "odd == 'c' || odd != 'c'".to_string(),
        ]
    );
}

#[test]
fn no_declared_facets_synthesizes_nothing() {
    let clauses = synthesize_facet_clauses(&BTreeMap::new());
    assert!(clauses.is_empty());
}

// ---- configflux-9xxq / ADR-0054 §5.1: selectors are not assertions -----

fn selector(condition: &str) -> Vec<String> {
    synthesize_selector_symbols(&parse_condition_expr(condition).expect("parses"))
}

#[test]
fn selector_contributes_symbols_instead_of_asserting_itself() {
    // The exact defect: `environment == 'prod'` on a parameter override
    // must not reach the BDD root, or it forces `environment.prod` true
    // model-wide and prunes `log_level.debug` through the policy
    // disjunction. It becomes a tautology over the same symbol.
    assert_eq!(
        selector("environment == 'prod'"),
        vec!["environment == 'prod' || environment != 'prod'".to_string()]
    );
}

#[test]
fn selector_symbols_are_emitted_for_both_operators() {
    // `!=` names the same variable as `==` (compile_predicate lowers NotEq
    // to not(var)), so a `!=` selector must still land its symbol.
    assert_eq!(
        selector("region != 'eu'"),
        vec!["region == 'eu' || region != 'eu'".to_string()]
    );
}

#[test]
fn multi_atom_selector_emits_one_tautology_per_pair_in_dfs_order() {
    assert_eq!(
        selector("environment == 'prod' && region != 'eu' || tier == 'paid'"),
        vec![
            "environment == 'prod' || environment != 'prod'".to_string(),
            "region == 'eu' || region != 'eu'".to_string(),
            "tier == 'paid' || tier != 'paid'".to_string(),
        ]
    );
}

#[test]
fn selector_repeating_a_pair_emits_it_once() {
    assert_eq!(
        selector("environment == 'prod' || environment != 'prod'"),
        vec!["environment == 'prod' || environment != 'prod'".to_string()]
    );
}

#[test]
fn selector_symbols_survive_negation_and_cardinality_wrappers() {
    // The walk must reach atoms nested under Not / any_of, or those
    // symbols would silently leave the universe.
    assert_eq!(
        selector("!(mode == 'fast')"),
        vec!["mode == 'fast' || mode != 'fast'".to_string()]
    );
    assert_eq!(
        selector("any_of(a == 'x', b == 'y')"),
        vec![
            "a == 'x' || a != 'x'".to_string(),
            "b == 'y' || b != 'y'".to_string(),
        ]
    );
}

#[test]
fn selector_preserves_the_exact_symbol_set_it_previously_contributed() {
    // The invariant ADR-0054 §9 requires: the symbol universe is
    // unchanged. Under the default FacetNameAscending heuristic the symbol
    // order is a function of the pair SET, so equal sets => byte-identical
    // ccm.symbols.json.
    let condition = "environment == 'prod' && log_level != 'debug' || region == 'eu'";
    let expr = parse_condition_expr(condition).expect("parses");

    let mut before: BTreeSet<(String, String)> = BTreeSet::new();
    for_each_predicate_symbol(&expr, |t, v| {
        before.insert((t.to_string(), v.to_string()));
    });

    let mut after: BTreeSet<(String, String)> = BTreeSet::new();
    for clause in synthesize_selector_symbols(&expr) {
        let parsed = parse_condition_expr(&clause).expect("synthesized clause parses");
        for_each_predicate_symbol(&parsed, |t, v| {
            after.insert((t.to_string(), v.to_string()));
        });
    }

    assert_eq!(before, after);
}

#[test]
fn selector_output_never_synthesizes_cardinality() {
    // The selector channel must stay inert, forever. Cardinality now exists
    // (ADR-0054 §5.2) but it is a SEPARATE producer keyed on DECLARED facets
    // (`synthesize_facet_cardinality`); routing any of it through the selector
    // path would assert something an author only ever wrote as an inclusion
    // condition, which is the configflux-9xxq defect in a new costume.
    // Every clause this path emits must be a bare symbol-introducing tautology
    // and nothing else.
    for clause in selector("environment == 'prod' && region == 'eu' && region == 'us'") {
        assert!(
            !clause.contains("exactly_one_of")
                && !clause.contains("any_of")
                && !clause.contains("all_of")
                && !clause.contains("&&"),
            "selector clause must be a bare tautology, got: {clause}"
        );
    }
}

#[test]
fn selector_clauses_reparse_and_carry_odd_values() {
    for clause in selector("odd == \"a'b\"") {
        assert!(
            parse_condition_expr(&clause).is_ok(),
            "synthesized selector clause must parse: {clause}"
        );
        assert_eq!(clause, "odd == \"a'b\" || odd != \"a'b\"");
    }
}

#[test]
fn synthesized_clauses_reparse_under_the_condition_grammar() {
    // The synthesized strings must round-trip through the same parser the
    // emitter uses (parse_clauses), or the compile would fail downstream.
    let region = facet(&["eu", "us", "apac"], false);
    let env = facet(&["prod"], true);
    let clauses = synthesize_facet_clauses(&declared(&[("region", &region), ("env", &env)]));
    for clause in &clauses {
        assert!(
            parse_condition_expr(clause).is_ok(),
            "synthesized clause must parse: {clause}"
        );
    }
}

// ---------------------------------------------------------------------------
// ADR-0054 §1 — the `constraints` namespace through ingest merge and link.
// ---------------------------------------------------------------------------

const POLICY_FACETS_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "prod_forbids_debug": {
      "condition": "environment != 'prod' || log_level != 'debug'",
      "doc": "Debug logging is not permitted in production."
    }
  }
}"#;

#[test]
fn constraints_reach_the_merged_repository_and_link_cleanly() {
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", POLICY_FACETS_CHUNK)
        .expect("ingest chunk");

    let repo = compiler.get_repo();
    let declared = repo
        .constraints
        .get("prod_forbids_debug")
        .expect("constraint merged into the repository");
    assert_eq!(
        declared.condition,
        "environment != 'prod' || log_level != 'debug'"
    );
    assert_eq!(
        declared.doc.as_deref(),
        Some("Debug logging is not permitted in production.")
    );
    compiler.link_and_verify().expect("model links");
}

#[test]
fn a_constraint_id_declared_by_two_chunks_is_rejected() {
    // Constraints are pack-global (ADR-0054 §1): one id, one declaration.
    let second = POLICY_FACETS_CHUNK.replace(
        r#""facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },"#,
        "",
    );
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", POLICY_FACETS_CHUNK)
        .expect("ingest first chunk");
    let err = compiler
        .add_chunk_auto("10_more.json", &second)
        .expect_err("duplicate constraint id must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("Duplicate constraint ID found: 'prod_forbids_debug'"),
        "err: {msg}"
    );
    // Deliberately NOT the facet phrasing: a duplicate constraint is not a
    // duplicate facet, and an author reading the message has to be able to tell
    // them apart (before configflux-py7w the wording also chose the code).
    assert!(
        !msg.contains("declared in more than one chunk"),
        "must not borrow the facet diagnostic's phrasing: {msg}"
    );
}

#[test]
fn an_unparseable_constraint_fails_the_link() {
    let chunk = r#"{
  "package": "p",
  "version": "1.0",
  "constraints": {"bogus": {"condition": "this is not <> a condition"}}
}"#;
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", chunk)
        .expect("ingest chunk");
    let err = compiler
        .link_and_verify()
        .expect_err("an unparseable constraint is an ingest error");
    assert!(format!("{err}").contains("does not parse"), "err: {err}");
}

#[test]
fn a_constraint_over_a_condition_inferred_facet_fails_before_anything_is_emitted() {
    // configflux-6j91: the rejection has to land at link/verify, BEFORE a
    // `.ccm` exists. A package emitted from this model would carry a policy the
    // options surface cannot enforce (ADR-0054 §5.2 synthesizes cardinality for
    // declared facets only), so the three surfaces would disagree about it —
    // exactly the divergence ADR-0054 exists to end. `arch` here is visible to
    // the author in the component condition, which is why the diagnostic says
    // "not declared" rather than "unknown".
    let chunk = r#"{
  "package": "p",
  "version": "1.0",
  "constraints": {"pinned_arch": {"condition": "arch == 'x86'"}},
  "components": {
    "agent": {"type": "service", "condition": "arch == 'x86'", "params": {}}
  }
}"#;
    let temp_dir = unique_temp_path("cfx-core", "inferred-constraint");
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", chunk)
        .expect("ingest chunk");

    let err = compiler
        .link_and_verify()
        .expect_err("a constraint over an undeclared facet must fail the link");
    let msg = format!("{err}");
    assert!(msg.contains("Constraint 'pinned_arch'"), "err: {msg}");
    assert!(msg.contains("facet 'arch'"), "err: {msg}");
    assert!(msg.contains("not declared"), "err: {msg}");

    // `emit_ir` calls `link_and_verify` first, so nothing reaches the disk.
    compiler
        .emit_ir(&temp_dir)
        .expect_err("emit must refuse the same model");
    assert!(
        !temp_dir.join(ir::CMP_DEFAULT_INDEX_REF).exists(),
        "no package may be written for a model that fails validation"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn declaring_the_facet_makes_the_same_model_link_cleanly() {
    // The paired half of the rejection above: byte-for-byte the same model with
    // `arch` declared. Declaration is what makes the facet nameable; a selector
    // mentioning it as well is irrelevant in both directions. Guards against a
    // fix that over-corrects into rejecting the ordinary case, where a facet is
    // both declared and used as a condition — which is most real models.
    let chunk = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {"arch": {"values": ["x86", "arm"], "default": "x86"}},
  "constraints": {"pinned_arch": {"condition": "arch == 'x86'"}},
  "components": {
    "agent": {"type": "service", "condition": "arch == 'x86'", "params": {}}
  }
}"#;
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", chunk)
        .expect("ingest chunk");
    compiler
        .link_and_verify()
        .expect("a constraint over a declared facet must link");
}

#[test]
fn emitted_ir_round_trips_constraints_through_verify_ir_dir() {
    let temp_dir = unique_temp_path("cfx-core", "constraints");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", POLICY_FACETS_CHUNK)
        .expect("ingest chunk");
    compiler.emit_ir(&temp_dir).expect("emit ir");

    // `verify_ir_dir` re-walks the emitted chunks, so this proves the
    // constraint survived the `.cfir` round trip and is re-validated on read.
    verify_ir_dir(&temp_dir).expect("emitted IR verifies");

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---- ADR-0054 §5.2: intra-facet cardinality synthesis -----------------

#[test]
fn closed_facet_synthesizes_exactly_one_of_over_declared_values() {
    // The soundness floor. A closed domain is exhaustive by declaration, so
    // exactly one declared value holds in every completion — at-least-one is
    // a theorem, and at-most-one is what stops a positive-equality constraint
    // from under-pruning on the options surface.
    let region = facet(&["eu", "us", "apac"], false);
    assert_eq!(
        synthesize_facet_cardinality(&declared(&[("region", &region)])),
        vec!["exactly_one_of(region == 'eu', region == 'us', region == 'apac')".to_string()]
    );
}

#[test]
fn open_facet_synthesizes_pairwise_at_most_one_without_at_least_one() {
    // An open domain is extensible, so "some DECLARED value holds" is not a
    // theorem and at-least-one would assert something false. At-most-one
    // still holds: a facet binds to one value whatever the domain.
    let env = facet(&["dev", "staging", "prod"], true);
    assert_eq!(
        synthesize_facet_cardinality(&declared(&[("env", &env)])),
        vec![
            "env != 'dev' || env != 'staging'".to_string(),
            "env != 'dev' || env != 'prod'".to_string(),
            "env != 'staging' || env != 'prod'".to_string(),
        ]
    );
}

#[test]
fn single_value_closed_facet_degenerates_to_the_bare_equality() {
    // `exactly_one_of` requires N >= 2 arguments in the grammar, and over an
    // exhaustive one-value domain the literal IS what "exactly one of" means.
    let mode = facet(&["only"], false);
    assert_eq!(
        synthesize_facet_cardinality(&declared(&[("mode", &mode)])),
        vec!["mode == 'only'".to_string()]
    );
}

#[test]
fn single_value_open_facet_synthesizes_no_cardinality() {
    // No pairs to mutex, and at-least-one is unsound on an open domain, so
    // the facet contributes nothing to the root.
    let flag = facet(&["on"], true);
    assert!(synthesize_facet_cardinality(&declared(&[("flag", &flag)])).is_empty());
}

#[test]
fn undeclared_facets_synthesize_no_cardinality() {
    // A condition-inferred facet's domain is an artifact of what conditions
    // happened to mention, not a declaration. Asserting cardinality over it
    // would assert something the author never wrote.
    assert!(synthesize_facet_cardinality(&BTreeMap::new()).is_empty());
}

#[test]
fn cardinality_emits_in_facet_name_ascending_then_declared_value_order() {
    // The byte-stability commitment: `ccm_hash` stays a pure function of
    // (declared constraints, declared facets), independent of map iteration.
    let region = facet(&["eu", "us"], false);
    let tier = facet(&["free", "paid"], false);
    assert_eq!(
        synthesize_facet_cardinality(&declared(&[("tier", &tier), ("region", &region)])),
        vec![
            "exactly_one_of(region == 'eu', region == 'us')".to_string(),
            "exactly_one_of(tier == 'free', tier == 'paid')".to_string(),
        ]
    );
}

#[test]
fn synthesized_cardinality_reparses_under_the_condition_grammar() {
    // The emitter re-parses these strings via `parse_condition_model`; a
    // clause that does not round-trip would fail the compile downstream.
    let region = facet(&["eu", "us", "apac"], false);
    let env = facet(&["dev", "prod"], true);
    let odd = facet(&["a'b", "c"], false);
    let clauses = synthesize_facet_cardinality(&declared(&[
        ("region", &region),
        ("env", &env),
        ("odd", &odd),
    ]));
    assert!(!clauses.is_empty());
    for clause in &clauses {
        assert!(
            parse_condition_expr(clause).is_ok(),
            "synthesized cardinality clause must parse: {clause}"
        );
    }
}

#[test]
fn cardinality_quotes_fall_back_for_values_containing_a_single_quote() {
    let odd = facet(&["a'b", "c"], false);
    assert_eq!(
        synthesize_facet_cardinality(&declared(&[("odd", &odd)])),
        vec!["exactly_one_of(odd == \"a'b\", odd == 'c')".to_string()]
    );
}

// ---- ADR-0054 §5.1: only `constraints` carry AUTHORED policy to the root ----

/// A pack shaped like the hero example after migration: a component condition
/// that is a genuine inclusion selector, a parameter-override condition, and
/// the policy living where policy belongs.
const ROUTING_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "prod_forbids_debug": {"condition": "environment != 'prod' || log_level != 'debug'"},
    "a_first_by_id": {"condition": "environment != 'dev' || log_level == 'info'"}
  },
  "definitions": {
    "timeout_ms": {"type": "integer", "value": 1000,
      "overrides": [{"condition": "environment == 'prod'", "value": 5000}]}
  },
  "components": {
    "prod_only_sidecar": {"type": "module", "condition": "environment == 'prod'",
      "params": {}}
  }
}"#;

/// The merged summary the CCM builders read (ADR-0058 §D4 stage 2).
///
/// Built here through the object grouping the compile path itself uses, so
/// these tests exercise the real input to the clause and constraint builders
/// rather than a hand-assembled one.
fn merged_summary(compiler: &Compiler) -> crate::interface_summary::MergedSummary {
    let summaries = compiler.interface_summaries();
    let headers = crate::link::in_memory_headers(compiler.source_chunks(), &summaries);
    crate::link::link_stage_headers(&headers).expect("model links")
}

fn routing_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", ROUTING_CHUNK)
        .expect("ingest chunk");
    compiler.link_and_verify().expect("model links");
    compiler
}

#[test]
fn every_harvested_condition_becomes_a_bare_tautology() {
    // Pins the ADR-0054 §5.1 PRODUCER RULE — a COMPONENT condition is an
    // inclusion selector exactly like a parameter-override one, and neither
    // reaches the root as an assertion. The rule, not the encoding, is what
    // stands between configflux-9xxq and a recurrence: the emitter folds any
    // non-tautological clause in. Pinned BY EXAMPLE, over one fixture
    // (`routing_compiler` / `ROUTING_CHUNK`) — extend it rather than lean on it.
    let compiler = routing_compiler();
    let merged = merged_summary(&compiler);
    let declared_facets = declared_facets_from_summary(&merged);
    let clauses = ccm_clauses(&merged.selectors, &declared_facets);

    assert!(!clauses.is_empty());
    for clause in &clauses {
        let (lhs, rhs) = clause
            .split_once(" || ")
            .unwrap_or_else(|| panic!("clause is not a two-atom tautology: {clause}"));
        assert_eq!(
            lhs.replace(" == ", " != "),
            rhs,
            "clause must be `f == 'v' || f != 'v'`, got: {clause}"
        );
    }

    // Specifically: the component's own condition is present as a tautology
    // and absent as an assertion.
    assert!(clauses.contains(&"environment == 'prod' || environment != 'prod'".to_string()));
    assert!(!clauses.iter().any(|c| c == "environment == 'prod'"));
}

#[test]
fn constraints_are_collected_id_ascending_with_verbatim_conditions() {
    // Id-ascending order is what `root_index` in the §5.4 manifest roster
    // means, so it is part of the artifact contract, not a tidiness choice.
    // The condition text is passed through verbatim because the roster shows
    // it to the operator.
    let compiler = routing_compiler();
    assert_eq!(
        ccm_constraints(&merged_summary(&compiler)),
        vec![
            (
                "a_first_by_id".to_string(),
                "environment != 'dev' || log_level == 'info'".to_string()
            ),
            (
                "prod_forbids_debug".to_string(),
                "environment != 'prod' || log_level != 'debug'".to_string()
            ),
        ]
    );
}

#[test]
fn a_model_with_no_constraints_contributes_no_authored_root_conjuncts() {
    // The permissive-root guarantee: declaring facets and writing selector
    // conditions must not constrain anything by itself. Cardinality is the
    // only thing such a model puts on the root.
    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto(
            "00_definitions.json",
            r#"{"package":"p","version":"1.0",
                "facets":{"environment":{"values":["dev","prod"],"default":"dev"}},
                "components":{"c":{"type":"m","condition":"environment == 'prod'","params":{}}}}"#,
        )
        .expect("ingest chunk");
    compiler.link_and_verify().expect("model links");

    let merged = merged_summary(&compiler);
    assert!(ccm_constraints(&merged).is_empty());
    let declared_facets = declared_facets_from_summary(&merged);
    assert_eq!(
        synthesize_facet_cardinality(&declared_facets),
        vec!["exactly_one_of(environment == 'dev', environment == 'prod')".to_string()]
    );
}
