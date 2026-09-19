// SPDX-License-Identifier: BUSL-1.1

// Unit tests for the routing half of configflux-8gah: which request each
// compare-and-swap expected-id field is accepted by.
//
// The two fields are named differently and live on different operations —
// `commit_configuration.expected_base_configuration_id` and
// `set_parameters_atomically.expected_working_configuration_id`. Neither request
// struct carried `#[serde(deny_unknown_fields)]`, so a field aimed at the
// operation that does not declare it was silently discarded by serde and the
// write proceeded with no expectation enforced, status ok. A mis-aimed guard was
// indistinguishable from no guard — including to a verification script reading
// only the exit code, which is exactly how a lost update goes unnoticed.
//
// The attribute is on those two request structs, on the three write shapes a
// caller authors by hand (configflux-8zcp, pinned at the end of this file), and
// on no others. Every other contract struct stays tolerant of an undeclared
// field, so the strictness is exactly as wide as the hazard: these five are
// where a dropped field silently disables an integrity control rather than
// merely being ignored.
//
// The refusal is a DESERIALIZATION failure, so it is raised before any operation
// runs and is reported by each transport's own request-invalid class — the
// runtime CLI's `E_RUNTIME_CLI_REQUEST_INVALID`, and the C ABI's InvalidJson.
// That is why these pins read serde's error rather than a `DiagnosticsReport`:
// there is no envelope to read, which is the point.
//
// Split out of `hash_format_tests.rs` (which sits at its line budget) rather
// than added to it, mirroring that file's own split from `tests.rs`; `super::*`
// reaches the same flat `runtime_api` namespace.

use super::*;

/// A canonical lowercase sha256 digest, spelled as a literal. These pins never
/// reach a format gate — deserialization fails or succeeds before any operation
/// runs — so the value only has to be shaped like an id a caller would send.
const DIGEST: &str = "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a";

/// The snapshot every v2 request embeds, at its minimum. The runtime's hash
/// format gates run inside the operations, not on the way in, so placeholder
/// hashes are enough for a pin about which FIELDS a request accepts.
fn minimal_snapshot() -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": "m",
        "resolve_hash": "r",
        "scope": "all",
        "resolved_output": {},
    })
}

/// A `set_parameters_atomically` payload carrying only fields the request
/// declares, including its own expectation.
fn atomic_payload() -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": minimal_snapshot(),
        "writes": [],
        "actor": "operator",
        "expected_working_configuration_id": DIGEST,
    })
}

/// A `commit_configuration` payload carrying only fields the request declares,
/// including its own expectation.
fn commit_payload() -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": minimal_snapshot(),
        "actor": "operator",
        "expected_base_configuration_id": DIGEST,
    })
}

/// Drop one key from a payload object, asserting it was there to drop — an
/// "omitted field" pin is vacuous if the key it removes was never present.
fn without(mut payload: serde_json::Value, key: &str) -> serde_json::Value {
    let removed = payload
        .as_object_mut()
        .expect("the payload is a JSON object")
        .remove(key);
    assert!(removed.is_some(), "'{key}' must be present for its removal to mean anything");
    payload
}

/// Assert that a serde failure is the unknown-field refusal naming `field`, not
/// some other parse error that would satisfy a bare `is_err()`.
fn assert_unknown_field(error: &serde_json::Error, field: &str, operation: &str) {
    let message = error.to_string();
    assert!(
        message.contains("unknown field") && message.contains(field),
        "'{field}' sent to {operation} must be refused as an UNKNOWN FIELD naming it, so the \
         caller learns the guard went to the wrong operation rather than being told the request \
         is malformed for some unrelated reason. Got: {message}"
    );
}

// --- the mis-aimed expectation ----------------------------------------------

#[test]
fn set_parameters_atomically_refuses_the_commit_operations_expected_id() {
    let mut payload = atomic_payload();
    payload["expected_base_configuration_id"] = serde_json::json!(DIGEST);

    let error = serde_json::from_value::<SetParametersAtomicallyRequest>(payload).expect_err(
        "an expectation named for commit_configuration must not deserialize into the atomic \
         request: dropping it applies the batch with no compare-and-swap enforced",
    );

    assert_unknown_field(&error, "expected_base_configuration_id", "set_parameters_atomically");
}

#[test]
fn commit_configuration_refuses_the_atomic_operations_expected_id() {
    let mut payload = commit_payload();
    payload["expected_working_configuration_id"] = serde_json::json!(DIGEST);

    let error = serde_json::from_value::<CommitConfigurationRequest>(payload).expect_err(
        "an expectation named for set_parameters_atomically must not deserialize into the commit \
         request: dropping it commits with no compare-and-swap enforced",
    );

    assert_unknown_field(&error, "expected_working_configuration_id", "commit_configuration");
}

#[test]
fn an_undeclared_field_is_refused_whatever_it_is_called() {
    // The rule is not a pair of special cases. A typo of the operation's own
    // field name is the likeliest way to send a guard that never runs, and it is
    // refused for the same reason.
    let mut atomic = atomic_payload();
    atomic["expected_working_configuraton_id"] = serde_json::json!(DIGEST);
    assert!(
        serde_json::from_value::<SetParametersAtomicallyRequest>(atomic).is_err(),
        "a misspelled expectation must be refused, not dropped"
    );

    let mut commit = commit_payload();
    commit["expected_base_config_id"] = serde_json::json!(DIGEST);
    assert!(
        serde_json::from_value::<CommitConfigurationRequest>(commit).is_err(),
        "a misspelled expectation must be refused, not dropped"
    );
}

// --- what the rule must NOT break -------------------------------------------

#[test]
fn both_requests_accept_a_payload_of_only_declared_fields() {
    let atomic = serde_json::from_value::<SetParametersAtomicallyRequest>(atomic_payload())
        .expect("a payload of only declared fields must still deserialize");
    assert_eq!(
        atomic.expected_working_configuration_id.as_deref(),
        Some(DIGEST),
        "the operation's own expectation still arrives"
    );

    let commit = serde_json::from_value::<CommitConfigurationRequest>(commit_payload())
        .expect("a payload of only declared fields must still deserialize");
    assert_eq!(
        commit.expected_base_configuration_id.as_deref(),
        Some(DIGEST),
        "the operation's own expectation still arrives"
    );
}

#[test]
fn an_omitted_or_null_expectation_still_deserializes_as_no_expectation() {
    // The two ways a caller says "I am not making a claim about the current
    // configuration" are unchanged. Only a PRESENT value is now held to the
    // format gate (configflux-8gah), so these must stay absent rather than
    // becoming a blank the gate would refuse.
    let mut atomic_null = atomic_payload();
    atomic_null["expected_working_configuration_id"] = serde_json::Value::Null;
    for (label, payload) in [
        ("null", atomic_null),
        ("omitted", without(atomic_payload(), "expected_working_configuration_id")),
    ] {
        let request = serde_json::from_value::<SetParametersAtomicallyRequest>(payload)
            .expect("an absent expectation deserializes");
        assert!(
            request.expected_working_configuration_id.is_none(),
            "an {label} expectation is no expectation"
        );
    }

    let mut commit_null = commit_payload();
    commit_null["expected_base_configuration_id"] = serde_json::Value::Null;
    for (label, payload) in [
        ("null", commit_null),
        ("omitted", without(commit_payload(), "expected_base_configuration_id")),
    ] {
        let request = serde_json::from_value::<CommitConfigurationRequest>(payload)
            .expect("an absent expectation deserializes");
        assert!(
            request.expected_base_configuration_id.is_none(),
            "an {label} expectation is no expectation"
        );
    }
}

#[test]
fn a_nested_payload_stays_tolerant_of_a_field_it_does_not_declare() {
    // `deny_unknown_fields` does not propagate, and the strictness is declared
    // struct by struct rather than inherited. The line is AUTHORSHIP, not depth:
    // the write entries below are strict because a caller hand-writes them, and
    // `runtime_snapshot` stays tolerant because a caller does not — it is handed
    // back from a previous response, so making it strict would break a caller
    // round-tripping a snapshot emitted by a runtime one version newer.
    let mut payload = atomic_payload();
    payload["runtime_snapshot"]["a_field_no_runtime_declares"] = serde_json::json!(1);

    serde_json::from_value::<SetParametersAtomicallyRequest>(payload)
        .expect("an undeclared field INSIDE the embedded snapshot is still tolerated");
}

// --- the guard nested one level too deep (configflux-8zcp) -------------------
//
// The routing pins above cover an expectation aimed at the wrong OPERATION. The
// same guard aimed at the wrong DEPTH — nested inside the very write entry it is
// meant to guard — was dropped by serde just as silently, and the batch applied
// with `applied_count` 1 and status ok. Nesting a guard alongside the writes it
// governs is a plausible caller mistake, not a contrived one, so the three write
// types a caller authors by hand now refuse an undeclared field too.

/// A `set_parameters_atomically` write entry carrying only its declared fields.
fn atomic_write() -> serde_json::Value {
    serde_json::json!({
        "path": "component.heat_exchanger.param.setpoint_trim",
        "value": 0.42,
    })
}

/// A `pull_updates` write entry carrying only its declared fields, both leaf
/// hashes included — the shape `docs/runtime-v2-contract.md` section 6 documents.
fn pull_write() -> serde_json::Value {
    serde_json::json!({
        "path": "component.heat_exchanger.param.setpoint_trim",
        "value": 0.5,
        "before_leaf_hash": DIGEST,
        "after_leaf_hash": DIGEST,
    })
}

/// A `set_parameter` payload carrying only fields the request declares. Unlike
/// its atomic sibling this operation has no compare-and-swap axis at all.
fn set_parameter_payload() -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": minimal_snapshot(),
        "path": "component.heat_exchanger.param.setpoint_trim",
        "value": 0.42,
    })
}

/// A `pull_updates` payload carrying only fields the request declares.
fn pull_payload(writes: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": minimal_snapshot(),
        "actor": "operator",
        "writes": writes,
    })
}

#[test]
fn an_atomic_write_entry_refuses_the_expectation_nested_inside_it() {
    // The reported case: the guard moved from the request into the write entry.
    // The request's own slot is left EMPTY, because that is the caller's belief —
    // the nested copy is the guard. Dropping it applied the write with no
    // compare-and-swap enforced and reported success, which is the one outcome
    // configflux-8gah exists to make impossible.
    let mut write = atomic_write();
    write["expected_working_configuration_id"] = serde_json::json!(DIGEST);
    let mut payload = without(atomic_payload(), "expected_working_configuration_id");
    payload["writes"] = serde_json::json!([write]);

    let error = serde_json::from_value::<SetParametersAtomicallyRequest>(payload).expect_err(
        "an expectation nested one level too deep must not deserialize: dropping it applies the \
         batch with no compare-and-swap enforced and reports status ok",
    );

    assert_unknown_field(
        &error,
        "expected_working_configuration_id",
        "a set_parameters_atomically write entry",
    );
}

#[test]
fn set_parameter_refuses_an_expectation_it_has_no_semantics_for() {
    // `set_parameter` declares no compare-and-swap field of any name, so a caller
    // who believes it guards the write gets an unguarded one. There is no slot
    // the field could have been meant for, which makes dropping it strictly worse
    // than refusing it.
    let mut payload = set_parameter_payload();
    payload["expected_working_configuration_id"] = serde_json::json!(DIGEST);

    let error = serde_json::from_value::<SetParameterRequest>(payload).expect_err(
        "an expectation sent to an operation with no compare-and-swap semantics must be refused, \
         not dropped so the write applies unguarded",
    );

    assert_unknown_field(&error, "expected_working_configuration_id", "set_parameter");
}

#[test]
fn a_pull_write_entry_refuses_a_leaf_hash_under_a_key_it_does_not_declare() {
    // `PullUpdateWrite` carries the two leaf hashes that decide whether an
    // incoming update conflicts (configflux-qfle). A hash under any other key is
    // no hash at all: the entry deserializes with both expectations absent, which
    // means "no conflict check", and the update overwrites a local edit silently.
    // A misspelling of its own field is the likeliest way to produce that.
    for misspelled in ["before_hash", "after_leaf_hashes", "expected_leaf_hash"] {
        let mut write = pull_write();
        write[misspelled] = serde_json::json!(DIGEST);

        let error =
            serde_json::from_value::<PullUpdatesRequest>(pull_payload(serde_json::json!([write])))
                .expect_err(
                    "a leaf hash under an undeclared key must be refused, not dropped so the \
                     update applies with no conflict check",
                );

        assert_unknown_field(&error, misspelled, "a pull_updates write entry");
    }
}

#[test]
fn a_pull_write_entry_refuses_another_operations_expectation() {
    // The routing hazard reaches this entry too: `pull_updates` is a write
    // surface, so a caller reaching for a guard may reach for the name the atomic
    // operation uses.
    let mut write = pull_write();
    write["expected_working_configuration_id"] = serde_json::json!(DIGEST);

    let error =
        serde_json::from_value::<PullUpdatesRequest>(pull_payload(serde_json::json!([write])))
            .expect_err("an expectation named for another operation must not be dropped here");

    assert_unknown_field(
        &error,
        "expected_working_configuration_id",
        "a pull_updates write entry",
    );
}

#[test]
fn every_write_type_still_accepts_a_payload_of_only_declared_fields() {
    // The strictness is exactly as wide as the hazard: the shapes these
    // operations document must still deserialize, optional fields present or not.
    let mut atomic = atomic_payload();
    atomic["writes"] = serde_json::json!([atomic_write()]);
    let request = serde_json::from_value::<SetParametersAtomicallyRequest>(atomic)
        .expect("a declared atomic write entry must still deserialize");
    assert_eq!(request.writes.len(), 1, "the write survives the round trip");

    serde_json::from_value::<SetParameterRequest>(set_parameter_payload())
        .expect("a declared set_parameter payload must still deserialize");

    let both =
        serde_json::from_value::<PullUpdatesRequest>(pull_payload(serde_json::json!([pull_write()])))
            .expect("a pull write entry carrying both leaf hashes must still deserialize");
    assert!(
        both.writes[0].before_leaf_hash.is_some() && both.writes[0].after_leaf_hash.is_some(),
        "both declared hashes still arrive"
    );

    // Both leaf hashes are optional, and omitting them is how a caller says "no
    // conflict check" — an existing, supported request, not a fault.
    let neither = serde_json::from_value::<PullUpdatesRequest>(pull_payload(serde_json::json!([
        {"path": "component.heat_exchanger.param.setpoint_trim", "value": 0.5}
    ])))
    .expect("a pull write entry omitting both leaf hashes must still deserialize");
    assert!(
        neither.writes[0].before_leaf_hash.is_none(),
        "an omitted hash is no expectation, exactly as before"
    );
}
