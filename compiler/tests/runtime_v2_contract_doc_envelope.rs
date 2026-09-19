// SPDX-License-Identifier: BUSL-1.1

//! Pins every claim `docs/runtime-v2-contract.md` makes about the response
//! envelope to the envelopes the runtime actually ships: section 4's list of
//! the fields every response carries, section 2's rule for the version on them,
//! and section 8's claim that no top-level field tells the two command families
//! apart. All three read the same structs out of
//! `compiler/src/runtime_api/contracts.rs`, which is why they share one target
//! rather than one apiece — configflux-8di0 renamed the file and the target
//! from `..._section4` to say so.
//!
//! configflux-3r8a: pins section 4 of `docs/runtime-v2-contract.md` to the
//! response envelopes the runtime actually ships.
//!
//! Section 4 is the document's "Common Envelope Fields" list — the one place an
//! integrator looks to learn what is on *every* response. It named three fields
//! no envelope carries: `event_sequence` exists nowhere in
//! `compiler/src/runtime_api/contracts.rs` under any name, and
//! `committed_configuration_id` / `working_configuration_id` exist only on
//! nested payloads. It also omitted two fields every envelope does carry,
//! `scope` and the optional `diagnostics_ref`.
//!
//! `//compiler:runtime_v2_contract_doc_section6_test` could not see it: that
//! guard round-trips the fenced examples of section 6 and never reads section
//! 4's prose, so a field invented there is invisible to it. Only five contract
//! structs set `deny_unknown_fields` — the two compare-and-swap requests
//! (configflux-8gah) and the three write shapes (configflux-8zcp) — and no
//! response envelope is among them, so nothing else rejects one.
//!
//! configflux-wf2f: the same blind spot covered sections 2 and 8, the document's
//! two versioning statements, so they are pinned here as well. Section 2 claimed
//! `schema_version = 2` identifies a v2 envelope and `schema_version = 1` stays
//! valid for v1; section 8 claimed a v1 response omits v2-only fields "unless
//! explicitly requested with `schema_version = 2`". The runtime has no such
//! axis: both families are versioned by `PRODUCT_SCHEMA_VERSION` alone. The
//! guards below hold the document to that — against the constant for the
//! literals it prints, and against the shipped envelopes for the claim that the
//! two families share one.
//!
//! # What "the shipped envelopes" means here
//!
//! Every response envelope the runtime CLI emits is a `pub struct *Result` in
//! `contracts.rs`, exactly: the twenty structs this test parses are the twenty
//! the CLI's `impl_has_status!` dispatch list enumerates. Parsing source is a
//! proxy for serde's own view of them, so [`parser_agrees_with_serde`] closes
//! the gap — a rename, a flatten or a layout change the scanner mishandles
//! fails there rather than quietly pinning the wrong set here.

use compiler::product_api::PRODUCT_SCHEMA_VERSION;
use compiler::runtime_api::SetParametersAtomicallyResult;
use std::collections::{BTreeMap, BTreeSet};

/// The contract document, compiled in so the binary carries the exact bytes
/// under review; `compile_data` supplies it, as for the section 6 guard.
const CONTRACT_DOC: &str = include_str!("../../docs/runtime-v2-contract.md");

/// The shipped structs, compiled in the same way. This is the oracle: the
/// document is checked against these bytes, never a list restated here.
const CONTRACTS_SRC: &str = include_str!("../src/runtime_api/contracts.rs");

const SECTION_1_HEADING: &str = "## 1. Scope";
const SECTION_2_HEADING: &str = "## 2. Versioning Rules";
const SECTION_3_HEADING: &str = "## 3. Deterministic Transport and Exit Codes";
const SECTION_4_HEADING: &str = "## 4. Common Envelope Fields";
const SECTION_5_HEADING: &str = "## 5. Runtime v2 Operation Set";
const SECTION_6_HEADING: &str = "## 6. Operation Contracts (v2 Additions)";
const SECTION_7_HEADING: &str = "## 7. Diagnostic Families (Frozen for v2)";
const SECTION_8_HEADING: &str = "## 8. Compatibility Strategy (v1 -> v2)";
const SECTION_9_HEADING: &str = "## 9. Hash and Delta Protocol Binding";

/// The lead-in to section 5's third group: the operation that joined the runtime
/// after the v2 set was frozen, listed apart from both families. Read the same
/// way as the two family markers, and panicking the same way when renamed.
const THIRD_GROUP_MARKER: &str = "Added after the v2 set was frozen:";

/// A top-level envelope field, with the one distinction the document must
/// reproduce: whether serde may leave it out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnvelopeField {
    name: String,
    optional: bool,
}

/// The slice between two headings, panicking rather than pinning nothing when
/// a heading is renamed.
fn section(start: &str, end: &str) -> &'static str {
    let from = CONTRACT_DOC
        .find(start)
        .unwrap_or_else(|| panic!("contract document has no heading {start:?}"));
    let to = CONTRACT_DOC
        .find(end)
        .unwrap_or_else(|| panic!("contract document has no heading {end:?}"));
    assert!(from < to, "heading {start:?} must precede {end:?}");
    &CONTRACT_DOC[from..to]
}

/// Every `pub struct *Result` in the contract source, with its top-level fields
/// in declaration order. A field is `optional` when its serde attributes carry
/// `skip_serializing_if`, the only mechanism these structs use to omit a field
/// from a response; attribute lines accumulate onto the field below them.
fn envelope_structs() -> BTreeMap<String, Vec<EnvelopeField>> {
    let mut parsed = BTreeMap::new();
    let mut lines = CONTRACTS_SRC.lines();

    while let Some(line) = lines.next() {
        let Some(name) = line
            .strip_prefix("pub struct ")
            .and_then(|rest| rest.strip_suffix(" {"))
        else {
            continue;
        };
        if !name.ends_with("Result") {
            continue;
        }

        let mut fields: Vec<EnvelopeField> = Vec::new();
        let mut optional = false;
        for body in lines.by_ref() {
            if body == "}" {
                break;
            }
            if body.contains("skip_serializing_if") {
                optional = true;
            }
            let Some(field) = field_name(body) else { continue };
            fields.push(EnvelopeField {
                name: field,
                optional,
            });
            optional = false;
        }
        assert!(
            !fields.is_empty(),
            "parsed no fields for {name}; the struct-body scanner has drifted \
             from the source layout"
        );
        parsed.insert(name.to_string(), fields);
    }

    assert!(
        parsed.len() >= 15,
        "parsed only {} response envelopes; the scanner has drifted from \
         contracts.rs and would pin an arbitrary subset",
        parsed.len()
    );
    parsed
}

/// The field name on a struct-body line, or `None` for an attribute, doc
/// comment or blank. Requiring a bare identifier keeps doc prose out.
fn field_name(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("pub ")?;
    let (name, _) = rest.split_once(':')?;
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    plain.then(|| name.to_string())
}

/// The fields on *every* response envelope, marked optional when any envelope
/// may omit one.
fn common_fields() -> Vec<EnvelopeField> {
    let structs = envelope_structs();
    let mut iter = structs.values();
    let first = iter.next().expect("at least one response envelope");

    let mut common: Vec<EnvelopeField> = first.clone();
    for fields in iter {
        let by_name: BTreeMap<&str, bool> = fields
            .iter()
            .map(|field| (field.name.as_str(), field.optional))
            .collect();
        common.retain(|field| by_name.contains_key(field.name.as_str()));
        for field in &mut common {
            field.optional |= by_name[field.name.as_str()];
        }
    }
    common
}

/// The first numbered field list in a section: entries are ``N. `name` `` with
/// any trailing gloss, and an entry is optional when its gloss says so. The
/// scan stops at the first line that is not such an entry, so later prose and
/// the determinism rules below the list cannot be mistaken for field names.
fn documented_fields(section: &str) -> Vec<EnvelopeField> {
    let mut fields = Vec::new();
    for line in section.lines() {
        let entry = line
            .split_once(". `")
            .filter(|(ordinal, _)| {
                !ordinal.is_empty() && ordinal.chars().all(|c| c.is_ascii_digit())
            })
            .and_then(|(_, rest)| rest.split_once('`'));
        match entry {
            Some((name, gloss)) => fields.push(EnvelopeField {
                name: name.to_string(),
                optional: gloss.contains("(optional)"),
            }),
            None if fields.is_empty() => continue,
            None => break,
        }
    }
    assert!(!fields.is_empty(), "found no numbered field list in the section");
    fields
}

/// The shipped response envelopes that omit `resolve_hash`, read out of
/// contracts.rs rather than named here.
///
/// ADR-0031 D2 froze `RuntimeExplainRejectionResult` without the field — its
/// response field list does not carry it — which is why section 4 names that
/// envelope in prose instead of promoting `resolve_hash` into the common list,
/// and why section 6's preamble claims `resolve_hash` on every example but one.
/// Three pins below read this set rather than restating the name, so a second
/// envelope dropping the field fails all three instead of silently widening an
/// exemption written for one.
fn envelopes_without_resolve_hash() -> BTreeSet<String> {
    envelope_structs()
        .into_iter()
        .filter(|(_, fields)| !fields.iter().any(|field| field.name == "resolve_hash"))
        .map(|(name, _)| name)
        .collect()
}

/// The names of `fields`, narrowed to the optional ones when `optional_only`.
fn names(fields: &[EnvelopeField], optional_only: bool) -> BTreeSet<&str> {
    fields
        .iter()
        .filter(|field| field.optional || !optional_only)
        .map(|field| field.name.as_str())
        .collect()
}

/// Every fenced JSON response example in section 6, with its caption's type.
fn section_6_response_examples() -> Vec<(String, serde_json::Value)> {
    let body = section(SECTION_6_HEADING, SECTION_7_HEADING);
    let mut examples = Vec::new();
    let mut lines = body.lines();

    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("Response (`") else {
            continue;
        };
        let Some((type_name, _)) = rest.split_once('`') else { continue };
        let mut json = String::new();
        let mut inside = false;
        for body_line in lines.by_ref() {
            if body_line == "```json" {
                inside = true;
                continue;
            }
            if inside && body_line == "```" {
                break;
            }
            if inside {
                json.push_str(body_line);
                json.push('\n');
            }
        }
        let value: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|err| panic!("section 6 example for {type_name} is not JSON: {err}"));
        examples.push((type_name.to_string(), value));
    }

    assert!(
        examples.len() >= 10,
        "found only {} response examples in section 6; the extractor has \
         drifted from the document layout",
        examples.len()
    );
    examples
}

/// The document's common-field list must be exactly the intersection of the
/// shipped envelopes — no invented field, no omitted one.
#[test]
fn section_4_lists_exactly_the_common_envelope_fields() {
    let documented = documented_fields(section(SECTION_4_HEADING, SECTION_5_HEADING));
    let shipped = common_fields();
    let documented_names = names(&documented, false);
    let shipped_names = names(&shipped, false);

    let invented: Vec<&&str> = documented_names.difference(&shipped_names).collect();
    let omitted: Vec<&&str> = shipped_names.difference(&documented_names).collect();
    assert!(
        invented.is_empty(),
        "section 4 names {invented:?}, which no response envelope in \
         contracts.rs carries at the top level"
    );
    assert!(
        omitted.is_empty(),
        "section 4 omits {omitted:?}, which every response envelope in \
         contracts.rs carries"
    );
}

/// Naming the right fields is not enough: an integrator branches on whether a
/// field can be absent, so the optional marking is pinned too.
#[test]
fn section_4_marks_exactly_the_optional_common_fields() {
    let documented = documented_fields(section(SECTION_4_HEADING, SECTION_5_HEADING));
    let shipped = common_fields();
    assert_eq!(
        names(&documented, true),
        names(&shipped, true),
        "section 4's `(optional)` markers disagree with the \
         `skip_serializing_if` attributes in contracts.rs"
    );
}

/// `resolve_hash` sits on all but one envelope, so it cannot join the common
/// list — but leaving it unmentioned would be its own drift, so the document
/// must name it together with the envelope that omits it.
#[test]
fn section_4_names_the_envelope_that_omits_resolve_hash() {
    let missing: Vec<String> = envelopes_without_resolve_hash().into_iter().collect();

    assert_eq!(
        missing.len(),
        1,
        "expected exactly one envelope without `resolve_hash`, found {missing:?}; \
         if that set changed, section 4's callout has to change with it"
    );

    let text = section(SECTION_4_HEADING, SECTION_5_HEADING);
    assert!(
        text.contains("resolve_hash"),
        "section 4 never mentions `resolve_hash`, which all but one envelope carries"
    );
    assert!(
        text.contains(missing[0].as_str()),
        "section 4 mentions `resolve_hash` without naming {}, the one envelope \
         that omits it",
        missing[0]
    );
}

/// Two of the three fields section 4 used to invent are real names living
/// elsewhere. The document now says where; this pins that redirection against
/// the source so it cannot rot into a second wrong claim.
#[test]
fn section_4_places_the_configuration_ids_on_their_real_owners() {
    let text = section(SECTION_4_HEADING, SECTION_5_HEADING);

    for phantom in ["committed_configuration_id", "working_configuration_id"] {
        let owners: Vec<&str> = CONTRACTS_SRC
            .split("pub struct ")
            .skip(1)
            .filter(|block| {
                block
                    .split("\n}")
                    .next()
                    .is_some_and(|body| body.contains(&format!("pub {phantom}:")))
            })
            .filter_map(|block| block.split_whitespace().next())
            .collect();

        assert!(
            !owners.is_empty(),
            "{phantom} is on no struct at all; section 4's redirection is stale"
        );
        for owner in owners {
            assert!(
                text.contains(owner),
                "section 4 redirects {phantom} without naming its owner {owner}"
            );
        }
    }

    assert!(
        !CONTRACTS_SRC.contains("event_sequence"),
        "`event_sequence` now exists in contracts.rs; section 4 says it does not"
    );
}

/// Section 6's preamble makes a claim about its own examples; check it against
/// the examples rather than trusting the sentence.
#[test]
fn section_6_preamble_matches_its_response_examples() {
    let shipped = common_fields();
    let all = names(&shipped, false);
    let required: BTreeSet<&str> = all.difference(&names(&shipped, true)).copied().collect();
    let exempt = envelopes_without_resolve_hash();
    let mut exempted: BTreeSet<String> = BTreeSet::new();

    for (type_name, value) in section_6_response_examples() {
        let keys: BTreeSet<&str> = value
            .as_object()
            .unwrap_or_else(|| panic!("section 6 example for {type_name} is not an object"))
            .keys()
            .map(String::as_str)
            .collect();
        let missing: Vec<&&str> = required.difference(&keys).collect();
        assert!(
            missing.is_empty(),
            "the section 6 response example for {type_name} omits {missing:?}, \
             which section 4 says every response carries"
        );
        assert!(
            !keys.contains("diagnostics_ref"),
            "the section 6 example for {type_name} now shows `diagnostics_ref`; \
             the preamble says no example exercises it"
        );
        if exempt.contains(&type_name) {
            // The ADR-0031-prescribed exception: D2 freezes this envelope's
            // response field list without `resolve_hash`, so the example must
            // not show one either.
            assert!(
                !keys.contains("resolve_hash"),
                "the section 6 example for {type_name} shows `resolve_hash`, but contracts.rs \
                 ships that envelope without the field; either the struct grew one or this \
                 example invented it"
            );
            exempted.insert(type_name);
            continue;
        }
        assert!(
            keys.contains("resolve_hash"),
            "the section 6 example for {type_name} omits `resolve_hash`; the \
             preamble says each example carries it, with only the envelopes \
             contracts.rs ships without the field excepted"
        );
    }

    assert_eq!(
        exempted, exempt,
        "the examples section 6 excuses from `resolve_hash` are not the envelopes contracts.rs \
         ships without it. A name only in the shipped set has no response example left to \
         excuse it; a name only in the encountered set means the exemption outlived the \
         struct that earned it"
    );
}

/// Ties the source parser to serde: a documented example, deserialized into the
/// real struct and serialized back, must emit exactly the required fields this
/// file parsed for that struct.
#[test]
fn parser_agrees_with_serde() {
    let target = "SetParametersAtomicallyResult";
    let (_, documented) = section_6_response_examples()
        .into_iter()
        .find(|(type_name, _)| type_name == target)
        .unwrap_or_else(|| panic!("section 6 has no response example for {target}"));

    let envelope: SetParametersAtomicallyResult =
        serde_json::from_value(documented).expect("documented example deserializes");
    let round_tripped = serde_json::to_value(&envelope).expect("envelope serializes");
    let serde_keys: BTreeSet<&str> = round_tripped
        .as_object()
        .expect("envelope serializes to an object")
        .keys()
        .map(String::as_str)
        .collect();

    let structs = envelope_structs();
    let parsed = structs
        .get(target)
        .unwrap_or_else(|| panic!("{target} was not parsed out of contracts.rs"));
    let parsed_required: BTreeSet<&str> = names(parsed, false)
        .difference(&names(parsed, true))
        .copied()
        .collect();

    assert_eq!(
        serde_keys, parsed_required,
        "the field names parsed from contracts.rs are not the keys serde emits \
         for {target}"
    );
}

/// The normative body: everything from section 1 on. The revision log above it
/// quotes the values this document used to state and no longer does — a history
/// that cannot name what it corrected is not a history — so the literal scans
/// below start after it.
fn normative_body() -> &'static str {
    let from = CONTRACT_DOC
        .find(SECTION_1_HEADING)
        .expect("contract document has no section 1 heading");
    &CONTRACT_DOC[from..]
}

/// Every `schema_version` in `text` that is immediately given a number, in
/// either form the document uses: the `"schema_version": N` of the JSON
/// examples and the `schema_version = N` of prose. A mention with no number
/// attached — a field list entry, the section 6 preamble sentence — yields
/// nothing, which is the point: naming the field is not stating a version.
fn schema_version_literals(text: &str) -> Vec<u32> {
    const KEY: &str = "schema_version";
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        rest = &rest[at + KEY.len()..];
        let digits: String = rest
            .chars()
            .skip_while(|c| matches!(c, '`' | '"' | ':' | '=' | ' '))
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(value) = digits.parse::<u32>() {
            found.push(value);
        }
    }
    found
}

/// The `*Result` struct declaring one operation's response envelope.
fn result_struct_name(operation: &str) -> String {
    let mut name = String::new();
    for word in operation.split('_') {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.extend(first.to_uppercase());
            name.push_str(chars.as_str());
        }
    }
    name.push_str("Result");
    name
}

/// The response envelopes of one command family, read from section 5's own list
/// of that family's operations. The v1/v2 split exists only in this document —
/// no module, type, op code or dispatch arm in the runtime draws it — so the
/// document is the only place the two families can be enumerated from.
fn family_envelopes(marker: &str) -> Vec<String> {
    let body = section(SECTION_5_HEADING, SECTION_6_HEADING);
    let from = body
        .find(marker)
        .unwrap_or_else(|| panic!("section 5 no longer introduces a family with {marker:?}"));
    documented_fields(&body[from + marker.len()..])
        .iter()
        .map(|entry| result_struct_name(&entry.name))
        .collect()
}

/// Every version this document prints must be the one the runtime implements.
/// Both command families are checked against `PRODUCT_SCHEMA_VERSION` and reject
/// anything else, so a second value anywhere in the body is a request the binary
/// refuses — and a rotation of the constant that left the document behind would
/// turn every example into one.
#[test]
fn every_documented_schema_version_literal_is_the_product_schema_version() {
    let literals = schema_version_literals(normative_body());
    assert!(
        !literals.is_empty(),
        "found no schema_version literal in the contract body; the scanner has \
         drifted from the document and is pinning nothing"
    );
    let wrong: Vec<u32> = literals
        .into_iter()
        .filter(|value| *value != PRODUCT_SCHEMA_VERSION)
        .collect();
    assert!(
        wrong.is_empty(),
        "the contract states schema_version {wrong:?}; every handler in both \
         command families compares the request against PRODUCT_SCHEMA_VERSION \
         ({PRODUCT_SCHEMA_VERSION}) and rejects any other value with \
         E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION"
    );
}

/// configflux-wf2f: sections 2 and 8 are the versioning rules themselves, so
/// they name the version the way section 6's preamble does rather than pinning a
/// number of their own — a number here would be a second place to rotate, and
/// the two they used to pin, 1 and 2, were both values the runtime rejects.
#[test]
fn sections_2_and_8_state_the_shipped_versioning_rule() {
    for (label, body) in [
        ("2", section(SECTION_2_HEADING, SECTION_3_HEADING)),
        ("8", section(SECTION_8_HEADING, SECTION_9_HEADING)),
    ] {
        let literals = schema_version_literals(body);
        assert!(
            literals.is_empty(),
            "section {label} states schema_version {literals:?}; both families \
             carry the current product schema version and nothing else, so the \
             section names it in prose instead of pinning a number"
        );
    }

    let two = section(SECTION_2_HEADING, SECTION_3_HEADING);
    assert!(
        two.contains("current product schema version"),
        "section 2 must name the version both families carry the way section 6's \
         preamble does"
    );
    assert!(
        two.contains("E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION"),
        "section 2 must name the code every handler raises for any other value"
    );

    let eight = section(SECTION_8_HEADING, SECTION_9_HEADING);
    assert!(
        !eight.contains("unless explicitly requested"),
        "section 8 gates response fields on a requested version again; no handler \
         branches on schema_version at all, so no request value adds a field to a \
         v1 response"
    );
}

/// One sentence of the document asserting the axis the runtime does not have,
/// with the word that made it an assertion of separateness.
#[derive(Debug)]
struct AxisClaim {
    sentence: String,
    marker: &'static str,
}

/// What one pass of [`scan_version_axis`] saw: the sentences that talk about
/// this axis at all, and the ones that get it wrong.
struct AxisScan {
    pairings: Vec<String>,
    claims: Vec<AxisClaim>,
}

/// The document's own names for the two command families. Section 2 calls them
/// "command families"; sections 5 and 8 call them `v1` and `v2`.
const FAMILY_TERMS: &[&str] = &[
    "v1",
    "v2",
    "command family",
    "command families",
    "family",
    "families",
];

/// The nouns a per-family version axis has to be stated in. `schema_version` is
/// listed in its own right: `_` counts as a word character below, so the field
/// name does not read as the bare noun.
const VERSION_TERMS: &[&str] = &[
    "version",
    "versions",
    "versioned",
    "versioning",
    "schema_version",
    "track",
    "tracks",
    "axis",
    "axes",
];

/// The words that turn a family/version pairing into an assertion that the two
/// are versioned *apart*.
const SEPARATION_TERMS: &[&str] = &[
    "separate",
    "separately",
    "separates",
    "distinct",
    "distinctly",
    "different",
    "differently",
    "its own",
    "their own",
    "per-family",
    "per-envelope",
    "second",
    "distinguish",
    "distinguishes",
    "distinguished",
    "distinguishing",
];

/// The words with which the document denies a separation term. Section 2 states
/// the shared axis by denying the other one — "no second version axis", "no
/// value distinguishes the two families" — so a separation term is read as an
/// assertion only when none of these stands in front of it.
const DENIAL_TERMS: &[&str] = &[
    "no", "not", "never", "nothing", "neither", "nor", "cannot", "without",
];

/// ASCII word characters for token matching. The document spells identifiers
/// with `_` and `-`, so `v2` inside `runtime_v2_contract_doc_envelope_test` and
/// inside `runtime-v2-contract.md` must not read as the family name `v2`.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Byte offsets at which `needle` stands in `lowered` as a whole token.
fn token_positions(lowered: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(hit) = lowered[from..].find(needle) {
        let at = from + hit;
        let before = lowered[..at].chars().next_back();
        let after = lowered[at + needle.len()..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            found.push(at);
        }
        from = at + 1;
    }
    found
}

fn contains_token(lowered: &str, needle: &str) -> bool {
    !token_positions(lowered, needle).is_empty()
}

/// True when `line` opens a new markdown block: a blank line, a heading, a
/// table row, or a list item. Splitting on `.` alone would run a bullet that
/// ends without one into the bullet below it, which blurs the offending
/// sentence in a failure message and, worse, can leave a denial from the
/// previous bullet standing in front of this one's separation term.
fn opens_block(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with('|')
    {
        return true;
    }
    match trimmed.split_once(". ") {
        Some((head, _)) => !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// One block's sentences, whitespace-normalised, split at `.`/`!`/`?` followed
/// by whitespace.
fn push_sentences(block: &str, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut chars = block.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let terminal = matches!(c, '.' | '!' | '?');
        let boundary = chars.peek().is_none_or(|next| next.is_whitespace());
        if terminal && boundary {
            let normalised = current.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalised.is_empty() {
                out.push(normalised);
            }
            current.clear();
        }
    }
    let normalised = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !normalised.is_empty() {
        out.push(normalised);
    }
}

/// `text` as sentences, block by block.
fn prose_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    for line in text.lines() {
        if opens_block(line) && !block.is_empty() {
            push_sentences(&block.join("\n"), &mut out);
            block.clear();
        }
        if !line.trim().is_empty() {
            block.push(line);
        }
    }
    if !block.is_empty() {
        push_sentences(&block.join("\n"), &mut out);
    }
    out
}

/// configflux-onvj: every sentence of `text` claiming the two command families
/// are versioned apart.
///
/// A pairing alone cannot be the test. Section 2's own sentences pair a family
/// name with a version noun — that is what a versioning rule *is* — and they
/// are precisely the sentences that must pass, because the axis they assert is
/// the shared one. The discriminator is therefore a separation word, and the
/// shared-axis accept condition is explicit: a separation word counts as an
/// assertion only when no denial stands in front of it in the same sentence.
/// That is how section 2 already states the rule ("the runtime has no second
/// version axis — no value distinguishes the two families"), and it is why the
/// denial must *precede*: a claim of separate tracks does not launder itself by
/// trailing a "not" behind it.
///
/// Scanning is scoped to whatever slice the caller passes, which is
/// [`normative_body`]. The revision log above section 1 narrates the axis that
/// was removed, and a history that cannot describe what it corrected is not a
/// history; quoting a withdrawn rule must not read as restating it.
///
/// The phrase the old assertion matched on is kept as its own claim. It named
/// no command family — it set the canonical-payload version against "the
/// Runtime envelope `schema_version`" — so the sentence scan would not see it,
/// and dropping it would trade one blind spot for another.
fn scan_version_axis(text: &str) -> AxisScan {
    const REVIVED: &str = "envelope `schema_version`";
    let mut scan = AxisScan {
        pairings: Vec::new(),
        claims: Vec::new(),
    };

    if text.to_lowercase().contains(REVIVED) {
        scan.claims.push(AxisClaim {
            sentence: REVIVED.to_string(),
            marker: REVIVED,
        });
    }

    for sentence in prose_sentences(text) {
        let lowered = sentence.to_lowercase();
        if !FAMILY_TERMS.iter().any(|term| contains_token(&lowered, term)) {
            continue;
        }
        if !VERSION_TERMS.iter().any(|term| contains_token(&lowered, term)) {
            continue;
        }
        let denials: Vec<usize> = DENIAL_TERMS
            .iter()
            .flat_map(|term| token_positions(&lowered, term))
            .collect();
        let asserted = SEPARATION_TERMS.iter().find(|term| {
            token_positions(&lowered, term)
                .iter()
                .any(|at| !denials.iter().any(|denial| denial < at))
        });
        if let Some(marker) = asserted {
            scan.claims.push(AxisClaim {
                sentence: sentence.clone(),
                marker,
            });
        }
        scan.pairings.push(sentence);
    }

    scan
}

/// The sentence configflux-onvj recorded as the gap: a per-family version axis
/// claimed in new words, carrying no digit and not the phrase the assertion
/// used to match on.
const REWORDED_REGRESSION: &str =
    "the two command families are distinguished by separate version tracks on the wire";

/// The document once described the canonical-payload version as distinct from
/// "the Runtime envelope `schema_version`". No such value exists: every runtime
/// request and result carries exactly one version field, and it is always
/// `PRODUCT_SCHEMA_VERSION`. Nothing may reintroduce the second axis, in that
/// wording or in any other — configflux-onvj widened this from the one phrase
/// to the claim.
#[test]
fn the_contract_claims_no_separate_envelope_version() {
    let claims = scan_version_axis(normative_body()).claims;
    if let Some(claim) = claims.first() {
        panic!(
            "the contract body says {:?}. On the word {:?} that asserts the two \
             command families are versioned apart, and they are not: every \
             request and result in both carries PRODUCT_SCHEMA_VERSION and the \
             runtime has no second version axis to describe. {} sentence(s) \
             offend.",
            claim.sentence,
            claim.marker,
            claims.len()
        );
    }
}

/// configflux-onvj: half the guard's non-vacuity proof. The scan above means
/// something only if it is reading the sentences that state the rule — and
/// those must pass, because the shared axis is what the document is supposed to
/// say. This asserts both: that section 2's shared-axis prose is in front of
/// the scanner, and that the scanner accepts it.
#[test]
fn the_separate_axis_guard_accepts_the_shipped_shared_axis_prose() {
    let scan = scan_version_axis(normative_body());
    assert!(
        !scan.pairings.is_empty(),
        "the scan found no sentence naming a command family beside a version at \
         all, so it has drifted off the document and is pinning nothing"
    );
    let shared = scan
        .pairings
        .iter()
        .filter(|sentence| {
            sentence
                .to_lowercase()
                .contains("current product schema version")
        })
        .count();
    assert!(
        shared > 0,
        "no sentence the scan read says both command families carry the current \
         product schema version; section 2 states the shared axis in those \
         words and the guard must be accepting it, not merely missing it. Read \
         {} pairing(s): {:?}",
        scan.pairings.len(),
        scan.pairings
    );
    assert!(
        scan.claims.is_empty(),
        "the guard rejects prose the document ships: {:?}",
        scan.claims
    );
}

/// configflux-onvj: the other half. A guard that cannot fail pins nothing, and
/// the phrase this one used to match on no longer appears in the document, so
/// the only way to show it still bites is to feed it a regression and watch it
/// reject one. The sentence carries no digit and not the historical phrase, so
/// the three assertions beside this one all pass on it.
#[test]
fn the_separate_axis_guard_rejects_a_reworded_claim() {
    let clean = normative_body();
    assert!(
        scan_version_axis(clean).claims.is_empty(),
        "the shipped body already trips the guard, so this case proves nothing"
    );

    let mutated = clean.replacen(
        SECTION_2_HEADING,
        &format!("{SECTION_2_HEADING}\n\n{REWORDED_REGRESSION}\n"),
        1,
    );
    let claims = scan_version_axis(&mutated).claims;
    let caught = claims
        .iter()
        .find(|claim| claim.sentence.contains(REWORDED_REGRESSION));
    let Some(caught) = caught else {
        panic!(
            "section 2 now says {REWORDED_REGRESSION:?} and the guard let it \
             through, so it still matches wording rather than the claim and any \
             rewording of the per-family version axis passes. Claims found: \
             {claims:?}"
        );
    };
    assert!(
        SEPARATION_TERMS.contains(&caught.marker),
        "the injected sentence was caught on {:?}, which is not one of the words \
         that assert separateness; the guard is rejecting it for the wrong \
         reason",
        caught.marker
    );
    assert_eq!(
        claims.len(),
        1,
        "injecting one sentence produced {} claims, so the guard is also \
         rejecting prose the document ships: {claims:?}",
        claims.len()
    );
}

/// configflux-n94v: section 5's third group, which neither family scan reads.
///
/// [`documented_fields`] stops at the blank line closing a numbered list, so the
/// group section 5 adds below the v2 list sits outside both family scans by
/// construction. That is deliberate and it is what keeps
/// [`section_8_v1_and_v2_share_one_response_envelope`] at full strength: a
/// fifteenth entry in either list would drop `resolve_hash` from that family's
/// field intersection and the two would stop being equal, turning a documented
/// operation into a reason to weaken the pin. The cost is that nothing guards the
/// group itself, which this closes — it names exactly one operation, and that
/// operation's result envelope is exactly the one contracts.rs ships without
/// `resolve_hash`.
#[test]
fn section_5_third_group_names_the_envelope_that_omits_resolve_hash() {
    let body = section(SECTION_5_HEADING, SECTION_6_HEADING);
    let from = body.find(THIRD_GROUP_MARKER).unwrap_or_else(|| {
        panic!("section 5 no longer introduces its third group with {THIRD_GROUP_MARKER:?}")
    });
    let listed = documented_fields(&body[from + THIRD_GROUP_MARKER.len()..]);
    assert_eq!(
        listed.len(),
        1,
        "section 5's third group lists {:?}. It exists for the operations that joined the \
         runtime after the v2 set was frozen, and today that is one; a second entry needs \
         its own justification here before it is documented there",
        names(&listed, false)
    );

    let suffix = result_struct_name(&listed[0].name);
    let missing: Vec<String> = envelopes_without_resolve_hash().into_iter().collect();
    assert_eq!(
        missing.len(),
        1,
        "expected exactly one envelope without `resolve_hash`, found {missing:?}; section 5's \
         third group and section 4's callout both describe a single exception"
    );
    assert!(
        missing[0].ends_with(&suffix),
        "section 5's third group names `{}`, whose result envelope would be `{suffix}`, but the \
         envelope contracts.rs ships without `resolve_hash` is `{}`. The shipped name carries a \
         `Runtime` prefix because `loader_api` declares a second `ExplainRejectionResult`, so \
         the suffix rather than the whole name is what ties the two together",
        listed[0].name,
        missing[0]
    );
}

/// Section 8's claim that no top-level field distinguishes a v1 response from a
/// v2 one, checked against the envelopes rather than trusted. Section 4 pins the
/// intersection across *all* envelopes, which cannot see a field common to every
/// v2 result and missing from a v1 one; this can.
#[test]
fn section_8_v1_and_v2_share_one_response_envelope() {
    let structs = envelope_structs();
    let common = |family: &[String]| -> BTreeSet<String> {
        let mut shared: Option<BTreeSet<String>> = None;
        for envelope in family {
            let fields = structs.get(envelope).unwrap_or_else(|| {
                panic!("section 5 names an operation whose {envelope} is not in contracts.rs")
            });
            let here: BTreeSet<String> = fields.iter().map(|f| f.name.clone()).collect();
            shared = Some(match shared {
                None => here,
                Some(acc) => acc.intersection(&here).cloned().collect(),
            });
        }
        shared.expect("section 5 lists at least one operation per family")
    };

    let v1 = common(&family_envelopes("Runtime v1 operations remain:"));
    let v2 = common(&family_envelopes("Runtime v2 adds:"));
    assert_eq!(
        v1, v2,
        "the v1 and v2 response envelopes no longer share one field set, so a \
         top-level field does distinguish the families and section 8 says none \
         does"
    );
}
