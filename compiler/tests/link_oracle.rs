// SPDX-License-Identifier: BUSL-1.1

//! The byte-identity oracle for separate compilation (ADR-0058 §D8,
//! configflux-p0jz.2 acceptance A1/T1).
//!
//! For every scenario pack and every example in the repository, the package a
//! one-shot `compile` writes and the package `compile-object` per unit followed
//! by `link` writes are compared file by file, byte for byte — the chunk files,
//! the index, the manifest, the `.ccm` directory and the provenance sidecars.
//! That is the whole claim of §D8: there is one code path, so the two forms
//! cannot disagree.
//!
//! Black box: everything goes through the public product API
//! (`compile_model`, `compile_object`, `link_model`), never through a crate
//! internal, so the test would still hold if the stages were rewritten
//! underneath it.
//!
//! **What this test does NOT pin, and where that lives.** Byte identity against
//! the COMMITTED goldens — the pinned `model_hash` / `resolve_hash` /
//! `bom_hash` of every scenario variant (T8) — is
//! `//compiler:scenario_byte_stability_test`, and the `.ccm` layout contract is
//! `//compiler:ccm_emitter_roundtrip_test` and
//! `//compiler:ccm_emitter_equivalence_test`. This test asserts the two forms
//! agree; those assert that what they agree on is still what shipped.

use compiler::object_compile::{compile_object, CompileObjectRequest};
use compiler::product_api::{
    compile_model, link_model, CompileModelRequest, LinkModelRequest, OperationStatus,
    SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[path = "temp_dirs.rs"]
mod temp_dirs;

/// One pack of the corpus: the `--source` chunks a one-shot compile is given,
/// under the ids it would give them.
struct Pack {
    label: &'static str,
    chunks: &'static [(&'static str, &'static str)],
}

/// A scenario pack's two chunks, under the paths the scenario tests use.
macro_rules! scenario {
    ($root:literal) => {
        &[
            (
                concat!("scenarios/", $root, "/cue/00_definitions.json"),
                include_str!(concat!("../scenarios/", $root, "/cue/00_definitions.json")),
            ),
            (
                concat!("scenarios/", $root, "/cue/10_components.json"),
                include_str!(concat!("../scenarios/", $root, "/cue/10_components.json")),
            ),
        ]
    };
}

/// One chunk of a flat pack, named by its file.
macro_rules! chunk {
    ($dir:literal, $file:literal) => {
        (
            concat!($dir, "/", $file),
            include_str!(concat!("../", $dir, "/", $file)),
        )
    };
}

/// One chunk of an example, named by its file.
macro_rules! example {
    ($dir:literal, $file:literal) => {
        (
            concat!("examples/", $dir, "/", $file),
            include_str!(concat!("../../examples/", $dir, "/", $file)),
        )
    };
}

/// Every pack the oracle covers. Adding a scenario or an example means adding a
/// row here; the corpus-coverage guard below fails until it is.
const CORPUS: &[Pack] = &[
    Pack { label: "s1-smoke", chunks: scenario!("s1_water_pump/smoke") },
    Pack { label: "s1-medium", chunks: scenario!("s1_water_pump/medium") },
    Pack { label: "s2-smoke", chunks: scenario!("s2_wind_turbine/smoke") },
    Pack { label: "s2-medium", chunks: scenario!("s2_wind_turbine/medium") },
    Pack { label: "s3-smoke", chunks: scenario!("s3_automation_cell/smoke") },
    Pack { label: "s3-medium", chunks: scenario!("s3_automation_cell/medium") },
    Pack { label: "s3-large", chunks: scenario!("s3_automation_cell/large") },
    Pack { label: "s4-smoke", chunks: scenario!("s4_mobile_robot/smoke") },
    Pack { label: "s4-medium", chunks: scenario!("s4_mobile_robot/medium") },
    Pack { label: "s4-large", chunks: scenario!("s4_mobile_robot/large") },
    Pack { label: "s5-smoke", chunks: scenario!("s5_building_hvac/smoke") },
    Pack {
        label: "s_catalogue_binding",
        chunks: &[
            chunk!("scenarios/s_catalogue_binding", "00_definitions.json"),
            chunk!("scenarios/s_catalogue_binding", "10_components.json"),
        ],
    },
    // Found by the corpus-coverage guard below the moment it started reading the
    // tree instead of a second hand-written list: this pack has been in the
    // repository, and outside the oracle, since it was written.
    Pack {
        label: "s_catalogue_binding_accepts",
        chunks: &[
            chunk!("scenarios/s_catalogue_binding_accepts", "00_definitions.json"),
            chunk!("scenarios/s_catalogue_binding_accepts", "10_components.json"),
        ],
    },
    Pack {
        label: "s_facet_equality",
        chunks: &[
            chunk!("scenarios/s_facet_equality", "00_definitions.json"),
            chunk!("scenarios/s_facet_equality", "10_components.json"),
        ],
    },
    Pack {
        label: "s_labeled_mus",
        chunks: &[
            chunk!("scenarios/s_labeled_mus", "00_definitions.json"),
            chunk!("scenarios/s_labeled_mus", "10_components.json"),
        ],
    },
    // Three units. The order below is the `--source` order, which is NOT unit
    // name order — the case ADR-0058 §A2's canonical clause order exists for.
    Pack {
        label: "s_requires_accepts",
        chunks: &[
            chunk!("scenarios/s_requires_accepts", "00_catalogue.json"),
            chunk!("scenarios/s_requires_accepts", "10_bindings.json"),
            chunk!("scenarios/s_requires_accepts", "20_components.json"),
        ],
    },
    Pack {
        label: "s_requires_delivery",
        chunks: &[
            chunk!("scenarios/s_requires_delivery", "00_catalogue.json"),
            chunk!("scenarios/s_requires_delivery", "10_bindings.json"),
            chunk!("scenarios/s_requires_delivery", "20_services.json"),
        ],
    },
    Pack {
        label: "example-00",
        chunks: &[
            example!("00-service-multi-env", "00_definitions.json"),
            example!("00-service-multi-env", "10_components.json"),
        ],
    },
    Pack { label: "example-01", chunks: &[example!("01-hello-led", "config.json")] },
    Pack {
        label: "example-02",
        chunks: &[
            example!("02-sensor-gateway", "00_definitions.json"),
            example!("02-sensor-gateway", "10_components.json"),
        ],
    },
    Pack {
        label: "example-03",
        chunks: &[
            example!("03-motor-controller", "00_definitions.json"),
            example!("03-motor-controller", "10_components.json"),
        ],
    },
    Pack {
        label: "example-04",
        chunks: &[
            example!("04-fleet-edge-node", "00_definitions.json"),
            example!("04-fleet-edge-node", "10_components.json"),
        ],
    },
    Pack {
        label: "example-05",
        chunks: &[
            example!("05-compose-fleet", "00_definitions.json"),
            example!("05-compose-fleet", "10_components.json"),
        ],
    },
    // Four units, compiled in `--source` order (site_catalogue, vision_service,
    // compute_service, sorter_service) which is again not name order.
    Pack { label: "example-06", chunks: EXAMPLE_06 },
];

const EXAMPLE_06: &[(&str, &str)] = &[
    example!("06-catalogue-polyrepo", "repos/catalogue/00_catalogue.json"),
    example!("06-catalogue-polyrepo", "repos/vision/10_vision.json"),
    example!("06-catalogue-polyrepo", "repos/compute/10_compute.json"),
    example!("06-catalogue-polyrepo", "repos/sorter/20_sorter.json"),
];

// ----------------------------------------------------------------------------
// T1 — the oracle
// ----------------------------------------------------------------------------

#[test]
fn t1_compile_and_object_plus_link_write_the_same_package_for_every_pack() {
    for pack in CORPUS {
        let dir = temp_dirs::unique_temp_dir("link-oracle", pack.label);
        let compiled = compile_pack(pack, &dir.join("compile"), None);
        let linked = link_pack(pack, &dir, None);
        assert_eq!(
            compiled.model_hash, linked.model_hash,
            "{}: model_hash differs between compile and link",
            pack.label
        );
        assert_trees_identical(&dir.join("compile"), &dir.join("link"), pack.label);
    }
}

// ----------------------------------------------------------------------------
// T7 / T10 — order and repetition cannot reach a byte
// ----------------------------------------------------------------------------

#[test]
fn t7_linking_twice_and_in_any_object_order_writes_the_same_bytes() {
    let pack = Pack { label: "example-06", chunks: EXAMPLE_06 };
    let dir = temp_dirs::unique_temp_dir("link-oracle", "order");
    let objects = build_objects(&pack, &dir);

    let forward = objects.clone();
    let mut reversed = objects.clone();
    reversed.reverse();
    // A third order that is neither the argument order nor its reverse.
    let mut rotated = objects.clone();
    rotated.rotate_left(2);

    let a = run_link(&forward, &dir.join("a"), None);
    let b = run_link(&forward, &dir.join("b"), None);
    let c = run_link(&reversed, &dir.join("c"), None);
    let d = run_link(&rotated, &dir.join("d"), None);
    assert_eq!(a.model_hash, b.model_hash);
    assert_eq!(a.model_hash, c.model_hash);
    assert_eq!(a.model_hash, d.model_hash);
    assert_trees_identical(&dir.join("a"), &dir.join("b"), "link twice");
    assert_trees_identical(&dir.join("a"), &dir.join("c"), "objects reversed");
    assert_trees_identical(&dir.join("a"), &dir.join("d"), "objects rotated");
}

#[test]
fn t10_the_four_unit_example_links_to_what_compile_writes_in_either_order() {
    // §A2's motivating case: `--source` order (catalogue, vision, compute,
    // sorter) is not unit-name order, so a package that came out identical by
    // accident of insertion order would not stay identical here.
    let dir = temp_dirs::unique_temp_dir("link-oracle", "example-06");
    let pack = Pack { label: "example-06", chunks: EXAMPLE_06 };
    compile_pack(&pack, &dir.join("compile"), None);

    let objects = build_objects(&pack, &dir);
    let mut by_name = objects.clone();
    by_name.sort();
    run_link(&by_name, &dir.join("name-order"), None);
    run_link(&objects, &dir.join("source-order"), None);

    assert_trees_identical(&dir.join("compile"), &dir.join("name-order"), "name order");
    assert_trees_identical(&dir.join("compile"), &dir.join("source-order"), "source order");
}

// ----------------------------------------------------------------------------
// T8 — the in-memory compile path is a function of the model alone
// ----------------------------------------------------------------------------

#[test]
fn t8_two_compiles_of_one_model_write_the_same_package() {
    // §D8 re-expressed `compile` as "group into units, then link". This pins
    // that the rewrite did not make the output depend on anything but the
    // model — the goldens themselves are pinned by
    // //compiler:scenario_byte_stability_test.
    let dir = temp_dirs::unique_temp_dir("link-oracle", "determinism");
    let pack = pack_named("s_requires_accepts");
    let first = compile_pack(pack, &dir.join("first"), None);
    let second = compile_pack(pack, &dir.join("second"), None);
    assert_eq!(first.model_hash, second.model_hash);
    assert_trees_identical(&dir.join("first"), &dir.join("second"), "compile twice");
}

// ----------------------------------------------------------------------------
// T9 — the partitioning flags mean the same thing on both paths
// ----------------------------------------------------------------------------

#[test]
fn t9_a_cluster_size_partitions_a_linked_package_exactly_as_it_partitions_a_compiled_one() {
    let dir = temp_dirs::unique_temp_dir("link-oracle", "cluster");
    let pack = Pack { label: "s3-smoke", chunks: scenario!("s3_automation_cell/smoke") };
    compile_pack(&pack, &dir.join("compile"), Some(3));
    link_pack(&pack, &dir, Some(3));
    assert_trees_identical(&dir.join("compile"), &dir.join("link"), "cluster-size 3");
}

// ----------------------------------------------------------------------------
// Corpus coverage
// ----------------------------------------------------------------------------

#[test]
fn every_pack_chunk_in_the_repository_is_in_the_corpus() {
    // The oracle is only as strong as its coverage, and a pack added without a
    // row above would be silently unchecked. So the expected set is READ OFF
    // THE TREE rather than listed: a second hand-written list would agree with
    // the first by construction, and a pack in neither would pass unnoticed —
    // which is the hole this guard exists to close.
    let covered: BTreeSet<String> = CORPUS
        .iter()
        .flat_map(|pack| pack.chunks.iter().map(|(source_id, _)| (*source_id).to_string()))
        .collect();
    let on_disk = pack_chunks_on_disk();
    let uncovered: Vec<&String> = on_disk.difference(&covered).collect();
    assert!(
        uncovered.is_empty(),
        "the repository holds pack chunks that no CORPUS row names, so the oracle never \
         runs over them: {uncovered:#?}"
    );
    // The other direction. `include_str!` already fails to compile if a row
    // names a file that moved, so this only bites on a row whose declared
    // source id and embedded path disagree — a row that would run the oracle
    // over a model nobody can find in the tree.
    let phantom: Vec<&String> = covered.difference(&on_disk).collect();
    assert!(
        phantom.is_empty(),
        "CORPUS rows name chunks the repository does not hold: {phantom:#?}"
    );
}

/// Every model chunk the repository holds under `compiler/scenarios` and
/// `examples`, under the source id a `CORPUS` row gives it.
///
/// The two roots reach this test through the `data` globs on
/// `//compiler:link_oracle_test`, so a pack directory added to either one turns
/// up here with no Bazel edit and no list edit.
fn pack_chunks_on_disk() -> BTreeSet<String> {
    let root = workspace_root();
    let mut found = BTreeSet::new();
    // `compiler/scenarios/...` is `scenarios/...` in a source id: a scenario
    // pack's chunks are named relative to the compiler crate.
    collect_json(&root.join("compiler").join("scenarios"), "scenarios", &mut found);
    collect_json(&root.join("examples"), "examples", &mut found);
    found.retain(|id| is_model_chunk(id));
    found
}

/// Whether a `*.json` under one of those roots is a model chunk a pack is
/// compiled from, rather than a pinned output or an input to some other verb.
///
/// Stated as exclusions, deliberately. A positive rule would have to name the
/// directory layouts packs happen to use today, and a pack laid out some new
/// way would then be skipped in silence — the same failure the guard above
/// exists to prevent. Anything not excluded here is a chunk, so a new shape
/// fails loudly and its author either adds the row or adds an exclusion with a
/// reason.
fn is_model_chunk(id: &str) -> bool {
    let segments: Vec<&str> = id.split('/').collect();
    // `golden/` (scenarios) and `expected/` (examples) hold pinned outputs:
    // what a model produces, never what it is compiled from.
    if segments.iter().any(|s| *s == "golden" || *s == "expected") {
        return false;
    }
    // `out/` is where every example's run.sh writes, and it holds staged copies
    // of the very chunks the example is compiled from, under `out/src/`. None of
    // it is committed, so no CORPUS row could ever name it, and without this a
    // developer who ran an example by hand — which the READMEs teach — would
    // fail this guard on their own working copy while CI stayed green: the
    // Bazel-driven example tests redirect output to $TEST_TMPDIR, so the tree
    // the runner sees is clean. //examples:example_model_chunks excludes
    // `*/out/**` for that reason and is the first line of defence. This is the
    // second, and it is not redundant: the guard also walks compiler/scenarios,
    // which reaches the runfiles through a glob carrying no such exclusion, so
    // the guard's own correctness must not rest on every producing glob
    // remembering. Only a segment named exactly `out` is skipped, so a genuine
    // new pack added anywhere else still fails until it has a row.
    if segments.iter().any(|s| *s == "out") {
        return false;
    }
    // An environment map names deployment environments for `cfx resolve`. It
    // declares no package and is a `--source` of nothing.
    segments.last() != Some(&"environments.json")
}

/// Every `*.json` under `dir`, keyed by its `/`-joined path below `prefix`.
fn collect_json(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("readable directory entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = format!("{prefix}/{name}");
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, &id, found);
        } else if name.ends_with(".json") {
            found.insert(id);
        }
    }
}

/// The workspace root inside this test's runfiles tree.
///
/// Bazel runs a test from its runfiles directory and also exports
/// `TEST_SRCDIR` / `TEST_WORKSPACE` naming that same place, so the first
/// candidate that actually holds `compiler/scenarios` wins. The panic lists
/// what was tried, because "no packs found" would otherwise read as a pass.
fn workspace_root() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(srcdir) = std::env::var("TEST_SRCDIR") {
        if let Ok(workspace) = std::env::var("TEST_WORKSPACE") {
            candidates.push(PathBuf::from(&srcdir).join(workspace));
        }
        // The main repository's canonical name under bzlmod, for a runner that
        // exports `TEST_SRCDIR` without `TEST_WORKSPACE`.
        candidates.push(PathBuf::from(&srcdir).join("_main"));
    }
    candidates.push(PathBuf::from("."));
    candidates
        .iter()
        .find(|candidate| candidate.join("compiler").join("scenarios").is_dir())
        .unwrap_or_else(|| {
            panic!(
                "no workspace root in the runfiles tree holds compiler/scenarios; tried \
                 {candidates:?}. The scenario and example chunks reach this test through \
                 the `data` globs on //compiler:link_oracle_test"
            )
        })
        .clone()
}

/// The corpus row with this label.
///
/// By name and never by index: a row inserted above shifts every index below
/// it, and a test carrying its label separately from the index it read would
/// then run over the wrong pack while still reporting the right name.
fn pack_named(label: &str) -> &'static Pack {
    CORPUS
        .iter()
        .find(|pack| pack.label == label)
        .unwrap_or_else(|| panic!("no corpus pack labelled '{label}'"))
}

// ----------------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------------

fn compile_pack(
    pack: &Pack,
    out: &Path,
    cluster_size: Option<usize>,
) -> compiler::product_api::CompileResult {
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: pack
            .chunks
            .iter()
            .map(|(source_id, content)| SourceManifestEntry {
                source_id: (*source_id).to_string(),
                inline_content: (*content).to_string(),
            })
            .collect(),
        output_dir: Some(out.to_string_lossy().into_owned()),
        cluster_size,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "{}: compile failed: {:?}",
        pack.label,
        result.verify_report.diagnostics.diagnostics
    );
    result
}

/// One object directory per unit, unit-name ascending.
///
/// No `--interface` is passed: an object records an unresolved reference as an
/// import and the linker resolves it (ADR-0058 §D3), so the oracle holds
/// without wiring the dependency order — and asserting that is part of the
/// point.
fn build_objects(pack: &Pack, dir: &Path) -> Vec<String> {
    let mut by_unit: BTreeMap<String, Vec<SourceManifestEntry>> = BTreeMap::new();
    for (source_id, content) in pack.chunks {
        let unit = package_of(content);
        by_unit.entry(unit).or_default().push(SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        });
    }
    by_unit
        .into_iter()
        .map(|(unit, sources)| {
            let out = dir.join(format!("{unit}.cfo")).to_string_lossy().into_owned();
            compile_object(CompileObjectRequest {
                sources,
                interfaces: Vec::new(),
                output_dir: out.clone(),
                stamp_time: false,
            })
            .unwrap_or_else(|diagnostic| {
                panic!("{}: compile-object failed: {diagnostic:?}", pack.label)
            });
            out
        })
        .collect()
}

fn link_pack(
    pack: &Pack,
    dir: &Path,
    cluster_size: Option<usize>,
) -> compiler::product_api::CompileResult {
    let objects = build_objects(pack, dir);
    let result = run_link(&objects, &dir.join("link"), cluster_size);
    assert_eq!(
        result.objects.len(),
        objects.len(),
        "{}: link must report every object it was given",
        pack.label
    );
    result
}

fn run_link(
    object_dirs: &[String],
    out: &Path,
    cluster_size: Option<usize>,
) -> compiler::product_api::CompileResult {
    let result = link_model(LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: object_dirs.to_vec(),
        output_dir: out.to_string_lossy().into_owned(),
        cluster_size,
        budget: None,
        stamp_time: false,
        lock_path: None,
        lock_allow_extra: false,
        write_lock_path: None,
        lock_sources: Default::default(),
        force_lock: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "link failed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
    result
}

/// The `package` value a chunk declares — the unit it belongs to.
fn package_of(content: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(content).expect("chunk parses");
    value["package"]
        .as_str()
        .expect("chunk declares a package")
        .to_string()
}

fn assert_trees_identical(left: &Path, right: &Path, label: &str) {
    let a = tree(left);
    let b = tree(right);
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "{label}: the two packages hold different files"
    );
    for (name, left_bytes) in &a {
        let right_bytes = &b[name];
        assert_eq!(
            left_bytes,
            right_bytes,
            "{label}: '{name}' differs ({} vs {} bytes)",
            left_bytes.len(),
            right_bytes.len()
        );
    }
}

/// Every file under `root`, keyed by its path relative to `root`.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read '{}': {err}", dir.display()));
        for entry in entries {
            let path: PathBuf = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .into_owned();
            out.insert(relative, std::fs::read(&path).expect("read file"));
        }
    }
    out
}
