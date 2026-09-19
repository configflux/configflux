// SPDX-License-Identifier: BUSL-1.1

//! The link's refusals (ADR-0058 §D4, configflux-p0jz.2 T2-T6).
//!
//! Black box through `compile_object` and `link_model`, over the shipped
//! four-unit example rather than an invented model, so every message quoted
//! here is one an author of `examples/06-catalogue-polyrepo` could actually
//! meet. Each case asserts the code, the names in the message, and — where the
//! stage says so — that nothing was written under `--out`.

use compiler::object::ObjectHeader;
use compiler::object_compile::{compile_object, CompileObjectRequest};
use compiler::product_api::{
    link_model, LinkModelRequest, OperationStatus, SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};

#[path = "temp_dirs.rs"]
mod temp_dirs;

const CATALOGUE: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json");
const VISION: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/vision/10_vision.json");
const SORTER: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/sorter/20_sorter.json");

// ----------------------------------------------------------------------------
// T2 — one id, one object
// ----------------------------------------------------------------------------

#[test]
fn t2_two_objects_exporting_one_component_name_both_units() {
    let dir = temp_dirs::unique_temp_dir("link-diag", "duplicate-id");
    let vision = object(&dir, "vision", &[("vision.json", VISION)]);
    // The same component, republished by another unit. Only `package` differs,
    // so the clash is exactly the cross-unit one the linker exists to catch.
    let twin_source = VISION.replace("\"package\": \"vision_service\"", "\"package\": \"twin_service\"");
    let twin = object(&dir, "twin", &[("twin.json", twin_source.as_str())]);
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);

    let out = dir.join("out");
    let result = link(&[catalogue, vision, twin], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_DUPLICATE_ID");
    assert_eq!(
        diagnostic.message,
        "Units 'twin_service' and 'vision_service' both export component 'vision_service'; an id \
         names one declaration across the whole linked set"
    );
    assert_nothing_written(&out);
}

// ----------------------------------------------------------------------------
// T3 — an import nothing provides
// ----------------------------------------------------------------------------

#[test]
fn t3_a_service_linked_without_its_catalogue_names_the_unit_and_the_id() {
    let dir = temp_dirs::unique_temp_dir("link-diag", "unresolved");
    let sorter = object(&dir, "sorter", &[("sorter.json", SORTER)]);

    let out = dir.join("out");
    let result = link(&[sorter], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_UNRESOLVED_IMPORT");
    assert!(
        diagnostic.message.contains("unit 'sorter_service'")
            && diagnostic.message.contains("no linked object declares it"),
        "message must name the unit and the verdict: {}",
        diagnostic.message
    );
    assert_nothing_written(&out);
}

#[test]
fn t3b_an_unresolved_requirement_names_the_binding_it_needs() {
    // The §D4 example message, on a unit whose only unresolved import is the
    // shared choice it requires: "unit 'x' requires binding 'y'; no linked
    // object declares it". Built by dropping the sorter's inherited parameter,
    // so the definition import is gone and the binding is what is left.
    let dir = temp_dirs::unique_temp_dir("link-diag", "unresolved-binding");
    let source = SORTER.replace("\"inherits\": \"container_dim_mm\",\n                    ", "");
    assert!(!source.contains("container_dim_mm"), "fixture must drop the inherited definition");
    let sorter = object(&dir, "sorter", &[("sorter.json", source.as_str())]);

    let out = dir.join("out");
    let result = link(&[sorter], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_UNRESOLVED_IMPORT");
    assert_eq!(
        diagnostic.message,
        "unit 'sorter_service' requires binding 'sorter_container'; no linked object declares it"
    );
    assert_nothing_written(&out);
}

// ----------------------------------------------------------------------------
// T4 — compiled against one interface, linked with another
// ----------------------------------------------------------------------------

#[test]
fn t4_an_interface_whose_hash_moved_is_refused_naming_both_hashes() {
    let dir = temp_dirs::unique_temp_dir("link-diag", "interface");
    let compiled_against = object(&dir, "catalogue-a", &[("catalogue.json", CATALOGUE)]);
    // vision_service compiled against site_catalogue@A.
    let vision = object_against(
        &dir,
        "vision",
        &[("vision.json", VISION)],
        &[compiled_against.clone()],
    );

    // One edited catalogue entry: same unit, different content, different hash.
    let edited = CATALOGUE.replace("\"width_mm\": 600", "\"width_mm\": 601");
    assert_ne!(edited, CATALOGUE, "fixture must actually edit an entry");
    let linked = object(&dir, "catalogue-b", &[("catalogue.json", edited.as_str())]);

    let out = dir.join("out");
    let result = link(&[linked, vision], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_INTERFACE_MISMATCH");
    assert!(
        diagnostic.message.contains("vision_service")
            && diagnostic.message.contains("site_catalogue"),
        "message must name the dependent unit and the interface: {}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("Recompile")),
        "the hint must offer both remedies: {:?}",
        diagnostic.hint
    );
    assert_nothing_written(&out);
}

// ----------------------------------------------------------------------------
// T5 — an object that does not match its own header
// ----------------------------------------------------------------------------

#[test]
fn t5_a_chunk_file_that_does_not_match_its_header_is_refused() {
    let dir = temp_dirs::unique_temp_dir("link-diag", "corrupt");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    let chunk = sole_chunk_file(Path::new(&catalogue));
    std::fs::write(&chunk, b"{ this is not a chunk").expect("overwrite chunk");

    let out = dir.join("out");
    let result = link(&[catalogue], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_OBJECT_CORRUPT");
    assert!(
        diagnostic.message.contains("site_catalogue"),
        "message must name the unit whose object is broken: {}",
        diagnostic.message
    );
    assert_nothing_written(&out);
}

#[test]
fn t5b_a_chunk_file_deleted_from_an_object_is_refused() {
    let dir = temp_dirs::unique_temp_dir("link-diag", "missing-chunk");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    std::fs::remove_file(sole_chunk_file(Path::new(&catalogue))).expect("remove chunk");

    let result = link(&[catalogue], &dir.join("out"));
    assert_eq!(sole_diagnostic(&result).code, "E_LINK_OBJECT_CORRUPT");
}

#[test]
fn t5c_a_value_edited_inside_a_chunk_file_is_refused_though_its_name_never_moved() {
    // The reproduction that forced ADR-0056 Amendment 1: one catalogue entry's
    // `length_mm` edited from 1200 to 9999 inside the emitted chunk, with the
    // file's own `chunk_hash` field and its name both left exactly as
    // `compile-object` wrote them. Under the previous preimage — the authored
    // `Config`, `package` and `version` included — nothing that reads a chunk
    // file could recompute its address, so the only check available was the
    // file's self-declared name against its actual name, and this edit moves
    // neither: the link exited 0 and wrote a package carrying 9999 under the
    // SAME `model_hash` as the untampered link. The address is now the hash of
    // the entity maps the chunk itself carries, so the edit moves it and
    // stage 3 sees it.
    let dir = temp_dirs::unique_temp_dir("link-diag", "edited-value");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    let chunk = sole_chunk_file(Path::new(&catalogue));
    let named = chunk_hash_in_name(&chunk);

    let written = std::fs::read_to_string(&chunk).expect("read chunk");
    let edited = written.replace("\"length_mm\":1200", "\"length_mm\":9999");
    assert_ne!(edited, written, "fixture must edit a value the chunk carries");
    // Both halves of the WEAKER check are left standing, so a link that refuses
    // this file can only be recomputing the address.
    assert!(
        edited.contains(&format!("\"chunk_hash\":\"{named}\"")),
        "fixture must leave the embedded chunk_hash field untouched"
    );
    std::fs::write(&chunk, edited).expect("rewrite chunk");
    assert!(chunk.is_file(), "fixture must leave the file name untouched");

    let out = dir.join("out");
    let result = link(&[catalogue], &out);
    // `status_exit_code` in main.rs maps `Error` to 2, which is the exit code
    // the CLI returns for a refused link.
    assert_eq!(result.status, OperationStatus::Error);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_OBJECT_CORRUPT");
    assert!(
        diagnostic.message.contains("site_catalogue")
            && diagnostic.message.contains(&format!("chunk-{named}.cfir"))
            && diagnostic.message.contains("hashes to"),
        "message must name the unit, the file, and what it actually hashes to: {}",
        diagnostic.message
    );
    assert_nothing_written(&out);
}

// ----------------------------------------------------------------------------
// T7 — a header that does not say what its chunk files say
//      (configflux-pa99, ADR-0063 Amendment 1 §2)
// ----------------------------------------------------------------------------
//
// Every case here rewrites the header and re-hashes it, so `read_from_dir`'s
// self-check passes and the chunk files are untouched and still hash to their
// own names. Only the header-versus-body rebuild can refuse them — which is the
// point: this is the version-skew shape (an object written by a compiler whose
// header said something its bodies did not) as well as the tamper shape.

#[test]
fn t7_an_injected_catalogue_entry_in_a_header_roster_is_refused_naming_the_field() {
    // The configflux-mrm6 class, reached through the header instead of through
    // ingest: an entry id crafted to close its own quoted literal and continue
    // with valid condition grammar. The BODY is clean, so `verify_complete_model`
    // has nothing to say about it, and the id sets still match, so the check
    // this one replaced passed it — after which the roster reached the clause
    // synthesizers and the `.ccm` was emitted from it.
    let dir = temp_dirs::unique_temp_dir("link-diag", "header-entries");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    retamper_header(&catalogue, |header| {
        header
            .catalogue_entries
            .get_mut("containers")
            .expect("the unit declares the containers catalogue")
            .push("c1' || site == 'x".to_string());
    });

    let out = dir.join("out");
    let result = link(&[catalogue.clone()], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_OBJECT_CORRUPT");
    assert!(
        diagnostic.message.contains(&catalogue)
            && diagnostic.message.contains("site_catalogue")
            && diagnostic.message.contains("catalogue_entries"),
        "message must name the object dir, the unit and the first differing field: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Recompile the object with `compile-object`")
    );
    assert_nothing_written(&out);
}

#[test]
fn t7b_a_phantom_value_in_a_header_facet_domain_is_refused_naming_the_field() {
    // The same class on the other roster the `.ccm` is synthesized from: a
    // value in a CLOSED facet's domain that the declaring chunk never declared.
    // Left standing, it is offered by `cfx options` as a legitimate choice.
    let dir = temp_dirs::unique_temp_dir("link-diag", "header-domains");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    retamper_header(&catalogue, |header| {
        header
            .facet_domains
            .get_mut("site")
            .expect("the unit declares the site facet")
            .push("factory_zz".to_string());
    });

    let out = dir.join("out");
    let result = link(&[catalogue.clone()], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_OBJECT_CORRUPT");
    assert!(
        diagnostic.message.contains("site_catalogue")
            && diagnostic.message.contains("facet_domains"),
        "message must name the unit and the first differing field: {}",
        diagnostic.message
    );
    assert_nothing_written(&out);
}

#[test]
fn t7c_an_export_dropped_from_a_header_keeps_the_wording_that_names_the_id() {
    // `exports` is the one field a difference has an ID to name, so it keeps
    // the two messages it carried when the id sets were the whole check. The
    // field is compared before `imports`, which this edit also moves, so the
    // more actionable of the two is what the author sees.
    let dir = temp_dirs::unique_temp_dir("link-diag", "header-exports");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    retamper_header(&catalogue, |header| {
        assert!(
            header.exports.definitions.remove("container_dim_mm"),
            "fixture must actually drop an export the unit declares"
        );
    });

    let out = dir.join("out");
    let result = link(&[catalogue.clone()], &out);
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_OBJECT_CORRUPT");
    assert_eq!(
        diagnostic.message,
        format!(
            "Object '{catalogue}' (unit 'site_catalogue') holds definition 'container_dim_mm' \
             in its chunk files, but its header does not export it"
        )
    );
    assert_nothing_written(&out);
}

#[test]
fn t7d_every_object_the_suite_compiles_still_links_with_its_header_rebuilt() {
    // The control the three cases above are meaningless without: the rebuild is
    // EXACT, so an object `compile-object` wrote must link unchanged. If the
    // two summarizers ever disagreed on one field `ObjectHeader::from_summaries`
    // reads, this would go red on an untouched object instead of a tampered
    // one. The corpus-wide form is `//compiler:link_oracle_test`, which drives
    // every scenario pack and example through compile-object + link and
    // compares the result to a one-shot compile byte for byte.
    let dir = temp_dirs::unique_temp_dir("link-diag", "header-roundtrip");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);
    let vision = object_against(&dir, "vision", &[("vision.json", VISION)], &[catalogue.clone()]);
    let sorter = object_against(&dir, "sorter", &[("sorter.json", SORTER)], &[catalogue.clone()]);

    let out = dir.join("out");
    let result = link(&[catalogue, vision, sorter], &out);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "an untampered set must link: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
    assert_eq!(result.objects.len(), 3);
    assert!(out.join("cmp.manifest.json").is_file());
}

// ----------------------------------------------------------------------------
// T6 — stage 1 opens no chunk file
// ----------------------------------------------------------------------------

#[test]
fn t6_a_stage_one_fault_is_reported_with_every_chunk_file_unreadable() {
    // The memory claim of §D4 stage 1, made observable: replace every chunk
    // file in every object with a placeholder that cannot be parsed, and the
    // link must still report the HEADER fault. If stage 1 ever started reading
    // chunks, this would report E_LINK_OBJECT_CORRUPT instead.
    let dir = temp_dirs::unique_temp_dir("link-diag", "headers-only");
    let sorter = object(&dir, "sorter", &[("sorter.json", SORTER)]);
    std::fs::write(sole_chunk_file(Path::new(&sorter)), b"not a chunk").expect("clobber chunk");

    let out = dir.join("out");
    let result = link(&[sorter], &out);
    assert_eq!(
        sole_diagnostic(&result).code,
        "E_LINK_UNRESOLVED_IMPORT",
        "stage 1 must answer from the header alone"
    );
    assert_nothing_written(&out);
}

#[test]
fn t6b_a_link_of_one_unit_that_needs_nothing_succeeds_with_its_objects_named() {
    // The positive half: an object whose imports are all satisfied links, and
    // the result names what was linked.
    let dir = temp_dirs::unique_temp_dir("link-diag", "self-contained");
    let catalogue = object(&dir, "catalogue", &[("catalogue.json", CATALOGUE)]);

    let result = link(&[catalogue], &dir.join("out"));
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.objects.len(), 1);
    assert_eq!(result.objects[0].unit, "site_catalogue");
    assert_eq!(result.objects[0].object_hash.len(), 64);
    assert!(dir.join("out").join("cmp.manifest.json").is_file());
}

// ----------------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------------

fn object(dir: &Path, name: &str, chunks: &[(&str, &str)]) -> String {
    object_against(dir, name, chunks, &[])
}

fn object_against(
    dir: &Path,
    name: &str,
    chunks: &[(&str, &str)],
    interfaces: &[String],
) -> String {
    let out = dir.join(format!("{name}.cfo")).to_string_lossy().into_owned();
    let headers = interfaces
        .iter()
        .map(|path| ObjectHeader::read_from_dir(Path::new(path)).expect("interface reads"))
        .collect();
    compile_object(CompileObjectRequest {
        sources: chunks
            .iter()
            .map(|(source_id, content)| SourceManifestEntry {
                source_id: (*source_id).to_string(),
                inline_content: (*content).to_string(),
            })
            .collect(),
        interfaces: headers,
        output_dir: out.clone(),
        stamp_time: false,
    })
    .unwrap_or_else(|diagnostic| panic!("compile-object '{name}' failed: {diagnostic:?}"));
    out
}

fn link(objects: &[String], out: &Path) -> compiler::product_api::CompileResult {
    link_model(LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: objects.to_vec(),
        output_dir: out.to_string_lossy().into_owned(),
        cluster_size: None,
        budget: None,
        stamp_time: false,
        lock_path: None,
        lock_allow_extra: false,
        write_lock_path: None,
        lock_sources: Default::default(),
        force_lock: false,
    })
}

/// Rewrite an object's header with `edit` applied and a FRESH `object_hash`.
///
/// The re-hash is what makes the fixture mean something: without it the edit is
/// caught by `ObjectHeader::read_from_dir`'s self-check in stage 1, and the test
/// would pass against a linker that never compared the header to its bodies at
/// all. The final read is the assertion that the rewritten header is
/// self-consistent, so the only check left to refuse it is the rebuild.
fn retamper_header(object_dir: &str, edit: impl FnOnce(&mut ObjectHeader)) {
    let dir = Path::new(object_dir);
    let mut header = ObjectHeader::read_from_dir(dir).expect("header reads");
    edit(&mut header);
    header.object_hash = header.compute_object_hash();
    header.write_to_dir(dir).expect("header rewrites");
    ObjectHeader::read_from_dir(dir).expect("the rewritten header must be self-consistent");
}

fn sole_diagnostic(
    result: &compiler::product_api::CompileResult,
) -> &compiler::product_api::Diagnostic {
    assert_eq!(
        result.status,
        OperationStatus::Error,
        "expected the link to be refused"
    );
    let diagnostics = &result.verify_report.diagnostics.diagnostics;
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {diagnostics:?}");
    &diagnostics[0]
}

fn sole_chunk_file(object_dir: &Path) -> PathBuf {
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(object_dir)
        .expect("read object dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            name.starts_with("chunk-").then_some(path)
        })
        .collect();
    chunks.sort();
    assert_eq!(chunks.len(), 1, "fixture object must hold one chunk");
    chunks.remove(0)
}

/// The address a chunk file is NAMED by, read off the file name.
fn chunk_hash_in_name(chunk: &Path) -> String {
    let name = chunk
        .file_name()
        .expect("chunk path has a file name")
        .to_string_lossy();
    name.trim_start_matches("chunk-")
        .trim_end_matches(".cfir")
        .to_string()
}

/// Nothing under `--out` when a stage refuses (ADR-0058 §D4).
fn assert_nothing_written(out: &Path) {
    if !out.exists() {
        return;
    }
    let entries: Vec<String> = std::fs::read_dir(out)
        .expect("read out dir")
        .map(|entry| entry.expect("dir entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.is_empty(),
        "a refused link must write nothing under --out, found: {entries:?}"
    );
}
