// SPDX-License-Identifier: BUSL-1.1

//! The object header — one unit's interface, content-addressed (ADR-0058 §D2,
//! amended §A1).
//!
//! An **object** is what `compile-object` writes for ONE unit: the unit's chunk
//! IR files exactly as the package would store them, a deterministic provenance
//! sidecar, and this header. The header is what a linker reads instead of the
//! chunks — who the unit exports, what it still needs from elsewhere, the
//! declarations a sibling unit must be checked against, the authored clauses,
//! and the hashes of the interface objects this unit was compiled against.
//!
//! **Why it is its own type and not [`crate::interface_summary::InterfaceSummary`]
//! serialized** (ADR-0058 §A1). That type is per CHUNK and carries `source_id`,
//! which is a path; ADR-0056's standing invariant is that no path string may
//! reach an identity, and `object_hash` is an identity. The header is therefore
//! the unit-level MERGE of the unit's per-chunk summaries, with the paths
//! dropped. It embeds the summary's own shapes ([`Exports`], [`Clause`],
//! [`BindingLink`], [`RequirementLink`]) rather than mirroring them, so the two
//! cannot drift; only [`Imports`] is re-expressed, because the summary maps an
//! imported id to the authored sites that reference it and the header carries
//! the ids alone.
//!
//! Ordering is fixed so that neither `--source` order nor path spelling can
//! change a byte: chunks are visited in `chunk_hash` ascending order (the
//! ADR-0056 §2 comparator), keyed maps merge by key, `requirements` stay
//! component-then-slot ascending, and `clauses` and `selectors` are chunk order
//! then in-chunk order.

use crate::interface_summary::{BindingLink, Clause, Exports, InterfaceSummary, RequirementLink};
use crate::ir::sha256_hex;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Wire version of `object.json`. Bumped whenever the header's shape changes,
/// exactly as `IrChunk::format_version` is: a header written under another
/// version is rejected rather than read under this one.
///
/// It did NOT move when configflux-p0jz.2 added `selectors` and `open_facets`.
/// ADR-0058 §D2 sketched the header's clause list as "conditions with owner
/// path, constraints with id" and fixed the wire version at 1; the §A1
/// reconciliation replaced the sketch with the ADR-0057 §D9 shapes and dropped
/// the conditions along the way. Adding them back completes version 1 rather
/// than superseding it, and the ADR — accepted, and the authority here — still
/// names 1. Nothing on disk is affected either way: an object is a build
/// product written to `--out`, none is committed, and the format has never
/// shipped in a release.
pub const OBJECT_FORMAT_VERSION: u32 = 1;

/// The header's filename inside an object directory.
pub const OBJECT_HEADER_FILENAME: &str = "object.json";

/// The ids a unit REFERENCES but does not declare itself — the link-time
/// obligations (ADR-0058 §D3).
///
/// Re-expressed rather than reused from [`crate::interface_summary::Imports`]:
/// that type maps an id to the authored sites that reference it, which is what
/// a *diagnostic* needs, and the header carries the ids alone.
///
/// An id is in here if some chunk of the unit references it and no chunk of the
/// unit declares it. The unit's INTERFACES are deliberately not subtracted: an
/// import stays recorded whether or not this compile happened to be given the
/// object that provides it, because it is the linker's job to check that every
/// one is provided exactly once, and a header that dropped the resolved ones
/// would not say which objects the unit needs.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectImports {
    /// `depends_on` targets declared by no chunk of this unit.
    pub components: BTreeSet<String>,
    /// `inherits` targets declared by no chunk of this unit.
    pub definitions: BTreeSet<String>,
    /// Facets (or bindings) named by a condition, constraint or `derive` table
    /// and declared by no chunk of this unit. A facet declared nowhere at all
    /// keeps its legacy condition-inferred domain (ADR-0047 §3); it is recorded
    /// here as information, not as an obligation the linker must satisfy.
    pub facets: BTreeSet<String>,
    /// Bindings a component `requires` and no chunk of this unit declares.
    pub bindings: BTreeSet<String>,
    /// Catalogues a binding names and no chunk of this unit declares.
    pub catalogues: BTreeSet<String>,
}

/// One interface object this unit was compiled against.
///
/// The hash is what makes the compilation checkable: the linker refuses a set
/// in which an object was compiled against a DIFFERENT version of an interface
/// than the one being linked (`E_LINK_INTERFACE_MISMATCH`, ADR-0058 §D4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InterfaceRef {
    pub unit: String,
    pub object_hash: String,
}

/// One unit's compiled interface — the contents of `object.json`.
///
/// Field order is the wire order: `serde_json` emits a struct's fields in
/// declaration order, and the header is hashed over exactly that rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectHeader {
    pub format_version: u32,
    /// The unit's name — the `package` value every chunk of it carries
    /// (ADR-0058 §D1). The object's identity in every link message and lock
    /// entry.
    pub unit: String,
    /// This unit's chunk hashes, ascending by the lowercase-hex string. The
    /// same comparator the package index commits to (ADR-0056 §2).
    pub chunk_hashes: Vec<String>,
    pub exports: Exports,
    pub imports: ObjectImports,
    pub facet_domains: BTreeMap<String, Vec<String>>,
    /// Of `facet_domains`, the facets this unit declares `open: true`. The
    /// cardinality channel of the constraint model asserts at-least-one over a
    /// CLOSED domain and only at-most-one over an open one (ADR-0054 §5.2), so
    /// a header without this flag would make the linker's `.ccm` differ from
    /// `compile`'s for any model that declares an open facet.
    pub open_facets: BTreeSet<String>,
    pub catalogue_entries: BTreeMap<String, Vec<String>>,
    pub binding_links: BTreeMap<String, BindingLink>,
    /// Component requirements, component-then-slot ascending (ADR-0057 §D4).
    pub requirements: Vec<RequirementLink>,
    /// Authored constraints in chunk-hash order, then in-chunk id order.
    pub clauses: Vec<Clause>,
    /// The unit's inclusion SELECTORS — every component activation `condition`
    /// and every parameter-override branch `condition`, keyed by the authored
    /// entity path that carries it — in the canonical clause order ADR-0058 §A2
    /// fixes: chunks by `chunk_hash` ascending, then, within a chunk,
    /// definitions by id, components by id, override order.
    ///
    /// This is what makes §D4 stage 2 header-only. The `.ccm` clause channel is
    /// one symbol-introducing tautology per `(facet, value)` pair a selector
    /// names (ADR-0054 §5.1), so a linker that could not read the selectors
    /// could not build the symbol universe without opening a chunk file — and
    /// not opening one is exactly what an object header exists for.
    pub selectors: Vec<Clause>,
    /// The interface objects this unit was compiled against, sorted.
    pub interfaces: Vec<InterfaceRef>,
    /// SHA-256 of this header serialized WITHOUT this field.
    pub object_hash: String,
}

/// The `object_hash` preimage: every field of the header except the hash.
///
/// A separate `Serialize`-only struct rather than a `skip_serializing`
/// attribute, mirroring `ir::IrIndexContent`: the preimage is a contract in its
/// own right, and writing it out is what makes "which bytes are hashed"
/// readable instead of inferable from an attribute.
#[derive(Serialize)]
struct ObjectHeaderPreimage<'a> {
    format_version: u32,
    unit: &'a str,
    chunk_hashes: &'a [String],
    exports: &'a Exports,
    imports: &'a ObjectImports,
    facet_domains: &'a BTreeMap<String, Vec<String>>,
    open_facets: &'a BTreeSet<String>,
    catalogue_entries: &'a BTreeMap<String, Vec<String>>,
    binding_links: &'a BTreeMap<String, BindingLink>,
    requirements: &'a [RequirementLink],
    clauses: &'a [Clause],
    selectors: &'a [Clause],
    interfaces: &'a [InterfaceRef],
}

impl ObjectHeader {
    /// Merge one unit's per-chunk summaries into its header.
    ///
    /// `summaries` MUST already be ordered by `chunk_hash` ascending and
    /// `chunk_hashes` must be that same order; the caller owns the sort because
    /// it is the caller that holds the hashes. `interfaces` is sorted here so
    /// the argument order on the command line cannot reach the bytes.
    ///
    /// `unit` is passed rather than read from the summaries because a unit with
    /// no chunks would otherwise have no name, and the unit-agreement check
    /// that produced it has already proved every chunk agrees.
    pub fn from_summaries(
        unit: &str,
        chunk_hashes: Vec<String>,
        summaries: &[InterfaceSummary],
        interfaces: Vec<InterfaceRef>,
    ) -> Self {
        let mut exports = Exports::default();
        let mut facet_domains: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut open_facets: BTreeSet<String> = BTreeSet::new();
        let mut catalogue_entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut binding_links: BTreeMap<String, BindingLink> = BTreeMap::new();
        let mut requirements: Vec<RequirementLink> = Vec::new();
        let mut clauses: Vec<Clause> = Vec::new();
        let mut selectors: Vec<Clause> = Vec::new();

        for summary in summaries {
            union_ids(&mut exports.definitions, &summary.exports.definitions);
            union_ids(&mut exports.components, &summary.exports.components);
            union_ids(&mut exports.artifacts, &summary.exports.artifacts);
            union_ids(&mut exports.facets, &summary.exports.facets);
            union_ids(&mut exports.bindings, &summary.exports.bindings);
            union_ids(&mut exports.catalogues, &summary.exports.catalogues);
            union_ids(&mut exports.constraints, &summary.exports.constraints);

            // First declaration wins. A facet, catalogue or binding is declared
            // by at most ONE chunk (ADR-0047 §2, ADR-0057 §D2/§D3) and ingest
            // rejects a second, so there is no second value to lose.
            for (name, values) in &summary.facet_domains {
                facet_domains
                    .entry(name.clone())
                    .or_insert_with(|| values.clone());
            }
            open_facets.extend(summary.open_facets.iter().cloned());
            for (name, entries) in &summary.catalogue_entries {
                catalogue_entries
                    .entry(name.clone())
                    .or_insert_with(|| entries.clone());
            }
            for (name, link) in &summary.binding_links {
                binding_links
                    .entry(name.clone())
                    .or_insert_with(|| link.clone());
            }

            requirements.extend(summary.requirements.iter().cloned());
            // Chunk order, then in-chunk order: `summarize` already sorted each
            // chunk's clauses by id, and the chunks arrive in `chunk_hash`
            // order, so this is the canonical clause order of ADR-0058 §A2
            // without a second sort.
            clauses.extend(summary.clauses.iter().cloned());
            // Same reasoning, and the order matters more here: the selector
            // list is the `.ccm` clause channel's input, so chunk-hash order
            // then in-chunk walk order IS ADR-0058 §A2's canonical order.
            selectors.extend(summary.selectors.iter().cloned());
        }

        // Component-then-slot ascending across the whole unit — a property of
        // the unit, not of the order its chunks happened to be visited in.
        requirements.sort_by(|a, b| a.component.cmp(&b.component).then_with(|| a.slot.cmp(&b.slot)));

        let imports = collect_imports(summaries, &exports);

        let mut interfaces = interfaces;
        interfaces.sort();
        interfaces.dedup();

        let mut header = Self {
            format_version: OBJECT_FORMAT_VERSION,
            unit: unit.to_string(),
            chunk_hashes,
            exports,
            imports,
            facet_domains,
            open_facets,
            catalogue_entries,
            binding_links,
            requirements,
            clauses,
            selectors,
            interfaces,
            object_hash: String::new(),
        };
        header.object_hash = header.compute_object_hash();
        header
    }

    /// SHA-256 (lowercase hex) of this header serialized without `object_hash`.
    ///
    /// Public because it is the check a reader performs: recompute, compare to
    /// the stored value, and know whether the header was edited after it was
    /// written.
    pub fn compute_object_hash(&self) -> String {
        let preimage = ObjectHeaderPreimage {
            format_version: self.format_version,
            unit: &self.unit,
            chunk_hashes: &self.chunk_hashes,
            exports: &self.exports,
            imports: &self.imports,
            facet_domains: &self.facet_domains,
            open_facets: &self.open_facets,
            catalogue_entries: &self.catalogue_entries,
            binding_links: &self.binding_links,
            requirements: &self.requirements,
            clauses: &self.clauses,
            selectors: &self.selectors,
            interfaces: &self.interfaces,
        };
        // Serializing a struct of owned/borrowed primitives cannot fail; the
        // fallback keeps the function total rather than panicking on a case
        // that does not exist.
        let bytes = serde_json::to_vec(&preimage).unwrap_or_default();
        sha256_hex(&bytes)
    }

    /// The first preimage field two headers disagree on, in wire order, or
    /// `None` when all thirteen agree.
    ///
    /// The list mirrors [`ObjectHeaderPreimage`] field for field and in the
    /// same order, so the field a diagnostic names is the first one a reader
    /// diffing the two renderings top-to-bottom would reach. `object_hash` is
    /// deliberately absent from it: the hash is a function of those thirteen,
    /// so a difference there is only ever a restatement of one of them, and
    /// naming it would report the symptom instead of the field.
    pub(crate) fn first_difference(&self, other: &Self) -> Option<&'static str> {
        [
            ("format_version", self.format_version == other.format_version),
            ("unit", self.unit == other.unit),
            ("chunk_hashes", self.chunk_hashes == other.chunk_hashes),
            ("exports", self.exports == other.exports),
            ("imports", self.imports == other.imports),
            ("facet_domains", self.facet_domains == other.facet_domains),
            ("open_facets", self.open_facets == other.open_facets),
            ("catalogue_entries", self.catalogue_entries == other.catalogue_entries),
            ("binding_links", self.binding_links == other.binding_links),
            ("requirements", self.requirements == other.requirements),
            ("clauses", self.clauses == other.clauses),
            ("selectors", self.selectors == other.selectors),
            ("interfaces", self.interfaces == other.interfaces),
        ]
        .into_iter()
        .find_map(|(field, agrees)| (!agrees).then_some(field))
    }

    /// This header's own reference, for a unit that compiles against it.
    pub fn as_interface_ref(&self) -> InterfaceRef {
        InterfaceRef {
            unit: self.unit.clone(),
            object_hash: self.object_hash.clone(),
        }
    }

    /// Canonical `object.json` bytes: pretty JSON with a trailing newline.
    ///
    /// Pretty rather than compact because a header is read by people as often
    /// as by the linker, and every map in it is a `BTreeMap`/`BTreeSet`, so the
    /// rendering is a function of the value alone.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>> {
        let mut bytes =
            serde_json::to_vec_pretty(self).context("Failed to serialize object header")?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Write `object.json` into an existing object directory.
    pub fn write_to_dir(&self, dir: &Path) -> Result<()> {
        let path = dir.join(OBJECT_HEADER_FILENAME);
        std::fs::write(&path, self.to_canonical_json()?)
            .with_context(|| format!("Failed to write object header '{}'", path.display()))
    }

    /// Read and validate the header of an object directory.
    ///
    /// The chunk files are NOT opened: reading a header is the whole of what
    /// compiling against an interface costs (ADR-0058 §D3), and that is only
    /// true if this function never widens.
    pub fn read_from_dir(dir: &Path) -> Result<Self> {
        let path = dir.join(OBJECT_HEADER_FILENAME);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("Failed to read object header '{}'", path.display()))?;
        let header: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse object header '{}'", path.display()))?;
        if header.format_version != OBJECT_FORMAT_VERSION {
            anyhow::bail!(
                "Object '{}' declares header format_version {}, but this build reads {}",
                dir.display(),
                header.format_version,
                OBJECT_FORMAT_VERSION
            );
        }
        let recomputed = header.compute_object_hash();
        if recomputed != header.object_hash {
            anyhow::bail!(
                "Object '{}' header hashes to {}, but records object_hash {}; it was edited \
                 after it was written",
                dir.display(),
                recomputed,
                header.object_hash
            );
        }
        Ok(header)
    }
}

fn union_ids(into: &mut BTreeSet<String>, from: &BTreeSet<String>) {
    for id in from {
        into.insert(id.clone());
    }
}

/// The unit's imports: every id its chunks reference, minus every id its own
/// chunks declare.
///
/// The facet namespace subtracts the unit's BINDINGS as well as its facets: a
/// binding IS a declared closed facet whose values are its catalogue's entries
/// (ADR-0057 §D3), so a condition naming a binding this unit declares is not an
/// import.
fn collect_imports(summaries: &[InterfaceSummary], exports: &Exports) -> ObjectImports {
    let mut imports = ObjectImports::default();
    for summary in summaries {
        imports
            .components
            .extend(summary.imports.components.keys().cloned());
        imports
            .definitions
            .extend(summary.imports.definitions.keys().cloned());
        imports.facets.extend(summary.imports.facets.keys().cloned());
        imports
            .bindings
            .extend(summary.imports.bindings.keys().cloned());
        imports
            .catalogues
            .extend(summary.imports.catalogues.keys().cloned());
    }
    imports.components.retain(|id| !exports.components.contains(id));
    imports
        .definitions
        .retain(|id| !exports.definitions.contains(id));
    imports
        .facets
        .retain(|id| !exports.facets.contains(id) && !exports.bindings.contains(id));
    imports.bindings.retain(|id| !exports.bindings.contains(id));
    imports
        .catalogues
        .retain(|id| !exports.catalogues.contains(id));
    imports
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod object_tests;
