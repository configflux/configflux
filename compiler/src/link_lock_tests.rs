// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the lockfile itself (ADR-0058 §D5).
//!
//! The rules that are cheaper to state here than to reach through a link: what
//! a unit name is, what a lockfile is not, and that the written bytes are a
//! function of the linked set and nothing else. The workflow these serve — pin,
//! edit, refuse, renew — is asserted end to end in `compiler/tests/link_lock.rs`.

use super::*;
use crate::interface_summary::summarize;
use crate::ir::chunk_hash_from_config;
use crate::schema::Config;

const UNIT: &str = r#"{
  "package": "cat_unit",
  "version": "1.0.0",
  "definitions": { "width_mm": { "type": "integer", "value": 10 } }
}"#;

fn header_for(json: &str) -> ObjectHeader {
    let config: Config = serde_json::from_str(json).expect("fixture parses");
    let hash = chunk_hash_from_config(&config).expect("hash");
    let summary = summarize(&config, "fixture");
    ObjectHeader::from_summaries(&config.package, vec![hash], &[summary], Vec::new())
}

/// A fixture lockfile on disk, at a path no other test in this process holds
/// (`unique_temp_path` is the configflux-rvpb atomic-sequence primitive).
fn write_temp(label: &str, contents: &str) -> String {
    let path = crate::scenario_test_support::unique_temp_path("cfx-lock", label);
    std::fs::write(&path, contents).expect("write fixture lock");
    path.to_string_lossy().into_owned()
}

// ----------------------------------------------------------------------------
// Unit names
// ----------------------------------------------------------------------------

#[test]
fn a_unit_name_is_snake_case_starting_with_a_letter() {
    for accepted in ["a", "site_catalogue", "unit2", "a_1", "trailing_"] {
        assert!(
            check_unit_name(accepted).is_ok(),
            "'{accepted}' matches the rule and must be accepted"
        );
    }
    // Every rejection the regex makes, and each for a different reason: empty,
    // uppercase, a leading digit, a leading underscore, a doubled underscore,
    // and a character the grammar has no place for.
    for refused in ["", "Site", "2unit", "_unit", "a__b", "site-catalogue", "site.catalogue"] {
        assert!(
            check_unit_name(refused).is_err(),
            "'{refused}' breaks the rule and must be refused"
        );
    }
}

#[test]
fn a_refused_unit_name_is_quoted_in_the_message() {
    let error = check_unit_name("Site-Catalogue").expect_err("refused");
    assert!(
        format!("{error}").contains("Site-Catalogue"),
        "the message must name what it refused: {error}"
    );
}

// ----------------------------------------------------------------------------
// Reading a lockfile
// ----------------------------------------------------------------------------

#[test]
fn a_lock_with_an_unknown_entry_key_names_the_key() {
    let path = write_temp(
        "unknown-key",
        r#"{"schema_version": 1, "objects": {"cat_unit": {"object_hash": "ab", "fetch": "x"}}}"#,
    );
    let error = read_lock(&path).expect_err("refused");
    assert_eq!(error.code, E_LINK_LOCK_INVALID);
    assert!(
        error.message.contains("fetch"),
        "the message must name the key it does not know: {}",
        error.message
    );
}

#[test]
fn a_lock_with_an_unknown_top_level_key_names_the_key() {
    // The one that would matter most if it were ignored: a future field that
    // changes what the pins mean, read by a binary that does not know it.
    let path = write_temp(
        "unknown-top",
        r#"{"schema_version": 1, "objects": {}, "registry": "https://example.invalid"}"#,
    );
    let error = read_lock(&path).expect_err("refused");
    assert_eq!(error.code, E_LINK_LOCK_INVALID);
    assert!(
        error.message.contains("registry"),
        "the message must name the key it does not know: {}",
        error.message
    );
}

#[test]
fn source_is_optional_and_reads_as_empty_when_absent() {
    let path = write_temp(
        "no-source",
        r#"{"schema_version": 1, "objects": {"cat_unit": {"object_hash": "ab"}}}"#,
    );
    let lock = read_lock(&path).expect("reads");
    assert_eq!(lock.objects["cat_unit"].source, "");
}

// ----------------------------------------------------------------------------
// The two checks
// ----------------------------------------------------------------------------

#[test]
fn an_equal_pin_passes_and_an_unequal_one_names_both_hashes() {
    let header = header_for(UNIT);
    let mut lock = LockFile {
        schema_version: LOCK_SCHEMA_VERSION,
        objects: BTreeMap::new(),
    };
    lock.objects.insert(
        "cat_unit".to_string(),
        LockEntry {
            object_hash: header.object_hash.clone(),
            source: String::new(),
        },
    );
    assert!(check_lock(&lock, &[header.clone()], false, "l.lock").is_ok());

    lock.objects.get_mut("cat_unit").expect("entry").object_hash = "deadbeef".to_string();
    let error = check_lock(&lock, &[header.clone()], false, "l.lock").expect_err("refused");
    assert_eq!(error.code, E_LINK_LOCK_MISMATCH);
    assert!(
        error.message.contains("deadbeef") && error.message.contains(&header.object_hash),
        "the message must name the pinned hash and the linked one: {}",
        error.message
    );
}

#[test]
fn allow_extra_waives_the_unlinked_half_and_not_the_mismatch_half() {
    // The asymmetry is the point: a subset link is a deliberate act, but no
    // flag lets an object through that the lock does not pin at its own hash.
    let header = header_for(UNIT);
    let mut objects = BTreeMap::new();
    objects.insert(
        "other_unit".to_string(),
        LockEntry {
            object_hash: "ab".to_string(),
            source: String::new(),
        },
    );
    let lock = LockFile {
        schema_version: LOCK_SCHEMA_VERSION,
        objects,
    };

    // `cat_unit` is linked and unpinned, so both directions are wrong here.
    let strict = check_lock(&lock, &[header.clone()], false, "l.lock").expect_err("refused");
    assert_eq!(strict.code, E_LINK_LOCK_MISMATCH);
    let waived = check_lock(&lock, &[header], true, "l.lock").expect_err("still refused");
    assert_eq!(waived.code, E_LINK_LOCK_MISMATCH);
}

#[test]
fn a_pinned_unit_that_was_not_linked_is_named() {
    let mut objects = BTreeMap::new();
    objects.insert(
        "absent_unit".to_string(),
        LockEntry {
            object_hash: "ab".to_string(),
            source: String::new(),
        },
    );
    let lock = LockFile {
        schema_version: LOCK_SCHEMA_VERSION,
        objects,
    };
    let error = check_lock(&lock, &[], false, "l.lock").expect_err("refused");
    assert_eq!(error.code, E_LINK_LOCK_UNLINKED);
    assert!(
        error.message.contains("absent_unit"),
        "the message must name the pin nothing linked: {}",
        error.message
    );
    assert!(check_lock(&lock, &[], true, "l.lock").is_ok());
}

// ----------------------------------------------------------------------------
// The written bytes
// ----------------------------------------------------------------------------

#[test]
fn the_written_bytes_are_a_function_of_the_linked_set_alone() {
    let one = header_for(UNIT);
    let two = header_for(&UNIT.replace("cat_unit", "svc_unit"));
    let sources = BTreeMap::new();

    let forward = lock_bytes(&[one.clone(), two.clone()], &sources).expect("bytes");
    let reversed = lock_bytes(&[two, one], &sources).expect("bytes");
    assert_eq!(forward, reversed, "header order must not reach a byte");

    let text = String::from_utf8(forward).expect("utf-8");
    assert!(text.ends_with('\n'), "a committed file ends in a newline: {text:?}");
    assert!(
        text.find("cat_unit").expect("present") < text.find("svc_unit").expect("present"),
        "units are written in name order: {text}"
    );
    assert!(
        text.contains("\"source\": \"\""),
        "an unannotated pin still carries the slot a note goes in: {text}"
    );
}

#[test]
fn a_lock_source_is_recorded_only_for_a_unit_that_was_linked() {
    let header = header_for(UNIT);
    let mut sources = BTreeMap::new();
    sources.insert("cat_unit".to_string(), "an artifact store".to_string());
    sources.insert("never_linked".to_string(), "nothing to annotate".to_string());

    let text = String::from_utf8(lock_bytes(&[header], &sources).expect("bytes")).expect("utf-8");
    assert!(text.contains("an artifact store"), "the note must be recorded: {text}");
    assert!(
        !text.contains("never_linked"),
        "a note for a unit nobody linked has nothing to attach to: {text}"
    );
}
