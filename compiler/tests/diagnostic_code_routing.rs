// SPDX-License-Identifier: BUSL-1.1

//! configflux-py7w: the diagnostic CODE a refusal carries names the RULE that
//! refused it, never text the author chose.
//!
//! MEASURED before the fix, not reasoned. Four TOML chunks, each carrying the
//! SAME fault (`value = inf`, the configflux-2yiq non-finite-float refusal) and
//! differing only in the definition id they name, came back with four different
//! codes through `verify --source`:
//!
//! ```text
//! [definitions."benign"]                       -> E_COMPILE_INPUT_INVALID
//! [definitions."depends_on unknown component"] -> E_UNKNOWN_COMPONENT_DEP
//! [definitions."closed facet"]                 -> E_FACET_VALUE_UNDECLARED
//! [definitions."dependency cycle detected"]    -> E_COMPONENT_DEP_CYCLE
//! ```
//!
//! The mappers recovered the code from the message PROSE, and every validator
//! message interpolates an authored id — so naming a definition after another
//! rule's routing substring handed the caller that rule's code. The code is a
//! frozen contract SDK, runtime and CI callers branch on
//! (`docs/interface-contracts.md` §3.4), so a steerable code is a broken
//! contract even though the model is still refused.
//!
//! Both mappers are covered, because each has its own substring set: the
//! link/verify one (`map_graph_error`) and the ingest one
//! (`map_compile_input_error`). Each is driven through `verify_model`, the entry
//! point `compiler verify --source` calls, so the codes asserted here are the
//! codes the binary reports. The reverse guards are what keep the fix honest:
//! a genuine cycle, a genuine unknown dependency and a genuine duplicate facet
//! must still carry their own codes when an authored id names another rule.

use compiler::product_api::{
    verify_model, Diagnostic, OperationStatus, SourceManifestEntry, VerifyCheckStatus,
    VerifyModelRequest, VerifyReport, E_COMPILE_INPUT_INVALID, E_COMPONENT_DEP_CYCLE,
    E_FACET_VALUE_UNDECLARED, E_INGEST_DUPLICATE_FACET, E_UNKNOWN_COMPONENT_DEP,
    PRODUCT_SCHEMA_VERSION,
};

/// Every substring a mapper used to route on that an authored id can carry.
///
/// A `starts_with` arm ("Catalogue '", "Binding '") is unreachable from an id —
/// an interpolated id never lands at offset 0 — so the steerable set is exactly
/// the `contains` arms. "declared in more than one chunk" is ingest-side only
/// and is exercised by its own case below.
const STEERING_IDS: [&str; 6] = [
    "depends_on unknown component",
    "dependency cycle detected",
    "closed facet",
    "is not declared under `facets`",
    "requires slot '",
    "has no entry every requirement accepts",
];

/// The control: an id that names no rule. Its code is the one every id in
/// `STEERING_IDS` must also produce, because they are all the same fault.
const BENIGN_ID: &str = "benign";

/// The remedy `E_FACET_VALUE_UNDECLARED` carries for a value outside a closed
/// domain. Written out as a literal rather than borrowed from the compiler, so
/// this agrees with the shipped text rather than with any rewording of itself.
const CLOSED_DOMAIN_HINT: &str = "Add the value to the facet's `values`, mark the facet `open: \
                                  true`, or fix the condition to use a declared value";

/// The remedy the SAME code carries for a constraint over an undeclared facet.
/// Two rules share one code and deliberately differ in remedy, so a fix that
/// keyed the remedy on the code alone would silently replace one of them.
const UNDECLARED_FACET_HINT: &str = "A constraint asserts over a declared domain, never one \
                                     inferred from conditions; declare the facet under `facets`";

/// The remedy a duplicate facet carries at ingest.
const DUPLICATE_FACET_HINT: &str =
    "A facet is a pack-global domain; declare each facet in exactly one chunk";

/// The remedy `E_COMPONENT_DEP_CYCLE` carries. Its code's default rather than a
/// rule of its own, so every cycle reports it — including the degenerate one.
const CYCLE_HINT: &str = "Break the cycle so the dependency graph is acyclic";

/// One definition whose value is `inf` — the configflux-2yiq refusal, raised by
/// `link_verify::validate_parameter_values` and reported through
/// `map_graph_error`.
fn non_finite_definition_chunk(id: &str) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[definitions."{id}"]
type = "float"
value = inf
"#
    )
}

/// A well-formed definition. Two chunks declaring the same id collide in
/// `Compiler::merge_partial`, the ingest-side refusal reported through
/// `map_compile_input_error`.
fn definition_chunk(id: &str, value: f32) -> String {
    format!(
        r#"
package = "p"
version = "1.0"

[definitions."{id}"]
type = "float"
value = {value}
"#
    )
}

/// Drive the real product verify path over inline chunks, in manifest order.
fn verify(chunks: &[&str]) -> VerifyReport {
    verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: chunks
            .iter()
            .enumerate()
            .map(|(index, content)| SourceManifestEntry {
                source_id: format!("{index:02}_chunk.toml"),
                inline_content: (*content).to_string(),
            })
            .collect(),
    })
}

/// Assert the verify failed with exactly one diagnostic carrying `expected_code`
/// and hand the diagnostic back.
///
/// Both the flat `diagnostics` list and the failing check's `diagnostic_codes`
/// are asserted: they are two separate fields of the JSON the CLI prints, and a
/// consumer may branch on either one.
fn sole_error_diagnostic(report: &VerifyReport, expected_code: &str, case: &str) -> Diagnostic {
    assert_eq!(
        report.status,
        OperationStatus::Error,
        "{case}: the model must be refused"
    );
    assert_eq!(
        report.diagnostics.diagnostics.len(),
        1,
        "{case}: expected exactly one diagnostic, got {:?}",
        report.diagnostics.diagnostics
    );
    let diagnostic = report.diagnostics.diagnostics[0].clone();
    assert_eq!(
        diagnostic.code, expected_code,
        "{case}: wrong code for message {:?}",
        diagnostic.message
    );
    let failed: Vec<&str> = report
        .checks
        .iter()
        .filter(|check| check.status == VerifyCheckStatus::Fail)
        .flat_map(|check| check.diagnostic_codes.iter().map(String::as_str))
        .collect();
    assert_eq!(
        failed,
        vec![expected_code],
        "{case}: the failing check must report the same code"
    );
    diagnostic
}

/// The defect itself, on the link/verify mapper: one fault, seven authored ids,
/// one code.
#[test]
fn an_authored_definition_id_cannot_steer_the_verify_refusal_off_its_code() {
    for id in std::iter::once(BENIGN_ID).chain(STEERING_IDS) {
        let report = verify(&[&non_finite_definition_chunk(id)]);
        let diagnostic = sole_error_diagnostic(&report, E_COMPILE_INPUT_INVALID, id);
        assert!(
            diagnostic
                .message
                .contains(&format!("Parameter 'definitions.{id}'")),
            "{id}: the non-finite-float rule must be the rule that refused, got {:?}",
            diagnostic.message
        );
    }
}

/// The same defect on the ingest mapper, which routed on its own substring set.
/// A duplicate DEFINITION id is not a duplicate facet, however the id is spelled.
#[test]
fn an_authored_definition_id_cannot_steer_the_ingest_refusal_off_its_code() {
    for id in [BENIGN_ID, "declared in more than one chunk", "requires slot '"] {
        let report = verify(&[&definition_chunk(id, 1.0), &definition_chunk(id, 2.0)]);
        let diagnostic = sole_error_diagnostic(&report, E_COMPILE_INPUT_INVALID, id);
        assert!(
            diagnostic
                .message
                .contains(&format!("Duplicate definition ID found: '{id}'")),
            "{id}: the duplicate-definition rule must be the rule that refused, got {:?}",
            diagnostic.message
        );
    }
}

/// Reverse guard: a GENUINE dependency cycle keeps its own code when the
/// components in it are named after the closed-facet rule.
#[test]
fn a_genuine_dependency_cycle_keeps_its_code_under_a_component_named_for_another_rule() {
    let chunk = r#"
package = "p"
version = "1.0"

[components."closed facet a"]
type = "service"
depends_on = ["closed facet b"]

[components."closed facet b"]
type = "service"
depends_on = ["closed facet a"]
"#;

    let report = verify(&[chunk]);
    let diagnostic = sole_error_diagnostic(&report, E_COMPONENT_DEP_CYCLE, "genuine cycle");
    assert!(
        diagnostic.message.contains("dependency cycle detected"),
        "the cycle rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
}

/// Reverse guard: a GENUINE unknown dependency keeps its own code when the
/// component is named after the cycle rule.
#[test]
fn a_genuine_unknown_dependency_keeps_its_code_under_a_component_named_for_another_rule() {
    let chunk = r#"
package = "p"
version = "1.0"

[components."dependency cycle detected"]
type = "service"
depends_on = ["nowhere"]
"#;

    let report = verify(&[chunk]);
    let diagnostic = sole_error_diagnostic(&report, E_UNKNOWN_COMPONENT_DEP, "genuine unknown dep");
    assert!(
        diagnostic.message.contains("depends_on unknown component"),
        "the unknown-dependency rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
}

/// Reverse guard with the steering in the VALUE rather than the id: a genuine
/// closed-domain violation whose offending value is spelled like the cycle rule.
/// Also the first of the two remedies `E_FACET_VALUE_UNDECLARED` carries.
#[test]
fn a_closed_domain_violation_keeps_its_code_and_remedy_under_a_value_named_for_another_rule() {
    let chunk = r#"
package = "p"
version = "1.0"

[facets.region]
values = ["eu", "us"]
default = "eu"

[components.agent]
type = "service"
condition = "region == 'dependency cycle detected'"
"#;

    let report = verify(&[chunk]);
    let diagnostic =
        sole_error_diagnostic(&report, E_FACET_VALUE_UNDECLARED, "genuine closed domain");
    assert!(
        diagnostic.message.contains("closed facet 'region'"),
        "the closed-domain rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some(CLOSED_DOMAIN_HINT),
        "the closed-domain remedy must survive"
    );
}

/// The SECOND remedy on that same code: a constraint over a facet nothing
/// declares (configflux-6j91). One code, two rules, two remedies — a fix that
/// derived the remedy from the code alone would lose one of them.
#[test]
fn an_undeclared_constraint_facet_keeps_its_own_remedy_on_the_shared_code() {
    let chunk = r#"
package = "p"
version = "1.0"

[constraints.pinned_arch]
condition = "arch == 'x86'"

[components.agent]
type = "service"
"#;

    let report = verify(&[chunk]);
    let diagnostic =
        sole_error_diagnostic(&report, E_FACET_VALUE_UNDECLARED, "undeclared constraint facet");
    assert!(
        diagnostic
            .message
            .contains("is not declared under `facets`"),
        "the undeclared-facet rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some(UNDECLARED_FACET_HINT),
        "the undeclared-facet remedy must survive, and must not be replaced by the \
         closed-domain one the same code also carries"
    );
}

/// Positive control for the ingest mapper: the fault its duplicate arm exists
/// for still reaches its own code and remedy once routing no longer reads prose.
#[test]
fn a_genuine_duplicate_facet_keeps_its_ingest_code_and_remedy() {
    let facet_chunk = r#"
package = "p"
version = "1.0"

[facets.region]
values = ["eu", "us"]
default = "eu"
"#;

    let report = verify(&[facet_chunk, facet_chunk]);
    let diagnostic =
        sole_error_diagnostic(&report, E_INGEST_DUPLICATE_FACET, "genuine duplicate facet");
    assert!(
        diagnostic
            .message
            .contains("Facet 'region' is declared in more than one chunk"),
        "the duplicate-facet rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some(DUPLICATE_FACET_HINT),
        "the duplicate-facet remedy must survive"
    );
}

/// configflux-5ge3: the degenerate cycle. A component that names ITSELF in
/// `depends_on` is refused in the edge loop, before `detect_cycle` — the arm
/// that carries the code — is ever reached, so the refusal used to arrive
/// uncoded and land in the generic bucket. `docs/diagnostics.md` promises
/// `E_COMPONENT_DEP_CYCLE` for "a cycle in the component dependency graph", and
/// a 1-cycle is one, so a caller branching on that code to report a cycle
/// missed exactly the case a human is most likely to author by hand.
#[test]
fn a_self_dependency_is_refused_as_a_dependency_cycle() {
    let chunk = r#"
package = "p"
version = "1.0"

[components.agent]
type = "service"
depends_on = ["agent"]
"#;

    let report = verify(&[chunk]);
    let diagnostic = sole_error_diagnostic(&report, E_COMPONENT_DEP_CYCLE, "self dependency");
    assert!(
        diagnostic.message.contains("depends_on itself"),
        "the self-dependency rule must be the rule that refused, got {:?}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some(CYCLE_HINT),
        "a 1-cycle must carry the same break-the-cycle remedy as any other cycle"
    );
}
