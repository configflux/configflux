// SPDX-License-Identifier: BUSL-1.1
//
// The `derive` / `accepts` lowering, measured against the real compiled BDD —
// configflux-secb.5, ADR-0057 §D4.
//
// The claim is about MEANING, so it is asserted through the solver rather than
// against the emitted text: a lowering that produced a plausible-looking
// conjunct which the BDD then read differently would pass a string test and
// still ship a model that offers the wrong entries.
//
// The conjuncts here are not hand-copied. They come out of
// `compiler::lowering::lowered_root_conjuncts` over an authored binding and two
// authored components, so what is compiled below is exactly what a real pack
// compiles — the same function `compiler_core::collect_ccm_constraints` calls.
//
// Two properties, both from §D4's test list:
//
//   * BACKEND EQUIVALENCE. The same model emitted through the in-crate BDD
//     builder and through CUDD must answer `valid_options` identically for
//     every facet. The two constructions share nothing but the parsed
//     expression tree, so agreement is evidence the lowering means one thing.
//   * BYTE STABILITY. Emitting the same model twice must produce identical
//     `ccm.bdd.bin` bytes, per construction path. `ccm_hash` is a content
//     identity (ADR-0056), so a lowering whose fold order depended on a hash
//     seed would break every downstream cache and every committed golden.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use compiler::ccm_emitter::{emit_ccm_dir_with_construction, ConditionModel};
use compiler::lowering::lowered_root_conjuncts;
use compiler::schema::{Binding, Component, Requirement};
use solver::{OxiddBackend, Session};

/// The declared facets of the `s_requires_accepts` fixture, in the shape
/// `compiler_core` hands the emitter: the binding projected onto the closed
/// facet it is (ADR-0057 §D3), plus the two authored facets.
const FACETS: [(&str, &[&str]); 3] = [
    ("line_container", &["c1", "c2", "c3"]),
    ("mode", &["x", "y"]),
    ("site", &["factory_a", "factory_b", "factory_c"]),
];

fn binding_with_derive(pairs: &[(&str, &str)]) -> Binding {
    let inner: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut table = BTreeMap::new();
    table.insert("site".to_string(), inner);
    Binding {
        catalogue: "containers".to_string(),
        default: None,
        derive: Some(table),
        doc: None,
    }
}

fn component(condition: Option<&str>, accepts: &[&str]) -> Component {
    let mut requires = BTreeMap::new();
    requires.insert(
        "container".to_string(),
        Requirement {
            binding: "line_container".to_string(),
            accepts: Some(accepts.iter().map(|e| e.to_string()).collect()),
        },
    );
    Component {
        r#type: Some("service".to_string()),
        condition: condition.map(str::to_string),
        depends_on: Vec::new(),
        requires,
        params: Default::default(),
    }
}

/// The `s_requires_accepts` model, built through the production lowering.
fn fixture_model() -> ConditionModel {
    let mut bindings = BTreeMap::new();
    bindings.insert(
        "line_container".to_string(),
        binding_with_derive(&[("factory_a", "c1"), ("factory_b", "c2")]),
    );

    let mut components = BTreeMap::new();
    components.insert(
        "compute_service".to_string(),
        component(None, &["c1", "c2"]),
    );
    components.insert(
        "edge_service".to_string(),
        component(Some("mode == 'x'"), &["c1"]),
    );

    // The symbol-introduction and cardinality channels, exactly as
    // `compiler_core` synthesizes them for a declared closed facet
    // (ADR-0047 §4 Amendment 1, ADR-0054 §5.2).
    let mut clauses: Vec<String> = Vec::new();
    let mut cardinality: Vec<String> = Vec::new();
    let mut facet_domains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, values) in FACETS {
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
        bound_model_hash: "55".repeat(32),
        clauses,
        constraints: lowered_root_conjuncts(&bindings, &components)
            .into_iter()
            .map(|lowered| (lowered.id, lowered.condition))
            .collect(),
        cardinality,
        facet_domains,
    }
}

fn emit(model: &ConditionModel, label: &str, construction: &str) -> PathBuf {
    let dir = tempdir_for(label);
    emit_ccm_dir_with_construction(model, &dir, "facet-name-ascending", construction)
        .unwrap_or_else(|e| panic!("emit via {construction} must succeed: {e}"));
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

/// The entry no requirement accepts is unreachable, and the two the
/// unconditional requirement accepts are not.
#[test]
fn an_entry_outside_every_accepts_list_is_unreachable() {
    let dir = emit(&fixture_model(), "requires_accepts_prune", "in-crate");
    assert_eq!(
        options(&load(&dir), "line_container"),
        vec!["c1".to_string(), "c2".to_string()],
        "c3 is accepted by no requirement, so the model must not offer it"
    );
}

/// The `derive` conjunct is an implication: naming the source value forces the
/// entry, and a source value the table does not cover forces nothing.
#[test]
fn a_derive_pair_forces_its_entry_and_a_partial_table_forces_nothing() {
    let dir = emit(&fixture_model(), "requires_accepts_derive", "in-crate");

    for (site, entry) in [("factory_a", "c1"), ("factory_b", "c2")] {
        let mut session = load(&dir);
        session
            .apply("site", site)
            .unwrap_or_else(|e| panic!("apply(site={site}): {e}"));
        assert_eq!(
            options(&session, "line_container"),
            vec![entry.to_string()],
            "site={site} must force line_container={entry}"
        );
    }

    // factory_c is deliberately absent from the derive table.
    let mut session = load(&dir);
    session.apply("site", "factory_c").expect("apply");
    assert_eq!(
        options(&session, "line_container"),
        vec!["c1".to_string(), "c2".to_string()],
        "an uncovered source value must imply nothing about the binding"
    );
}

/// ADR-0054 §3, the guard: a component that is not included asserts nothing.
/// `edge_service` accepts only c1, and must narrow the binding only where its
/// own condition holds.
#[test]
fn a_conditional_requirement_narrows_only_where_its_component_is_included() {
    let dir = emit(&fixture_model(), "requires_accepts_guard", "in-crate");

    let mut included = load(&dir);
    included.apply("mode", "x").expect("apply");
    assert_eq!(
        options(&included, "line_container"),
        vec!["c1".to_string()],
        "with the component included its accepts list must narrow the binding"
    );

    let mut excluded = load(&dir);
    excluded.apply("mode", "y").expect("apply");
    assert_eq!(
        options(&excluded, "line_container"),
        vec!["c1".to_string(), "c2".to_string()],
        "with the component excluded its accepts list must assert nothing"
    );
}

/// Backend equivalence: the in-crate builder and CUDD must agree on every
/// facet, unbound and after a choice.
#[test]
fn the_two_constructions_agree_on_every_facet() {
    let model = fixture_model();
    let in_crate = emit(&model, "requires_accepts_eq_in_crate", "in-crate");
    let cudd = emit(&model, "requires_accepts_eq_cudd", "cudd");

    for (facet, _) in FACETS {
        assert_eq!(
            options(&load(&in_crate), facet),
            options(&load(&cudd), facet),
            "the two constructions disagree on the unbound options of '{facet}'"
        );
    }

    for site in ["factory_a", "factory_b", "factory_c"] {
        let mut a = load(&in_crate);
        let mut b = load(&cudd);
        a.apply("site", site).expect("apply");
        b.apply("site", site).expect("apply");
        assert_eq!(
            options(&a, "line_container"),
            options(&b, "line_container"),
            "the two constructions disagree after site={site}"
        );
    }
}

/// Byte stability, per construction path: the same model twice, the same bytes.
#[test]
fn emitting_the_same_model_twice_is_byte_identical() {
    let model = fixture_model();
    for construction in ["in-crate", "cudd"] {
        let label = construction.replace('-', "_");
        let first = emit(&model, &format!("requires_accepts_bytes_a_{label}"), construction);
        let second = emit(&model, &format!("requires_accepts_bytes_b_{label}"), construction);
        for artifact in ["ccm.symbols.json", "ccm.manifest.json"] {
            assert_eq!(
                fs::read(first.join(artifact)).expect(artifact),
                fs::read(second.join(artifact)).expect(artifact),
                "{artifact} is not byte-stable under {construction}"
            );
        }
        // The BDD lives one level down, under the partition directories, so it
        // is collected by walk rather than by name — the comparison must not
        // depend on the partition layout.
        let a = read_partition_bdds(&first);
        let b = read_partition_bdds(&second);
        assert!(
            !a.is_empty(),
            "expected at least one emitted BDD partition under {construction}"
        );
        assert_eq!(a, b, "the BDD bytes are not stable under {construction}");
    }
}

/// Every emitted `*.bin` under `dir`, keyed by path relative to `dir`.
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
    temp_dirs::unique_temp_dir("configflux-secb5", test_name)
}
