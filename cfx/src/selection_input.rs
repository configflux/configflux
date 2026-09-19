// SPDX-License-Identifier: BUSL-1.1
//
// How a selection reaches `cfx` from the outside: the repeatable
// `--select facet=option` flags and the `--selection-file` document.
//
// This module exists to hold ONE rule that both inputs share (ADR-0057 §D8):
// binding the same facet twice AT ONE LEVEL is a usage error. Two `--select`
// flags naming one facet with different options, or a duplicate key inside the
// file's `choices`/`context_tags`, are exit-2 failures naming the facet and
// both values. The same pair repeated is accepted — a user who passes an
// argument twice has said one thing twice, not two things.
//
// A flag overriding the file for the same facet is UNCHANGED and remains an
// override: the file and the flags are different levels, and layering them is
// the documented precedence (ADR-0042 §2/§3). The distinction is the whole
// point of the rule — "which of my two answers wins" has an answer across
// levels and no answer within one.
//
// The interpreter's `select` verb already refused the same-level case with
// `E_SELECTION_CONFLICT`; before this module `cfx` silently kept the last
// value, so the two surfaces disagreed about whether a model was even
// well-formed. `cfx` adds no diagnostic namespace of its own (ADR-0042 §3), so
// the refusal is an exit-2 message rather than a new code.
//
// A separate module rather than more of `pipeline.rs`/`main.rs` because both
// are at their lint caps, and because the flag half and the file half of one
// rule belong beside each other — split across two grandfathered files they
// would drift.

use std::collections::BTreeMap;
use std::path::Path;

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use compiler::loader_api::SelectionState;

use crate::input_file::read_input_file;
use crate::pipeline::{parse_select_pair, PipelineError, SelectPair};

/// Parse every `--select facet=option` pair up front so a malformed pair — or a
/// facet bound twice with different options — fails fast (exit 2) before any
/// model I/O.
pub fn parse_selects(raw: &[String]) -> Result<Vec<SelectPair>, PipelineError> {
    let pairs: Vec<SelectPair> = raw
        .iter()
        .map(|arg| parse_select_pair(arg))
        .collect::<Result<_, _>>()?;
    reject_conflicting_selects(&pairs)?;
    Ok(pairs)
}

/// Reject a facet bound twice by the flags with two DIFFERENT options
/// (ADR-0057 §D8). Repeating one pair verbatim is accepted.
///
/// Reported in argument order, first conflict only: the user fixes one
/// contradiction at a time, and a stable "first wins the report" rule keeps the
/// message reproducible for a given command line.
pub(crate) fn reject_conflicting_selects(pairs: &[SelectPair]) -> Result<(), PipelineError> {
    let mut bound: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in pairs {
        let first = bound.entry(pair.facet.as_str()).or_insert(&pair.option);
        if *first != pair.option {
            return Err(PipelineError::usage(format!(
                "facet '{}' selected twice with different options: '{}' and '{}'",
                pair.facet, first, pair.option
            )));
        }
    }
    Ok(())
}

/// Read and parse the `--selection-file` as the existing `SelectionState` JSON
/// shape (ADR-0042 §3). Only `scope`, `context_tags`, and `choices` are
/// consumed; the file's `selection_state_hash` is IGNORED and re-derived
/// canonically by the pipeline, so a hand-written file need not compute it.
///
/// A duplicate key inside `choices`/`context_tags` is refused BEFORE the typed
/// parse, because the typed parse cannot see it (below).
pub fn read_selection_file(path: &Path) -> Result<SelectionState, PipelineError> {
    let bytes = read_input_file(path, "--selection-file")?;
    reject_duplicate_keys(path, &bytes, "--selection-file")?;
    serde_json::from_slice::<SelectionState>(&bytes).map_err(|err| {
        PipelineError::usage(format!(
            "malformed --selection-file '{}' ({err})",
            path.display()
        ))
    })
}

/// Refuse a selection document whose `context_tags` or `choices` object binds
/// one key twice (ADR-0057 §D8).
///
/// Detection has to happen at the RAW JSON level. `serde_json` streams the
/// document, but every map target it can deserialize into — `Map`, `BTreeMap`,
/// `HashMap` — is filled by `insert` in a loop, so a repeated key overwrites
/// its predecessor and the typed value that comes back is indistinguishable
/// from a file that named the key once. `object_keys` below therefore collects
/// keys into a `Vec` through a `visit_map` visitor, which sees every entry the
/// map would have collapsed.
///
/// `label` names the flag in the message so this check can serve any selection
/// document `cfx` reads. The environment-manifest loader (`crate::manifest`,
/// configflux-dkmm.4) carries the same `choices`/`context_tags` objects one
/// level deeper, per environment, and has the same hole; it reuses this
/// function rather than growing a second copy of the rule (configflux-fs1h).
///
/// A document that is not valid JSON, or whose two objects are not objects, is
/// NOT this check's business: it returns `Ok` and lets the typed parse below
/// produce the message that names the real fault.
fn reject_duplicate_keys(path: &Path, bytes: &[u8], label: &str) -> Result<(), PipelineError> {
    let Ok(scan) = serde_json::from_slice::<DuplicateKeyScan>(bytes) else {
        return Ok(());
    };
    // Checked in the order the fields are declared in the document shape, so a
    // file with duplicates in both objects always reports the same one first.
    for (field, keys) in [
        ("context_tags", &scan.context_tags),
        ("choices", &scan.choices),
    ] {
        if let Some(duplicate) = first_duplicate(keys) {
            return Err(PipelineError::usage(format!(
                "malformed {label} '{}': duplicate key '{duplicate}' in {field}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// The two objects of a selection document, reduced to their key sequences.
/// Every other field is ignored: this type answers one question and the typed
/// parse answers the rest.
#[derive(Debug, Deserialize)]
struct DuplicateKeyScan {
    #[serde(default, deserialize_with = "object_keys")]
    context_tags: Vec<String>,
    #[serde(default, deserialize_with = "object_keys")]
    choices: Vec<String>,
}

/// Every key of a JSON object, in document order, duplicates included.
///
/// Values are read as `IgnoredAny`: whether a value is a string is the typed
/// parse's question, and a duplicate key is a fault regardless of what the two
/// values are.
fn object_keys<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct KeysVisitor;

    impl<'de> Visitor<'de> for KeysVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut keys = Vec::new();
            while let Some((key, _)) = access.next_entry::<String, IgnoredAny>()? {
                keys.push(key);
            }
            Ok(keys)
        }
    }

    deserializer.deserialize_map(KeysVisitor)
}

/// The first key that appears more than once, in document order.
pub(crate) fn first_duplicate(keys: &[String]) -> Option<&str> {
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    keys.iter()
        .find(|key| seen.insert(key.as_str(), ()).is_some())
        .map(String::as_str)
}
