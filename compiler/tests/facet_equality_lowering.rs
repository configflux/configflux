// SPDX-License-Identifier: BUSL-1.1
//
// Lowering of the facet-to-facet comparison form — configflux-secb.2,
// ADR-0057 §D5.
//
// `a == b` means "a and b are bound to the same value", and the ADR pins its
// meaning as a pairwise equivalence over the UNION of the two declared
// domains, with a side's symbol read as the constant false for a value that
// side cannot take. These tests assert that meaning against the real compiled
// BDD, through the solver, rather than against the expanded AST — the point of
// the feature is what the emitted `.ccm` says, and an expansion that were
// merely plausible would pass an AST-shaped test.
//
// T2 enumerates every assignment of two three-valued facets and requires the
// model to be satisfiable exactly when the two agree. T3 covers asymmetric
// domains, where the union rule has observable content. T9 pins byte
// stability across two emissions of the same model (ADR-0006 §5 fold order).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use compiler::ccm_emitter::{emit_ccm_dir_with_construction, ConditionModel};
use solver::{OxiddBackend, Session};

/// A model with two closed facets and one authored constraint over them.
///
/// The channels mirror what `compiler_core` produces for a real pack: a
/// symbol-introducing tautology per declared value (ADR-0047 §4 Amendment 1),
/// the authored constraint verbatim (ADR-0054 §5.1), and the closed-facet
/// `exactly_one_of` cardinality conjunct (ADR-0054 §5.2). Only
/// `facet_domains` is new, and it is what lets the emitter expand the
/// comparison.
fn two_facet_model(
    left: (&str, &[&str]),
    right: (&str, &[&str]),
    constraint: &str,
) -> ConditionModel {
    let facets = [left, right];
    let mut clauses: Vec<String> = Vec::new();
    let mut cardinality: Vec<String> = Vec::new();
    let mut facet_domains: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (name, values) in facets {
        for value in values {
            clauses.push(format!("{name} == '{value}' || {name} != '{value}'"));
        }
        let args: Vec<String> = values
            .iter()
            .map(|value| format!("{name} == '{value}'"))
            .collect();
        cardinality.push(format!("exactly_one_of({})", args.join(", ")));
        facet_domains.insert(
            name.to_string(),
            values.iter().map(|v| v.to_string()).collect(),
        );
    }

    ConditionModel {
        bound_model_hash: "44".repeat(32),
        clauses,
        constraints: vec![("groups_equal".to_string(), constraint.to_string())],
        cardinality,
        facet_domains,
    }
}

fn emit(model: &ConditionModel, label: &str) -> PathBuf {
    let dir = tempdir_for(label);
    emit_ccm_dir_with_construction(model, &dir, "facet-name-ascending", "in-crate")
        .expect("emit must succeed");
    dir
}

fn load(dir: &Path) -> Session<OxiddBackend> {
    let ccm = Session::<OxiddBackend>::load_ccm(dir).expect("load_ccm");
    Session::<OxiddBackend>::new(ccm).expect("session")
}

fn options(session: &Session<OxiddBackend>, facet: &str) -> Vec<String> {
    let mut names = session
        .valid_options(facet)
        .unwrap_or_else(|e| panic!("valid_options({facet}): {e}"))
        .options;
    names.sort();
    names
}

/// T2. Two three-valued facets tied by `a == b`: of the nine assignments,
/// exactly the three diagonal ones are satisfiable.
#[test]
fn equality_over_identical_domains_admits_exactly_the_agreeing_assignments() {
    let values = ["c1", "c2", "c3"];
    let model = two_facet_model(
        ("line_container", &values),
        ("sorter_container", &values),
        "line_container == sorter_container",
    );
    let dir = emit(&model, "facet_eq_identical_domains");

    // Nothing bound yet: both facets keep their full domain, because every
    // value is still reachable in agreement with the other side.
    let session = load(&dir);
    assert_eq!(options(&session, "line_container"), values);
    assert_eq!(options(&session, "sorter_container"), values);

    // Binding one side collapses the other to the single agreeing value. The
    // three runs together decide all nine assignments: the diagonal is
    // satisfiable and the six off-diagonal pairs are not.
    for chosen in values {
        let mut session = load(&dir);
        session
            .apply("line_container", chosen)
            .unwrap_or_else(|e| panic!("apply(line_container={chosen}): {e}"));
        assert_eq!(
            options(&session, "sorter_container"),
            vec![chosen.to_string()],
            "line_container={chosen} must force sorter_container={chosen}"
        );
    }
}

/// `a != b` is the negation: every assignment EXCEPT the agreeing ones.
#[test]
fn inequality_over_identical_domains_excludes_the_agreeing_assignments() {
    let values = ["c1", "c2", "c3"];
    let model = two_facet_model(
        ("line_container", &values),
        ("sorter_container", &values),
        "line_container != sorter_container",
    );
    let dir = emit(&model, "facet_ne_identical_domains");

    for chosen in values {
        let mut session = load(&dir);
        session
            .apply("line_container", chosen)
            .unwrap_or_else(|e| panic!("apply(line_container={chosen}): {e}"));
        let remaining = options(&session, "sorter_container");
        let expected: Vec<String> = values
            .iter()
            .filter(|v| **v != chosen)
            .map(|v| v.to_string())
            .collect();
        assert_eq!(
            remaining, expected,
            "line_container={chosen} must forbid exactly sorter_container={chosen}"
        );
    }
}

/// T3. Asymmetric domains: `c3` is declared only for the right-hand facet, so
/// under `a == b` it is unreachable — the left side has no `c3` to agree with,
/// and ADR-0057 §D5 reads the missing symbol as the constant false.
#[test]
fn equality_over_asymmetric_domains_forbids_the_value_only_one_side_declares() {
    let model = two_facet_model(
        ("line_container", &["c1", "c2"]),
        ("sorter_container", &["c1", "c2", "c3"]),
        "line_container == sorter_container",
    );
    let dir = emit(&model, "facet_eq_asymmetric_domains");
    let session = load(&dir);

    assert_eq!(
        options(&session, "sorter_container"),
        vec!["c1".to_string(), "c2".to_string()],
        "c3 is not in the left-hand domain, so equality must rule it out"
    );
    assert_eq!(
        options(&session, "line_container"),
        vec!["c1".to_string(), "c2".to_string()]
    );

    // And the surviving values still behave as an agreement.
    let mut session = load(&dir);
    session.apply("sorter_container", "c2").expect("apply");
    assert_eq!(
        options(&session, "line_container"),
        vec!["c2".to_string()]
    );
}

/// Under `a != b`, the value only one side declares stays available: the two
/// facets trivially differ there.
#[test]
fn inequality_over_asymmetric_domains_keeps_the_one_sided_value() {
    let model = two_facet_model(
        ("line_container", &["c1", "c2"]),
        ("sorter_container", &["c1", "c2", "c3"]),
        "line_container != sorter_container",
    );
    let dir = emit(&model, "facet_ne_asymmetric_domains");
    let session = load(&dir);

    assert_eq!(
        options(&session, "sorter_container"),
        vec!["c1".to_string(), "c2".to_string(), "c3".to_string()]
    );
}

/// T9. Two emissions of the same model produce byte-identical artifacts. The
/// expansion runs inside the emitter, so a fold order that depended on hash
/// iteration rather than the declared value order would show up here.
#[test]
fn facet_equality_lowering_is_byte_stable_across_emissions() {
    let values = ["c1", "c2", "c3"];
    let model = two_facet_model(
        ("line_container", &values),
        ("sorter_container", &values),
        "line_container == sorter_container",
    );
    let first = emit(&model, "facet_eq_byte_stable_a");
    let second = emit(&model, "facet_eq_byte_stable_b");

    for name in ["ccm.manifest.json", "ccm.symbols.json"] {
        let a = fs::read(first.join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"));
        let b = fs::read(second.join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"));
        assert_eq!(a, b, "{name} must be byte-identical across emissions");
    }

    let a = read_partition_bdds(&first);
    let b = read_partition_bdds(&second);
    assert!(!a.is_empty(), "expected at least one emitted BDD partition");
    assert_eq!(a, b, "ccm.bdd.bin bytes must be identical across emissions");
}

/// Collect every emitted `*.bdd.bin` under `dir`, keyed by path relative to
/// `dir`, so the comparison does not depend on the partition layout.
fn read_partition_bdds(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    collect_bdds(dir, dir, &mut out);
    out
}

fn collect_bdds(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_bdds(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "bin") {
            let key = path
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .into_owned();
            out.insert(key, fs::read(&path).expect("read bdd"));
        }
    }
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-secb2", test_name)
}
