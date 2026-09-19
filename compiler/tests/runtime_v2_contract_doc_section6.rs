// SPDX-License-Identifier: BUSL-1.1

//! configflux-fa0o: pins section 6 of `docs/runtime-v2-contract.md` to the
//! shipped v2 request/response structs in `compiler/src/runtime_api/contracts.rs`.
//!
//! Section 6 froze shapes that never shipped. It named `scope` where the struct
//! has `scope_root`, listed dirty metadata flat where the result nests it,
//! carried `actor` per write where it lives on the request, and flattened the
//! configuration identity block. Nothing executed the document, so an integrator
//! shaping a request from it got a rejection or a silently wrong call.
//!
//! The document is the input here, not a copy of it: every example is extracted
//! from `runtime-v2-contract.md` itself and fed through the real serializer, so
//! the shapes cannot rotate in code without the doc going red, or in the doc
//! without the code agreeing.
//!
//! # Why a value round-trip rather than a parse
//!
//! Five contract structs carry `#[serde(deny_unknown_fields)]`: the two
//! compare-and-swap requests, so an expectation aimed at the other operation is
//! refused rather than dropped (configflux-8gah), and the three write shapes a
//! caller authors by hand, so one nested a level too deep is too
//! (configflux-8zcp). Every other struct silently drops a field it does not have.
//! It is therefore a *value* round-trip: documented JSON, deserialized into the
//! struct and serialized back, must equal the documented JSON. A documented
//! field the struct lacks disappears on the way back, and a struct field the
//! document omits appears. Either way the two values differ and the test fails
//! naming the operation. (For the five strict types a documented field the
//! struct lacks fails earlier, on the way in — a stricter report of the same
//! fault, not a gap.)
//!
//! # The one elision
//!
//! Every v2 *request* embeds a whole `RuntimeSnapshot` — required, not skippable,
//! and roughly two dozen serialized fields of session state. Spelling one into a
//! contract document would bury the operation's own fields, so the document
//! elides it and says so. This test injects [`minimal_snapshot`] under that one
//! key before deserializing a request and drops it again before comparing.
//! Responses need no such handling: their `runtime_snapshot` is optional and
//! skipped when absent.

use compiler::loader_api::UnsatCore;
use compiler::product_api::{Diagnostic, OperationStatus, PRODUCT_SCHEMA_VERSION};
use compiler::runtime_api::{
    AtomicParameterWrite, AutoResetPathPolicy, AutoResetPolicy, CheckForUpdatesRequest,
    CheckForUpdatesResult, CommitConfigurationRequest, CommitConfigurationResult,
    DirtyEntryMetadata, ExportPendingSyncBundleRequest, ExportPendingSyncBundleResult,
    GetAutoResetPolicyRequest, GetAutoResetPolicyResult, GetConfigurationIdentityRequest,
    GetConfigurationIdentityResult, GetDirtyMetadataRequest, GetDirtyMetadataResult,
    GetSyncStatusRequest, GetSyncStatusResult, ListDirtyParametersRequest,
    ListDirtyParametersResult, OfflineReconciliationBundle, PullUpdateWrite, PullUpdatesRequest,
    PullUpdatesResult, PushAuditEventsRequest, PushAuditEventsResult, RollbackDirtyRequest,
    RollbackDirtyResult, RuntimeAuditEvent, RuntimeConfigurationIdentity, RuntimeDeltaManifest,
    RuntimeDeltaPathChange, RuntimeEvent, RuntimeEventPayload, RuntimeExplainRejectionRequest,
    RuntimeExplainRejectionResult, RuntimeSyncStatus,
    SetAutoResetPolicyRequest, SetAutoResetPolicyResult, SetParametersAtomicallyRequest,
    SetParametersAtomicallyResult, SubscribeEventsRequest, SubscribeEventsResult,
};
use compiler::runtime_api::{
    commit_configuration, get_configuration_identity, pull_updates, set_parameters_atomically,
    E_RUNTIME_COMMIT_BASE_MISMATCH, E_RUNTIME_DIRTY_INVALID, E_RUNTIME_SYNC_BASE_MISMATCH,
    E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH, E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED,
    E_RUNTIME_SYNC_TARGET_HASH_MISMATCH,
};
use compiler::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

/// The contract document, compiled in so the test binary carries the exact bytes
/// under review. `compile_data` in the BUILD target supplies it; mirrors the
/// `include_str!` idiom in `runtime_v2_lineage_doc.rs`.
const CONTRACT_DOC: &str = include_str!("../../docs/runtime-v2-contract.md");

const SECTION_HEADING: &str = "## 6. Operation Contracts (v2 Additions)";

const SUBSECTION_6_1_HEADING: &str = "### 6.1 `set_parameters_atomically`";

const SUBSECTION_6_5_HEADING: &str = "### 6.5 `commit_configuration`";

const SUBSECTION_6_8_HEADING: &str = "### 6.8 Sync Operations";

/// The smallest `RuntimeSnapshot` that deserializes: every other field of that
/// struct carries a serde default. Substituted for the key section 6 elides from
/// its request examples.
const SNAPSHOT_KEY: &str = "runtime_snapshot";

fn minimal_snapshot() -> Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": "0000000000000000000000000000000000000000000000000000000000000000",
        "resolve_hash": "1111111111111111111111111111111111111111111111111111111111111111",
        "scope": "all",
        "resolved_output": {},
    })
}

/// Which side of an operation a documented example describes. Only a request
/// gets the elided snapshot injected, so the marker word is load-bearing rather
/// than decorative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Request,
    Response,
    Nested,
}

impl Side {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "Request" => Some(Side::Request),
            "Response" => Some(Side::Response),
            "Nested" => Some(Side::Nested),
            _ => None,
        }
    }
}

/// One documented example: the payload type it claims to describe, which side of
/// the operation it is, and the JSON body of the fence beneath it.
struct DocumentedExample {
    side: Side,
    type_name: String,
    body: String,
}

/// Return the text of section 6, from its heading to the start of section 7.
///
/// Fails closed: a renamed or removed heading is a contract change that must not
/// silently disable the pinning.
fn section_6() -> &'static str {
    let start = CONTRACT_DOC
        .find(SECTION_HEADING)
        .unwrap_or_else(|| panic!("runtime-v2-contract.md must still contain '{SECTION_HEADING}'"));
    let rest = &CONTRACT_DOC[start + SECTION_HEADING.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}

/// Return the text of one `### 6.N` subsection, from its heading to the start
/// of the next one.
///
/// Fails closed on a renamed heading, like [`section_6`]: a pin scoped to a
/// subsection must not silently evaporate when that subsection is retitled.
fn subsection(heading: &str) -> &'static str {
    let section = section_6();
    let start = section
        .find(heading)
        .unwrap_or_else(|| panic!("runtime-v2-contract.md must still contain '{heading}'"));
    let rest = &section[start + heading.len()..];
    let end = rest.find("\n### ").unwrap_or(rest.len());
    &rest[..end]
}

/// Return the text of subsection 6.8, from its heading to the start of 6.9.
///
/// The `pull_updates` pins below stay scoped to this one subsection because
/// they assert against the *format gate* the runtime applies to four request
/// fields, and because one of them hands 6.8's own request to the shipped
/// `pull_updates`. That is a narrower claim than the one
/// [`section_6_hash_and_configuration_id_literals_are_sha256_hex`] makes over
/// the whole section, which asserts against what the runtime *emits*. The two
/// overlap here by design: 6.8 is both gated on input and hashed on output.
fn section_6_8() -> &'static str {
    subsection(SUBSECTION_6_8_HEADING)
}

/// Read the payload type name out of an example's introducing line.
///
/// The line reads `Request (\`SetParametersAtomicallyRequest\`):` — the marker
/// word names the side, the backticked identifier names the shipped struct.
fn introducing_line(line: &str) -> Option<(Side, String)> {
    let (word, rest) = line.split_once(" (`")?;
    let side = Side::parse(word)?;
    let (type_name, _) = rest.split_once("`)")?;
    if type_name.is_empty() {
        return None;
    }
    Some((side, type_name.to_string()))
}

/// Extract every documented example in section 6, in document order.
///
/// An example is an introducing line followed by a ```json fence. A fence with
/// no introducing line above it is ignored, so prose snippets elsewhere in the
/// section stay free-form.
fn documented_examples() -> Vec<DocumentedExample> {
    documented_examples_in(section_6())
}

/// [`documented_examples`] over an arbitrary slice of the document, so a pin
/// that applies to one subsection can be scoped to it.
fn documented_examples_in(section: &str) -> Vec<DocumentedExample> {
    let mut examples = Vec::new();
    let mut pending: Option<(Side, String)> = None;
    let mut open: Option<DocumentedExample> = None;

    for line in section.lines() {
        match (line.strip_prefix("```"), open.as_mut()) {
            (Some(info), None) => {
                let info = info.trim();
                if let Some((side, type_name)) = pending.take() {
                    assert_eq!(
                        info, "json",
                        "the fence documenting `{type_name}` must be a ```json fence, not ```{info}"
                    );
                    open = Some(DocumentedExample {
                        side,
                        type_name,
                        body: String::new(),
                    });
                }
            }
            (Some(_), Some(_)) => examples.push(open.take().expect("fence is open")),
            (None, Some(block)) => {
                if !block.body.is_empty() {
                    block.body.push('\n');
                }
                block.body.push_str(line);
            }
            (None, None) => {
                if let Some(found) = introducing_line(line) {
                    pending = Some(found);
                }
            }
        }
    }

    assert!(open.is_none(), "section 6 has an unterminated code fence");
    examples
}

/// Deserialize a documented example into the shipped struct and serialize it
/// back, asserting the two values agree.
///
/// Both failure directions land here: a field the document invents is dropped by
/// the deserializer and missing from the round-trip, and a field the struct
/// serializes unconditionally appears in the round-trip and is missing from the
/// document.
fn round_trip<T>(example: &DocumentedExample)
where
    T: Serialize + DeserializeOwned,
{
    let type_name = &example.type_name;
    let documented: Value = serde_json::from_str(&example.body).unwrap_or_else(|error| {
        panic!("section 6's `{type_name}` example must be valid JSON: {error}")
    });
    let documented_object = documented.as_object().unwrap_or_else(|| {
        panic!("section 6's `{type_name}` example must be a JSON object")
    });

    let mut input = documented_object.clone();
    if example.side == Side::Request {
        assert!(
            !input.contains_key(SNAPSHOT_KEY),
            "section 6's `{type_name}` example must elide `{SNAPSHOT_KEY}` — every v2 request \
             carries the snapshot the previous response returned, and spelling one out would \
             bury the operation's own fields"
        );
        input.insert(SNAPSHOT_KEY.to_string(), minimal_snapshot());
    }

    let parsed: T = serde_json::from_value(Value::Object(input)).unwrap_or_else(|error| {
        panic!(
            "section 6's `{type_name}` example must deserialize into the shipped struct \
             of that name: {error}"
        )
    });

    let mut round_tripped: Map<String, Value> = match serde_json::to_value(&parsed) {
        Ok(Value::Object(map)) => map,
        other => panic!("`{type_name}` must serialize to a JSON object, got {other:?}"),
    };
    if example.side == Side::Request {
        round_tripped.remove(SNAPSHOT_KEY);
    }

    assert_eq!(
        &Value::Object(round_tripped),
        &documented,
        "section 6's `{type_name}` example must round-trip through the shipped struct \
         unchanged. A field the document names but the struct does not have is dropped here; \
         a field the struct always serializes but the document omits appears here. Correct the \
         document to the shipped surface — the structs in compiler/src/runtime_api/contracts.rs \
         are what the runtime accepts"
    );
}

/// Dispatch one documented example to the shipped struct it names.
///
/// An unrecognized name fails closed: a document that invents a payload type,
/// or renames one out from under this list, must not pass silently.
fn check(example: &DocumentedExample) {
    match example.type_name.as_str() {
        "SetParametersAtomicallyRequest" => round_trip::<SetParametersAtomicallyRequest>(example),
        "SetParametersAtomicallyResult" => round_trip::<SetParametersAtomicallyResult>(example),
        "AtomicParameterWrite" => round_trip::<AtomicParameterWrite>(example),
        "ListDirtyParametersRequest" => round_trip::<ListDirtyParametersRequest>(example),
        "ListDirtyParametersResult" => round_trip::<ListDirtyParametersResult>(example),
        "GetDirtyMetadataRequest" => round_trip::<GetDirtyMetadataRequest>(example),
        "GetDirtyMetadataResult" => round_trip::<GetDirtyMetadataResult>(example),
        "DirtyEntryMetadata" => round_trip::<DirtyEntryMetadata>(example),
        "RollbackDirtyRequest" => round_trip::<RollbackDirtyRequest>(example),
        "RollbackDirtyResult" => round_trip::<RollbackDirtyResult>(example),
        "CommitConfigurationRequest" => round_trip::<CommitConfigurationRequest>(example),
        "CommitConfigurationResult" => round_trip::<CommitConfigurationResult>(example),
        "RuntimeDeltaPathChange" => round_trip::<RuntimeDeltaPathChange>(example),
        "RuntimeDeltaManifest" => round_trip::<RuntimeDeltaManifest>(example),
        "GetConfigurationIdentityRequest" => round_trip::<GetConfigurationIdentityRequest>(example),
        "GetConfigurationIdentityResult" => round_trip::<GetConfigurationIdentityResult>(example),
        "RuntimeConfigurationIdentity" => round_trip::<RuntimeConfigurationIdentity>(example),
        "SetAutoResetPolicyRequest" => round_trip::<SetAutoResetPolicyRequest>(example),
        "SetAutoResetPolicyResult" => round_trip::<SetAutoResetPolicyResult>(example),
        "GetAutoResetPolicyRequest" => round_trip::<GetAutoResetPolicyRequest>(example),
        "GetAutoResetPolicyResult" => round_trip::<GetAutoResetPolicyResult>(example),
        "AutoResetPolicy" => round_trip::<AutoResetPolicy>(example),
        "AutoResetPathPolicy" => round_trip::<AutoResetPathPolicy>(example),
        "CheckForUpdatesRequest" => round_trip::<CheckForUpdatesRequest>(example),
        "CheckForUpdatesResult" => round_trip::<CheckForUpdatesResult>(example),
        "PullUpdatesRequest" => round_trip::<PullUpdatesRequest>(example),
        "PullUpdateWrite" => round_trip::<PullUpdateWrite>(example),
        "PullUpdatesResult" => round_trip::<PullUpdatesResult>(example),
        "GetSyncStatusRequest" => round_trip::<GetSyncStatusRequest>(example),
        "GetSyncStatusResult" => round_trip::<GetSyncStatusResult>(example),
        "RuntimeSyncStatus" => round_trip::<RuntimeSyncStatus>(example),
        "SubscribeEventsRequest" => round_trip::<SubscribeEventsRequest>(example),
        "SubscribeEventsResult" => round_trip::<SubscribeEventsResult>(example),
        "RuntimeEvent" => round_trip::<RuntimeEvent>(example),
        "RuntimeEventPayload" => round_trip::<RuntimeEventPayload>(example),
        "PushAuditEventsRequest" => round_trip::<PushAuditEventsRequest>(example),
        "PushAuditEventsResult" => round_trip::<PushAuditEventsResult>(example),
        "ExportPendingSyncBundleRequest" => round_trip::<ExportPendingSyncBundleRequest>(example),
        "ExportPendingSyncBundleResult" => round_trip::<ExportPendingSyncBundleResult>(example),
        "OfflineReconciliationBundle" => round_trip::<OfflineReconciliationBundle>(example),
        "RuntimeAuditEvent" => round_trip::<RuntimeAuditEvent>(example),
        // configflux-n94v: section 6.13, the `explain_rejection` contract. Its
        // result is the one envelope ADR-0031 D2 froze without `resolve_hash`,
        // and `UnsatCore` is the ADR-0031 D3 payload section 6.12 refers to
        // without showing — both now round-trip through the shipped structs.
        "RuntimeExplainRejectionRequest" => round_trip::<RuntimeExplainRejectionRequest>(example),
        "RuntimeExplainRejectionResult" => round_trip::<RuntimeExplainRejectionResult>(example),
        "UnsatCore" => round_trip::<UnsatCore>(example),
        other => panic!(
            "section 6 documents `{other}`, which this test does not know how to check. \
             Add an arm binding it to the shipped struct of that name"
        ),
    }
}

/// Every payload type section 6 must document, in document order. Deleting an
/// example, or adding one this list does not expect, fails here rather than
/// quietly shrinking the guarded surface.
const EXPECTED_TYPES: &[&str] = &[
    "SetParametersAtomicallyRequest",
    "AtomicParameterWrite",
    "SetParametersAtomicallyResult",
    "ListDirtyParametersRequest",
    "ListDirtyParametersResult",
    "GetDirtyMetadataRequest",
    "GetDirtyMetadataResult",
    "DirtyEntryMetadata",
    "RollbackDirtyRequest",
    "RollbackDirtyResult",
    "CommitConfigurationRequest",
    "CommitConfigurationResult",
    "RuntimeDeltaPathChange",
    "RuntimeDeltaManifest",
    "GetConfigurationIdentityRequest",
    "GetConfigurationIdentityResult",
    "RuntimeConfigurationIdentity",
    "SetAutoResetPolicyRequest",
    "SetAutoResetPolicyResult",
    "GetAutoResetPolicyRequest",
    "GetAutoResetPolicyResult",
    "AutoResetPolicy",
    "AutoResetPathPolicy",
    "CheckForUpdatesRequest",
    "CheckForUpdatesResult",
    "PullUpdatesRequest",
    "PullUpdateWrite",
    "PullUpdatesResult",
    "GetSyncStatusRequest",
    "GetSyncStatusResult",
    "RuntimeSyncStatus",
    "SubscribeEventsRequest",
    "SubscribeEventsResult",
    "RuntimeEvent",
    "RuntimeEventPayload",
    "PushAuditEventsRequest",
    "PushAuditEventsResult",
    "ExportPendingSyncBundleRequest",
    "ExportPendingSyncBundleResult",
    "OfflineReconciliationBundle",
    "RuntimeAuditEvent",
    "RuntimeExplainRejectionRequest",
    "RuntimeExplainRejectionResult",
    "UnsatCore",
];

#[test]
fn section_6_documents_every_shipped_v2_payload_type() {
    let found: Vec<String> = documented_examples()
        .into_iter()
        .map(|example| example.type_name)
        .collect();
    assert_eq!(
        found, EXPECTED_TYPES,
        "section 6 must carry one fenced json example per shipped v2 payload type, introduced \
         by a `Request (`Type`)` / `Response (`Type`)` / `Nested (`Type`)` line, in this order"
    );
}

#[test]
fn section_6_examples_round_trip_through_the_shipped_structs() {
    let examples = documented_examples();
    assert!(
        !examples.is_empty(),
        "section 6 must document the shipped request and response shapes with fenced json \
         examples; found none"
    );
    for example in &examples {
        check(example);
    }
}

#[test]
fn section_6_examples_pin_the_current_product_schema_version() {
    for example in documented_examples() {
        let documented: Value = serde_json::from_str(&example.body)
            .expect("checked for validity by the round-trip test");
        let Some(version) = documented.get("schema_version") else {
            continue;
        };
        assert_eq!(
            version,
            &Value::from(PRODUCT_SCHEMA_VERSION),
            "section 6's `{}` example must state the current product schema version \
             ({PRODUCT_SCHEMA_VERSION}); the runtime rejects any other value with \
             E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION",
            example.type_name
        );
    }
}

/// The scope-qualification convention section 6 now states in prose has to be
/// visible in the examples themselves, or a reader copying one still builds the
/// wrong path. Response path lists are canonical (`<scope_root>/<path>`);
/// `rejected_paths` echoes the caller's raw supplied path.
#[test]
fn section_6_examples_show_scope_qualified_response_paths() {
    let examples = documented_examples();
    let by_name = |name: &str| -> String {
        examples
            .iter()
            .find(|example| example.type_name == name)
            .unwrap_or_else(|| panic!("section 6 must document `{name}`"))
            .body
            .clone()
    };

    for (name, field) in [
        ("ListDirtyParametersResult", "dirty_paths"),
        ("RollbackDirtyResult", "rolled_back_paths"),
    ] {
        let documented: Value =
            serde_json::from_str(&by_name(name)).expect("valid JSON, per the round-trip test");
        let paths = documented[field]
            .as_array()
            .unwrap_or_else(|| panic!("`{name}.{field}` must be an array"));
        assert!(
            !paths.is_empty(),
            "`{name}.{field}` must show at least one path, or the scope-qualified form is \
             not actually demonstrated"
        );
        for path in paths {
            let path = path.as_str().unwrap_or_else(|| panic!("`{name}.{field}` holds strings"));
            assert!(
                path.contains('/'),
                "`{name}.{field}` entries are scope-qualified as `<scope_root>/<path>` \
                 (canonical_runtime_path); got '{path}'"
            );
        }
    }

    let rejection: Value = serde_json::from_str(&by_name("SetParametersAtomicallyResult"))
        .expect("valid JSON, per the round-trip test");
    let rejected = rejection["rejected_paths"]
        .as_array()
        .expect("`rejected_paths` must be an array");
    assert!(
        !rejected.is_empty(),
        "section 6 must document the REJECTION response of set_parameters_atomically — the \
         all-or-nothing shape is the one an integrator gets wrong, and an empty \
         `rejected_paths` demonstrates nothing about which path form it carries"
    );
    for path in rejected {
        let path = path.as_str().expect("`rejected_paths` holds strings");
        assert!(
            !path.contains('/'),
            "`rejected_paths` echoes the caller's raw supplied path, not the scope-qualified \
             form the response's other path lists use; got '{path}'"
        );
    }
}

/// The `pull_updates` payload fields the runtime hands to `is_sha256_hex`
/// (compiler/src/runtime_api/shared_ops.rs), named as they appear on the wire.
/// Each is optional; the gate applies to a value that is present and non-blank,
/// and rejects anything that is not exactly 64 lowercase ASCII hex characters
/// with `E_RUNTIME_SYNC_PAYLOAD_INVALID`.
const SHA256_HEX_FIELDS: &[&str] = &[
    "base_configuration_id",
    "target_configuration_id",
    "before_leaf_hash",
    "after_leaf_hash",
];

/// Collect every string in `value` whose field name satisfies `wanted`, at any
/// depth, tagged with the path that reaches it. Examples nest the interesting
/// fields inside `writes[]`, `identity`, `delta_manifest`, `changed_paths[]`,
/// `bundle` and `pending_audit_events[]`, so a top-level scan would miss most
/// of the literals.
fn literals_where(
    value: &Value,
    path: &str,
    wanted: &dyn Fn(&str) -> bool,
    found: &mut Vec<(String, String)>,
) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let child_path = format!("{path}.{key}");
                if wanted(key.as_str()) {
                    if let Some(text) = child.as_str() {
                        found.push((child_path.clone(), text.to_string()));
                    }
                }
                literals_where(child, &child_path, wanted, found);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                literals_where(item, &format!("{path}[{index}]"), wanted, found);
            }
        }
        _ => {}
    }
}

/// Collect every [`SHA256_HEX_FIELDS`] string in `value`, at any depth.
fn sha256_hex_literals(value: &Value, path: &str, found: &mut Vec<(String, String)>) {
    literals_where(
        value,
        path,
        &|key| SHA256_HEX_FIELDS.contains(&key),
        found,
    );
}

/// Digest-bearing wire fields whose names match neither suffix below, so the
/// suffix rule on its own cannot see them.
///
/// `manifest_id` is the one section 6 shows. `commit_configuration` derives it
/// with `stable_hash` over the schema version, the journal sequence, both
/// configuration ids, the change set, the actor and the reason
/// (`compiler/src/runtime_api/commit_identity_ops.rs`), and `stable_hash` is
/// `sha256_hex` over the serialized payload
/// (`compiler/src/runtime_api/shared_ops.rs`) — so the value is 64 lowercase hex
/// characters exactly like a suffix-matched field. A list rather than a one-off
/// disjunction: the next field in this position costs one line.
const EXTRA_DIGEST_FIELDS: &[&str] = &["manifest_id"];

/// Whether a wire field name carries a hash or a configuration id.
///
/// Every such field in `compiler/src/runtime_api/contracts.rs` ends in one of
/// these two suffixes, and no field that ends in either carries anything else —
/// so the suffix rule names exactly the fourteen of them (`model_hash`,
/// `resolve_hash`, `diff_hash`, `dirty_diff_hash`, `before_leaf_hash`,
/// `after_leaf_hash`, `old_value_hash`, `new_value_hash`,
/// `base_configuration_id`, `target_configuration_id`,
/// `expected_base_configuration_id`, `committed_configuration_id`,
/// `working_configuration_id`, `expected_working_configuration_id`).
/// [`EXTRA_DIGEST_FIELDS`] carries the digest-bearing names it misses.
///
/// It deliberately excludes the section's identifiers that are *not* digests —
/// `commit_id`, `bundle_id`, `event_id`, `audit_event_id` — each of which is a
/// sequence or a truncated hash behind a prefix, shown in the derived form its
/// own subsection states.
fn is_hash_bearing_field(key: &str) -> bool {
    key.ends_with("_hash")
        || key.ends_with("_configuration_id")
        || EXTRA_DIGEST_FIELDS.contains(&key)
}

/// configflux-r6ag: section 6.8's own example values must survive the format
/// gate section 6.8 documents.
///
/// They did not. The ids read `cc-88b2` and `cc-9d41` and both leaf hashes were
/// eight hex characters, so the documented request, submitted verbatim, came
/// back `E_RUNTIME_SYNC_PAYLOAD_INVALID` rather than performing the operation
/// the example describes. Round-tripping the example through the struct cannot
/// catch that: every one of these fields is a plain `Option<String>`, and the
/// serializer is indifferent to what the string says.
#[test]
fn section_6_8_hash_literals_are_sha256_hex() {
    let mut checked = 0usize;
    for example in documented_examples_in(section_6_8()) {
        let documented: Value = serde_json::from_str(&example.body)
            .expect("checked for validity by the round-trip test");
        let mut found = Vec::new();
        sha256_hex_literals(&documented, &example.type_name, &mut found);
        for (path, literal) in found {
            assert!(
                literal.len() == 64 && literal.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f')),
                "section 6.8's `{path}` is '{literal}', which the shipped runtime refuses: \
                 every configuration id and leaf hash a pull_updates payload carries must be \
                 exactly 64 lowercase ASCII hex characters (is_sha256_hex in \
                 compiler/src/runtime_api/shared_ops.rs). An example the binary rejects \
                 teaches the wrong request"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "section 6.8 must keep showing both configuration ids and both leaf hashes on the \
         pull_updates request, on its nested write and on its result; found only {checked} \
         such literals, so the format rule is no longer demonstrated anywhere"
    );
}

/// configflux-r6ag: the codes section 6.8's prose cites must be the codes the
/// shipped runtime emits.
///
/// Imported from the source constants rather than spelled out here, so renaming
/// one in `contracts.rs` or `sync_transport.rs` turns the document red. Section
/// 6.8 named only the two an integrator meets last; a precondition the document
/// does not mention is one an integrator discovers by being refused.
#[test]
fn section_6_8_names_the_shipped_sync_precondition_codes() {
    let section = section_6_8();
    for code in [
        E_RUNTIME_SYNC_PAYLOAD_INVALID,
        E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED,
        E_RUNTIME_SYNC_BASE_MISMATCH,
        E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH,
        E_RUNTIME_SYNC_TARGET_HASH_MISMATCH,
    ] {
        assert!(
            section.contains(code),
            "section 6.8 must name `{code}`: pull_updates rejects a request with it, and the \
             section is the only place an integrator shaping that request will look"
        );
    }
}

/// configflux-r6ag: submit the documented request to the shipped `pull_updates`
/// rather than only reading it.
///
/// The two tests above pin the document against the *rule*; this one pins it
/// against the *binary*, which is how the defect was found — the reporter
/// copied section 6.8's request, added the snapshot the section says to add,
/// and was refused. Under [`minimal_snapshot`] the documented ids cannot be
/// this runtime's own, so the request is expected to fail; what matters is
/// which gate it fails at. Reaching `E_RUNTIME_SYNC_BASE_MISMATCH` — the
/// equality check on `base_configuration_id` — proves it cleared the format
/// check that runs immediately before it, and that no earlier precondition
/// (schema version, actor, transport) turned this into a vacuous pass.
#[test]
fn section_6_8_documented_pull_updates_request_clears_the_format_gate() {
    let example = documented_examples_in(section_6_8())
        .into_iter()
        .find(|example| example.type_name == "PullUpdatesRequest")
        .expect("section 6.8 must document `PullUpdatesRequest`");
    let mut input: Map<String, Value> = serde_json::from_str(&example.body)
        .expect("checked for validity by the round-trip test");
    input.insert(SNAPSHOT_KEY.to_string(), minimal_snapshot());
    let request: PullUpdatesRequest = serde_json::from_value(Value::Object(input))
        .expect("checked by the round-trip test");

    let result = pull_updates(request);
    let codes: Vec<&str> = result
        .diagnostics
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec![E_RUNTIME_SYNC_BASE_MISMATCH],
        "the request section 6.8 documents must reach the base-configuration equality check \
         when the shipped `pull_updates` is handed it. `{E_RUNTIME_SYNC_PAYLOAD_INVALID}` here \
         means a documented id or leaf hash is not 64 lowercase ASCII hex characters and the \
         runtime refused the payload before reading it; any other code means an earlier \
         precondition the section does not describe rejected the example first"
    );
}

/// The subsections whose examples must keep demonstrating the digest form, and
/// how many hash-bearing literals each one has to carry.
///
/// A minimum rather than an equality: adding an example is fine, quietly
/// deleting the fields that make the rule visible is not. Without this arm the
/// pin below is vacuously satisfiable by removing every literal it checks.
const HASH_BEARING_COVERAGE: &[(&str, usize)] = &[
    (SUBSECTION_6_1_HEADING, 3),
    (SUBSECTION_6_5_HEADING, 19),
    ("### 6.6 `get_configuration_identity`", 10),
    (SUBSECTION_6_8_HEADING, 14),
    ("### 6.9 Event Subscription", 6),
    ("### 6.11 `export_pending_sync_bundle`", 12),
];

/// configflux-ah5s: every hash and configuration id section 6 shows must be in
/// the form the runtime actually produces.
///
/// `configflux-r6ag` fixed this for 6.8's four `pull_updates` fields, which the
/// runtime format-gates on input, and scoped its pin there. The remaining
/// sixty-six literals were short placeholders — `cc-31f0`, `wc-7f3a`,
/// `9f2c4b1e`, `6b0e1d77` — spread across 6.1, 6.5, 6.6, 6.9 and 6.11.
///
/// Nothing rejects them, which is exactly why they survived: they are response
/// fields, and the round-trip guard above is indifferent to what an
/// `Option<String>` says. But every one of them is produced by `sha256_hex`,
/// `stable_hash`, `root_hash_from_leaves`, `identity_leaf_hash` or `value_hash`
/// (compiler/src/runtime_api/shared_ops.rs), each of which formats a full
/// SHA-256 as 64 lowercase hex characters. A reader who takes an abbreviated id
/// for the real thing writes a consumer that never matches a live value, and —
/// for `commit_configuration.expected_base_configuration_id` and
/// `set_parameters_atomically.expected_working_configuration_id`, which are
/// compare-and-swap inputs — one whose writes are refused on every call.
///
/// Lowercase is asserted, not merely hex, because the gate itself now requires
/// it: `is_sha256_hex` accepts `0-9a-f` only (configflux-07t1), so an uppercase
/// example is a value the runtime refuses, not merely one no session carries.
///
/// configflux-x1yz: `delta_manifest.manifest_id` is in scope here too, via
/// [`EXTRA_DIGEST_FIELDS`]. ah5s left it out because the suffix rule cannot see
/// it, and 6.5 read `delta-0000000000000007` as a result — a counter, in the
/// shape of the three identifiers beside it that genuinely are sequences, for a
/// field the runtime content-addresses. The coverage minimum for 6.5 and the
/// total below count its two literals, so removing them fails here rather than
/// silently narrowing the rule back to where it started.
#[test]
fn section_6_hash_and_configuration_id_literals_are_sha256_hex() {
    let mut checked = 0usize;
    for example in documented_examples() {
        let documented: Value = serde_json::from_str(&example.body)
            .expect("checked for validity by the round-trip test");
        let mut found = Vec::new();
        literals_where(&documented, &example.type_name, &is_hash_bearing_field, &mut found);
        for (path, literal) in found {
            assert!(
                literal.len() == 64
                    && literal
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch)),
                "section 6's `{path}` is '{literal}'. Every hash and configuration id the \
                 runtime emits is a full SHA-256 rendered as 64 lowercase hex characters; an \
                 abbreviated one is a value no session can produce, and for the two \
                 compare-and-swap request fields it is one every write would be refused on"
            );
            checked += 1;
        }
    }

    for (heading, minimum) in HASH_BEARING_COVERAGE {
        let mut found = Vec::new();
        for example in documented_examples_in(subsection(heading)) {
            let documented: Value = serde_json::from_str(&example.body)
                .expect("checked for validity by the round-trip test");
            literals_where(&documented, &example.type_name, &is_hash_bearing_field, &mut found);
        }
        assert!(
            found.len() >= *minimum,
            "'{heading}' must keep showing at least {minimum} hash or configuration-id values; \
             found {}. Dropping them would leave the digest form undemonstrated in the one \
             subsection an integrator reads for it",
            found.len()
        );
    }

    assert!(
        checked >= 76,
        "section 6 must keep showing at least 76 hash and configuration-id literals across its \
         examples; found only {checked}, so this pin no longer covers the surface it was \
         written for"
    );
}

/// The identity a session built on [`minimal_snapshot`] actually carries, read
/// out of the shipped `get_configuration_identity`.
///
/// Two jobs below, both load-bearing. It is the vacuity guard on the snapshot
/// precondition the two pins share: `commit_configuration` and
/// `set_parameters_atomically` each run `validate_runtime_snapshot` before they
/// compute the identity they compare, so a snapshot this operation accepts is
/// one that cannot be what either of them refused. And it supplies the ids the
/// documented literals are asserted to differ from. `minimal_snapshot` carries
/// an empty `resolved_output`, so the identity hashes zero leaves and cannot be
/// a real session's — but that is a property of today's defaults rather than a
/// guarantee, and a pin that assumed it would pass for the wrong reason on the
/// day it stopped holding.
fn minimal_snapshot_identity() -> RuntimeConfigurationIdentity {
    let mut input = Map::new();
    input.insert("schema_version".to_string(), Value::from(PRODUCT_SCHEMA_VERSION));
    input.insert(SNAPSHOT_KEY.to_string(), minimal_snapshot());
    let request: GetConfigurationIdentityRequest = serde_json::from_value(Value::Object(input))
        .expect("`GetConfigurationIdentityRequest` carries the schema version and the snapshot");

    let result = get_configuration_identity(request);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "`minimal_snapshot` must be a snapshot the runtime accepts, or the two pins below \
         prove nothing about the fields they name: both operations validate the snapshot \
         before computing an identity, so a rejection here is the failure they would observe. \
         Got {:?}",
        result.diagnostics.diagnostics
    );
    result
        .identity
        .expect("`get_configuration_identity` carries the identity on an ok result")
}

/// One documented request, as the JSON object its subsection shows, with the
/// elided snapshot injected — ready for the shipped struct.
///
/// Generalizes the extraction
/// [`section_6_8_documented_pull_updates_request_clears_the_format_gate`] does
/// inline, so the two pins below submit the document rather than a copy of it.
/// Fails closed on a subsection that stops documenting the request, or that
/// introduces it as a response: a pin that silently found nothing to submit
/// would be worse than no pin at all.
fn documented_request(heading: &str, type_name: &str) -> Map<String, Value> {
    let example = documented_examples_in(subsection(heading))
        .into_iter()
        .find(|example| example.type_name == type_name)
        .unwrap_or_else(|| panic!("'{heading}' must document `{type_name}`"));
    assert_eq!(
        example.side,
        Side::Request,
        "'{heading}' must introduce `{type_name}` as a Request: a response example is not \
         something the shipped operation can be handed"
    );
    let mut input: Map<String, Value> =
        serde_json::from_str(&example.body).expect("checked for validity by the round-trip test");
    input.insert(SNAPSHOT_KEY.to_string(), minimal_snapshot());
    input
}

/// A documented request field that has to be present, a string, and non-blank,
/// returned owned. `why` states what goes wrong when it is not, because that
/// differs field by field and is the whole reason these are checked up front.
fn documented_text(input: &Map<String, Value>, field: &str, heading: &str, why: &str) -> String {
    let value = input
        .get(field)
        .unwrap_or_else(|| panic!("'{heading}' must keep showing `{field}` on its request"))
        .as_str()
        .unwrap_or_else(|| panic!("'{heading}' must show `{field}` as a string"))
        .to_string();
    assert!(
        !value.trim().is_empty(),
        "'{heading}' must show a non-blank `{field}`: {why}"
    );
    value
}

/// Assert that a compare-and-swap rejection is the identity comparison itself,
/// and not an earlier gate that happens to share its code.
///
/// The code alone would do for `commit_configuration`, where only the comparison
/// emits `E_RUNTIME_COMMIT_BASE_MISMATCH`. It does not do for
/// `set_parameters_atomically`: `E_RUNTIME_DIRTY_INVALID` is equally what a
/// blank `actor` and an empty `writes` return, both from gates that run *before*
/// the comparison, so a documented request that quietly lost either one would
/// satisfy a code-only pin while never reaching the field under test.
/// `entity_path` carries the compare-and-swap field only on the comparison, and
/// the message has to name both the id the document supplied and the id the
/// session actually has — which is how the pin knows the comparison read the
/// identity [`minimal_snapshot_identity`] reports for the same snapshot, rather
/// than some other value.
fn assert_identity_mismatch(
    diagnostics: &[Diagnostic],
    code: &str,
    field: &str,
    documented: &str,
    current: &str,
) {
    let codes: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec![code],
        "the documented request must reach the equality check on `{field}` and be refused \
         there with `{code}`. Any other code means an earlier precondition rejected it first; \
         no code at all means the operation accepted the request, which for these two fields \
         is what a blank documented id looks like — trimmed, filtered out, and treated as no \
         expectation at all"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some(field),
        "`{code}` must be reported against `{field}`; got {:?}. The code is shared with \
         rejections that fire before the comparison, so the entity path is what tells the \
         identity check apart from them",
        diagnostic.entity_path
    );
    assert!(
        diagnostic.message.contains(documented) && diagnostic.message.contains(current),
        "the mismatch must name the id the document supplied ('{documented}') and the id the \
         session has ('{current}'); got '{}'",
        diagnostic.message
    );
}

/// configflux-3d8y: submit section 6.5's own documented request to the shipped
/// `commit_configuration`.
///
/// Section 6 documents three compare-and-swap request fields.
/// [`section_6_8_documented_pull_updates_request_clears_the_format_gate`] pins
/// one; this pins `expected_base_configuration_id` and the sibling below pins
/// `expected_working_configuration_id`. Both of those were driven against the
/// real binaries by hand during configflux-ah5s and left no guard behind, so the
/// code each returns and the ordering of the gates ahead of each comparison were
/// unpinned — a precondition inserted before the comparison, or the
/// `changed_paths_hint` check that today runs after it moved ahead of it, would
/// change where the documented request lands with nothing going red.
///
/// Under [`minimal_snapshot`] the documented id cannot be this session's, so the
/// request is expected to fail; what matters is where. Reaching
/// `E_RUNTIME_COMMIT_BASE_MISMATCH` proves the schema version, the actor and the
/// snapshot all cleared, since each of those is refused with a different code
/// before the identity is ever computed — and that the `changed_paths_hint`
/// assertion still runs after the comparison rather than before it, since it
/// refuses this same request with `E_RUNTIME_UNKNOWN_PATH`.
#[test]
fn section_6_5_documented_commit_request_reaches_the_base_identity_check() {
    let identity = minimal_snapshot_identity();
    let input = documented_request(SUBSECTION_6_5_HEADING, "CommitConfigurationRequest");
    documented_text(
        &input,
        "actor",
        SUBSECTION_6_5_HEADING,
        "`commit_configuration` refuses a blank one with `E_RUNTIME_COMMIT_INVALID` before it \
         computes any identity, so the request would never reach the comparison below",
    );
    let expected = documented_text(
        &input,
        "expected_base_configuration_id",
        SUBSECTION_6_5_HEADING,
        "the operation trims the field and treats a blank one as no expectation at all, so \
         the commit would be attempted rather than refused",
    );
    assert_ne!(
        expected, identity.committed_configuration_id,
        "section 6.5's `expected_base_configuration_id` must not be the id this snapshot \
         already has, or the comparison succeeds and this pin passes without ever exercising \
         the mismatch it was written for"
    );

    let request: CommitConfigurationRequest =
        serde_json::from_value(Value::Object(input)).expect("checked by the round-trip test");
    let result = commit_configuration(request);
    assert_identity_mismatch(
        &result.diagnostics.diagnostics,
        E_RUNTIME_COMMIT_BASE_MISMATCH,
        "request.expected_base_configuration_id",
        &expected,
        &identity.committed_configuration_id,
    );
}

/// configflux-3d8y: submit section 6.1's own documented request to the shipped
/// `set_parameters_atomically`.
///
/// The weaker of the two codes, and the reason this pin is shaped the way it is:
/// `E_RUNTIME_DIRTY_INVALID` is not the identity comparison's own. The same code
/// answers a blank `actor` and an empty `writes`, both checked ahead of the
/// comparison, so a documented request that lost either would still be refused
/// with exactly this code, and a pin that read only the code would stay green
/// while the field it names went untested. Hence the two preconditions asserted
/// against the document below, and hence [`assert_identity_mismatch`] reading
/// the entity path and both ids rather than the code alone.
///
/// The ordering is pinned from the other side too: under [`minimal_snapshot`]
/// the documented writes name a path no session has, so write validation
/// refuses this same request with `E_RUNTIME_UNKNOWN_PATH`. Moving the
/// comparison after it would surface here as that code.
#[test]
fn section_6_1_documented_atomic_request_reaches_the_working_identity_check() {
    let identity = minimal_snapshot_identity();
    let input = documented_request(SUBSECTION_6_1_HEADING, "SetParametersAtomicallyRequest");
    documented_text(
        &input,
        "actor",
        SUBSECTION_6_1_HEADING,
        "`set_parameters_atomically` refuses a blank one with the same \
         `E_RUNTIME_DIRTY_INVALID` the comparison below emits, from a gate that runs before \
         it — the one rejection this pin could mistake for the one it asserts",
    );
    let writes = input
        .get("writes")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("'{SUBSECTION_6_1_HEADING}' must show `writes` as an array"));
    assert!(
        !writes.is_empty(),
        "'{SUBSECTION_6_1_HEADING}' must show at least one write: an empty batch is refused \
         with `E_RUNTIME_DIRTY_INVALID` before the comparison, which is the second rejection \
         this pin could mistake for the one it asserts"
    );
    let expected = documented_text(
        &input,
        "expected_working_configuration_id",
        SUBSECTION_6_1_HEADING,
        "the operation trims the field and treats a blank one as no expectation at all, so \
         the batch would be attempted rather than refused",
    );
    assert_ne!(
        expected, identity.working_configuration_id,
        "section 6.1's `expected_working_configuration_id` must not be the id this snapshot \
         already has, or the comparison succeeds and this pin passes without ever exercising \
         the mismatch it was written for"
    );

    let request: SetParametersAtomicallyRequest =
        serde_json::from_value(Value::Object(input)).expect("checked by the round-trip test");
    let result = set_parameters_atomically(request);
    assert_identity_mismatch(
        &result.diagnostics.diagnostics,
        E_RUNTIME_DIRTY_INVALID,
        "request.expected_working_configuration_id",
        &expected,
        &identity.working_configuration_id,
    );
}
