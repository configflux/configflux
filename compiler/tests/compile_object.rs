// SPDX-License-Identifier: BUSL-1.1

//! configflux-p0jz.1 / ADR-0058 §D1-§D3: `compile-object` turns ONE unit into a
//! content-addressed object directory — the unit's chunk IR files exactly as
//! the package stores them, plus a header that merges the unit's interface and
//! records the hashes of the interface objects it was compiled against.
//!
//! Black box: every assertion is driven through the public
//! `compiler::object_compile::compile_object` entry point and the files it
//! writes, never through the merge internals. The fixture is the SHIPPED
//! `examples/06-catalogue-polyrepo` — four units, of which one is a pure
//! interface pack — because what is under test is that a real polyrepo model
//! splits along its unit boundaries, and an invented fixture would only test
//! the fixture.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use compiler::object::{ObjectHeader, OBJECT_HEADER_FILENAME};
use compiler::object_compile::{compile_object, CompileObjectRequest};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};

#[path = "temp_dirs.rs"]
mod temp_dirs;

/// The catalogue unit: definition `container_dim_mm`, facet `site`, catalogue
/// `containers`, bindings `line_container` and `sorter_container`. Declares no
/// component — the "header file of a pack" an ADR-0058 §D2 interface object is.
const CATALOGUE_SRC: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json");
/// A service unit: one component requiring `line_container`, one parameter
/// inheriting `container_dim_mm`, one override on `site`. Everything it names
/// lives in another unit.
const VISION_SRC: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/vision/10_vision.json");
/// A service unit whose requirement carries an `accepts` list — the input T5
/// perturbs.
const COMPUTE_SRC: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/compute/10_compute.json");

const CATALOGUE_ID: &str = "examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json";
const VISION_ID: &str = "examples/06-catalogue-polyrepo/repos/vision/10_vision.json";
const COMPUTE_ID: &str = "examples/06-catalogue-polyrepo/repos/compute/10_compute.json";

fn tempdir(label: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compile-object", label)
}

fn source(source_id: &str, content: &str) -> SourceManifestEntry {
    SourceManifestEntry {
        source_id: source_id.to_string(),
        inline_content: content.to_string(),
    }
}

/// Compile one unit into `<base>/<label>.cfo` and return its header.
fn build_object(
    base: &Path,
    label: &str,
    sources: Vec<SourceManifestEntry>,
    interfaces: Vec<ObjectHeader>,
) -> Result<(ObjectHeader, PathBuf), compiler::product_api::Diagnostic> {
    let out = base.join(format!("{label}.cfo"));
    let header = compile_object(CompileObjectRequest {
        sources,
        interfaces,
        output_dir: out.to_string_lossy().into_owned(),
        stamp_time: false,
    })?;
    Ok((header, out))
}

fn ids(values: &BTreeSet<String>) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

fn chunk_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read object dir")
        .map(|entry| entry.expect("dir entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("chunk-") && name.ends_with(".cfir"))
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// T1 — exports, imports, and the interface record
// ---------------------------------------------------------------------------

#[test]
fn t1_interface_unit_exports_its_declarations_and_imports_nothing() {
    let base = tempdir("t1-catalogue");
    let (header, dir) = build_object(
        &base,
        "site_catalogue",
        vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
        Vec::new(),
    )
    .expect("the catalogue unit compiles alone");

    assert_eq!(header.unit, "site_catalogue");
    assert_eq!(ids(&header.exports.definitions), ["container_dim_mm"]);
    assert_eq!(ids(&header.exports.facets), ["site"]);
    assert_eq!(ids(&header.exports.catalogues), ["containers"]);
    assert_eq!(
        ids(&header.exports.bindings),
        ["line_container", "sorter_container"]
    );
    assert!(ids(&header.exports.components).is_empty());
    assert!(ids(&header.exports.artifacts).is_empty());
    assert!(ids(&header.exports.constraints).is_empty());

    // Everything this unit names, it declares. `site` is named by
    // `line_container`'s derive table and `containers` by both bindings, so an
    // imports map that did not subtract the unit's own exports would list them.
    assert!(
        ids(&header.imports.components).is_empty()
            && ids(&header.imports.definitions).is_empty()
            && ids(&header.imports.facets).is_empty()
            && ids(&header.imports.bindings).is_empty()
            && ids(&header.imports.catalogues).is_empty(),
        "a self-contained unit imports nothing, got {:?}",
        header.imports
    );
    assert!(header.interfaces.is_empty());
    assert_eq!(header.chunk_hashes.len(), 1);
    assert!(dir.join(OBJECT_HEADER_FILENAME).is_file());
}

#[test]
fn t1_service_unit_records_the_same_imports_with_and_without_the_interface() {
    let base = tempdir("t1-vision");
    let (catalogue, _) = build_object(
        &base,
        "site_catalogue",
        vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
        Vec::new(),
    )
    .expect("the catalogue unit compiles alone");

    let (with_interface, _) = build_object(
        &base,
        "vision_with",
        vec![source(VISION_ID, VISION_SRC)],
        vec![catalogue.clone()],
    )
    .expect("vision compiles against the catalogue interface");

    assert_eq!(with_interface.unit, "vision_service");
    assert_eq!(ids(&with_interface.exports.components), ["vision_service"]);
    assert_eq!(
        ids(&with_interface.imports.definitions),
        ["container_dim_mm"]
    );
    assert_eq!(ids(&with_interface.imports.bindings), ["line_container"]);
    assert_eq!(ids(&with_interface.imports.facets), ["site"]);
    assert_eq!(
        with_interface.interfaces,
        vec![catalogue.as_interface_ref()],
        "the interface it was compiled against is recorded by unit and hash"
    );

    // Without the interface the unit still compiles: an unresolved reference is
    // a LINK obligation, not an object-time error (ADR-0058 §D3).
    let (alone, _) = build_object(
        &base,
        "vision_alone",
        vec![source(VISION_ID, VISION_SRC)],
        Vec::new(),
    )
    .expect("vision compiles with no interface at all");

    assert_eq!(alone.imports, with_interface.imports);
    assert!(alone.interfaces.is_empty());
}

// ---------------------------------------------------------------------------
// T2 — unit agreement
// ---------------------------------------------------------------------------

#[test]
fn t2_chunks_from_two_units_are_refused_naming_both() {
    let base = tempdir("t2-mismatch");
    let error = build_object(
        &base,
        "mixed",
        vec![
            source(CATALOGUE_ID, CATALOGUE_SRC),
            source(VISION_ID, VISION_SRC),
        ],
        Vec::new(),
    )
    .expect_err("two units in one object is refused");

    assert_eq!(error.code, "E_OBJECT_UNIT_MISMATCH");
    for needle in [
        "site_catalogue",
        "vision_service",
        CATALOGUE_ID,
        VISION_ID,
    ] {
        assert!(
            error.message.contains(needle),
            "message must name both units and both files, missing {needle}: {}",
            error.message
        );
    }
    assert!(
        !base.join("mixed.cfo").join(OBJECT_HEADER_FILENAME).exists(),
        "a refused compile writes no header"
    );
}

// ---------------------------------------------------------------------------
// T3 — path invariance
// ---------------------------------------------------------------------------

#[test]
fn t3_object_hash_survives_a_copied_path_and_a_reversed_source_order() {
    let base = tempdir("t3-invariance");

    // The catalogue unit is one chunk, so `--source` order needs a second
    // chunk to permute: the vision component joins it under the SAME package,
    // which is what makes them one unit.
    let vision_as_catalogue = VISION_SRC.replace(
        "\"package\": \"vision_service\"",
        "\"package\": \"site_catalogue\"",
    );
    assert!(
        vision_as_catalogue.contains("\"package\": \"site_catalogue\""),
        "the fixture's package field must be rewritable"
    );

    let (forward, forward_dir) = build_object(
        &base,
        "forward",
        vec![
            source("repo/a/00_catalogue.json", CATALOGUE_SRC),
            source("repo/a/10_vision.json", &vision_as_catalogue),
        ],
        Vec::new(),
    )
    .expect("two chunks of one unit compile");

    // A copy at another path, with the two sources given in the other order.
    let (moved, moved_dir) = build_object(
        &base,
        "moved",
        vec![
            source("/elsewhere/checkout/b/10_vision.json", &vision_as_catalogue),
            source("/elsewhere/checkout/b/00_catalogue.json", CATALOGUE_SRC),
        ],
        Vec::new(),
    )
    .expect("the same unit at another path compiles");

    assert_eq!(
        forward.object_hash, moved.object_hash,
        "object_hash must not depend on path spelling or --source order"
    );
    assert_eq!(forward.chunk_hashes, moved.chunk_hashes);
    assert_eq!(
        chunk_files(&forward_dir),
        chunk_files(&moved_dir),
        "the chunk files are named by content, so both objects hold the same set"
    );
    assert_eq!(
        std::fs::read(forward_dir.join(OBJECT_HEADER_FILENAME)).expect("read header"),
        std::fs::read(moved_dir.join(OBJECT_HEADER_FILENAME)).expect("read header"),
        "the header carries no path, so its bytes are identical too"
    );

    // The chunk BODIES differ in exactly one place: `source_id`, which
    // ADR-0056 keeps as provenance inside the chunk and out of every identity.
    for name in chunk_files(&forward_dir) {
        let a = std::fs::read_to_string(forward_dir.join(&name)).expect("read chunk");
        let b = std::fs::read_to_string(moved_dir.join(&name)).expect("read chunk");
        assert_ne!(a, b, "the two chunks were written from different paths");
        assert!(a.contains("\"source_id\":\"repo/a/"));
        assert!(b.contains("\"source_id\":\"/elsewhere/checkout/b/"));
    }
}

// ---------------------------------------------------------------------------
// T4 — an undeclared facet keeps its legacy inferred domain
// ---------------------------------------------------------------------------

#[test]
fn t4_condition_on_an_undeclared_facet_is_recorded_not_rejected() {
    let base = tempdir("t4-inferred");
    // `region` is declared by no chunk of this unit and by no interface. The
    // legacy condition-inferred domain applies (ADR-0047 §3): the compile
    // succeeds and the facet is recorded as an import.
    let with_unknown_facet = VISION_SRC.replace("site == 'factory_b'", "region == 'apac'");
    assert!(with_unknown_facet.contains("region == 'apac'"));

    let (header, _) = build_object(
        &base,
        "vision_region",
        vec![source(VISION_ID, &with_unknown_facet)],
        Vec::new(),
    )
    .expect("an undeclared facet in a condition is not an object-time error");

    assert!(
        header.imports.facets.contains("region"),
        "the facet is recorded as an import, got {:?}",
        header.imports.facets
    );
    assert!(
        !header.exports.facets.contains("region"),
        "an inferred facet is not a declaration"
    );
    assert!(header.facet_domains.is_empty());
}

// ---------------------------------------------------------------------------
// T5 — accepts is checked against the interface's catalogue
// ---------------------------------------------------------------------------

#[test]
fn t5_accepts_entry_outside_the_interfaces_catalogue_is_refused() {
    let base = tempdir("t5-accepts");
    let (catalogue, _) = build_object(
        &base,
        "site_catalogue",
        vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
        Vec::new(),
    )
    .expect("the catalogue unit compiles alone");

    // `c9` is not an entry of `containers`.
    let bad_accepts = COMPUTE_SRC.replace("\"c2\"", "\"c9\"");
    assert!(bad_accepts.contains("\"c9\""));

    let error = build_object(
        &base,
        "compute_bad",
        vec![source(COMPUTE_ID, &bad_accepts)],
        vec![catalogue.clone()],
    )
    .expect_err("an accepts entry outside the catalogue is refused at object time");

    assert_eq!(error.code, "E_REQUIRES_INVALID");
    assert!(
        error.message.contains("compute_service") && error.message.contains("c9"),
        "the message names the component and the offending entry: {}",
        error.message
    );

    // Without the interface the binding is invisible, so the requirement is a
    // link obligation and nothing is checkable here.
    let (alone, _) = build_object(
        &base,
        "compute_alone",
        vec![source(COMPUTE_ID, &bad_accepts)],
        Vec::new(),
    )
    .expect("with no interface the requirement is only recorded");
    assert!(alone.imports.bindings.contains("line_container"));
}

// ---------------------------------------------------------------------------
// T6 — the chunk files are the package's own
// ---------------------------------------------------------------------------

#[test]
fn t6_object_chunk_files_are_byte_identical_to_the_packages() {
    let base = tempdir("t6-chunks");
    let (catalogue, _) = build_object(
        &base,
        "site_catalogue",
        vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
        Vec::new(),
    )
    .expect("the catalogue unit compiles alone");
    let (_, vision_dir) = build_object(
        &base,
        "vision_service",
        vec![source(VISION_ID, VISION_SRC)],
        vec![catalogue],
    )
    .expect("vision compiles against the catalogue interface");

    // The whole model through the ordinary one-shot compile.
    let cmp_dir = base.join("cmp");
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            source(CATALOGUE_ID, CATALOGUE_SRC),
            source(VISION_ID, VISION_SRC),
            source(COMPUTE_ID, COMPUTE_SRC),
        ],
        output_dir: Some(cmp_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "the polyrepo example compiles: {:?}",
        result.verify_report.diagnostics
    );

    let object_chunks = chunk_files(&vision_dir);
    assert_eq!(object_chunks.len(), 1);
    for name in object_chunks {
        let from_object = std::fs::read(vision_dir.join(&name)).expect("read object chunk");
        let from_package = std::fs::read(cmp_dir.join(&name)).expect("read package chunk");
        assert_eq!(
            from_object, from_package,
            "the object stores {name} exactly as the package does"
        );
    }
    assert!(
        !vision_dir.join("ccm").exists() && !vision_dir.join("index.cfir.json").exists(),
        "an object holds no constraint model and no package index"
    );
    assert!(vision_dir.join("provenance.json").is_file());
}

/// The claim T6 makes is only worth anything if the package's own chunk files
/// are a function of the model rather than of the run. They were NOT: a chunk
/// carrying a component rendered `Component::params` — a `HashMap` — in one
/// instance's iteration order, so two compiles of one model wrote different
/// bytes while reporting the same `model_hash` (which is computed over a
/// canonical rendering and never saw it). Found by T6; fixed by writing every
/// chunk file through `ir::chunk_file_bytes`. This pins it from the outside,
/// over a COMPONENT-bearing model, which is the only kind that could regress.
#[test]
fn t6b_the_packages_chunk_files_are_a_function_of_the_model_not_of_the_run() {
    let base = tempdir("t6b-package-stability");
    let sources = || {
        vec![
            source(CATALOGUE_ID, CATALOGUE_SRC),
            source(VISION_ID, VISION_SRC),
            source(COMPUTE_ID, COMPUTE_SRC),
        ]
    };
    let compile_into = |label: &str| {
        let dir = base.join(label);
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: sources(),
            output_dir: Some(dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        assert_eq!(result.status, OperationStatus::Ok);
        dir
    };

    // Two compiles in ONE process: each parse builds fresh `HashMap`s with
    // different hash seeds, so this is exactly the case that used to differ.
    let first = compile_into("cmp_a");
    let second = compile_into("cmp_b");

    let names = chunk_files(&first);
    assert_eq!(names.len(), 3, "one file per source chunk");
    assert_eq!(names, chunk_files(&second));
    for name in names {
        assert_eq!(
            std::fs::read(first.join(&name)).expect("read first"),
            std::fs::read(second.join(&name)).expect("read second"),
            "{name} must be identical across two compiles of one model"
        );
    }
}

// ---------------------------------------------------------------------------
// T7 — determinism
// ---------------------------------------------------------------------------

#[test]
fn t7_two_compiles_of_one_unit_produce_identical_directories() {
    let base = tempdir("t7-determinism");
    let build = |label: &str| {
        build_object(
            &base,
            label,
            vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
            Vec::new(),
        )
        .expect("the catalogue unit compiles alone")
    };
    let (first, first_dir) = build("run_a");
    let (second, second_dir) = build("run_b");

    assert_eq!(first, second);
    let mut names = chunk_files(&first_dir);
    assert_eq!(names, chunk_files(&second_dir));
    names.push(OBJECT_HEADER_FILENAME.to_string());
    names.push("provenance.json".to_string());
    for name in names {
        assert_eq!(
            std::fs::read(first_dir.join(&name)).expect("read first"),
            std::fs::read(second_dir.join(&name)).expect("read second"),
            "{name} must be byte-identical across runs"
        );
    }
}

// ---------------------------------------------------------------------------
// T8 — the header round-trips and its hash is checkable
// ---------------------------------------------------------------------------

#[test]
fn t8_header_round_trips_through_serde_and_recomputes_its_hash() {
    let base = tempdir("t8-roundtrip");
    let (header, dir) = build_object(
        &base,
        "site_catalogue",
        vec![source(CATALOGUE_ID, CATALOGUE_SRC)],
        Vec::new(),
    )
    .expect("the catalogue unit compiles alone");

    let written = std::fs::read(dir.join(OBJECT_HEADER_FILENAME)).expect("read header");
    let parsed: ObjectHeader = serde_json::from_slice(&written).expect("header parses");
    assert_eq!(parsed, header);
    assert_eq!(
        parsed.to_canonical_json().expect("re-render"),
        written,
        "deserialize -> serialize is byte-identical"
    );
    assert_eq!(
        parsed.compute_object_hash(),
        parsed.object_hash,
        "the stored hash is recomputable from the header itself"
    );

    // The reader performs both checks.
    let reread = ObjectHeader::read_from_dir(&dir).expect("read_from_dir accepts its own output");
    assert_eq!(reread, header);

    // No path reaches the header (ADR-0056).
    let text = String::from_utf8(written).expect("utf8 header");
    assert!(
        !text.contains("source_id"),
        "the header carries no source_id"
    );
    assert!(!text.contains(CATALOGUE_ID), "the header carries no path");
}
