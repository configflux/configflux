// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the CCM emitter and the variable-order heuristic
//! plumbing. Lives next to `ccm_emitter.rs` (in the
//! `compiler::ccm_emitter` submodule directory) and is included via
//! `#[cfg(test)] mod tests;` from the parent so it has access to all
//! the parent's `pub(super)`/private items via `super::*`.

use super::*;
use std::collections::BTreeMap;

const HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// Test-only entry point: build a CCM artifact with a custom
/// apply-memo cap. Mirrors the production `build_ccm_artifact_inner`
/// shape but seeds the BDD builder with a caller-controlled memo cap.
/// The eviction-correctness regression
/// (`aggressive_memo_eviction_preserves_bdd_bytes`) sets the cap so
/// low that essentially every operation triggers a clear, then asserts
/// byte-identity vs the default-cap build. This proves the eviction
/// policy preserves the ADR-0005 §6 byte-stability contract.
fn build_ccm_artifact_with_memo_cap_for_test(
    model: &ConditionModel,
    memo_cap: usize,
) -> Result<CcmEmission> {
    validate_hash(&model.bound_model_hash)?;
    let parsed = parse_clauses(&model.clauses)?;
    let symbols = compute_variable_order(&parsed, VarOrderHeuristic::FacetNameAscending);
    let mut builder = BddBuilder::with_memo_cap(memo_cap);
    let bdd_bin = build_bdd_bin_with_builder_progress(&parsed, &symbols, &mut builder, None)?;
    let symbols_json = serialize_symbols(&symbols)?;
    let manifest_json = serialize_manifest(
        &model.bound_model_hash,
        symbols.len() as u32,
        node_count(&bdd_bin)? as u64,
        &symbols_json,
        &bdd_bin,
        VarOrderHeuristic::FacetNameAscending,
        Construction::InCrate,
    )?;
    Ok(CcmEmission {
        manifest_json,
        symbols_json,
        bdd_bin,
    })
}

/// Test-only entry point exercising the live adaptive memo-cap shrink
/// (configflux-9pjy.4 / ADR-0039 §5). Builds the BDD with a starting
/// `memo_cap`, a soft RSS budget (in KiB), and an injectable RSS sampler,
/// then returns both the emitted artifact and the post-build
/// [`MemoAdaptation`] so a test can assert (a) the cap dropped and (b) the
/// bytes match the unbudgeted build. The injected sampler removes the
/// `/proc` dependency so the shrink mechanism is deterministic on every
/// platform.
fn build_ccm_artifact_with_rss_budget_for_test(
    model: &ConditionModel,
    memo_cap: usize,
    rss_budget_kib: Option<u64>,
    rss_sampler: fn() -> Option<u64>,
) -> Result<(CcmEmission, super::bdd::MemoAdaptation)> {
    validate_hash(&model.bound_model_hash)?;
    let parsed = parse_clauses(&model.clauses)?;
    let symbols = compute_variable_order(&parsed, VarOrderHeuristic::FacetNameAscending);
    let mut builder = BddBuilder::with_memo_cap_rss_budget_and_sampler_for_test(
        memo_cap,
        rss_budget_kib,
        rss_sampler,
    );
    let bdd_bin = build_bdd_bin_with_builder_progress(&parsed, &symbols, &mut builder, None)?;
    let adaptation = builder.adaptation();
    let symbols_json = serialize_symbols(&symbols)?;
    let manifest_json = serialize_manifest(
        &model.bound_model_hash,
        symbols.len() as u32,
        node_count(&bdd_bin)? as u64,
        &symbols_json,
        &bdd_bin,
        VarOrderHeuristic::FacetNameAscending,
        Construction::InCrate,
    )?;
    Ok((
        CcmEmission {
            manifest_json,
            symbols_json,
            bdd_bin,
        },
        adaptation,
    ))
}

/// Always reports a huge RSS so the adaptive shrink fires on every clause
/// boundary (used to drive the cap down to the floor deterministically).
fn always_over_budget_rss() -> Option<u64> {
    Some(u64::MAX / 2)
}

#[test]
fn parse_condition_model_exposes_condition_exprs_for_partitioner() {
    // configflux-lz70: lifted `pub(crate) parse_condition_model` is
    // the partitioner-facing entry point. (a) validates
    // `bound_model_hash` first (matching full-build error order),
    // (b) returns the SAME `Vec<ConditionExpr>` the inline
    // `parse_clauses` call inside `build_ccm_artifact_inner`
    // previously produced — byte-stability of emitted CCM depends
    // transitively on this. Sole consumer: configflux-0qo3
    // (rung-3 scope partitioner).
    let bad = ConditionModel {
        bound_model_hash: "not-a-hash".to_string(),
        clauses: vec!["a == 'x'".to_string()],
    };
    assert!(parse_condition_model(&bad).is_err());
    let model = sample_model();
    let lifted = parse_condition_model(&model).expect("parse");
    let direct = parse_clauses(&model.clauses).expect("direct parse");
    assert_eq!(lifted.len(), model.clauses.len());
    assert_eq!(lifted, direct);
}

#[test]
fn empty_model_emits_terminal_true() {
    let model = ConditionModel {
        bound_model_hash: HASH.to_string(),
        clauses: Vec::new(),
    };
    let emission = build_ccm_artifact(&model).expect("emit");
    assert_eq!(&emission.bdd_bin[0..4], bdd::CCM_BDD_BIN_MAGIC);
    assert_eq!(u32_at(&emission.bdd_bin, 8), 0);
    assert_eq!(u32_at(&emission.bdd_bin, 12), 2);
    assert_eq!(u32_at(&emission.bdd_bin, 20), bdd::TERMINAL_TRUE);
}

#[test]
fn invalid_bound_model_hash_is_rejected() {
    let model = ConditionModel {
        bound_model_hash: "ABC".to_string(),
        clauses: Vec::new(),
    };
    assert!(build_ccm_artifact(&model).is_err());
}

#[test]
fn compiles_full_condition_grammar() {
    let expr = parse_condition_expr("!(a == 'on') || (b == 'enabled' && c != 'skip')").unwrap();
    let symbols = [
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
        ("c".to_string(), "skip".to_string()),
    ];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let root = compile_expr(&mut builder, &expr, &index).unwrap();
    assert!(eval(&builder, root, &[(0, false), (1, false), (2, true)]));
    assert!(eval(&builder, root, &[(0, true), (1, true), (2, false)]));
    assert!(!eval(&builder, root, &[(0, true), (1, true), (2, true)]));
    assert!(!eval(&builder, root, &[(0, true), (1, false), (2, false)]));
}

#[test]
fn any_of_lowers_to_or_reduction() {
    // configflux-ccs.4: `any_of(...)` lowers to an OR-reduction (ADR-0006
    // §4) over its children in ascending `Vec` index order (§5, pinned
    // left fold). Truth check: satisfiable iff at least one child predicate
    // holds; unsatisfiable only when none hold.
    let expr = parse_condition_expr("any_of(a == 'x', b == 'y', c == 'z')").unwrap();
    assert!(
        matches!(&expr, ConditionExpr::AnyOf(args) if args.len() == 3),
        "parser must yield a 3-arg AnyOf node: {expr:?}"
    );
    let symbols = [
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
        ("c".to_string(), "z".to_string()),
    ];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let root = compile_expr(&mut builder, &expr, &index).unwrap();
    // Unsatisfiable iff none of the three predicates hold.
    assert!(!eval(&builder, root, &[(0, false), (1, false), (2, false)]));
    // Satisfiable when any single predicate holds.
    assert!(eval(&builder, root, &[(0, true), (1, false), (2, false)]));
    assert!(eval(&builder, root, &[(0, false), (1, true), (2, false)]));
    assert!(eval(&builder, root, &[(0, false), (1, false), (2, true)]));
    // Satisfiable when several or all hold.
    assert!(eval(&builder, root, &[(0, true), (1, true), (2, false)]));
    assert!(eval(&builder, root, &[(0, true), (1, true), (2, true)]));
}

#[test]
fn all_of_lowers_to_and_reduction() {
    // configflux-ccs.5: `all_of(...)` lowers to an AND-reduction (ADR-0006
    // §4) over its children in ascending `Vec` index order (§5, pinned
    // left fold). Truth check: satisfiable iff EVERY child predicate
    // holds; unsatisfiable as soon as any one fails.
    let expr = parse_condition_expr("all_of(a == 'x', b == 'y')").unwrap();
    assert!(
        matches!(&expr, ConditionExpr::AllOf(args) if args.len() == 2),
        "parser must yield a 2-arg AllOf node: {expr:?}"
    );
    let symbols = [
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
    ];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let root = compile_expr(&mut builder, &expr, &index).unwrap();
    // Full truth table over (a == 'x', b == 'y'): satisfiable iff BOTH hold.
    assert!(eval(&builder, root, &[(0, true), (1, true)]));
    assert!(!eval(&builder, root, &[(0, true), (1, false)]));
    assert!(!eval(&builder, root, &[(0, false), (1, true)]));
    assert!(!eval(&builder, root, &[(0, false), (1, false)]));
}

#[test]
fn exactly_one_of_lowers_to_alo_and_pairwise_amo() {
    // configflux-ccs.6: `exactly_one_of(...)` lowers to at-least-one (OR)
    // ∧ pairwise at-most-one (AMO) (ADR-0006 §4), folded in the §5 pinned
    // order. The function must be satisfiable iff EXACTLY one child
    // predicate holds — unsatisfiable for zero, and for two or more.
    let expr = parse_condition_expr("exactly_one_of(a == 'x', b == 'y', c == 'z')").unwrap();
    assert!(
        matches!(&expr, ConditionExpr::ExactlyOneOf(args) if args.len() == 3),
        "parser must yield a 3-arg ExactlyOneOf node: {expr:?}"
    );
    let symbols = [
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
        ("c".to_string(), "z".to_string()),
    ];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let root = compile_expr(&mut builder, &expr, &index).unwrap();

    // Exhaustive truth table over (a == 'x', b == 'y', c == 'z'). The
    // formula is true on exactly the three single-true rows and false on
    // all others (zero true, two true, three true).
    for a in [false, true] {
        for b in [false, true] {
            for c in [false, true] {
                let trues = [a, b, c].iter().filter(|&&v| v).count();
                let expected = trues == 1;
                let got = eval(&builder, root, &[(0, a), (1, b), (2, c)]);
                assert_eq!(
                    got, expected,
                    "exactly_one_of(a,b,c) at (a={a}, b={b}, c={c}): \
                     {trues} predicates hold, expected satisfiable={expected}"
                );
            }
        }
    }
}

#[test]
fn exactly_one_of_two_args_is_xor() {
    // ADR-0006 §4: for N = 2, `exactly_one_of` is `(c_1 ∨ c_2) ∧ ¬(c_1 ∧ c_2)`
    // — exclusive-or. True iff exactly one of the two predicates holds.
    let expr = parse_condition_expr("exactly_one_of(a == 'x', b == 'y')").unwrap();
    let symbols = [
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
    ];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let root = compile_expr(&mut builder, &expr, &index).unwrap();
    assert!(!eval(&builder, root, &[(0, false), (1, false)]), "neither: unsat");
    assert!(eval(&builder, root, &[(0, true), (1, false)]), "only a: sat");
    assert!(eval(&builder, root, &[(0, false), (1, true)]), "only b: sat");
    assert!(!eval(&builder, root, &[(0, true), (1, true)]), "both: unsat");
}

#[test]
fn exactly_one_of_single_arg_is_the_child() {
    // ADR-0006 §4: for N = 1, AMO is vacuously true and the result reduces
    // to the single child `c_1`. The parser permits N = 1 (configflux-ccs.3);
    // construct the AST directly to pin the lowering edge case independent
    // of the parser's minimum-arg policy.
    let child = parse_condition_expr("a == 'x'").unwrap();
    let expr = ConditionExpr::ExactlyOneOf(vec![child.clone()]);
    let symbols = [("a".to_string(), "x".to_string())];
    let index = symbols
        .iter()
        .enumerate()
        .map(|(i, (tag, value))| (symbol_name(tag, value), i as u32))
        .collect();
    let mut builder = BddBuilder::default();
    let one_root = compile_expr(&mut builder, &expr, &index).unwrap();
    let child_root = compile_expr(&mut builder, &child, &index).unwrap();
    // ROBDD canonicity: `exactly_one_of(c_1)` and `c_1` are the same
    // function, so the builder hands back the same canonical node ref.
    assert_eq!(
        one_root, child_root,
        "exactly_one_of with a single child must reduce to that child's node"
    );
    assert!(eval(&builder, one_root, &[(0, true)]));
    assert!(!eval(&builder, one_root, &[(0, false)]));
}

fn eval(builder: &BddBuilder, node: u32, assignments: &[(u32, bool)]) -> bool {
    let map: BTreeMap<u32, bool> = assignments.iter().copied().collect();
    bdd::eval(builder, node, &map)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn sample_model() -> ConditionModel {
    // Representative input that exercises symbol de-dup (a.enabled
    // appears in multiple clauses), distinct values for the same
    // tag (z.on / z.off), nested unary negation, conjunctions and
    // disjunctions. This is the fixture used by every byte-
    // stability assertion in this module.
    ConditionModel {
        bound_model_hash: HASH.to_string(),
        clauses: vec![
            "z == 'on' && a == 'enabled'".to_string(),
            "a == 'enabled' || m == 'auto'".to_string(),
            "!(b == 'off')".to_string(),
            "z == 'off' || (m == 'auto' && a == 'enabled')".to_string(),
        ],
    }
}

#[test]
fn default_heuristic_byte_stable_against_explicit_facet_name_ascending() {
    // ADR-0005 §6 byte-stability contract: a caller that omits
    // `var_order_heuristic` (i.e. uses `build_ccm_artifact`) and a
    // caller that explicitly passes "facet-name-ascending" via the
    // public string-based API MUST produce a byte-identical
    // emission. This is the load-bearing test for the entire
    // qjjh refactor: if it ever fails, every committed fixture
    // (FAMA, scenario loop*, synthetic 10k) is at risk.
    let model = sample_model();
    let default_emission = build_ccm_artifact(&model).expect("default emit");
    let explicit_emission =
        build_ccm_artifact_with_heuristic(&model, "facet-name-ascending")
            .expect("explicit emit");
    assert_eq!(default_emission.bdd_bin, explicit_emission.bdd_bin);
    assert_eq!(default_emission.symbols_json, explicit_emission.symbols_json);
    assert_eq!(default_emission.manifest_json, explicit_emission.manifest_json);
}

#[test]
fn manifest_records_actual_heuristic_chosen() {
    // ADR-0005 §2: the manifest's
    // `algorithm_params.var_order_heuristic` MUST reflect the
    // heuristic actually applied at build time, not always the
    // default. This guards against the trivial bug where a
    // refactor leaves the field hard-coded to
    // `"facet-name-ascending"` and the new heuristic silently
    // emits with a misleading tag.
    let model = sample_model();
    let m_default = build_ccm_artifact(&model).expect("default emit");
    let m_dfs = build_ccm_artifact_with_heuristic(&model, "clause-grouped-dfs")
        .expect("dfs emit");
    let s_default =
        std::str::from_utf8(&m_default.manifest_json).expect("manifest is utf8");
    let s_dfs = std::str::from_utf8(&m_dfs.manifest_json).expect("manifest is utf8");
    assert!(
        s_default.contains("\"var_order_heuristic\":\"facet-name-ascending\""),
        "default manifest missing facet-name-ascending tag: {s_default}"
    );
    assert!(
        s_dfs.contains("\"var_order_heuristic\":\"clause-grouped-dfs\""),
        "dfs manifest missing clause-grouped-dfs tag: {s_dfs}"
    );
}

#[test]
fn manifest_records_in_crate_algorithm_tag() {
    // ADR-0011 Amendment 1 §A1.1 (configflux-ew24): the in-crate hand-rolled
    // BddBuilder path emits `algorithm: "robdd-handrolled-v1"`. The previous
    // value `"robdd-oxidd-v1"` was wrong — the compiler crate has no oxidd
    // dependency. The CUDD path (configflux-wbzw) will emit
    // `"robdd-cudd-v1"`; that is NOT under this test.
    let model = sample_model();
    let emission = build_ccm_artifact(&model).expect("default emit");
    let manifest = std::str::from_utf8(&emission.manifest_json)
        .expect("manifest is utf8");
    assert!(
        manifest.contains("\"algorithm\":\"robdd-handrolled-v1\""),
        "in-crate manifest missing robdd-handrolled-v1 tag: {manifest}"
    );
    assert!(
        !manifest.contains("robdd-oxidd-v1"),
        "in-crate manifest still carries the wrong robdd-oxidd-v1 tag: {manifest}"
    );
}

#[test]
fn manifest_records_accurate_in_crate_algorithm_params() {
    // ADR-0011 Amendment 1 §A1.4 (configflux-3t3z): the in-crate
    // hand-rolled `BddBuilder` path emits
    // `apply_cache=hashmap-default` (names the HashMap-backed apply
    // memos with clear-when-full eviction) and `manager=in-crate-v1`
    // (names the in-crate BddBuilder manager-equivalent, versioned in
    // the configflux schema). The previous oxidd-shaped values
    // `apply_cache=direct-mapped` / `manager=index` were inaccurate
    // descriptions of the hand-rolled implementation; the configflux-ew24
    // rotation left them alone, and configflux-3t3z rotates them here.
    let model = sample_model();
    let emission = build_ccm_artifact(&model).expect("default emit");
    let manifest = std::str::from_utf8(&emission.manifest_json)
        .expect("manifest is utf8");
    assert!(
        manifest.contains("\"apply_cache\":\"hashmap-default\""),
        "in-crate manifest missing apply_cache=hashmap-default: {manifest}"
    );
    assert!(
        manifest.contains("\"manager\":\"in-crate-v1\""),
        "in-crate manifest missing manager=in-crate-v1: {manifest}"
    );
    assert!(
        manifest.contains("\"reorder\":\"static\""),
        "in-crate manifest missing reorder=static: {manifest}"
    );
    assert!(
        manifest.contains("\"threads\":\"1\""),
        "in-crate manifest missing threads=1: {manifest}"
    );
    assert!(
        !manifest.contains("\"apply_cache\":\"direct-mapped\""),
        "in-crate manifest still carries the inaccurate apply_cache=direct-mapped: {manifest}"
    );
    assert!(
        !manifest.contains("\"manager\":\"index\""),
        "in-crate manifest still carries the inaccurate manager=index: {manifest}"
    );
}

#[test]
fn different_heuristics_produce_different_ccm_hash() {
    // ADR-0005 §10 G2: different `algorithm_params` MUST produce a
    // different `ccm_hash`. This is what lets a cache layer key on
    // the hash without over-sharing entries between heuristics.
    let model = sample_model();
    let m_asc = build_ccm_artifact(&model).expect("ascending emit");
    let m_dfs = build_ccm_artifact_with_heuristic(&model, "clause-grouped-dfs")
        .expect("dfs emit");
    let s_asc = std::str::from_utf8(&m_asc.manifest_json).expect("utf8");
    let s_dfs = std::str::from_utf8(&m_dfs.manifest_json).expect("utf8");
    let asc_hash = extract_ccm_hash(s_asc);
    let dfs_hash = extract_ccm_hash(s_dfs);
    assert_ne!(
        asc_hash, dfs_hash,
        "different heuristics must produce different ccm_hash"
    );
}

fn extract_ccm_hash(manifest: &str) -> String {
    let key = "\"ccm_hash\":\"";
    let start = manifest.find(key).expect("ccm_hash present") + key.len();
    let rest = &manifest[start..];
    let end = rest.find('"').expect("ccm_hash terminator present");
    rest[..end].to_string()
}

#[test]
fn unknown_heuristic_tag_is_rejected() {
    let model = sample_model();
    let err = build_ccm_artifact_with_heuristic(&model, "force-v1").unwrap_err();
    assert!(
        format!("{err}").contains("unknown var_order_heuristic"),
        "unexpected error: {err}"
    );
}

#[test]
fn build_ccm_artifact_is_deterministic() {
    // ADR-0005 §10 G1: same input + same heuristic → same bytes.
    let model = sample_model();
    let a = build_ccm_artifact(&model).expect("emit a");
    let b = build_ccm_artifact(&model).expect("emit b");
    assert_eq!(a.bdd_bin, b.bdd_bin);
    assert_eq!(a.symbols_json, b.symbols_json);
    assert_eq!(a.manifest_json, b.manifest_json);
}

#[test]
fn clause_grouped_dfs_is_deterministic() {
    let model = sample_model();
    let a = build_ccm_artifact_with_heuristic(&model, "clause-grouped-dfs")
        .expect("emit a");
    let b = build_ccm_artifact_with_heuristic(&model, "clause-grouped-dfs")
        .expect("emit b");
    assert_eq!(a.bdd_bin, b.bdd_bin);
    assert_eq!(a.symbols_json, b.symbols_json);
    assert_eq!(a.manifest_json, b.manifest_json);
}

#[test]
fn aggressive_memo_eviction_preserves_bdd_bytes() {
    // bd-93oj: the three memo tables (not_memo / and_memo / or_memo)
    // are caches, not canonical indices — `unique` is the canonical
    // interner. Aggressively clearing the memos between operations
    // (or between clauses, as `build_bdd_bin` now does) must NOT
    // change the emitted ccm.bdd.bin bytes. This test sets the memo
    // cap so low that essentially every operation triggers a clear,
    // and asserts the result is byte-identical to the default-cap
    // build of the same model.
    let model = sample_model();
    let normal = build_ccm_artifact(&model).expect("emit normal");
    let evicted = build_ccm_artifact_with_memo_cap_for_test(&model, /* memo_cap */ 1)
        .expect("emit with cap=1");
    assert_eq!(
        normal.bdd_bin, evicted.bdd_bin,
        "memo eviction must not change bdd_bin bytes"
    );
    assert_eq!(normal.symbols_json, evicted.symbols_json);
    assert_eq!(normal.manifest_json, evicted.manifest_json);
}

#[test]
fn per_clause_memo_clear_path_is_byte_stable() {
    // bd-93oj: `build_bdd_bin` clears the BDD memos between
    // top-level clause `and()` calls to bound peak memory on
    // 10k+ workloads. That clear must not perturb output bytes
    // for any model — same input, same output, regardless of
    // whether the per-clause clear fired N times or zero times.
    // (The default cap is large enough that no clear-when-full
    // fires on the small sample model, so this is the explicit
    // companion to the byte-stability tests above.)
    let model = sample_model();
    let a = build_ccm_artifact(&model).expect("emit a");
    let b = build_ccm_artifact(&model).expect("emit b");
    assert_eq!(a.bdd_bin, b.bdd_bin);
}

#[test]
fn live_memo_shrink_drops_cap_below_initial_and_is_byte_neutral() {
    // configflux-9pjy.4 / ADR-0039 §5 — the headline acceptance (AC1).
    // A deliberately tight RSS budget, with a sampler that always reports
    // over-budget, must drive the live memo_cap BELOW its initial derived
    // value AND the produced artifact bytes must equal the unbudgeted
    // (no-RSS-budget) build of the same input. The shrink is byte-neutral:
    // the memos are caches, not the canonical `unique` table.
    let model = sample_model();

    // Unbudgeted baseline (the bytes the budgeted build must match).
    let baseline = build_ccm_artifact(&model).expect("unbudgeted emit");

    // Start well above the floor so there is room to shrink, then force a
    // shrink at every one of the model's clause boundaries via the
    // always-over-budget sampler. `rss_budget_kib` value is irrelevant to
    // the bytes — only the cap (a cache size) changes.
    let initial_cap = 1 << 16; // 65536, four halvings above the 4096 floor
    let (budgeted, adaptation) = build_ccm_artifact_with_rss_budget_for_test(
        &model,
        initial_cap,
        Some(1), // tiny budget; the always-over sampler trips it every clause
        always_over_budget_rss,
    )
    .expect("budgeted emit");

    // (a) the cap dropped below its initial derived value.
    assert!(
        adaptation.shrink_count > 0,
        "expected at least one live memo-cap shrink, got shrink_count=0"
    );
    assert!(
        adaptation.final_memo_cap < adaptation.initial_memo_cap,
        "final memo_cap {} must be below initial {}",
        adaptation.final_memo_cap,
        adaptation.initial_memo_cap
    );
    assert_eq!(adaptation.initial_memo_cap, initial_cap);

    // (b) byte-neutral: the budgeted (shrunk) build equals the unbudgeted
    // build, file for file.
    assert_eq!(
        baseline.bdd_bin, budgeted.bdd_bin,
        "live memo-cap shrink must not change ccm.bdd.bin bytes"
    );
    assert_eq!(baseline.symbols_json, budgeted.symbols_json);
    assert_eq!(baseline.manifest_json, budgeted.manifest_json);
}

#[test]
fn live_memo_shrink_never_falls_below_floor() {
    // ADR-0039 §3 Risks: a tight budget must not thrash the cap to zero.
    // Even with the sampler always over budget across every clause, the
    // final cap stays at or above the floor and the build still produces
    // the byte-identical artifact.
    let model = sample_model();
    let baseline = build_ccm_artifact(&model).expect("unbudgeted emit");
    let (budgeted, adaptation) = build_ccm_artifact_with_rss_budget_for_test(
        &model,
        1 << 20, // start at the default cap
        Some(1),
        always_over_budget_rss,
    )
    .expect("budgeted emit");
    assert!(
        adaptation.final_memo_cap >= crate::resource_budget::MEMO_CAP_FLOOR,
        "final memo_cap {} fell below the floor {}",
        adaptation.final_memo_cap,
        crate::resource_budget::MEMO_CAP_FLOOR
    );
    assert_eq!(baseline.bdd_bin, budgeted.bdd_bin);
}

#[test]
fn no_rss_budget_means_no_shrink_and_byte_identical() {
    // The None-budget path must not sample, must not shrink, and must be
    // byte-identical to the plain build — even if the (unused) sampler
    // would have reported over budget. Guards the byte-stability invariant
    // for the default (unbudgeted) compile.
    let model = sample_model();
    let baseline = build_ccm_artifact(&model).expect("unbudgeted emit");
    let (no_budget, adaptation) = build_ccm_artifact_with_rss_budget_for_test(
        &model,
        1 << 16,
        None, // no RSS budget ⇒ no sampling, no shrink
        always_over_budget_rss,
    )
    .expect("no-budget emit");
    assert_eq!(adaptation.shrink_count, 0, "no budget must mean no shrink");
    assert_eq!(adaptation.final_memo_cap, adaptation.initial_memo_cap);
    assert_eq!(baseline.bdd_bin, no_budget.bdd_bin);
    assert_eq!(baseline.symbols_json, no_budget.symbols_json);
    assert_eq!(baseline.manifest_json, no_budget.manifest_json);
}

#[test]
fn under_budget_sampler_does_not_shrink() {
    // The complement of the headline test: when the sampler reports a
    // reading comfortably under the budget, no shrink fires even though a
    // budget is set. Pins that the shrink is driven by the RSS reading, not
    // merely by the presence of a budget.
    fn well_under_budget_rss() -> Option<u64> {
        Some(1) // 1 KiB, far below any real budget
    }
    let model = sample_model();
    let (emission, adaptation) = build_ccm_artifact_with_rss_budget_for_test(
        &model,
        1 << 16,
        Some(1_000_000), // ~1 GiB budget; a 1 KiB reading never trips 0.85×
        well_under_budget_rss,
    )
    .expect("budgeted emit");
    assert_eq!(
        adaptation.shrink_count, 0,
        "an under-budget RSS reading must not trigger a shrink"
    );
    assert_eq!(adaptation.final_memo_cap, adaptation.initial_memo_cap);
    let baseline = build_ccm_artifact(&model).expect("unbudgeted emit");
    assert_eq!(baseline.bdd_bin, emission.bdd_bin);
}

#[test]
fn adaptive_shrink_count_non_decreasing_as_budget_shrinks() {
    // configflux-9pjy.5 / ADR-0039 §8 — the ADAPTIVE half of the core
    // graceful-degradation contract. As the soft RSS budget shrinks, the
    // live adaptive memo-clear count (shrink_count) must be NON-DECREASING:
    // a tighter budget trips the 0.85×budget shrink trigger at more clause
    // boundaries, trading more clear/rebuild cycles for a lower peak.
    //
    // This is made DETERMINISTIC by injecting a FIXED-RSS sampler (no /proc,
    // no clock): the sampler always reports the same resident set `FIXED_RSS`,
    // and we sweep the budget DOWN so the trigger `0.85 × budget` crosses
    // `FIXED_RSS`. A large budget keeps the trigger above the reading (no
    // shrink); a small budget drops the trigger below it (shrink every
    // clause, until the floor). shrink_count is therefore monotonic in the
    // budget. The companion pure half (memo_cap non-increasing) lives in
    // `scenario_resource_budget_tests`.
    const FIXED_RSS_KIB: u64 = 1_000_000; // ~1 GiB, constant every sample.
    fn fixed_rss() -> Option<u64> {
        Some(FIXED_RSS_KIB)
    }

    let model = sample_model();
    // Start at the default cap so there is ample room to halve before the
    // MEMO_CAP_FLOOR is reached (the model has a handful of clauses, so the
    // floor is never the binding constraint here).
    let initial_cap = DEFAULT_MEMO_CAP;

    // Descending budgets (KiB). The trigger is 0.85×budget:
    //   2_000_000 → 1_700_000 (> FIXED_RSS ⇒ no shrink)
    //   1_200_000 → 1_020_000 (> FIXED_RSS ⇒ no shrink)
    //   1_000_000 →   850_000 (< FIXED_RSS ⇒ shrink)
    //     500_000 →   425_000 (< FIXED_RSS ⇒ shrink)
    //     100_000 →    85_000 (< FIXED_RSS ⇒ shrink)
    // so the sequence of shrink counts is non-decreasing and strictly rises
    // once the trigger crosses the fixed reading.
    let budgets_kib: [u64; 5] = [2_000_000, 1_200_000, 1_000_000, 500_000, 100_000];
    let mut prev_count: u32 = 0;
    let mut saw_increase = false;
    for &budget_kib in &budgets_kib {
        let (_emission, adaptation) = build_ccm_artifact_with_rss_budget_for_test(
            &model,
            initial_cap,
            Some(budget_kib),
            fixed_rss,
        )
        .expect("budgeted emit");
        assert!(
            adaptation.shrink_count >= prev_count,
            "shrink_count must be non-decreasing as the budget shrinks: \
             budget={budget_kib} KiB gave {}, the previous (larger) budget gave {prev_count}",
            adaptation.shrink_count
        );
        if adaptation.shrink_count > prev_count {
            saw_increase = true;
        }
        prev_count = adaptation.shrink_count;
    }
    // The sweep must actually exercise the trigger crossing — otherwise the
    // "non-decreasing" assertion is vacuously satisfied by all-zero counts.
    assert!(
        saw_increase,
        "the budget sweep must drive shrink_count up at least once \
         (the fixed RSS reading must cross the 0.85×budget trigger)"
    );
    assert!(
        prev_count > 0,
        "the tightest budget in the sweep must have fired at least one shrink"
    );
}

#[test]
fn with_timings_byte_identical_to_legacy() {
    // configflux-o89x: the timed entry point must produce a
    // byte-identical `CcmEmission` to the legacy entry point on the
    // same input. This is the load-bearing guarantee that the
    // `--profile` plumbing did not perturb the canonical bytes —
    // ADR-0005 §6 byte-stability MUST stay intact whether
    // instrumentation is on or off. If this ever fails, every
    // committed fixture (FAMA, scenario loop*, synthetic 10k) is at
    // risk.
    let model = sample_model();
    let legacy = build_ccm_artifact(&model).expect("legacy emit");
    let (timed, t) =
        build_ccm_artifact_with_timings(&model).expect("timed emit");
    assert_eq!(legacy.bdd_bin, timed.bdd_bin);
    assert_eq!(legacy.symbols_json, timed.symbols_json);
    assert_eq!(legacy.manifest_json, timed.manifest_json);
    // disk_write is meaningful only on the dir-writing path; the
    // in-memory build leaves it at the default (zero) duration.
    assert_eq!(t.disk_write, std::time::Duration::ZERO);
}

#[test]
fn with_timings_records_nonzero_stages_for_real_input() {
    // configflux-o89x: a non-empty model exercises every measured
    // stage at least briefly. We do not assert specific absolute
    // wall-clock numbers — the test machine is non-deterministic and
    // a strict bound would be flaky. We only assert that the parse,
    // var-order, apply-loop, and serialize stages each show >0 ns of
    // wall time, i.e. that the instrumentation is wired through to
    // every stage and the timer is actually advancing.
    let model = sample_model();
    let (_e, t) =
        build_ccm_artifact_with_timings(&model).expect("timed emit");
    assert!(t.clause_parse > std::time::Duration::ZERO);
    assert!(t.var_order > std::time::Duration::ZERO);
    assert!(t.bdd_apply_loop > std::time::Duration::ZERO);
    assert!(t.manifest_serialize > std::time::Duration::ZERO);
    // total() sums all five stages — sanity check the helper too.
    assert!(t.total() >= t.clause_parse + t.bdd_apply_loop);
}

#[test]
fn emit_ccm_dir_with_timings_records_disk_write_and_matches_legacy() {
    // configflux-o89x: the dir-writing path records disk_write as a
    // distinct stage AND produces byte-identical output to the
    // legacy `emit_ccm_dir_with_heuristic` call (when the same
    // heuristic is passed). We round-trip the three written files
    // through `std::fs::read` and compare against the in-memory
    // emission to pin the contract.
    let model = sample_model();
    let dir =
        std::env::temp_dir().join("configflux-o89x-emit-dir-with-timings");
    let _ = std::fs::remove_dir_all(&dir);
    let timings = emit_ccm_dir_with_timings(&model, &dir, "facet-name-ascending")
        .expect("timed dir emit");
    // disk_write must have advanced (we wrote 3 small files).
    assert!(timings.disk_write > std::time::Duration::ZERO);
    // configflux-vmlb / ADR-0012 §4: under the v2 multi-part wire
    // format, the legacy single-triple lives inside `partition-0000/`
    // (single-partition collapse). The in-memory `build_ccm_artifact`
    // return is the per-partition shape — bit-equal to the bytes
    // written into `partition-0000/`. The top-level files
    // (`<dir>/ccm.manifest.json`, `<dir>/ccm.symbols.json`,
    // `<dir>/partition-manifest.json`) are new v2 artifacts and have
    // no v1 byte-equality contract.
    let legacy = build_ccm_artifact(&model).expect("legacy emit");
    let p0 = dir.join("partition-0000");
    let bdd_on_disk = std::fs::read(p0.join("ccm.bdd.bin")).expect("read bdd");
    let symbols_on_disk =
        std::fs::read(p0.join("ccm.symbols.json")).expect("read symbols");
    let manifest_on_disk =
        std::fs::read(p0.join("ccm.manifest.json")).expect("read manifest");
    assert_eq!(bdd_on_disk, legacy.bdd_bin);
    assert_eq!(symbols_on_disk, legacy.symbols_json);
    assert_eq!(manifest_on_disk, legacy.manifest_json);
    // Cleanup; ignore errors — the next run will re-create.
    let _ = std::fs::remove_dir_all(&dir);
}

// configflux-4jmc: in the lean default build (compiler crate without the `cudd`
// feature), the `Construction::Cudd` arm fails closed instead of linking the
// CUDD C backend. This runs ONLY in that configuration — the cudd-feature build
// (//compiler:compiler_lib_cudd) exercises the real path via
// `cudd_construction_roundtrip_test` and `compiler_cudd_test` instead.
#[cfg(not(feature = "cudd"))]
#[test]
fn cudd_construction_fails_closed_in_lean_build() {
    let model = sample_model();
    // (1) single-file entry point (gated arm in `build_ccm_artifact_inner`):
    // "cudd" parses, then bails at the build arm with the unavailable message.
    let err = build_ccm_artifact_with_construction(&model, "facet-name-ascending", "cudd")
        .expect_err("cudd construction must fail closed without the `cudd` feature");
    assert!(
        err.to_string().contains("CUDD construction path is unavailable"),
        "fail-closed message must explain the cudd path is unavailable, got: {err}"
    );
    // (2) multi-part dir entry point (the second gated arm via
    // `build_bdd_bin_via_construction`) must also fail closed.
    let dir = std::env::temp_dir().join(format!("cfx_4jmc_lean_cudd_{}", std::process::id()));
    let dir_err = emit_ccm_dir_with_construction(&model, &dir, "facet-name-ascending", "cudd")
        .expect_err("cudd dir emit must fail closed without the `cudd` feature");
    assert!(
        dir_err.to_string().contains("CUDD construction path is unavailable"),
        "dir-emit fail-closed message must mention the unavailable path, got: {dir_err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    // (3) in-crate construction must STILL succeed — only the CUDD path is gated,
    // not the selector or the default builder.
    build_ccm_artifact_with_construction(&model, "facet-name-ascending", "in-crate")
        .expect("in-crate construction still works in the lean build");
}
