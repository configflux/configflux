// SPDX-License-Identifier: BUSL-1.1

//! ADR-0063 / configflux-mrm6: the authored symbol charset, re-validated on the
//! Rust ingest path.
//!
//! `compile --source` ingests JSON directly and never evaluates CUE, so the
//! `#snakeId` constraint in `compiler/cue/schema.cue` reaches nothing here.
//! Before ADR-0063 that left the compiler interpolating a declared facet key
//! **bare** into a synthesized condition clause, and picking the value's quote
//! character with a `contains` test — so a key or a value crafted to close its
//! own literal continued into the surrounding clause with VALID grammar. The
//! clause parsed, `compiler_core::unrepresentable_facet_symbol` (a PARSEABILITY
//! oracle, never a validity one) returned `None`, and a corrupted model was
//! emitted at `status=ok`.
//!
//! The three cases at the top of this file are the reproducers configflux-mrm6
//! MEASURED against freshly built binaries: an injected disjunction that made a
//! valid selection unsatisfiable, a phantom value injected into a CLOSED facet's
//! domain, and a declared value silently truncated. Each is asserted through
//! BOTH `verify_model` and `compile_model`, because the whole point of putting
//! the rule in `link_verify` rather than at emit time is that the two agree
//! (configflux-8o20 fact 2).
//!
//! Black-box: everything below drives the public product API, never the
//! validators directly. The predicate's accept/reject table against the
//! `#snakeId` regex is the unit-level twin, in
//! `compiler/src/link_verify_symbol_charset_tests.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use compiler::ir;
use compiler::loader_api::{
    canonical_selection_state, get_selection_options, open_model, GetSelectionOptionsRequest,
    ModelHandle, OpenModelRequest, E_LOADER_INDEX_INVALID,
};
use compiler::product_api::{
    compile_model, verify_model, CompileModelRequest, CompileResult, Diagnostic, OperationStatus,
    SourceManifestEntry, VerifyModelRequest, VerifyReport, E_COMPILE_INPUT_INVALID,
    PRODUCT_SCHEMA_VERSION,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A minimal model carrying one extra facet, built from an authored key, an
/// authored value domain and a default.
///
/// `environment` is the closed facet the mrm6 reproducers inject INTO, so it is
/// present in every fixture and the component condition references it — that is
/// what makes a phantom value in its domain observable.
fn facet_model(key: &str, values: &str, default: &str) -> String {
    format!(
        r#"{{
    "package": "mrm6_symbol_charset",
    "version": "1.0.0",
    "facets": {{
        "environment": {{ "values": ["dev", "prod"], "default": "dev" }},
        "{key}": {{ "values": {values}, "default": "{default}" }}
    }},
    "components": {{
        "svc": {{ "type": "service", "condition": "environment == 'prod'" }}
    }}
}}"#
    )
}

/// The clean control: the same shape with a plainly legal key and domain.
fn legal_model() -> String {
    facet_model("replica_class", r#"["single", "pair"]"#, "single")
}

/// A model declaring one catalogue and one binding over it, from authored ids.
fn catalogue_model(catalogue_id: &str, entry_id: &str, binding_id: &str) -> String {
    format!(
        r#"{{
    "package": "mrm6_symbol_charset",
    "version": "1.0.0",
    "facets": {{
        "environment": {{ "values": ["dev", "prod"], "default": "dev" }}
    }},
    "catalogues": {{
        "{catalogue_id}": {{
            "fields": {{ "length_mm": {{ "type": "integer" }} }},
            "entries": {{ "{entry_id}": {{ "length_mm": 1200 }} }}
        }}
    }},
    "bindings": {{
        "{binding_id}": {{ "catalogue": "{catalogue_id}", "default": "{entry_id}" }}
    }},
    "components": {{
        "svc": {{ "type": "service" }}
    }}
}}"#
    )
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn verify(source: &str) -> VerifyReport {
    verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "00_model.json".to_string(),
            inline_content: source.to_string(),
        }],
    })
}

fn compile(label: &str, source: &str) -> (CompileResult, PathBuf) {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "00_model.json".to_string(),
            inline_content: source.to_string(),
        }],
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    (result, output_dir)
}

fn sole_diagnostic(report: &VerifyReport, case: &str) -> Diagnostic {
    assert_eq!(
        report.status,
        OperationStatus::Error,
        "{case}: the model must be refused"
    );
    let diags = &report.diagnostics.diagnostics;
    assert_eq!(diags.len(), 1, "{case}: expected one diagnostic, got {diags:?}");
    diags[0].clone()
}

/// The load-bearing assertion of this file: `verify` and `compile` refuse the
/// same model, with the same code and the same remedy.
///
/// Asserted as ONE helper rather than as two tests, because the claim is that
/// the two AGREE — a pair of separate assertions can both pass while the
/// answers differ (configflux-8o20 fact 2).
fn refused_by_both(label: &str, source: &str) -> Diagnostic {
    let verified = sole_diagnostic(&verify(source), &format!("{label}/verify"));

    let (compiled, output_dir) = compile(label, source);
    let from_compile = sole_diagnostic(&compiled.verify_report, &format!("{label}/compile"));
    fs::remove_dir_all(&output_dir).ok();

    assert_eq!(
        verified.code, E_COMPILE_INPUT_INVALID,
        "{label}: a charset refusal keeps the frozen input code (ADR-0063 D3)"
    );
    assert_eq!(
        (from_compile.code.as_str(), from_compile.message.as_str()),
        (verified.code.as_str(), verified.message.as_str()),
        "{label}: verify and compile must refuse identically"
    );
    assert_eq!(
        from_compile.hint, verified.hint,
        "{label}: verify and compile must carry the same remedy"
    );

    let hint = verified.hint.clone().unwrap_or_else(|| {
        panic!("{label}: a charset refusal must carry the charset remedy");
    });
    assert!(
        hint.contains("snake_case identifier") && hint.contains("no quote, no space"),
        "{label}: the remedy must state BOTH rules, got: {hint}"
    );
    verified
}

fn accepted_by_both(label: &str, source: &str) {
    let report = verify(source);
    assert_eq!(
        report.status,
        OperationStatus::Ok,
        "{label}: verify must accept, got {:?}",
        report.diagnostics.diagnostics
    );

    let (compiled, output_dir) = compile(label, source);
    let status = compiled.status;
    let diags = compiled.verify_report.diagnostics.diagnostics.clone();
    fs::remove_dir_all(&output_dir).ok();
    assert_eq!(status, OperationStatus::Ok, "{label}: compile must accept, got {diags:?}");
}

// ---------------------------------------------------------------------------
// The three configflux-mrm6 reproducers
// ---------------------------------------------------------------------------

/// Reproducer 1. The key closes its literal and continues with a disjunction,
/// which `synthesize_facet_cardinality` then wraps in `exactly_one_of(...)`:
/// both arms are true under `environment=prod`, the conjunct is false, and a
/// selection that is valid on the clean model was refused with
/// `E_SELECTION_CONFLICT` at RESOLVE time, long after a `status=ok` compile.
#[test]
fn a_facet_key_injecting_an_equality_disjunction_is_refused_at_ingest() {
    let source = facet_model(
        "environment == 'prod' || replica_class",
        r#"["single", "pair"]"#,
        "single",
    );
    let diagnostic = refused_by_both("key-eq-injection", &source);
    assert!(
        diagnostic.message.contains("environment == 'prod' || replica_class"),
        "the refusal must name the offending key, got: {}",
        diagnostic.message
    );
}

/// Reproducer 2. The `!=` form injects a PHANTOM value into the closed
/// `environment` facet: `cfx options` reported `['prod','zz']` against a
/// declared domain of exactly `[dev, prod]`, so closed-facet exhaustiveness
/// (ADR-0054 §5.2) was broken and the guided walk offered an undeclared value.
#[test]
fn a_facet_key_injecting_a_phantom_closed_facet_value_is_refused_at_ingest() {
    let source = facet_model(
        "environment != 'zz' || replica_class",
        r#"["single", "pair"]"#,
        "single",
    );
    let diagnostic = refused_by_both("key-ne-injection", &source);
    assert!(
        diagnostic.message.contains("environment != 'zz' || replica_class"),
        "the refusal must name the offending key, got: {}",
        diagnostic.message
    );
}

/// Reproducer 3. The value holds BOTH quote characters — the exact class the
/// pre-ADR-0063 hint text claimed was refused, and was not. `contains('\'')`
/// selected double quotes, the value closed that literal and continued with
/// valid grammar, the clause parsed, and the compile emitted a model in which
/// the declared value was silently TRUNCATED to `a'b`.
#[test]
fn a_facet_value_holding_both_quote_characters_is_refused_at_ingest() {
    let source = facet_model(
        "replica_class",
        r#"["single", "a'b\" || environment == \"zz"]"#,
        "single",
    );
    let diagnostic = refused_by_both("value-quote-injection", &source);
    assert!(
        diagnostic.message.contains("replica_class")
            && diagnostic.message.contains("a'b\" || environment == \"zz"),
        "the refusal must name the owning facet and the offending value, got: {}",
        diagnostic.message
    );
}

// ---------------------------------------------------------------------------
// The rules themselves, through the product surface
// ---------------------------------------------------------------------------

#[test]
fn a_facet_key_carrying_a_space_is_refused() {
    let source = facet_model("replica class", r#"["single", "pair"]"#, "single");
    let diagnostic = refused_by_both("key-space", &source);
    assert!(
        diagnostic.message.contains("replica class"),
        "the refusal must name the offending key, got: {}",
        diagnostic.message
    );
}

#[test]
fn a_facet_value_carrying_a_space_is_refused() {
    let source = facet_model("log_level", r#"["info", "de bug"]"#, "info");
    let diagnostic = refused_by_both("value-space", &source);
    assert!(
        diagnostic.message.contains("de bug"),
        "the refusal must name the offending value, got: {}",
        diagnostic.message
    );
}

/// The token set is NOT snake_case (ADR-0063 D2): a facet VALUE is a file-safe
/// token, the same set `cfx` accepts for an environment name (ADR-0059 D1). A
/// region id and a version-like number are the shapes that rule exists to keep
/// legal, and they are the control that stops D2 from quietly becoming D1.
#[test]
fn a_dotted_or_dashed_facet_value_stays_legal() {
    accepted_by_both(
        "value-token-ok",
        &facet_model("region", r#"["eu-west-1", "us-east-2"]"#, "eu-west-1"),
    );
    accepted_by_both(
        "value-dotted-ok",
        &facet_model("api_tier", r#"["1.5", "2.0"]"#, "1.5"),
    );
}

#[test]
fn the_clean_model_still_compiles() {
    accepted_by_both("legal-control", &legal_model());
}

// ---------------------------------------------------------------------------
// Catalogues and bindings (ADR-0057 §D2/§D3, reached only through JSON)
// ---------------------------------------------------------------------------

#[test]
fn a_binding_id_violating_snake_id_is_refused() {
    let source = catalogue_model("containers", "c1", "line Container");
    let diagnostic = refused_by_both("binding-id", &source);
    assert!(
        diagnostic.message.contains("line Container"),
        "the refusal must name the offending binding, got: {}",
        diagnostic.message
    );
}

#[test]
fn a_catalogue_entry_id_violating_snake_id_is_refused() {
    let source = catalogue_model("containers", "C 1", "line_container");
    let diagnostic = refused_by_both("entry-id", &source);
    assert!(
        diagnostic.message.contains("containers") && diagnostic.message.contains("C 1"),
        "the refusal must name the catalogue and the offending entry, got: {}",
        diagnostic.message
    );
}

#[test]
fn a_catalogue_id_violating_snake_id_is_refused() {
    let source = catalogue_model("Containers", "c1", "line_container");
    let diagnostic = refused_by_both("catalogue-id", &source);
    assert!(
        diagnostic.message.contains("Containers"),
        "the refusal must name the offending catalogue, got: {}",
        diagnostic.message
    );
}

#[test]
fn a_clean_catalogue_and_binding_still_compile() {
    accepted_by_both(
        "catalogue-control",
        &catalogue_model("containers", "c1", "line_container"),
    );
}

// ---------------------------------------------------------------------------
// The CONSUMPTION path (ADR-0063 Amendment 1, configflux-h3rm)
// ---------------------------------------------------------------------------

/// A package whose symbols violate the rule is refused when it is LOADED, not
/// only when it is compiled.
///
/// The ingest rule above runs in whichever compiler PRODUCED a package. It
/// says nothing about a package this binary is handed: every package hash is
/// self-consistent and unkeyed, so a rewrite that recomputes them passes every
/// integrity check `open_model` makes, and ADR-0063 kept
/// `PRODUCT_SCHEMA_VERSION` at 5, so a package built before the rule existed is
/// accepted on its version alone. Either way the loader re-derives condition
/// text from the package's own catalogue entry ids through the same bare
/// interpolation configflux-mrm6 measured.
///
/// So this case builds the clean package with the real compiler, rewrites ONE
/// entry id into the mrm6 shape, and RE-ADDRESSES the package around the edit —
/// the chunk is renamed to the hash of its new content, the index follows it,
/// `config_hash` is recomputed and the manifest is rewritten from the index.
/// Nothing is left for an integrity check to catch, which is what makes the
/// refusal below a statement about the symbol rule rather than about tampering.
#[test]
fn a_package_carrying_an_injected_symbol_is_refused_by_the_loader() {
    let injected = "c1' || site == 'x";
    let (compiled, package_dir) = compile(
        "loader-injected-entry-id",
        &catalogue_model("containers", "c1", "line_container"),
    );
    assert_eq!(
        compiled.status,
        OperationStatus::Ok,
        "the clean model must compile: {:?}",
        compiled.verify_report
    );

    // The package still loads, and the binding offers the clean entry id — the
    // control the refusal below is read against.
    let clean = open_and_list_options(&package_dir, "line_container");
    assert_eq!(clean, vec!["c1".to_string()]);

    rewrite_entry_id(&package_dir, "c1", injected);

    // Integrity is intact: `open_model` still accepts the package.
    let handle = open_package(&package_dir).expect("the re-addressed package must still open");

    let options = get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        facet: "line_container".to_string(),
        selection_state: canonical_selection_state(
            handle.model_hash.clone(),
            "all".to_string(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("canonical selection state"),
        include_pruned_reasons: false,
    });
    assert_eq!(
        options.status,
        OperationStatus::Error,
        "the loader must refuse a package declaring an injected symbol"
    );
    assert!(
        options.valid_options.is_empty(),
        "a refused package must publish no options: {:?}",
        options.valid_options
    );
    let diagnostic = &options.diagnostics.diagnostics[0];
    assert_eq!(
        diagnostic.code, E_LOADER_INDEX_INVALID,
        "the refusal keeps the code this operation already reports for a package \
         it cannot read"
    );
    assert!(
        diagnostic.message.contains(injected)
            && diagnostic.message.contains("entry id")
            && diagnostic.message.contains("must be recompiled"),
        "the refusal must name the class, the symbol and the remedy, got: {}",
        diagnostic.message
    );

    fs::remove_dir_all(&package_dir).ok();
}

/// Open `dir` as a CMP model, returning its handle.
fn open_package(dir: &Path) -> Option<ModelHandle> {
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: dir
            .join(ir::CMP_DEFAULT_MANIFEST_FILENAME)
            .to_string_lossy()
            .into_owned(),
    });
    result.model_handle
}

/// The options one facet offers on the package at `dir`, through the public
/// selection API.
fn open_and_list_options(dir: &Path, facet: &str) -> Vec<String> {
    let handle = open_package(dir).expect("the package must open");
    let options = get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        facet: facet.to_string(),
        selection_state: canonical_selection_state(
            handle.model_hash,
            "all".to_string(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("canonical selection state"),
        include_pruned_reasons: false,
    });
    assert_eq!(
        options.status,
        OperationStatus::Ok,
        "the clean package must load: {:?}",
        options.diagnostics.diagnostics
    );
    options.valid_options
}

/// Rename one catalogue entry id inside the sole chunk of the package at `dir`
/// and re-address the package around the edit.
///
/// A text substitution rather than a re-serialization: the chunk file is the
/// canonical byte encoding of its entity maps, and rewriting it through a JSON
/// round trip would risk moving bytes the edit is not about. `from` is an
/// authored entry id and `to` contains no JSON metacharacter, so both are their
/// own JSON spelling.
fn rewrite_entry_id(dir: &Path, from: &str, to: &str) {
    let index_path = dir.join(ir::CMP_DEFAULT_INDEX_REF);
    let mut index = ir::load_index(&index_path).expect("load the emitted index");
    assert_eq!(index.chunks.len(), 1, "the fixture is a single-chunk package");
    let before = index.chunks[0].chunk_hash.clone();

    let chunk_path = dir.join(format!("chunk-{before}.cfir"));
    let authored = fs::read_to_string(&chunk_path).expect("read the chunk");
    let edited = authored.replace(&format!("\"{from}\""), &format!("\"{to}\""));
    assert_ne!(edited, authored, "the entry id must appear in the chunk");
    fs::write(&chunk_path, &edited).expect("write the edited chunk");

    // The chunk's content address is a function of its seven entity maps and
    // nothing else (ADR-0056 Amendment 1), so it is recomputable from the file
    // in front of us — and the embedded `chunk_hash` field is outside that
    // preimage, which is why it can be stamped after the fact.
    let after = ir::chunk_hash_of_chunk(&ir::load_chunk(&chunk_path).expect("parse edited chunk"))
        .expect("recompute the edited chunk's address");
    let stamped = edited.replace(
        &format!("\"chunk_hash\":\"{before}\""),
        &format!("\"chunk_hash\":\"{after}\""),
    );
    assert_ne!(stamped, edited, "the chunk must carry its own address");
    fs::write(dir.join(format!("chunk-{after}.cfir")), &stamped).expect("write re-addressed chunk");
    fs::remove_file(&chunk_path).expect("remove the pre-edit chunk file");

    index.chunks[0].chunk_hash = after.clone();
    for namespace in [
        &mut index.component_index,
        &mut index.definition_index,
        &mut index.artifact_index,
        &mut index.facet_index,
        &mut index.catalogue_index,
        &mut index.binding_index,
    ] {
        for chunk_hash in namespace.values_mut() {
            if *chunk_hash == before {
                *chunk_hash = after.clone();
            }
        }
    }
    index.config_hash = index
        .compute_config_hash()
        .expect("recompute the index config hash");
    fs::write(
        &index_path,
        serde_json::to_vec_pretty(&index).expect("serialize the rewritten index"),
    )
    .expect("write the rewritten index");

    ir::write_cmp_manifest(
        &dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME),
        &ir::CmpManifest::from_index(&index),
    )
    .expect("rewrite the manifest from the re-addressed index");

    ir::verify_index_integrity(&index, dir)
        .expect("the re-addressed package must pass every integrity check");
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-mrm6", test_name)
}
