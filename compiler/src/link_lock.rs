// SPDX-License-Identifier: BUSL-1.1

//! The lockfile — checked, never fetched (ADR-0058 §D5).
//!
//! A lockfile pins, per unit, the `object_hash` an integration expects:
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "objects": {
//!     "site_catalogue": { "object_hash": "9f2c…", "source": "" }
//!   }
//! }
//! ```
//!
//! `link --lock <path>` refuses a set that does not match those pins;
//! `link --write-lock <path>` produces the file from a set that linked. The
//! conventional filename is `configflux.lock`, and nothing here depends on it —
//! any path is accepted, because which file an integration reviews is the
//! integration's business.
//!
//! **What the product does NOT do.** It never fetches an object, and `source`
//! is why that has to be said out loud: it is free text an operator records
//! about where a unit came from, and no code path in this repository reads it
//! for any purpose. Objects reach the linker the way build inputs always reach
//! a build — a monorepo checkout, a submodule, an artifact store, a CI download
//! — and the linker's job begins after they have arrived. That is the same
//! boundary ADR-0032 drew for reference tooling, and it is drawn here on
//! purpose: a pin format that fetches is a package manager, and a package
//! manager is a supply chain.
//!
//! **What the two checks mean, and why both exist.** A pin that does not equal
//! the object being linked (`E_LINK_LOCK_MISMATCH`) says the integration is
//! about to ship something other than what was reviewed. A pin nothing linked
//! (`E_LINK_LOCK_UNLINKED`) says the reviewed set is not the set being built —
//! a unit was dropped, and a lock that stayed quiet about it would be a record
//! of a build nobody ran. The second is waived by `--lock-allow-extra`, because
//! a deliberate subset link — an agent linking only the closure it needs — is a
//! real use and it is the caller who knows which of the two it is doing.
//!
//! The lock is a pure function of the linked headers, so its bytes cannot
//! depend on `--object` order, on where the objects sit on disk, or on anything
//! else a second machine would spell differently.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::object::ObjectHeader;
use crate::product_api::{
    Diagnostic, DiagnosticSeverity, E_COMPILE_EMIT_FAILED, E_LINK_LOCK_INVALID,
    E_LINK_LOCK_MISMATCH, E_LINK_LOCK_UNLINKED,
};

/// Wire version of the lockfile, frozen at 1 (ADR-0058 §D5).
///
/// A file at any other version is refused rather than read under this one, for
/// the reason every other versioned artifact here is: a pin that was written to
/// mean something else is worse than no pin.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

/// A lockfile, in the wire order it is written in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockFile {
    pub schema_version: u32,
    /// Unit name to pin, sorted by unit name — the `BTreeMap` is the sort.
    pub objects: BTreeMap<String, LockEntry>,
}

/// One unit's pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockEntry {
    /// The `object_hash` the integration expects for this unit.
    pub object_hash: String,
    /// Free text about where the unit came from. Optional in the file and
    /// written as an empty string when no `--lock-source` names the unit; read
    /// for nothing, ever.
    #[serde(default)]
    pub source: String,
}

/// A unit name, held to the rule the `package` field is held to:
/// `^[a-z]([a-z0-9]|_[a-z0-9])*_?$`.
///
/// Public because the CLI checks `--lock-source <unit>=<text>` against exactly
/// this rule before a lockfile is opened. One rule, one implementation: a lock
/// key and a `--lock-source` unit that disagreed about what a unit name is
/// would let an operator write a note that could never attach to anything.
pub fn check_unit_name(unit: &str) -> Result<()> {
    let bytes = unit.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        bail!(
            "'{unit}' is not a unit name: a unit is snake_case and starts with a lowercase \
             letter (^[a-z]([a-z0-9]|_[a-z0-9])*_?$)"
        );
    }
    let mut previous_was_underscore = false;
    for &byte in &bytes[1..] {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_was_underscore = false;
        } else if byte == b'_' {
            if previous_was_underscore {
                bail!("'{unit}' is not a unit name: a unit never carries two '_' in a row");
            }
            previous_was_underscore = true;
        } else {
            bail!(
                "'{unit}' is not a unit name: a unit is lowercase letters, digits and single \
                 underscores (^[a-z]([a-z0-9]|_[a-z0-9])*_?$)"
            );
        }
    }
    Ok(())
}

/// Read a lockfile and check that it is one (`E_LINK_LOCK_INVALID`).
///
/// Everything this refuses is a fault in the FILE rather than in the linked
/// set: it could not be read, it is not JSON of this shape, it carries a key
/// this version does not know, it is at another `schema_version`, or one of its
/// keys is not a unit name. An unknown key is named rather than ignored,
/// because the keys a future version might add are exactly the ones that would
/// change what a pin means, and silently dropping one would make an old binary
/// claim to have honoured a lock it only half read.
pub(crate) fn read_lock(path: &str) -> Result<LockFile, Diagnostic> {
    let bytes = std::fs::read(path).map_err(|err| {
        invalid(
            format!("The lockfile '{path}' could not be read: {err}"),
            "Check the path, or write one from a link with `--write-lock`",
        )
    })?;
    let lock: LockFile = serde_json::from_slice(&bytes).map_err(|err| {
        invalid(
            format!("The lockfile '{path}' is not a lockfile: {err}"),
            "A lockfile is {\"schema_version\": 1, \"objects\": {\"<unit>\": \
             {\"object_hash\": \"<hex>\", \"source\": \"<optional text>\"}}}; write one with \
             `link --write-lock`",
        )
    })?;
    if lock.schema_version != LOCK_SCHEMA_VERSION {
        return Err(invalid(
            format!(
                "The lockfile '{}' is at schema_version {}, but this build reads {}",
                path, lock.schema_version, LOCK_SCHEMA_VERSION
            ),
            "Use a binary built for that lockfile version, or rewrite the lock with \
             `link --write-lock --force-lock`",
        ));
    }
    for unit in lock.objects.keys() {
        check_unit_name(unit).map_err(|err| {
            invalid(
                format!("The lockfile '{path}' pins '{unit}', which is not a unit name: {err}"),
                "A lock is keyed by unit — the `package` value every chunk of the unit \
                 declares — not by a path or a repository name",
            )
        })?;
    }
    Ok(lock)
}

/// The two §D5 checks, in the order the ADR lists them.
///
/// Mismatch first: it is the question the lock exists to answer, and reporting
/// "a unit is missing from the build" ahead of "the build is not the reviewed
/// one" would put the smaller news first.
pub(crate) fn check_lock(
    lock: &LockFile,
    headers: &[ObjectHeader],
    allow_extra: bool,
    path: &str,
) -> Result<(), Diagnostic> {
    // A Vec rather than a map: two objects claiming one unit is
    // `E_LINK_DUPLICATE_UNIT`, raised in the stage-1 checks that follow, and a
    // map would silently drop one of them before it got there.
    let mut linked: Vec<(&str, &str)> = headers
        .iter()
        .map(|header| (header.unit.as_str(), header.object_hash.as_str()))
        .collect();
    linked.sort_unstable();

    for (unit, object_hash) in &linked {
        match lock.objects.get(*unit) {
            Some(entry) if entry.object_hash == *object_hash => continue,
            Some(entry) => {
                return Err(mismatch(
                    format!(
                        "The lockfile '{}' pins unit '{}' at object_hash {}, but the object \
                         linked here is {}",
                        path, unit, entry.object_hash, object_hash
                    ),
                    "Link the object the lock pins, or renew the pin with \
                     `link --write-lock --force-lock` once the change has been reviewed",
                ));
            }
            None => {
                return Err(mismatch(
                    format!(
                        "Unit '{unit}' is linked at object_hash {object_hash}, but the lockfile \
                         '{path}' pins no object for it"
                    ),
                    "Add the unit to the lock with `link --write-lock --force-lock`, or link \
                     only the units the lock pins",
                ));
            }
        }
    }

    if allow_extra {
        return Ok(());
    }
    let present: BTreeSet<&str> = linked.iter().map(|(unit, _)| *unit).collect();
    for unit in lock.objects.keys() {
        if !present.contains(unit.as_str()) {
            return Err(unlinked(format!(
                "The lockfile '{path}' pins unit '{unit}', but no object for it was linked"
            )));
        }
    }
    Ok(())
}

/// The bytes a lock for this linked set has, as a function of the set alone.
///
/// Pretty-printed with a trailing newline because the file is committed and
/// reviewed: a lock whose diff is one long line tells a reviewer nothing about
/// which pin moved.
pub(crate) fn lock_bytes(
    headers: &[ObjectHeader],
    sources: &BTreeMap<String, String>,
) -> Result<Vec<u8>, Diagnostic> {
    let lock = LockFile {
        schema_version: LOCK_SCHEMA_VERSION,
        objects: headers
            .iter()
            .map(|header| {
                (
                    header.unit.clone(),
                    LockEntry {
                        object_hash: header.object_hash.clone(),
                        source: sources.get(&header.unit).cloned().unwrap_or_default(),
                    },
                )
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&lock).map_err(|err| {
        emit_failed(format!("The lockfile could not be serialized: {err}"))
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Write the lock for a linked set, refusing to clobber a different one.
///
/// The refusal carries `E_LINK_LOCK_MISMATCH` rather than a code of its own,
/// because it is the same fault the `--lock` check reports from the other
/// side: what the file pins is not what was linked. `--force-lock` is the
/// operator saying they have looked at the difference.
pub(crate) fn write_lock(
    path: &str,
    headers: &[ObjectHeader],
    sources: &BTreeMap<String, String>,
    force: bool,
) -> Result<(), Diagnostic> {
    let bytes = lock_bytes(headers, sources)?;
    let target = Path::new(path);
    if !force && target.exists() {
        let existing = std::fs::read(target).map_err(|err| {
            invalid(
                format!("The lockfile '{path}' exists but could not be read: {err}"),
                "Make the file readable, or overwrite it with `--force-lock`",
            )
        })?;
        if existing != bytes {
            return Err(mismatch(
                format!(
                    "The lockfile '{path}' differs from the one this link would write, so it \
                     was left as it is"
                ),
                "Review the difference and re-run with `--force-lock` to renew the pins",
            ));
        }
    }
    std::fs::write(target, &bytes).map_err(|err| {
        emit_failed(format!("The lockfile '{path}' could not be written: {err}"))
    })
}

// ----------------------------------------------------------------------------
// Internals
// ----------------------------------------------------------------------------

fn invalid(message: String, hint: &str) -> Diagnostic {
    diagnostic(E_LINK_LOCK_INVALID, message, hint)
}

fn mismatch(message: String, hint: &str) -> Diagnostic {
    diagnostic(E_LINK_LOCK_MISMATCH, message, hint)
}

fn unlinked(message: String) -> Diagnostic {
    diagnostic(
        E_LINK_LOCK_UNLINKED,
        message,
        "Link the object the lock pins, drop the entry with `link --write-lock --force-lock`, \
         or pass `--lock-allow-extra` if the subset is deliberate",
    )
}

fn emit_failed(message: String) -> Diagnostic {
    diagnostic(
        E_COMPILE_EMIT_FAILED,
        message,
        "Choose a writable path for the lockfile and check the directory's permissions",
    )
}

fn diagnostic(code: &str, message: String, hint: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message,
        source_id: None,
        entity_path: None,
        hint: Some(hint.to_string()),
    }
}

#[cfg(test)]
#[path = "link_lock_tests.rs"]
mod link_lock_tests;
