// SPDX-License-Identifier: BUSL-1.1

//! The lockfile — checked, never fetched (ADR-0058 §D5, configflux-p0jz.3
//! T1-T6).
//!
//! Black box through `compile_object` and `link_model`, over the shipped
//! `examples/06-catalogue-polyrepo` units rather than an invented model, so the
//! workflow these assert is one an integrator could actually run. The last case
//! is that workflow end to end: a unit is edited in a worktree, the pinned link
//! refuses it, and the pin is renewed from the integration unit after review.

use compiler::object::ObjectHeader;
use compiler::object_compile::{compile_object, CompileObjectRequest};
use compiler::product_api::{
    link_model, Diagnostic, LinkModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[path = "temp_dirs.rs"]
mod temp_dirs;

const CATALOGUE: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json");
const VISION: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/vision/10_vision.json");

// ----------------------------------------------------------------------------
// T1 — write the lock, then link against it
// ----------------------------------------------------------------------------

#[test]
fn t1_a_lock_written_from_a_link_pins_the_same_link() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "roundtrip");
    let objects = [
        object(&dir, "catalogue", CATALOGUE),
        object(&dir, "vision", VISION),
    ];
    let lock = dir.join("configflux.lock");

    let written = link(&objects, &dir.join("out-a"), Lock::write(&lock));
    assert_eq!(
        written.status,
        OperationStatus::Ok,
        "the unpinned link must succeed: {:?}",
        written.verify_report.diagnostics.diagnostics
    );
    assert!(lock.is_file(), "--write-lock must write the file it names");

    // Every linked unit is pinned at the hash the link reported, and the file
    // reads as the documented shape rather than as an opaque blob.
    let text = std::fs::read_to_string(&lock).expect("lock reads");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("lock is JSON");
    assert_eq!(parsed["schema_version"], 1);
    for object in &written.objects {
        assert_eq!(
            parsed["objects"][&object.unit]["object_hash"],
            serde_json::Value::String(object.object_hash.clone()),
            "unit '{}' must be pinned at the hash the link reported",
            object.unit
        );
        assert_eq!(
            parsed["objects"][&object.unit]["source"],
            serde_json::Value::String(String::new()),
            "no --lock-source was given, so the note must be empty"
        );
    }

    let pinned = link(&objects, &dir.join("out-b"), Lock::check(&lock));
    assert_eq!(
        pinned.status,
        OperationStatus::Ok,
        "the same objects must satisfy the lock they wrote: {:?}",
        pinned.verify_report.diagnostics.diagnostics
    );
}

#[test]
fn t1b_a_lock_source_is_recorded_per_unit_and_read_for_nothing() {
    // `source` is informational (§D5): the product records what the operator
    // typed and reads it for no purpose. The proof that it is not consulted is
    // that a link pinned by a lock carrying a nonsense source still succeeds.
    let dir = temp_dirs::unique_temp_dir("link-lock", "source");
    let objects = [object(&dir, "catalogue", CATALOGUE)];
    let lock = dir.join("configflux.lock");

    let mut options = Lock::write(&lock);
    options.sources.insert(
        "site_catalogue".to_string(),
        "git@example.invalid:site/catalogue.git#v4".to_string(),
    );
    assert_ok(link(&objects, &dir.join("out-a"), options));

    let text = std::fs::read_to_string(&lock).expect("lock reads");
    assert!(
        text.contains("git@example.invalid:site/catalogue.git#v4"),
        "the source note must be recorded verbatim: {text}"
    );
    assert_ok(link(&objects, &dir.join("out-b"), Lock::check(&lock)));
}

// ----------------------------------------------------------------------------
// T2 — an edited unit no longer matches its pin
// ----------------------------------------------------------------------------

#[test]
fn t2_an_edited_unit_is_refused_naming_the_pinned_and_the_linked_hash() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "mismatch");
    let catalogue = object(&dir, "catalogue", CATALOGUE);
    let vision = object(&dir, "vision", VISION);
    let lock = dir.join("configflux.lock");
    assert_ok(link(
        &[catalogue.clone(), vision],
        &dir.join("out-a"),
        Lock::write(&lock),
    ));

    let pinned_hash = header(&object(&dir, "vision", VISION)).object_hash;
    let edited = object(&dir, "vision-edited", &edit_vision());
    let linked_hash = header(&edited).object_hash;
    assert_ne!(pinned_hash, linked_hash, "the edit must move the object hash");

    let result = link(&[catalogue, edited], &dir.join("out-b"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_MISMATCH");
    assert!(
        diagnostic.message.contains("vision_service")
            && diagnostic.message.contains(&pinned_hash)
            && diagnostic.message.contains(&linked_hash),
        "message must name the unit, the pinned hash and the linked hash: {}",
        diagnostic.message
    );
}

#[test]
fn t2b_a_linked_unit_the_lock_does_not_pin_at_all_is_a_mismatch() {
    // The other half of "every linked object's unit must appear in the lock
    // with an equal object_hash": a unit the lock is silent about was never
    // reviewed, and a silent pass would make a lock mean less than it says.
    let dir = temp_dirs::unique_temp_dir("link-lock", "unpinned");
    let catalogue = object(&dir, "catalogue", CATALOGUE);
    let vision = object(&dir, "vision", VISION);
    let lock = dir.join("configflux.lock");
    assert_ok(link(&[catalogue.clone()], &dir.join("out-a"), Lock::write(&lock)));

    let result = link(&[catalogue, vision], &dir.join("out-b"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_MISMATCH");
    assert!(
        diagnostic.message.contains("vision_service"),
        "message must name the unlisted unit: {}",
        diagnostic.message
    );
}

// ----------------------------------------------------------------------------
// T3 — a pinned unit that was not linked
// ----------------------------------------------------------------------------

#[test]
fn t3_a_lock_entry_that_was_not_linked_names_the_unit() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "unlinked");
    let catalogue = object(&dir, "catalogue", CATALOGUE);
    let vision = object(&dir, "vision", VISION);
    let lock = dir.join("configflux.lock");
    assert_ok(link(
        &[catalogue.clone(), vision],
        &dir.join("out-a"),
        Lock::write(&lock),
    ));

    let result = link(&[catalogue.clone()], &dir.join("out-b"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_UNLINKED");
    assert!(
        diagnostic.message.contains("vision_service"),
        "message must name the pinned unit that was not linked: {}",
        diagnostic.message
    );

    // The same subset link, declared deliberate.
    let mut allowed = Lock::check(&lock);
    allowed.allow_extra = true;
    assert_ok(link(&[catalogue], &dir.join("out-c"), allowed));
}

// ----------------------------------------------------------------------------
// T4 — a file that is not a lock file
// ----------------------------------------------------------------------------

#[test]
fn t4_an_unknown_key_is_refused_naming_the_key() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "unknown-key");
    let objects = [object(&dir, "catalogue", CATALOGUE)];
    let hash = header(&objects[0]).object_hash;
    let lock = dir.join("configflux.lock");
    let text = format!(
        "{{\"schema_version\": 1, \"objects\": {{\"site_catalogue\": \
         {{\"object_hash\": \"{hash}\", \"fetch_from\": \"https://example.invalid\"}}}}}}"
    );
    std::fs::write(&lock, text).expect("write lock");

    let result = link(&objects, &dir.join("out"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_INVALID");
    assert!(
        diagnostic.message.contains("fetch_from"),
        "message must name the key it does not know: {}",
        diagnostic.message
    );
}

#[test]
fn t4b_a_unit_name_that_is_not_snake_case_is_refused_naming_it() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "bad-unit");
    let objects = [object(&dir, "catalogue", CATALOGUE)];
    let lock = dir.join("configflux.lock");
    std::fs::write(
        &lock,
        r#"{"schema_version": 1, "objects": {"Site-Catalogue": {"object_hash": "abc"}}}"#,
    )
    .expect("write lock");

    let result = link(&objects, &dir.join("out"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_INVALID");
    assert!(
        diagnostic.message.contains("Site-Catalogue"),
        "message must name the key that is not a unit name: {}",
        diagnostic.message
    );
}

#[test]
fn t4c_an_unsupported_schema_version_is_refused_naming_both_versions() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "bad-version");
    let objects = [object(&dir, "catalogue", CATALOGUE)];
    let lock = dir.join("configflux.lock");
    std::fs::write(&lock, r#"{"schema_version": 2, "objects": {}}"#).expect("write lock");

    let result = link(&objects, &dir.join("out"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_INVALID");
    assert!(
        diagnostic.message.contains('2') && diagnostic.message.contains('1'),
        "message must name the version read and the version implemented: {}",
        diagnostic.message
    );
}

#[test]
fn t4d_a_lock_path_that_does_not_exist_is_refused_naming_it() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "absent");
    let objects = [object(&dir, "catalogue", CATALOGUE)];
    let lock = dir.join("nowhere").join("configflux.lock");

    let result = link(&objects, &dir.join("out"), Lock::check(&lock));
    let diagnostic = sole_diagnostic(&result);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_INVALID");
    assert!(
        diagnostic.message.contains("configflux.lock"),
        "message must name the path it could not read: {}",
        diagnostic.message
    );
}

// ----------------------------------------------------------------------------
// T5 — the written bytes are a function of the linked set
// ----------------------------------------------------------------------------

#[test]
fn t5_write_lock_is_deterministic_and_refuses_to_clobber_a_different_file() {
    let dir = temp_dirs::unique_temp_dir("link-lock", "determinism");
    let catalogue = object(&dir, "catalogue", CATALOGUE);
    let vision = object(&dir, "vision", VISION);

    // Written twice, from the same objects given in opposite orders.
    let first = dir.join("first.lock");
    let second = dir.join("second.lock");
    assert_ok(link(
        &[catalogue.clone(), vision.clone()],
        &dir.join("out-a"),
        Lock::write(&first),
    ));
    assert_ok(link(
        &[vision.clone(), catalogue.clone()],
        &dir.join("out-b"),
        Lock::write(&second),
    ));
    assert_eq!(
        std::fs::read(&first).expect("first reads"),
        std::fs::read(&second).expect("second reads"),
        "--object order must not reach a byte of the lock"
    );

    // Re-writing the identical lock is not a clobber, so it needs no override.
    assert_ok(link(
        &[catalogue.clone(), vision],
        &dir.join("out-d"),
        Lock::write(&second),
    ));

    // A lock that pins a different set is not overwritten by accident.
    let refused = link(&[catalogue.clone()], &dir.join("out-c"), Lock::write(&first));
    let diagnostic = sole_diagnostic(&refused);
    assert_eq!(diagnostic.code, "E_LINK_LOCK_MISMATCH");
    assert!(
        diagnostic.message.contains("first.lock"),
        "message must name the file it refused to overwrite: {}",
        diagnostic.message
    );
    assert_eq!(
        std::fs::read(&first).expect("first still reads"),
        std::fs::read(&second).expect("second still reads"),
        "a refused write must leave the file exactly as it was"
    );

    let mut forced = Lock::write(&first);
    forced.force = true;
    assert_ok(link(&[catalogue], &dir.join("out-e"), forced));
    let text = std::fs::read_to_string(&first).expect("first reads");
    assert!(
        !text.contains("vision_service"),
        "--force-lock must replace the pins, not merge them: {text}"
    );
}

// ----------------------------------------------------------------------------
// T6 — the workflow the lock exists for
// ----------------------------------------------------------------------------

#[test]
fn t6_a_worktree_edit_is_refused_until_the_pin_is_renewed() {
    // The intended flow, in the order an integrator meets it. The catalogue and
    // the vision service are pinned; someone edits vision in a worktree; the
    // pinned link refuses the edited object by name; the integration unit
    // renews the pin after review, and the link passes again.
    let dir = temp_dirs::unique_temp_dir("link-lock", "worktree");
    let catalogue = object(&dir, "catalogue", CATALOGUE);
    let vision = object(&dir, "vision", VISION);
    let lock = dir.join("configflux.lock");
    assert_ok(link(
        &[catalogue.clone(), vision],
        &dir.join("out-a"),
        Lock::write(&lock),
    ));
    let pinned = std::fs::read(&lock).expect("lock reads");

    // The worktree: one value edited, that unit's object rebuilt, nothing else
    // touched.
    let edited = object(&dir, "vision-worktree", &edit_vision());
    let refused = link(
        &[catalogue.clone(), edited.clone()],
        &dir.join("out-b"),
        Lock::check(&lock),
    );
    assert_eq!(sole_diagnostic(&refused).code, "E_LINK_LOCK_MISMATCH");
    assert_eq!(
        std::fs::read(&lock).expect("lock reads"),
        pinned,
        "a refused link must not renew the pin behind the reviewer's back"
    );

    // The review lands: the pin is renewed from the integration unit.
    let mut renew = Lock::write(&lock);
    renew.force = true;
    assert_ok(link(&[catalogue.clone(), edited.clone()], &dir.join("out-c"), renew));
    assert_ne!(
        std::fs::read(&lock).expect("lock reads"),
        pinned,
        "renewing the pin must move the file"
    );

    assert_ok(link(&[catalogue, edited], &dir.join("out-d"), Lock::check(&lock)));
}

// ----------------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------------

/// The vision unit as a worktree edit leaves it: one parameter value moved.
fn edit_vision() -> String {
    let edited = VISION.replace("\"value\": 30", "\"value\": 45");
    assert_ne!(edited, VISION, "the fixture edit must change something");
    edited
}

/// The lock flags one `link` invocation carries.
#[derive(Default)]
struct Lock {
    path: Option<String>,
    write_path: Option<String>,
    allow_extra: bool,
    force: bool,
    sources: BTreeMap<String, String>,
}

impl Lock {
    fn check(path: &Path) -> Self {
        Self {
            path: Some(path.to_string_lossy().into_owned()),
            ..Self::default()
        }
    }

    fn write(path: &Path) -> Self {
        Self {
            write_path: Some(path.to_string_lossy().into_owned()),
            ..Self::default()
        }
    }
}

fn object(dir: &Path, name: &str, chunk: &str) -> String {
    let out = dir.join(format!("{name}.cfo")).to_string_lossy().into_owned();
    compile_object(CompileObjectRequest {
        sources: vec![SourceManifestEntry {
            source_id: format!("{name}.json"),
            inline_content: chunk.to_string(),
        }],
        interfaces: Vec::new(),
        output_dir: out.clone(),
        stamp_time: false,
    })
    .unwrap_or_else(|diagnostic| panic!("compile-object '{name}' failed: {diagnostic:?}"));
    out
}

fn header(object_dir: &str) -> ObjectHeader {
    ObjectHeader::read_from_dir(&PathBuf::from(object_dir)).expect("object header reads")
}

fn link(objects: &[String], out: &Path, lock: Lock) -> compiler::product_api::CompileResult {
    link_model(LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: objects.to_vec(),
        output_dir: out.to_string_lossy().into_owned(),
        cluster_size: None,
        budget: None,
        stamp_time: false,
        lock_path: lock.path,
        lock_allow_extra: lock.allow_extra,
        write_lock_path: lock.write_path,
        lock_sources: lock.sources,
        force_lock: lock.force,
    })
}

fn assert_ok(result: compiler::product_api::CompileResult) {
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "expected the link to succeed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
}

fn sole_diagnostic(result: &compiler::product_api::CompileResult) -> &Diagnostic {
    assert_eq!(
        result.status,
        OperationStatus::Error,
        "expected the link to be refused"
    );
    let diagnostics = &result.verify_report.diagnostics.diagnostics;
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {diagnostics:?}");
    &diagnostics[0]
}
