// SPDX-License-Identifier: BUSL-1.1

//! ADR-0063 D1/D2: the accept/reject table for the two authored-symbol
//! predicates.
//!
//! The identifier rule is now written TWICE — as the `#snakeId` regex
//! `^[a-z]([a-z0-9]|_[a-z0-9])*_?$` in `compiler/cue/schema.cue:45`, and as the
//! hand-written character walk in `link_verify::is_snake_id`. That duplication
//! is the cost ADR-0027 Decision 4 was avoiding, accepted by ADR-0063 for four
//! symbol classes because `compile --source` never evaluates CUE and the
//! alternative to duplicating one rule there is having no rule at all.
//!
//! This file is what keeps the two spellings from drifting. Every case below is
//! derived from the REGEX, not from the walk, and each carries the clause of the
//! pattern it exercises — so a change to either side that breaks agreement fails
//! here rather than in whatever a JSON-direct model happens to declare.
//!
//! The behavioural twin — these rules reaching an author through `verify_model`
//! and `compile_model` — is `compiler/tests/symbol_charset.rs`.

use super::{is_snake_id, is_symbol_token};

// ---------------------------------------------------------------------------
// D1 — identifiers, against `^[a-z]([a-z0-9]|_[a-z0-9])*_?$`
// ---------------------------------------------------------------------------

/// `^[a-z]` then zero or more `([a-z0-9]|_[a-z0-9])`, then `_?$`.
///
/// One entry per clause of the pattern, plus the real ids this repository
/// declares, so the table reads as the regex rather than as a sample.
const SNAKE_ID_ACCEPTED: &[(&str, &str)] = &[
    ("a", "`^[a-z]` alone: zero repetitions of the group is legal"),
    ("z", "the other end of the leading class"),
    ("ab", "`[a-z0-9]` repetition, letter"),
    ("a1", "`[a-z0-9]` repetition, digit"),
    ("a1b2c3", "the repetition mixed and repeated"),
    ("a_b", "`_[a-z0-9]`: an underscore GLUED to a following letter"),
    ("a_1", "`_[a-z0-9]`: glued to a following digit"),
    ("a1_2b", "a digit on both sides of the glued underscore"),
    ("a_", "`_?$`: a SINGLE trailing underscore, which ADR-0027 permits"),
    ("ab_cd_", "a glued underscore AND the trailing one in one id"),
    ("region", "a facet id this repository declares"),
    ("line_container", "a binding id this repository declares"),
    ("s1_water_pump", "a scenario pack id, digits inside a segment"),
];

/// Everything the regex refuses, one entry per way it can be refused.
const SNAKE_ID_REJECTED: &[(&str, &str)] = &[
    ("", "`^[a-z]` needs a first character"),
    ("A", "`[a-z]` is lowercase only"),
    ("Region", "an uppercase lead, the shape a JSON author reaches for"),
    ("1a", "a digit cannot lead"),
    ("_a", "an underscore cannot lead"),
    ("_", "nor alone: the trailing `_?` cannot supply the leading `[a-z]`"),
    ("a__b", "`__`: the second underscore is glued to no alnum"),
    ("a__", "`__` at the end: `_?` consumes one and leaves the other"),
    ("ab__cd", "the same, mid-id"),
    ("aB", "an uppercase letter inside the repetition"),
    ("a-b", "`-` is a VALUE token character, never an id one"),
    ("a.b", "`.` likewise"),
    ("a b", "a space — configflux-7xsy's reproducer, unreachable through CUE"),
    ("a;b", "a condition-grammar separator"),
    ("a'b", "the quote character the emitter's literal is delimited by"),
    ("a\"b", "and the one it falls back to"),
    ("a\nb", "a control character, which an echo has to escape"),
    ("a\u{1b}b", "the ESC that opens an ANSI sequence"),
    ("aé", "non-ASCII: every accepted byte is ASCII, so this is refused whole"),
    (
        "environment == 'prod' || replica_class",
        "configflux-mrm6 case 1: the injected equality disjunction",
    ),
    (
        "environment != 'zz' || replica_class",
        "configflux-mrm6 case 2: the injected phantom closed-facet value",
    ),
];

#[test]
fn snake_id_accepts_every_shape_the_regex_accepts() {
    for (id, clause) in SNAKE_ID_ACCEPTED {
        assert!(
            is_snake_id(id),
            "#snakeId accepts {id:?} ({clause}); is_snake_id refused it"
        );
    }
}

#[test]
fn snake_id_rejects_every_shape_the_regex_rejects() {
    for (id, clause) in SNAKE_ID_REJECTED {
        assert!(
            !is_snake_id(id),
            "#snakeId rejects {id:?} ({clause}); is_snake_id accepted it"
        );
    }
}

/// A byte walk over UTF-8 must not split a multi-byte character into bytes it
/// then judges separately: the verdict has to be about the ID, not about its
/// encoding. Every non-ASCII lead is >= 0x80 and falls into the reject arm, so
/// the id is refused once and whole — and a trailing multi-byte character
/// cannot be mistaken for the `_?` the pattern ends with.
#[test]
fn snake_id_refuses_a_multi_byte_character_wherever_it_sits() {
    for id in ["é", "éa", "aé", "a_é", "aéb", "a…"] {
        assert!(!is_snake_id(id), "{id:?} is not ASCII and must be refused");
    }
}

// ---------------------------------------------------------------------------
// D2 — facet values, against `[A-Za-z0-9_.-]+`
// ---------------------------------------------------------------------------

const SYMBOL_TOKEN_ACCEPTED: &[(&str, &str)] = &[
    ("a", "one letter"),
    ("A", "UPPERCASE is legal in a value, unlike in an id"),
    ("1", "one digit"),
    ("dev", "a facet value this repository declares"),
    ("c1", "a catalogue entry id, which reaches this rule as a binding value"),
    ("eu-west-1", "`-`: the region shape D2 exists to keep legal"),
    ("1.5", "`.`: the version/number shape, likewise"),
    ("a_b", "`_`"),
    ("A.B-c_9", "all four extra characters and both letter cases at once"),
];

const SYMBOL_TOKEN_REJECTED: &[(&str, &str)] = &[
    ("", "a value domain entry is never empty"),
    ("a b", "a space"),
    (" a", "a leading space, which a trim would have hidden"),
    ("a'b", "the single quote the emitter's literal is normally delimited by"),
    ("a\"b", "the double quote it falls back to"),
    (
        "a'b\" || environment == \"zz",
        "configflux-mrm6 case 3: BOTH quotes, the class the old hint claimed was refused",
    ),
    ("a;b", "a condition-grammar separator"),
    ("a|b", "the disjunction operator the injection is built from"),
    ("a=b", "the comparison operator likewise"),
    ("a(b", "a grouping character"),
    ("a\nb", "a control character"),
    ("a\u{1b}b", "the ESC that opens an ANSI sequence"),
    ("aé", "non-ASCII"),
    ("de bug", "the value cfx's own suite used to declare (configflux-egyj)"),
];

#[test]
fn symbol_token_accepts_every_shape_the_token_set_accepts() {
    for (value, clause) in SYMBOL_TOKEN_ACCEPTED {
        assert!(
            is_symbol_token(value),
            "the D2 token set accepts {value:?} ({clause}); is_symbol_token refused it"
        );
    }
}

#[test]
fn symbol_token_rejects_every_shape_the_token_set_rejects() {
    for (value, clause) in SYMBOL_TOKEN_REJECTED {
        assert!(
            !is_symbol_token(value),
            "the D2 token set rejects {value:?} ({clause}); is_symbol_token accepted it"
        );
    }
}

// ---------------------------------------------------------------------------
// The relationship between the two
// ---------------------------------------------------------------------------

/// D1 is STRICTLY NARROWER than D2, and the compiler depends on it.
///
/// A catalogue entry id is held to D1 in `validate_catalogues`, and then
/// reaches `validate_facets` a second time as one of the values of the closed
/// facet its binding IS (ADR-0057 §D3). If D1 ever admitted a character D2
/// refuses, an id accepted by the catalogue rule would be refused by the facet
/// rule for the same model — one fault, two contradictory messages, and the
/// entry-id rule silently unable to do its job.
#[test]
fn every_legal_identifier_is_also_a_legal_value_token() {
    for (id, clause) in SNAKE_ID_ACCEPTED {
        assert!(
            is_symbol_token(id),
            "{id:?} ({clause}) is a legal id, so it must survive the value rule too"
        );
    }
}

/// The converse must NOT hold, or D2 has collapsed into D1 and `eu-west-1`
/// stopped being a declarable facet value.
#[test]
fn the_value_token_set_is_strictly_wider_than_the_identifier_rule() {
    for value in ["eu-west-1", "1.5", "A", "prod.eu"] {
        assert!(is_symbol_token(value), "{value:?} must be a legal value");
        assert!(
            !is_snake_id(value),
            "{value:?} must NOT be a legal id — if it is, the two rules have merged"
        );
    }
}
