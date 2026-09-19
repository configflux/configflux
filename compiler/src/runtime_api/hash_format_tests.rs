// SPDX-License-Identifier: BUSL-1.1

// Unit tests for `is_sha256_hex`, the runtime's only format gate on hash-shaped
// input, and for the five call-site families that depend on it (configflux-07t1
// for the first three, configflux-11wx for the two compare-and-swap fields at the
// foot of this file). Split into its own `#[cfg(test)]` module because `tests.rs`
// sits at its line budget (mirroring the `authority_tests.rs` split); `super::*`
// reaches the flat `runtime_api` namespace the `include!`d `shared_ops.rs`
// contributes to, so the private gate is testable directly.
//
// The gate used to accept `A-F`. Every producer emits lowercase — `sha256_hex`
// is `format!("{:x}", ...)` and `stable_hash`, `root_hash_from_leaves`,
// `identity_leaf_hash` and `value_hash` all go through it — so an uppercase
// 64-character digest passed the format check and could then never compare equal
// to anything the runtime computes. On `runtime_open` that bought a session
// whose every later cross-validation fails; on the four `pull_updates` fields it
// reached the equality check and was reported as a configuration divergence,
// which names the wrong cause. Canonical lowercase is fail-closed and matches
// what every producer already emits, so nothing in the tree changes behaviour.
// The case is NOT normalized on ingest: that would silently rewrite a caller's
// value on an attestation surface.

use super::*;

/// A real digest, as `sha256_hex` renders it. Lowercase by construction.
fn a_digest() -> String {
    sha256_hex(b"configflux-07t1")
}

/// The same digest with every hex letter upper-cased: still 64 characters, still
/// hex, and still a value no producer in the tree can emit.
fn an_uppercase_digest() -> String {
    a_digest().to_uppercase()
}

/// A `runtime_open` request carrying the two hashes under test. Every other
/// field is `#[serde(default)]`, so the payload is built through serde rather
/// than spelled out — the gate runs ahead of scope normalization and the
/// `resolved_output` decode, so nothing further has to be well-formed for the
/// request to reach it.
fn open_request(model_hash: &str, resolve_hash: &str) -> RuntimeOpenRequest {
    serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": model_hash,
        "resolve_hash": resolve_hash,
        "scope": "all",
        "resolved_output": {},
    }))
    .expect("a five-field open payload deserializes: every other field defaults")
}

/// A snapshot shaped the way `runtime_open` emits one, with both hashes supplied
/// so a test can put exactly one of them out of canonical form.
fn snapshot(model_hash: &str, resolve_hash: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": model_hash,
        "resolve_hash": resolve_hash,
        "scope": "all",
        "resolved_output": {},
    })
}

/// The first diagnostic's code and entity path — the pair that tells a caller
/// both what was refused and which field carried it.
fn first_refusal(report: &DiagnosticsReport) -> (String, String) {
    let diagnostic = report
        .diagnostics
        .first()
        .expect("a refused operation reports at least one diagnostic");
    (
        diagnostic.code.clone(),
        diagnostic.entity_path.clone().unwrap_or_default(),
    )
}

// --- the gate itself ---------------------------------------------------------

#[test]
fn is_sha256_hex_accepts_what_sha256_hex_produces() {
    let digest = a_digest();
    assert_eq!(digest.len(), 64, "sha256_hex renders 32 bytes as 64 hex characters");
    assert!(
        is_sha256_hex(&digest),
        "the gate must admit the form every producer emits, or it refuses the runtime's own \
         hashes: got '{digest}'"
    );
}

#[test]
fn is_sha256_hex_refuses_an_uppercase_digest() {
    let uppercase = an_uppercase_digest();
    assert_eq!(uppercase.len(), 64, "upper-casing a digest changes no length");
    assert!(
        uppercase.chars().any(|ch| ch.is_ascii_uppercase()),
        "this digest must carry at least one hex letter for the case to differ: '{uppercase}'"
    );
    assert!(
        !is_sha256_hex(&uppercase),
        "an uppercase digest must be refused on shape. It is 64 hex characters, so the old \
         `is_ascii_hexdigit` gate admitted it — and then no comparison against a value the \
         runtime computes could ever succeed: '{uppercase}'"
    );
}

#[test]
fn is_sha256_hex_refuses_a_single_uppercase_character() {
    let digest = a_digest();
    let offset = digest
        .find(|ch: char| ch.is_ascii_alphabetic())
        .expect("a sha256 digest of this input carries a hex letter");
    let mixed = format!(
        "{}{}{}",
        &digest[..offset],
        digest[offset..offset + 1].to_uppercase(),
        &digest[offset + 1..]
    );
    assert!(
        !is_sha256_hex(&mixed),
        "one upper-cased character is enough to make a digest unequal to the runtime's own, so \
         the gate must reject it rather than leave the mismatch to a later comparison: '{mixed}'"
    );
}

#[test]
fn is_sha256_hex_still_refuses_the_wrong_length_and_non_hex() {
    let digest = a_digest();
    assert!(!is_sha256_hex(&digest[..63]), "63 characters is not a sha256 digest");
    assert!(!is_sha256_hex(&format!("{digest}0")), "65 characters is not one either");
    assert!(!is_sha256_hex(""), "a blank string is not one");
    assert!(
        !is_sha256_hex(&format!("{}z", &digest[..63])),
        "a character outside 0-9a-f is not hex at all"
    );
}

// --- runtime_open (open_read_ops.rs) -----------------------------------------

#[test]
fn runtime_open_refuses_an_uppercase_model_hash() {
    let result = runtime_open(open_request(&an_uppercase_digest(), &a_digest()));

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(
        first_refusal(&result.diagnostics),
        (E_RUNTIME_OPEN_INVALID.to_string(), "request.model_hash".to_string()),
        "an uppercase `model_hash` must be refused by the open-time format gate, naming the \
         field that carried it. Admitting it opens a session whose every later \
         cross-validation fails against a hash the caller never sent in the form the runtime \
         renders: {:?}",
        result.diagnostics
    );
}

#[test]
fn runtime_open_refuses_an_uppercase_resolve_hash() {
    // The `model_hash` here is canonical, so reaching the `resolve_hash`
    // diagnostic is also the proof that a lowercase digest CLEARS the gate: a
    // gate that refused both would report `request.model_hash` instead.
    let result = runtime_open(open_request(&a_digest(), &an_uppercase_digest()));

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(
        first_refusal(&result.diagnostics),
        (E_RUNTIME_OPEN_INVALID.to_string(), "request.resolve_hash".to_string()),
        "an uppercase `resolve_hash` must be refused on shape, and a canonical `model_hash` \
         must pass so the refusal names the field that is actually malformed: {:?}",
        result.diagnostics
    );
}

// --- validate_runtime_snapshot (shared_ops.rs) -------------------------------

#[test]
fn a_snapshot_carrying_an_uppercase_model_hash_is_refused() {
    // The snapshot's copies are re-validated on every post-open request, which
    // is the second place the old gate let an unmatchable value through.
    let request: ListParametersRequest = serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": snapshot(&an_uppercase_digest(), &a_digest()),
        "scope_root": "all",
    }))
    .expect("a three-field list_parameters payload deserializes");

    let result = list_parameters(request);

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(
        first_refusal(&result.diagnostics),
        (
            E_RUNTIME_OPEN_INVALID.to_string(),
            "runtime_snapshot.model_hash".to_string()
        ),
        "a snapshot whose `model_hash` is not canonical lowercase must be refused wherever it \
         is presented, not only at open: {:?}",
        result.diagnostics
    );
}

// --- pull_updates (sync_ops.rs) ----------------------------------------------

#[test]
fn pull_updates_refuses_an_uppercase_base_configuration_id() {
    // This is the field where admitting uppercase did the most damage: the
    // request cleared the format check and failed the equality check, telling
    // the operator the configuration had diverged when the real fault was the
    // case of a hex string. Both ids are supplied so the delta payload reaches
    // the format gate rather than the full-snapshot precondition ahead of it.
    let request: PullUpdatesRequest = serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "runtime_snapshot": snapshot(&a_digest(), &a_digest()),
        "actor": "operator",
        "base_configuration_id": an_uppercase_digest(),
        "target_configuration_id": a_digest(),
    }))
    .expect("a five-field pull_updates payload deserializes");

    let result = pull_updates(request);

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(
        first_refusal(&result.diagnostics),
        (
            crate::sync_transport::E_RUNTIME_SYNC_PAYLOAD_INVALID.to_string(),
            "request.base_configuration_id".to_string()
        ),
        "an uppercase `base_configuration_id` must be refused as a malformed payload, not \
         reported later as a base-configuration mismatch: {:?}",
        result.diagnostics
    );
}

// --- the two compare-and-swap fields (configflux-11wx) ------------------------
//
// `commit_configuration.expected_base_configuration_id` and
// `set_parameters_atomically.expected_working_configuration_id` are the last two
// caller-supplied hash-shaped fields, and they reached their equality check with
// no shape check at all: each is trimmed, treated as absent when blank, and then
// compared for exact equality, so a string of any length arrived at the
// comparison and came back as a configuration mismatch. That is the same
// mis-attribution 07t1 removed from `pull_updates`, here on a compare-and-swap
// guard: the caller is told the configuration moved under it when the
// configuration did not move and the fault was the value it sent. The write was
// refused either way, so the gate changes no outcome — only which fault is
// reported, and against which field.
//
// Each operation reuses the rejection code it already owns, so no diagnostic code
// is minted and the frozen registry is untouched. For the atomic operation that
// code is ALSO its mismatch's own (`E_RUNTIME_DIRTY_INVALID` answers a blank
// actor and an empty batch too), so the pins below read the message as well: the
// code alone cannot tell a shape fault from a divergence there.

/// A snapshot the runtime accepts, carrying canonical hashes and no overrides.
fn cas_snapshot() -> RuntimeSnapshot {
    serde_json::from_value(snapshot(&a_digest(), &a_digest()))
        .expect("the fixture snapshot deserializes into the runtime's own type")
}

/// The identity the runtime computes for [`cas_snapshot`] — the value both
/// comparisons are made against, read through the same function the operations
/// use rather than restated here as a literal that could drift from it.
fn cas_identity() -> RuntimeConfigurationIdentity {
    compute_configuration_identity(&cas_snapshot())
        .expect("the fixture snapshot has a computable configuration identity")
}

/// The three malformed shapes, labeled: a well-formed digest in the wrong case, a
/// truncation of one, and a value that is not hex at all. The first is the one
/// that matters most — it is 64 characters of hex, so only the case rule refuses
/// it, and no producer in the tree can emit it.
fn malformed_ids() -> Vec<(&'static str, String)> {
    vec![
        ("an uppercase digest", an_uppercase_digest()),
        ("a 12-character truncation", a_digest()[..12].to_string()),
        ("a non-hex word", "not-a-configuration-id".to_string()),
    ]
}

/// The four shapes the trim/blank filter used to swallow, labeled. The first
/// three are the blanks a caller sends by accident — an unset variable, a failed
/// read, an empty environment variable. The fourth is the sharp one: the
/// session's OWN id with padding around it, which the filter trimmed back to an
/// exact match, so the write went through with the caller none the wiser that
/// the value it sent was not the value that was compared (configflux-8gah).
fn blank_and_padded_ids(current: &str) -> Vec<(&'static str, String)> {
    vec![
        ("an empty string", String::new()),
        ("a run of spaces", "   ".to_string()),
        ("a tab and a newline", "\t\n".to_string()),
        ("the session's own id with padding", format!(" {current} ")),
    ]
}

/// A commit request carrying the expectation under test. No `changed_paths_hint`:
/// that check runs AFTER the comparison and refuses with the same code, so a hint
/// here would let a pin pass while reading the wrong rejection.
fn commit_request(expected: Option<&str>) -> CommitConfigurationRequest {
    CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: cas_snapshot(),
        actor: "operator".to_string(),
        reason: None,
        expected_base_configuration_id: expected.map(str::to_string),
        changed_paths_hint: Vec::new(),
    }
}

/// An atomic request carrying the expectation under test. The batch must be
/// non-empty — an empty one is refused ahead of the comparison with the same code
/// — and its path deliberately names nothing this fixture resolves, because write
/// validation runs after the comparison: that is what makes a request which
/// CLEARS the comparison observable.
fn atomic_request(expected: Option<&str>) -> SetParametersAtomicallyRequest {
    SetParametersAtomicallyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: cas_snapshot(),
        writes: vec![AtomicParameterWrite {
            path: "component.ctl.param.threshold".to_string(),
            value: crate::schema::Value::Float(1.0),
        }],
        actor: "operator".to_string(),
        reason: None,
        expected_working_configuration_id: expected.map(str::to_string),
        intent: OverrideIntent::default(),
    }
}

// --- commit_configuration (commit_identity_ops.rs) ---------------------------

#[test]
fn commit_configuration_refuses_a_malformed_expected_base_id() {
    let current = cas_identity().committed_configuration_id;
    for (label, value) in malformed_ids() {
        let result = commit_configuration(commit_request(Some(&value)));

        assert_eq!(result.status, OperationStatus::Error, "{label} must be refused");
        assert_eq!(
            first_refusal(&result.diagnostics),
            (
                E_RUNTIME_COMMIT_INVALID.to_string(),
                "request.expected_base_configuration_id".to_string()
            ),
            "{label} ('{value}') must be refused as malformed input against the field that \
             carried it, with the code this operation already owns for an unusable request — \
             not reported as a base-configuration mismatch: {:?}",
            result.diagnostics
        );
        let message = result.diagnostics.diagnostics[0].message.clone();
        assert!(
            !message.contains(&current),
            "a shape fault must not name the session's current id ('{current}'). Naming the \
             expected-vs-current pair is what tells an operator the configuration moved under \
             them, and it did not: {label} never reached the comparison. Got '{message}'"
        );
    }
}

#[test]
fn commit_configuration_still_reports_a_well_formed_expected_base_id_as_a_mismatch() {
    // A canonical digest that is simply not this session's. The gate must let it
    // through to the comparison, or every real divergence would be reported as an
    // input error — and configflux-3d8y's pin, which hands section 6.5's own
    // documented request (a canonical id of exactly this kind) to this operation
    // and asserts the mismatch, would go red.
    let current = cas_identity().committed_configuration_id;
    let other = a_digest();
    assert_ne!(
        other, current,
        "the fixture digest must differ from the session's own id or this pin is vacuous"
    );

    let result = commit_configuration(commit_request(Some(&other)));

    assert_eq!(
        first_refusal(&result.diagnostics),
        (
            E_RUNTIME_COMMIT_BASE_MISMATCH.to_string(),
            "request.expected_base_configuration_id".to_string()
        ),
        "a well-formed digest that is not the session's must still be reported as a base \
         mismatch: {:?}",
        result.diagnostics
    );
    let message = result.diagnostics.diagnostics[0].message.clone();
    assert!(
        message.contains(&other) && message.contains(&current),
        "the mismatch must still name the id the caller sent ('{other}') and the id the session \
         has ('{current}'); got '{message}'"
    );
}

#[test]
fn commit_configuration_accepts_the_expected_base_id_it_computes() {
    let current = cas_identity().committed_configuration_id;

    let result = commit_configuration(commit_request(Some(&current)));

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "an expectation that matches the session's committed id must commit: {:?}",
        result.diagnostics
    );
    assert_eq!(
        result.base_configuration_id.as_deref(),
        Some(current.as_str()),
        "the commit reports the base it was asked to match"
    );
}

#[test]
fn commit_configuration_refuses_a_blank_or_padded_expected_base_id() {
    // A field the caller SENT is an expectation, whatever it carries
    // (configflux-8gah). The trim/blank filter that used to sit ahead of the gate
    // made a blank mean "no expectation at all", so a request assembled from an
    // unset variable came back as a committed configuration with status ok and no
    // compare-and-swap enforced — the one outcome the guard exists to prevent.
    // Only an omitted (or null) field is absent now.
    let omitted = commit_configuration(commit_request(None));
    assert_eq!(
        omitted.status,
        OperationStatus::Ok,
        "an omitted expectation is still no expectation: {:?}",
        omitted.diagnostics
    );

    let current = cas_identity().committed_configuration_id;
    for (label, value) in blank_and_padded_ids(&current) {
        let result = commit_configuration(commit_request(Some(&value)));

        assert_eq!(result.status, OperationStatus::Error, "{label} must be refused");
        assert_eq!(
            first_refusal(&result.diagnostics),
            (
                E_RUNTIME_COMMIT_INVALID.to_string(),
                "request.expected_base_configuration_id".to_string()
            ),
            "{label} ({value:?}) is a present expectation the runtime cannot honour as sent, so \
             it must be refused as malformed against the field that carried it — never dropped, \
             which would commit with no compare-and-swap enforced: {:?}",
            result.diagnostics
        );
    }
}

// --- set_parameters_atomically (write_reset_ops.rs) --------------------------

#[test]
fn set_parameters_atomically_refuses_a_malformed_expected_working_id() {
    let current = cas_identity().working_configuration_id;
    for (label, value) in malformed_ids() {
        let result = set_parameters_atomically(atomic_request(Some(&value)));

        assert_eq!(result.status, OperationStatus::Error, "{label} must be refused");
        assert_eq!(
            first_refusal(&result.diagnostics),
            (
                E_RUNTIME_DIRTY_INVALID.to_string(),
                "request.expected_working_configuration_id".to_string()
            ),
            "{label} ('{value}') must be refused against the field that carried it, with the \
             code this operation already owns: {:?}",
            result.diagnostics
        );
        let message = result.diagnostics.diagnostics[0].message.clone();
        assert!(
            !message.contains(&current),
            "this operation answers both the shape fault and the divergence with one code, so \
             the message is what separates them: a shape fault must not name the session's \
             current id ('{current}') and claim the working configuration moved. Got '{message}'"
        );
    }
}

#[test]
fn set_parameters_atomically_still_reports_a_well_formed_expected_working_id_as_a_mismatch() {
    // The atomic half of the same guarantee: a canonical digest reaches the
    // comparison, so configflux-3d8y's pin on section 6.1's documented request
    // stays green.
    let current = cas_identity().working_configuration_id;
    let other = a_digest();
    assert_ne!(
        other, current,
        "the fixture digest must differ from the session's own working id or this pin is vacuous"
    );

    let result = set_parameters_atomically(atomic_request(Some(&other)));

    assert_eq!(
        first_refusal(&result.diagnostics),
        (
            E_RUNTIME_DIRTY_INVALID.to_string(),
            "request.expected_working_configuration_id".to_string()
        ),
        "a well-formed digest that is not the session's must still reach the comparison: {:?}",
        result.diagnostics
    );
    let message = result.diagnostics.diagnostics[0].message.clone();
    assert!(
        message.contains(&other) && message.contains(&current),
        "the mismatch must name the id the caller sent ('{other}') and the id the session has \
         ('{current}'); got '{message}'"
    );
}

#[test]
fn set_parameters_atomically_is_transparent_to_the_working_id_it_computes() {
    // This fixture resolves no parameter, so write validation — which runs after
    // the comparison — refuses the batch. That makes the pin an equality: a
    // matching expectation must leave the outcome exactly as supplying none does,
    // which is what "the comparison passed through" means here.
    let current = cas_identity().working_configuration_id;

    let matched = set_parameters_atomically(atomic_request(Some(&current)));
    let absent = set_parameters_atomically(atomic_request(None));

    assert_eq!(
        matched, absent,
        "an expectation that matches the session's working id must change nothing about the \
         outcome"
    );
    assert_ne!(
        first_refusal(&matched.diagnostics).1,
        "request.expected_working_configuration_id",
        "nothing may be refused against the compare-and-swap field when the id matches: {:?}",
        matched.diagnostics
    );
}

#[test]
fn set_parameters_atomically_refuses_a_blank_or_padded_expected_working_id() {
    // The atomic half of the same guarantee. The padded case is the one that
    // shows what the trim cost: this fixture's own working id, surrounded by
    // whitespace, used to be trimmed back to an exact match and pass the
    // comparison. A value that is not the id is now refused rather than silently
    // rewritten into one (configflux-8gah).
    let current = cas_identity().working_configuration_id;
    for (label, value) in blank_and_padded_ids(&current) {
        let result = set_parameters_atomically(atomic_request(Some(&value)));

        assert_eq!(result.status, OperationStatus::Error, "{label} must be refused");
        assert_eq!(
            first_refusal(&result.diagnostics),
            (
                E_RUNTIME_DIRTY_INVALID.to_string(),
                "request.expected_working_configuration_id".to_string()
            ),
            "{label} ({value:?}) must be refused as malformed against the field that carried it \
             — never dropped, which would apply the batch with no compare-and-swap enforced: \
             {:?}",
            result.diagnostics
        );
    }
}
