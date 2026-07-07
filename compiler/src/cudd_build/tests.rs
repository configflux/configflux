// SPDX-License-Identifier: BUSL-1.1
//
// Unit tests for the CUDD-side compiler build path
// (`compiler::cudd_build::build_bdd_bin_via_cudd`).
//
// Split from `cudd_build/mod.rs` per ADR-0011 Amendment 1 §A1.3 which
// permits N files per side where N is the minimum needed for clean
// separation of concerns. Tests do NOT import `cudd_sys` directly —
// they reach through `super::*` so the file-level single-importer
// boundary (only `cudd_build/mod.rs` imports `cudd_sys`) is preserved.

use super::*;
use crate::conditions::parse_condition_expr;

/// `Cudd_Init` succeeds and the resulting builder has the right
/// number of per-variable literals. Refcounts balance at Drop.
#[test]
fn cudd_builder_initialises_with_var_count() {
    let builder = CuddBuilder::new(3, None).expect("CUDD init must succeed for 3 variables");
    assert_eq!(builder.var_functions.len(), 3);
    drop(builder);
}

/// Compiling an empty clause list produces a valid `ccm.bdd.bin`
/// encoding the constant TRUE: header + root_table=[TERMINAL_TRUE]
/// + two terminal records.
#[test]
fn empty_clause_list_emits_constant_true() {
    let bytes = build_bdd_bin_via_cudd(&[], &[], None).expect("emit");
    assert_eq!(&bytes[0..4], CCM_BDD_BIN_MAGIC, "magic header");
    assert_eq!(bytes[4], CCM_BDD_BIN_VERSION, "version byte");
    // var_count at offset 8.
    let var_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    assert_eq!(var_count, 0);
    // node_count at offset 12 — two terminals.
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    assert_eq!(node_count, 2);
    // root_count at offset 16 — always 1.
    let root_count = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    assert_eq!(root_count, 1);
    // root[0] at offset 20 — must be TERMINAL_TRUE for empty clauses.
    let root = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    assert_eq!(root, TERMINAL_TRUE);
}

/// Compiling a single trivial predicate (`a == 'on'`) produces a
/// non-trivial BDD with exactly one non-terminal node. The CUDD
/// path's byte stream must decode through the standard ADR-0005 §4
/// reader.
#[test]
fn single_predicate_emits_one_nonterminal() {
    let expr = parse_condition_expr("a == 'on'").expect("parse");
    let symbols = vec![("a".to_string(), "on".to_string())];
    let bytes = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    // Two terminals + one a-node = 3 records.
    assert_eq!(node_count, 3, "single predicate must yield 3 records");
}

/// Conjunction of two predicates produces three non-terminals:
/// terminals + node-for-b + node-for-a (post-order).
#[test]
fn two_var_and_emits_post_order_table() {
    let expr = parse_condition_expr("a == 'on' && b == 'enabled'").expect("parse");
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let bytes = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    // 2 terminals + 2 non-terminals (one per variable) = 4 records.
    assert_eq!(node_count, 4);
}

/// Multiple clauses are AND-folded; the result is the same shape
/// as a single conjunction.
#[test]
fn multi_clause_and_fold() {
    let exprs = vec![
        parse_condition_expr("a == 'on'").expect("parse"),
        parse_condition_expr("b == 'enabled'").expect("parse"),
    ];
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let bytes = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    // Same result as `a && b`: 2 terminals + 2 non-terminals.
    assert_eq!(node_count, 4);
}

/// Negation: `!(a == 'on')` produces an a-node whose low/high are
/// swapped relative to the bare `a == 'on'` encoding.
#[test]
fn negation_emits_complementary_node() {
    let expr = parse_condition_expr("!(a == 'on')").expect("parse");
    let symbols = vec![("a".to_string(), "on".to_string())];
    let bytes = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    assert_eq!(node_count, 3, "negated single predicate: 2 terminals + 1 a-node");
    // Read the third record (the a-node) and verify low_id=TRUE,
    // high_id=FALSE — the negation of the standard a-node (which
    // would be low=FALSE, high=TRUE).
    let rec_offset = 20 + 4 + 2 * 16; // header + root + two terminal records
    let var = u32::from_le_bytes([
        bytes[rec_offset],
        bytes[rec_offset + 1],
        bytes[rec_offset + 2],
        bytes[rec_offset + 3],
    ]);
    let low = u32::from_le_bytes([
        bytes[rec_offset + 4],
        bytes[rec_offset + 5],
        bytes[rec_offset + 6],
        bytes[rec_offset + 7],
    ]);
    let high = u32::from_le_bytes([
        bytes[rec_offset + 8],
        bytes[rec_offset + 9],
        bytes[rec_offset + 10],
        bytes[rec_offset + 11],
    ]);
    assert_eq!(var, 0, "negated a-node has var=0");
    assert_eq!(low, TRUE_NODE_INDEX, "negated low points to TRUE");
    assert_eq!(high, FALSE_NODE_INDEX, "negated high points to FALSE");
}

/// Disjunction: `a == 'on' || b == 'enabled'`. Two variables, two
/// non-terminals.
#[test]
fn disjunction_emits_or_shape() {
    let expr = parse_condition_expr("a == 'on' || b == 'enabled'").expect("parse");
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let bytes = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    assert_eq!(node_count, 4, "two-var OR: 2 terminals + 2 non-terminals");
}

/// A trivially-false clause (`a == 'on' && !(a == 'on')`) reduces
/// to the FALSE terminal. The root_table entry must encode the
/// FALSE sentinel.
#[test]
fn trivially_false_emits_terminal_false_root() {
    let expr = parse_condition_expr("a == 'on' && !(a == 'on')").expect("parse");
    let symbols = vec![("a".to_string(), "on".to_string())];
    let bytes = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit");
    let node_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    assert_eq!(node_count, 2, "tautologically-false formula has only the two terminals");
    let root = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    assert_eq!(root, TERMINAL_FALSE, "root_table must encode TERMINAL_FALSE sentinel");
}

/// Determinism: building the same model twice produces byte-
/// identical output. CUDD's apply cache is deterministic for a
/// fresh manager + fixed inputs; this test pins that contract.
#[test]
fn double_build_is_byte_stable() {
    let expr = parse_condition_expr("a == 'on' && b == 'enabled'").expect("parse");
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let bytes_a = build_bdd_bin_via_cudd(&[expr.clone()], &symbols, None).expect("emit A");
    let bytes_b = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit B");
    assert_eq!(bytes_a, bytes_b, "two CUDD-path builds of the same input MUST be byte-identical");
}

/// configflux-ccs.6: `exactly_one_of(a, b, c)` lowers (ADR-0006 §4) to
/// at-least-one (OR) ∧ pairwise at-most-one (AMO). The CUDD path must
/// produce the SAME reduced BDD — and therefore the byte-identical
/// `ccm.bdd.bin` under a fixed variable order — as the hand-written
/// logically-equivalent expansion `(a || b || c) && !(a && b) &&
/// !(a && c) && !(b && c)`. Because both inputs go through the same CUDD
/// reduce and the same ADR-0005 §4 canonical serialization with the same
/// `symbols` order, byte-equality is a sound semantic oracle for the
/// lowering. This also exercises the `ExactlyOneOf` arm's `Cudd_Ref` /
/// `Cudd_RecursiveDeref` discipline: a refcount imbalance would surface
/// as a debug-CUDD assertion at `Cudd_Quit` or as nondeterministic bytes.
#[test]
fn exactly_one_of_matches_hand_written_alo_and_amo() {
    let symbols = vec![
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
        ("c".to_string(), "z".to_string()),
    ];
    let sugar = parse_condition_expr("exactly_one_of(a == 'x', b == 'y', c == 'z')")
        .expect("parse exactly_one_of");
    // ALO ∧ pairwise AMO, written out by hand. Logically identical to
    // exactly_one_of over the same three predicates.
    let expanded = parse_condition_expr(
        "(a == 'x' || b == 'y' || c == 'z') \
         && !(a == 'x' && b == 'y') \
         && !(a == 'x' && c == 'z') \
         && !(b == 'y' && c == 'z')",
    )
    .expect("parse hand-written expansion");

    let sugar_bytes = build_bdd_bin_via_cudd(&[sugar], &symbols, None).expect("emit sugar");
    let expanded_bytes = build_bdd_bin_via_cudd(&[expanded], &symbols, None).expect("emit expanded");
    assert_eq!(
        sugar_bytes, expanded_bytes,
        "exactly_one_of must reduce to the same BDD as its ALO ∧ pairwise-AMO expansion"
    );
}

/// Determinism for the `ExactlyOneOf` arm specifically: two CUDD builds
/// of the same `exactly_one_of(...)` model must be byte-identical. The
/// pairwise AMO fold reads each lowered child multiple times; this pins
/// that the ref/deref bookkeeping does not perturb the apply cache or
/// node-allocation order between runs (configflux-ccs.6).
#[test]
fn exactly_one_of_double_build_is_byte_stable() {
    let symbols = vec![
        ("a".to_string(), "x".to_string()),
        ("b".to_string(), "y".to_string()),
        ("c".to_string(), "z".to_string()),
    ];
    let expr = parse_condition_expr("exactly_one_of(a == 'x', b == 'y', c == 'z')")
        .expect("parse");
    let bytes_a = build_bdd_bin_via_cudd(&[expr.clone()], &symbols, None).expect("emit A");
    let bytes_b = build_bdd_bin_via_cudd(&[expr], &symbols, None).expect("emit B");
    assert_eq!(
        bytes_a, bytes_b,
        "two CUDD-path builds of the same exactly_one_of model MUST be byte-identical"
    );
}

/// Unknown predicate (`c == 'missing'`) where `c` is not in
/// `symbols` should bail with a clear error message rather than
/// panic or silently produce garbage.
#[test]
fn unknown_predicate_errors_cleanly() {
    let expr = parse_condition_expr("c == 'missing'").expect("parse");
    let symbols = vec![("a".to_string(), "on".to_string())];
    let result = build_bdd_bin_via_cudd(&[expr], &symbols, None);
    assert!(result.is_err(), "unknown predicate must surface as Err");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("missing symbol index"), "error names the missing symbol: {msg}");
}

// ---------------------------------------------------------------------
// configflux-xh57 / ADR-0039 (cudd-path amendment): the soft resource
// budget's derived `memo_cap` now reaches the CUDD apply (computed)
// cache via `Cudd_SetMaxCacheHard`. The cache is a PURE memoization
// table (the unique table is the sole canonicity authority — see
// third_party/cudd/cudd/cuddCache.c + cuddTable.c::cuddUniqueInter), so
// capping it only trades wall-clock for resident RAM and MUST NOT change
// the emitted `ccm.bdd.bin` bytes (ADR-0039 §8 byte-stability invariant).
// ---------------------------------------------------------------------

/// The mapping helper is deterministic and documented: a `None` derived
/// memo_cap leaves the CUDD cache uncapped (`None`, byte-identical to
/// today); a `Some(memo_cap)` maps to a single CUDD hard cap of
/// `memo_cap * 3` entries (the in-crate path keeps THREE per-table
/// memos, so the equivalent total CUDD computed-cache footprint is 3×
/// the per-table cap), floored so a tight budget never wedges the cache
/// below a usable size.
#[test]
fn derive_cudd_cache_cap_maps_memo_cap() {
    // None in -> None out (no setter call, CUDD heuristic default stands).
    assert_eq!(derive_cudd_cache_cap(None), None);
    // Some(cap) -> Some(cap * 3), floored.
    let cap = 100_000_usize;
    assert_eq!(derive_cudd_cache_cap(Some(cap)), Some((cap * 3) as u32));
}

/// A tiny derived memo_cap must still yield a usable (non-zero, floored)
/// CUDD cache cap — capping to 0 would tell CUDD to derive its own
/// default (defeating the budget), and an extreme-tight cache thrashes.
#[test]
fn derive_cudd_cache_cap_floor_holds() {
    let cap = derive_cudd_cache_cap(Some(1)).expect("Some(_) in must give Some(_) out");
    assert!(cap >= CUDD_CACHE_CAP_FLOOR, "derived CUDD cap {cap} fell below floor");
    assert!(cap > 0, "CUDD cache cap must never be zero (0 => CUDD default)");
}

/// The derived cap actually reaches the CUDD manager: build a
/// `CuddBuilder` with a cache cap and assert the live manager reports it
/// via `Cudd_ReadMaxCacheHard`. This proves the budget is no longer a
/// silent no-op on the cudd path (the whole point of configflux-xh57).
#[test]
fn cudd_cache_cap_applied_to_manager() {
    let cap_entries = 50_000_usize;
    let mapped = derive_cudd_cache_cap(Some(cap_entries)).expect("Some in -> Some out");
    let builder =
        CuddBuilder::new(2, Some(mapped)).expect("CUDD init with cache cap must succeed");
    assert_eq!(
        builder.read_max_cache_hard(),
        mapped,
        "Cudd_ReadMaxCacheHard must report the cap we set"
    );
    drop(builder);
}

/// Byte-stability (ADR-0039 §8): capping the apply cache MUST NOT change
/// the emitted bytes. Build the same multi-clause model with the cache
/// uncapped vs. capped small; the `ccm.bdd.bin` must be byte-identical
/// because the cache is a pure memo (a miss recomputes the SAME
/// canonical node through the unique table).
#[test]
fn cudd_cache_cap_does_not_change_bytes() {
    let exprs = vec![
        parse_condition_expr("a == 'on' && b == 'enabled'").expect("parse"),
        parse_condition_expr("a == 'on' || c == 'ready'").expect("parse"),
        parse_condition_expr("!(b == 'enabled') && c == 'ready'").expect("parse"),
    ];
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
        ("c".to_string(), "ready".to_string()),
    ];
    let uncapped = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit uncapped");
    // A small but floored memo_cap drives a small CUDD cache cap.
    let capped =
        build_bdd_bin_via_cudd(&exprs, &symbols, Some(MEMO_CAP_FLOOR)).expect("emit capped");
    assert_eq!(
        uncapped, capped,
        "capping the CUDD apply cache MUST NOT change the emitted bytes (cache is pure memo)"
    );
}

/// §8 invariant, explicit: the `memo_cap = None` cudd build is the path
/// the existing two-arg call used to take. The empty-budget build must
/// stay byte-for-byte identical to itself across runs (the budget plumb
/// adds no perturbation when unset).
#[test]
fn cudd_budget_none_is_byte_identical_across_runs() {
    let exprs = vec![
        parse_condition_expr("a == 'on'").expect("parse"),
        parse_condition_expr("b == 'enabled'").expect("parse"),
        parse_condition_expr("a == 'on' && b == 'enabled'").expect("parse"),
    ];
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let a = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit A");
    let b = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit B");
    assert_eq!(a, b, "budget=None cudd build must be byte-identical across runs");
}

/// configflux-8l3b: the CUDD-CHECKPOINT path defaults to byte-
/// stable silence — when `CONFIGFLUX_CUDD_BUILDER_PROFILE` is
/// unset, `maybe_checkpoint` is a no-op and the bytes produced by
/// a multi-clause build are byte-identical to the bytes produced
/// when checkpointing is forcibly off. Lower-level invariants
/// (env-gate, RSS fallback, MB rounding) are covered by the
/// inline test module in `cudd_build/checkpoint.rs`.
#[test]
fn checkpoint_does_not_perturb_bytes_when_env_unset() {
    // Removing an env var here is process-global; the test suite
    // runs sequentially (one binary) so we accept the cross-test
    // race risk and just unset to be explicit. Rust 2021 keeps
    // `std::env::remove_var` safe — Rust 2024 will make it unsafe;
    // when the workspace moves to 2024 the call here will need an
    // explicit `unsafe` block (configflux-8l3b).
    std::env::remove_var("CONFIGFLUX_CUDD_BUILDER_PROFILE");
    let exprs = vec![
        parse_condition_expr("a == 'on'").expect("parse"),
        parse_condition_expr("b == 'enabled'").expect("parse"),
        parse_condition_expr("a == 'on' && b == 'enabled'").expect("parse"),
    ];
    let symbols = vec![
        ("a".to_string(), "on".to_string()),
        ("b".to_string(), "enabled".to_string()),
    ];
    let bytes_a = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit A");
    let bytes_b = build_bdd_bin_via_cudd(&exprs, &symbols, None).expect("emit B");
    assert_eq!(
        bytes_a, bytes_b,
        "CUDD-CHECKPOINT must not perturb the byte stream when the env gate is unset"
    );
}
