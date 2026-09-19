// SPDX-License-Identifier: BUSL-1.1

use crate::ccm_emitter::ConditionModel;
// configflux-py7w: an ingest duplicate reports its code on the refusal itself,
// not through the wording of its message.
use crate::coded_error::{coded_bail, coded_bail_hint};
use crate::product_api::{
    E_INGEST_DUPLICATE_CATALOGUE, E_INGEST_DUPLICATE_FACET, HINT_DUPLICATE_BINDING_ID,
};
use crate::conditions::{for_each_predicate_symbol, parse_condition_expr, ConditionExpr};
use crate::interface_summary::{self, InterfaceSummary};
use crate::ir;
use crate::link_verify::{
    validate_catalogues, validate_component_dependencies, validate_constraints,
    validate_definition_inheritance, validate_facet_bindings_scoped, validate_facets,
    validate_link_summary, validate_parameter_values, Scope,
};
use crate::schema::{self, Component, Config, Facet, Parameter};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// The "Grinder." Holds the accumulated state of the repository.
pub struct Compiler {
    repository: Config,
    chunks: Vec<SourceChunk>,
}

pub(crate) struct SourceChunk {
    pub(crate) source_id: String,
    pub(crate) chunk_hash: String,
    pub(crate) config: Config,
}

/// Wire format of an authored chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkFormat {
    Toml,
    Json,
}

/// Detect a chunk's wire format from its content.
///
/// A JSON object begins with `{` after optional leading whitespace; a TOML
/// document never does (its top level is `key = value` / `[table]` /
/// `# comment`), so a leading brace is an unambiguous discriminator.
///
/// Detection is by *content*, not by a filename extension, on purpose: a
/// `source_id` is an opaque label and need not carry an extension at all — the
/// inline path below names its chunks `inline-<n>`, and the CLI passes whatever
/// string it was given. There is nothing in a `source_id` to dispatch on.
///
/// Since ADR-0056 the chunk vector is ordered by `chunk_hash` and `source_id`
/// is OUTSIDE the `model_hash` preimage entirely (it survives on disk as
/// provenance). So the equivalence this detection serves is now stronger than
/// the ADR-0021 phase-6 gate needed: a CUE-emitted JSON chunk and its
/// equivalent TOML chunk share a `model_hash` under *any* pair of source ids,
/// not only under an identical one.
pub fn detect_chunk_format(content: &str) -> ChunkFormat {
    if content.trim_start().starts_with('{') {
        ChunkFormat::Json
    } else {
        ChunkFormat::Toml
    }
}

/// Reject an authored TOML chunk that carries `inherits` anywhere
/// (configflux-qofj). Post-L2 (ADR-0027) `inherits` resolution moved to CUE
/// whole-pack export and the Rust resolve-time engine was deleted; a TOML
/// `inherits` would otherwise SILENTLY default safety/lifecycle/access at
/// resolve — a silent safety-level downgrade.
fn reject_authored_inherits(config: &Config) -> Result<()> {
    for (def_id, param) in &config.definitions {
        reject_inherits_in_parameter(param, &format!("definitions.{def_id}"))?;
    }
    for (comp_id, component) in &config.components {
        for (param_key, param) in &component.params {
            let path = format!("components.{comp_id}.params.{param_key}");
            reject_inherits_in_parameter(param, &path)?;
        }
    }
    Ok(())
}

/// Recursively reject `inherits` on a parameter and its nested override
/// payloads (`Box<Parameter>`), so a buried `inherits` cannot slip past.
fn reject_inherits_in_parameter(param: &Parameter, path: &str) -> Result<()> {
    if param.inherits.is_some() {
        bail!(
            "Authored TOML at '{path}' carries `inherits`, which is no longer \
             supported: inheritance is resolved by CUE during whole-pack export \
             (ADR-0027). Author this chunk in CUE instead of `inherits` in TOML."
        );
    }
    for (idx, block) in param.overrides.iter().enumerate() {
        reject_inherits_in_parameter(&block.payload, &format!("{path}.overrides[{idx}]"))?;
    }
    Ok(())
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            repository: Config {
                package: "merged_root".to_string(),
                version: "0.0.0".to_string(),
                definitions: Default::default(),
                components: Default::default(),
                artifacts: Default::default(),
                facets: Default::default(),
                constraints: Default::default(),
                catalogues: Default::default(),
                bindings: Default::default(),
            },
            chunks: Vec::new(),
        }
    }

    /// Parses a JSON string (e.g. emitted by the CUE authoring front-end) and
    /// merges it into the repository. The resulting model — and therefore the
    /// chunk hash — is identical to the equivalent TOML chunk (ADR 0021).
    pub fn add_chunk_json(&mut self, content: &str) -> Result<()> {
        let source_id = format!("inline-{}", self.chunks.len());
        self.add_chunk_json_with_source(source_id, content)?;
        Ok(())
    }

    pub fn add_chunk_with_source(
        &mut self,
        source_id: impl Into<String>,
        content: &str,
    ) -> Result<()> {
        let partial: Config = toml::from_str(content).context("Failed to parse TOML chunk")?;
        // TOML-only `inherits` rejection, on the parse entry before resolve so
        // the silent-default path is unreachable via TOML (see the helper).
        reject_authored_inherits(&partial)?;
        self.add_parsed_chunk(source_id.into(), partial)
    }

    pub fn add_chunk_json_with_source(
        &mut self,
        source_id: impl Into<String>,
        content: &str,
    ) -> Result<()> {
        let partial: Config =
            serde_json::from_str(content).context("Failed to parse JSON chunk")?;
        self.add_parsed_chunk(source_id.into(), partial)
    }

    /// Parse and merge a chunk, auto-detecting TOML vs JSON from its content
    /// (see [`detect_chunk_format`]).
    ///
    /// This is the format-agnostic entry point used by the product compile path
    /// (`product_api`): a CUE-emitted JSON chunk and its equivalent TOML chunk,
    /// presented under the same `source_id`, produce a byte-identical CMP
    /// (ADR 0021, phase 6). TOML content never trips the JSON branch. The TOML
    /// branch additionally rejects authored `inherits` at ingest (configflux-qofj).
    pub fn add_chunk_auto(
        &mut self,
        source_id: impl Into<String>,
        content: &str,
    ) -> Result<()> {
        match detect_chunk_format(content) {
            ChunkFormat::Json => self.add_chunk_json_with_source(source_id, content),
            ChunkFormat::Toml => self.add_chunk_with_source(source_id, content),
        }
    }

    /// Shared ingestion tail for both source formats: dedup, validate, merge,
    /// and record the chunk with a model-derived (format-agnostic) hash.
    fn add_parsed_chunk(&mut self, source_id: String, partial: Config) -> Result<()> {
        if self.chunks.iter().any(|chunk| chunk.source_id == source_id) {
            bail!("Duplicate source_id found: '{}'", source_id);
        }

        // Authored-ID snake_case enforcement moved to the CUE authoring layer
        // (`compiler/cue/schema.cue` `#snakeId`, ADR-0027 Decision 4); the
        // Rust `validate_snake_case_ids` ingest check was deleted in B-5.
        //
        // ADR-0063 narrows that for FOUR classes — facet keys, binding ids,
        // catalogue ids, catalogue entry ids — which `link_verify` now
        // re-validates because the compiler interpolates them into synthesized
        // condition clauses and `compile --source` never evaluates CUE. The
        // rule lives at link-verify rather than here so `verify` and `compile`
        // give the same answer; every other authored key stays CUE-only.
        self.merge_partial(partial.clone())?;

        let chunk_hash = ir::chunk_hash_from_config(&partial)?;
        // ADR-0056 §3: two chunks with one `chunk_hash` are rejected here.
        //
        // The package cannot represent the duplicate — chunks are written as
        // `chunk-<chunk_hash>.cfir` and resolved back by the same construction
        // on every read path, so both chunks write ONE file and the loser's
        // `source_id` is lost. Multiplicity would live only in the index's
        // vector, and identity must not depend on state the on-disk form
        // cannot hold. The verdict is not new: `verify_index_integrity` already
        // bails one stage later with "Duplicate chunk hash '<h>' in index".
        // Moving it here changes no outcome, only the diagnostic — the index
        // check knows the hash but not which two sources produced it.
        //
        // Checked after `merge_partial` rather than beside the `source_id`
        // check above because that is where `chunk_hash` first exists, and
        // moving the hash earlier would reorder existing diagnostics: two
        // entity-bearing duplicates are already reported by the duplicate-id
        // rules, which name the offending entity. The only input that reaches
        // here is an entity-free chunk, which merge ignores.
        //
        // Carries no diagnostic code of its own, so it reports the generic
        // ingest code. Since configflux-py7w the wording is free: the duplicate
        // codes below are named on their own refusals, so a phrase appearing
        // here can no longer claim one of them.
        if let Some(existing) = self
            .chunks
            .iter()
            .find(|chunk| chunk.chunk_hash == chunk_hash)
        {
            bail!(
                "Duplicate chunk content: '{}' and '{}' have identical content \
                 (chunk hash {}). Remove one, or give them distinct content.",
                existing.source_id,
                source_id,
                chunk_hash
            );
        }
        self.chunks.push(SourceChunk {
            source_id,
            chunk_hash,
            config: partial,
        });
        Ok(())
    }

    fn merge_partial(&mut self, partial: Config) -> Result<()> {
        // Merge Definitions
        for (key, value) in partial.definitions {
            if self.repository.definitions.contains_key(&key) {
                bail!("Duplicate definition ID found: '{}'", key);
            }
            self.repository.definitions.insert(key, value);
        }

        // Merge Artifacts
        for (key, value) in partial.artifacts {
            if self.repository.artifacts.contains_key(&key) {
                bail!("Duplicate artifact ID found: '{}'", key);
            }
            self.repository.artifacts.insert(key, value);
        }

        // Merge Components
        //
        // Cross-file component unification is now owned by CUE (ADR-0027,
        // Track B): whole-pack CUE evaluation unifies overlapping component
        // definitions before emission, so each component lands in exactly one
        // emitted chunk. The hand-rolled `merge_components` cross-file conflict
        // engine was deleted in B-5 (Decision 5). A component appearing in two
        // chunks is therefore a duplicate, treated identically to duplicate
        // definitions/artifacts above (and caught structurally by
        // `build_ir_index`'s one-entity-one-chunk invariant).
        for (key, incoming) in partial.components {
            if self.repository.components.contains_key(&key) {
                bail!("Duplicate component ID found: '{}'", key);
            }
            self.repository.components.insert(key, incoming);
        }

        // Merge Facets (ADR-0047). A facet is a pack-global domain declared by
        // AT MOST ONE chunk (§2); a second chunk declaring the same key is a
        // hard ingest error (E_INGEST_DUPLICATE_FACET), symmetric with the
        // duplicate-ID invariant above but with a dedicated code. Facets pass
        // through verbatim — no inheritance or gap-fill (contrast components).
        // The per-facet shape invariants (non-empty/unique values, default in
        // values) and the closed-domain condition check are re-validated at
        // link time in `validate_facets`, over the fully merged model.
        for (key, facet) in partial.facets {
            if self.repository.facets.contains_key(&key) {
                coded_bail!(
                    E_INGEST_DUPLICATE_FACET,
                    "Facet '{}' is declared in more than one chunk",
                    key
                );
            }
            // ADR-0057 §D3: bindings share the facet id space, so a facet named
            // like a binding declared by an earlier chunk is the same collision.
            if self.repository.bindings.contains_key(&key) {
                coded_bail!(
                    E_INGEST_DUPLICATE_FACET,
                    "Facet '{}' is declared in more than one chunk: '{}' is already \
                     declared as a binding, and a binding shares the facet id space",
                    key,
                    key
                );
            }
            self.repository.facets.insert(key, facet);
        }

        // Merge Constraints (ADR-0054 §1). Pack-global and verbatim, like
        // facets: no inheritance, no gap-fill, no merge. A repeated id is a
        // duplicate, phrased like the definition/component/artifact duplicates
        // above and carrying no code of its own, so it lands in the same generic
        // ingest diagnostic. ADR-0054 adds no diagnostic code.
        for (key, constraint) in partial.constraints {
            if self.repository.constraints.contains_key(&key) {
                bail!("Duplicate constraint ID found: '{}'", key);
            }
            self.repository.constraints.insert(key, constraint);
        }

        // Merge Catalogues (ADR-0057 §D2). Pack-global and verbatim like
        // facets, with a dedicated code: a duplicated TABLE is a different
        // authoring mistake from a duplicated domain, and the fix ("keep the
        // table in one chunk and let the others bind to it") is different too.
        for (key, catalogue) in partial.catalogues {
            if self.repository.catalogues.contains_key(&key) {
                coded_bail!(
                    E_INGEST_DUPLICATE_CATALOGUE,
                    "Catalogue '{}' is declared in more than one chunk",
                    key
                );
            }
            self.repository.catalogues.insert(key, catalogue);
        }

        // Merge Bindings (ADR-0057 §D3). A binding IS a facet, so the two share
        // ONE id space and the collision is checked in BOTH directions: this
        // loop runs after the facet loop above, so `repository.facets` already
        // holds this chunk's own facets and an intra-chunk collision is caught
        // here, while a facet colliding with an EARLIER chunk's binding is
        // caught by the facet loop's own binding check.
        //
        // Both refusals carry E_INGEST_DUPLICATE_FACET — the shared id space
        // means one code — with the duplicate-BINDING remedy rather than that
        // code's default, and the message says "binding" so the author knows
        // which declaration to rename.
        for (key, binding) in partial.bindings {
            if self.repository.bindings.contains_key(&key) {
                coded_bail_hint!(
                    E_INGEST_DUPLICATE_FACET,
                    HINT_DUPLICATE_BINDING_ID,
                    "Binding '{}' is declared in more than one chunk",
                    key
                );
            }
            if self.repository.facets.contains_key(&key) {
                coded_bail_hint!(
                    E_INGEST_DUPLICATE_FACET,
                    HINT_DUPLICATE_BINDING_ID,
                    "Binding '{}' is declared in more than one chunk: a binding shares \
                     the facet id space, and '{}' is also declared as a facet",
                    key,
                    key
                );
            }
            self.repository.bindings.insert(key, binding);
        }
        Ok(())
    }

    pub fn get_repo(&self) -> &Config {
        &self.repository
    }

    /// The ingested chunks, in `--source` order.
    ///
    /// ADR-0058's object compile needs what `emit_ir` reads internally — each
    /// chunk's `package`, its content hash, and the config the IR chunk is
    /// written from — and re-parsing the sources to get them would put a second
    /// parse of every input on the path, with a second chance to disagree about
    /// what a chunk says. Crate-internal: `SourceChunk` is not a public shape.
    pub(crate) fn source_chunks(&self) -> &[SourceChunk] {
        &self.chunks
    }

    pub fn link_and_verify(&self) -> Result<()> {
        self.link_and_verify_with(&self.interface_summaries())
    }

    /// One [`InterfaceSummary`] per chunk (ADR-0057 §D9). Built once per
    /// compile and shared by `link_and_verify`, the emit, and the object
    /// grouping, because building one walks every authored condition.
    pub(crate) fn interface_summaries(&self) -> Vec<InterfaceSummary> {
        self.chunks
            .iter()
            .map(|chunk| interface_summary::summarize(&chunk.config, &chunk.source_id))
            .collect()
    }

    fn link_and_verify_with(&self, summaries: &[InterfaceSummary]) -> Result<()> {
        verify_complete_model(&self.repository, summaries)
    }

    /// Verify this model and write its Compiled Model Package.
    ///
    /// The write goes through [`crate::link_emit::write_package`] — the linker's own
    /// emit (ADR-0058 §D8). There is one writer, so a package produced from
    /// `--source` chunks and one produced from objects are the same file set by
    /// construction rather than by two code paths agreeing.
    pub fn emit_ir(&self, output_dir: impl AsRef<Path>) -> Result<ir::IrIndex> {
        let summaries = self.interface_summaries();
        self.link_and_verify_with(&summaries)?;
        let chunks = crate::link_emit::link_chunks_of(self, &summaries)?;
        let index = crate::link_emit::build_package_index(&chunks)?;
        crate::link_emit::write_package(output_dir.as_ref(), &chunks, &index)?;
        Ok(index)
    }
}

/// Every check that needs the WHOLE model, in the order the compile path has
/// always run them (ADR-0057 §D9, ADR-0058 §D4).
///
/// A free function over a `Config` rather than a method, because `link`
/// assembles its model from the linked objects' chunk files and holds no
/// [`Compiler`]. Both callers reach the same rules through this one function,
/// which is what makes "the linker cannot write a package `compile` would have
/// rejected" true by construction rather than by two lists agreeing.
///
/// The order is not cosmetic. Catalogue SHAPE comes before any rule that reads
/// a catalogue's entry roster, and the summary checks come before the facet and
/// constraint checks that treat a binding as one of the declared facets.
pub(crate) fn verify_complete_model(
    repository: &Config,
    summaries: &[InterfaceSummary],
) -> Result<()> {
    validate_definition_inheritance(&repository.definitions)?;
    validate_component_dependencies(&repository.components)?;
    // Catalogue SHAPE first: a binding's checks read a catalogue's entry
    // roster, and a roster is only meaningful once the table itself is known to
    // be well formed.
    validate_catalogues(&repository.catalogues)?;
    // configflux-2yiq: the parameter-side twin of the catalogue rule above. Both
    // halves reject the same thing — an authored float the canonical JSON every
    // hash preimage is built from cannot represent — so they sit together.
    validate_parameter_values(&repository.definitions, &repository.components)?;
    validate_link_summary(summaries)?;
    // ADR-0057 §D3: from here down a binding is simply one of the declared
    // facets. Passing the effective map — rather than teaching each validator
    // about bindings — is what makes "nothing downstream special-cases a
    // binding" hold: a condition naming an entry outside a binding's domain is
    // E_FACET_VALUE_UNDECLARED for free, and a constraint may name a binding
    // without `validate_constraints` changing at all.
    let facets = effective_facets(repository);
    validate_facets(&facets, &repository.components, &repository.definitions)?;
    // ADR-0064 D2: a declared parameter-to-facet binding is checked against the
    // same effective map, in the slot right after the facets it names are known
    // to be well formed. `Scope::Complete` is what makes the whole-model
    // one-handle-per-facet rule (D2.4) run here rather than being deferred.
    validate_facet_bindings_scoped(&repository.components, &facets, Scope::Complete)?;
    validate_constraints(&repository.constraints, &facets)
}

/// The authored facets plus the closed facet every binding IS (ADR-0057 §D3).
/// The two namespaces share one id space, so the union is a plain insert with
/// no collision to resolve — ingest already rejected one.
fn effective_facets(repository: &Config) -> HashMap<String, Facet> {
    let mut facets = repository.facets.clone();
    facets.extend(interface_summary::binding_facets(
        &repository.catalogues,
        &repository.bindings,
    ));
    facets
}

/// The DECLARED facets of a linked model, from its merged summary alone
/// (ADR-0058 §D4 stage 2).
///
/// The header-driven twin of what the compile path used to read off its chunks.
/// Every declared facet, plus the closed facet each binding IS — projected here
/// from the binding's catalogue roster rather than through
/// [`interface_summary::binding_facets`], because a linker holds the roster and
/// not the `Catalogue` it came from.
///
/// `doc` is dropped and `default` is carried only for bindings: the three
/// consumers ([`ccm_clauses`], [`synthesize_facet_cardinality`],
/// [`collect_facet_domains`]) read `values` and `open` and nothing else, so
/// anything more would be a field no reader can observe.
pub(crate) fn declared_facets_from_summary(
    merged: &interface_summary::MergedSummary,
) -> BTreeMap<String, schema::Facet> {
    let mut declared: BTreeMap<String, schema::Facet> = BTreeMap::new();
    for (name, values) in &merged.facet_domains {
        declared.insert(
            name.clone(),
            schema::Facet {
                values: values.clone(),
                default: None,
                open: merged.open_facets.contains(name),
                doc: None,
            },
        );
    }
    // Bindings last, mirroring the `extend` the chunk-walking version ended
    // with: a binding and a facet cannot share an id (ingest rejects it), so
    // this overwrites nothing in practice and the order is a statement about
    // which namespace is authoritative, not a tiebreak that fires.
    for (id, link) in &merged.binding_links {
        let Some(entries) = merged.catalogue_entries.get(&link.catalogue) else {
            continue;
        };
        declared.insert(
            id.clone(),
            schema::Facet {
                values: entries.clone(),
                default: link.default.clone(),
                open: false,
                doc: None,
            },
        );
    }
    declared
}

/// The `.ccm` clause channel: the symbol universe, and nothing asserted.
///
/// Pass 1 — the walk that gathers the raw selector strings in a stable,
/// content-derived order — now lives in [`interface_summary::summarize`] and
/// reaches here through the object headers, in the canonical order ADR-0058 §A2
/// fixes: objects by unit name ascending, chunks by `chunk_hash` ascending,
/// then definitions by id, components by id, override order. The caller owns
/// that order; this function never re-sorts.
///
/// **No selector is asserted** (ADR-0054 §5.1). Every one of them — a
/// component's activation `condition` and a parameter override's branch
/// `condition` alike — is an *inclusion selector*, so it contributes its
/// `(facet, value)` symbols and nothing else, via the same ADR-0047 tautology
/// [`synthesize_symbol_introduction`] emits. The *authored* root conjuncts come
/// from `constraints` ([`ccm_constraints`]).
///
/// That closes the configflux-9xxq defect class at the PRODUCER, not in the
/// encoding: the emitter folds this list into the root like any other channel,
/// so all that keeps a selector off it is this function emitting nothing but
/// tautologies — pinned by example in `compiler_core_tests`.
///
/// Only conditions that parse under the typed condition grammar are included —
/// exactly mirroring the legacy `register_condition` fall-through, where a
/// condition that does not parse widens no facet domain and forms no group.
/// This keeps the product compile from failing on a pass-through condition
/// string the BDD grammar cannot represent. (A `constraints` entry that does
/// not parse is the opposite: a hard ingest error in
/// `link_verify::validate_constraints`, because a policy that cannot be
/// understood must never be silently dropped.) De-duplication keeps an
/// identical condition that appears on multiple entities from inflating the
/// clause set (the BDD AND-fold is idempotent, so this is also a
/// semantics-preserving normalization).
pub(crate) fn ccm_clauses(
    selectors: &[interface_summary::Clause],
    declared: &BTreeMap<String, schema::Facet>,
) -> Vec<String> {
    // Keep first-seen, parseable, non-empty selectors only. Every survivor is
    // replaced by one symbol-introducing tautology per `(facet, value)` pair it
    // names, so its symbols still land while the AND-fold sees only `TRUE`
    // (ADR-0054 §5.1).
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut clauses: Vec<String> = Vec::new();
    for selector in selectors {
        let trimmed = selector.condition.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(expr) = parse_condition_expr(trimmed) else {
            continue;
        };
        for clause in synthesize_selector_symbols(&expr) {
            if seen.insert(clause.clone()) {
                clauses.push(clause);
            }
        }
    }

    // ADR-0047 §4, Amendment 1: append one symbol-introducing tautology per
    // DECLARED facet value, AFTER all authored selector clauses, in
    // facet-name-ascending order. This lands every declared value — including a
    // default arm no condition names — into the symbol universe WITHOUT
    // asserting any intra-facet constraint on the permissive BDD root. The
    // ordering is the byte-stability commitment: deterministic, non-perturbing
    // of the authored clauses, so `ccm_hash` is a pure function of (authored
    // selectors, declared facets). A model that declares no facet appends
    // nothing and is byte-identical to the pre-ADR path.
    clauses.extend(synthesize_facet_clauses(declared));
    clauses
}

/// The `.ccm` root conjuncts: the authored `constraints` namespace
/// id-ascending, then the ADR-0057 §D4 lowered `derive` and `accepts`
/// conjuncts (ADR-0054 §5.1).
///
/// The only producer of *authored* root conjuncts — not of root conjuncts as
/// such ([`synthesize_facet_cardinality`]). Constraints are pack-global — no
/// inheritance, no gap-fill, no merge — so the collapse is flat, and duplicate
/// ids across chunks are rejected at ingest.
///
/// The lowered conjuncts carry the two reserved prefixes —
/// `derive:<binding>:<source>=<value>` and `accepts:<component>.<slot>` — which
/// an authored id can never spell, because a constraint id is snake_case and
/// cannot contain `:`. Everything downstream reads them as ordinary roster
/// entries: they get a `root_index`, they are folded into the BDD root, and
/// `loader_api::unsat_attribution` names them in an unsat core exactly as it
/// names an authored policy.
///
/// They are appended AFTER the authored block rather than merged into it: §D4
/// fixes the fold order as authored, then derive, then accepts, and id-sorting
/// the union would interleave them (`accepts:` sorts before an authored
/// `alpha_rule`). A model that declares neither appends nothing and its
/// `ccm_hash` does not move.
///
/// Conditions are passed through VERBATIM rather than re-serialized: the text
/// is what the §5.4 manifest roster shows the operator, so it must stay the
/// string the author wrote. Every constraint is known to parse by this point
/// (`link_verify::validate_constraints` makes a parse failure a hard ingest
/// error), so unlike a selector there is no skip-on-unparseable path here — a
/// policy is never silently dropped.
pub(crate) fn ccm_constraints(merged: &interface_summary::MergedSummary) -> Vec<(String, String)> {
    let by_id: BTreeMap<String, String> = merged
        .clauses
        .iter()
        .map(|clause| (clause.id.clone(), clause.condition.clone()))
        .collect();
    let mut out: Vec<(String, String)> = by_id.into_iter().collect();
    out.extend(
        crate::lowering::lowered_root_conjuncts_from_summary(
            &merged.binding_links,
            &merged.requirements,
        )
        .into_iter()
        .map(|lowered| (lowered.id, lowered.condition)),
    );
    out
}

/// The constraint model a linked package's `.ccm` is emitted from, built from
/// the merged object headers alone (ADR-0058 §D4 stage 2).
///
/// Every field is a function of the merged summary and `model_hash`, so
/// `compile` and `link` cannot build a different model for one set of objects —
/// they call this.
pub(crate) fn condition_model_from_summary(
    model_hash: &str,
    merged: &interface_summary::MergedSummary,
) -> ConditionModel {
    let declared = declared_facets_from_summary(merged);
    ConditionModel {
        bound_model_hash: model_hash.to_string(),
        clauses: ccm_clauses(&merged.selectors, &declared),
        constraints: ccm_constraints(merged),
        cardinality: synthesize_facet_cardinality(&declared),
        facet_domains: collect_facet_domains(&declared),
    }
}

/// The symbol-universe contribution of one branch selector: a
/// [`synthesize_symbol_introduction`] tautology per `(facet, value)` pair the
/// selector names, in left-to-right DFS pre-order, first-sight only.
///
/// configflux-9xxq / ADR-0054 §5.1. Each emitted string lowers to the canonical
/// `TRUE` terminal, so the AND-fold is a no-op (`and(root, TRUE) = root`) while
/// the `var_order` predicate walk still collects every symbol. Emitting one
/// tautology per pair — rather than wrapping the whole condition — keeps the
/// pairs, and hence the symbols, exactly those the selector contributed before:
/// the default `FacetNameAscending` heuristic orders symbols from the *set* of
/// pairs, so `ccm.symbols.json` is byte-identical across this change.
///
/// Both `==` and `!=` atoms are collected, because either operator names the
/// same variable (`compile_predicate` lowers `NotEq` to `not(var)`).
fn synthesize_selector_symbols(expr: &ConditionExpr) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for_each_predicate_symbol(expr, |tag, value| {
        if seen.insert((tag.to_string(), value.to_string())) {
            out.push(synthesize_symbol_introduction(tag, value));
        }
    });
    out
}

/// Synthesize the ADR-0047 §4 (Amendment 1) declared-facet symbols: one
/// symbol-introducing tautology per declared value, in `facets` key order
/// (facet-name-ascending) and declared-value order within each facet. The
/// returned strings are appended after the authored clauses by
/// `collect_ccm_clauses`; each is re-parsed by `parse_clauses` and lowered by
/// `compile_expr` to the canonical TRUE terminal, so it lands the
/// `{facet}.{value}` symbol in the variable order / `ccm.symbols.json` (via the
/// `var_order` predicate walk) while the AND-fold into the single permissive
/// BDD root is a no-op (`and(root, TRUE) = root`) — every declared value,
/// including a default arm that no condition names, becomes a first-class
/// symbol without asserting any constraint. This is the F2 fix.
///
/// **Closed and open facets are handled identically here** — symbol-
/// introduction only. Cardinality is a separate, later channel
/// ([`synthesize_facet_cardinality`]) and deliberately does not ride on this
/// pass: this one must stay a pure no-op on the BDD root so a model that
/// declares a facet but no policy keeps a permissive root.
fn synthesize_facet_clauses(facets: &BTreeMap<String, schema::Facet>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (name, facet) in facets {
        for value in &facet.values {
            out.push(synthesize_symbol_introduction(name, value));
        }
    }
    out
}

/// Synthesize the ADR-0054 §5.2 intra-facet cardinality conjuncts: the
/// soundness floor that makes a *constraint* mean the same thing on the solver
/// surfaces as it does under a concrete resolve.
///
/// Per DECLARED facet, in facet-name-ascending then declared-value order:
///
/// * **Closed facet** — `exactly_one_of(f == 'v1', …, f == 'vN')`: at-least-one
///   conjoined with pairwise at-most-one. Sound because a closed domain is
///   exhaustive by declaration (`E_FACET_VALUE_UNDECLARED` rejects anything
///   outside it), so exactly one declared value holds in every completion. A
///   one-value closed facet degenerates to the bare `f == 'v1'` — the grammar's
///   cardinality operators require N ≥ 2 arguments, and with an exhaustive
///   single-value domain that literal is what "exactly one of" means.
/// * **Open facet** — pairwise at-most-one ONLY, emitted as one
///   `f != 'vi' || f != 'vj'` mutex per unordered pair in ascending `(i, j)`
///   order. An open domain is extensible, so "some *declared* value holds" is
///   not a theorem and at-least-one would be unsound.
/// * **Undeclared (condition-inferred) facet** — nothing. Its domain is an
///   artifact of what conditions happened to mention rather than a
///   declaration, so asserting cardinality over it would assert something the
///   author never wrote.
///
/// Why this is needed at all, and why the hero example is NOT the proof: option
/// validity is an *existential* SAT query (`is_var_sat_under`), a selection
/// asserts only the positive literal, and `==`/`!=` share one variable. A
/// constraint that positively equates a facet to a value (`log_level == 'info'`)
/// therefore UNDER-prunes without at-most-one — the query
/// `root ∧ log_level.info ∧ log_level.debug` is satisfiable because the two
/// variables are independent, so `debug` stays on the options list while a
/// concrete resolve correctly rejects it. At-least-one closes the mirror gap: a
/// genuinely over-constrained model would otherwise be satisfiable by setting
/// every arm false, and `explain` would fail to report it. Cardinality is
/// neither necessary nor sufficient for the hero example's disjunction-of-
/// negations constraint; it is a general soundness floor (ADR-0054 §5.3).
///
/// This does NOT re-create the ADR-0047 Amendment 1 trap. That amendment
/// removed exactly this synthesis, and was right to under its premise: every
/// selector `condition` was a hard root assertion, so a synthesized
/// at-most-one collided with a *forced* guard and pruned the unforced default
/// arm. ADR-0054 §5.1 removes the guards from the root, so there is nothing
/// left to collide with — a per-arm query fails only when a genuine constraint
/// forbids that arm.
///
/// No auxiliary variables: the emitted strings lower through ADR-0006 §4's
/// `ExactlyOneOf` path, whose AMO is pairwise, so the `.ccm` variable space
/// keeps modelling only real `(tag, value)` pairs and the byte-layout contract
/// is untouched. The emitter's advisory `EXACTLY_ONE_OF_PAIRWISE_AMO_BOUND`
/// (16, a stderr warning and never a build failure) applies unchanged.
fn synthesize_facet_cardinality(facets: &BTreeMap<String, schema::Facet>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (name, facet) in facets {
        if facet.open {
            // At-most-one only: one pairwise mutex ¬(vi ∧ vj) per unordered
            // pair, in ascending (i, j) order.
            for i in 0..facet.values.len() {
                for j in (i + 1)..facet.values.len() {
                    out.push(format!(
                        "{} || {}",
                        facet_ne_predicate(name, &facet.values[i]),
                        facet_ne_predicate(name, &facet.values[j])
                    ));
                }
            }
        } else if facet.values.len() == 1 {
            out.push(facet_eq_predicate(name, &facet.values[0]));
        } else if facet.values.len() >= 2 {
            let args: Vec<String> = facet
                .values
                .iter()
                .map(|value| facet_eq_predicate(name, value))
                .collect();
            out.push(format!("exactly_one_of({})", args.join(", ")));
        }
    }
    out
}

/// Project the declared facets down to the value domains the emitter needs to
/// expand a facet-to-facet comparison (configflux-secb.2 / ADR-0057 §D5).
///
/// Declared ORDER is preserved per facet, because that order is the pinned
/// AND-fold order of the expansion and therefore determines emitted bytes
/// (ADR-0006 §5). Both closed and open facets are included: ADR-0057 §D5
/// requires only that both operands be DECLARED. For an open facet the
/// equivalence ranges over its declared values, which is what the author
/// wrote down; a value outside that set is not something the model can reason
/// about here.
///
/// Deliberately not `facet.open`-aware and deliberately not filtered: a facet
/// missing from this map is an internal error in the expansion, not a silent
/// no-op, so the map must contain exactly what `declared_facets` does.
fn collect_facet_domains(
    facets: &BTreeMap<String, schema::Facet>,
) -> crate::conditions::FacetDomains {
    facets
        .iter()
        .map(|(name, facet)| (name.clone(), facet.values.clone()))
        .collect()
}

/// A symbol-introducing tautology `f == 'v' || f != 'v'` for one declared
/// value. `compile_expr` lowers `x ∨ ¬x` to the canonical TRUE terminal, so the
/// AND-fold into the BDD root is a no-op; the `var_order` walk collects the
/// `{f}.{v}` symbol from the `==` predicate. Reuses the quote-selection of
/// [`facet_eq_predicate`] / [`facet_ne_predicate`] so a value carrying a single
/// quote stays representable.
fn synthesize_symbol_introduction(name: &str, value: &str) -> String {
    format!(
        "{} || {}",
        facet_eq_predicate(name, value),
        facet_ne_predicate(name, value)
    )
}

/// The first `(facet, value)` pair whose symbol-introduction clause the
/// condition grammar cannot parse — in ascending facet-name order and declared
/// -value order within a facet — or `None` when every pair parses.
///
/// A BACKSTOP since ADR-0063, and no longer the guard. The guard is the ingest
/// charset rule in [`crate::link_verify`] (`is_snake_id` / `is_symbol_token`),
/// which `verify_complete_model`, `verify_ir_dir` and the object compile all
/// reach through `validate_facets` and `validate_catalogues` — so a DECLARED
/// facet key or value that is not a symbol token is refused long before the
/// emitter, by `verify` and `compile` alike.
///
/// configflux-mrm6 is why that rule exists and why it could not live here.
/// This is a PARSEABILITY oracle, not a validity one, and it is NOT a charset
/// rule: it reports the pair whose clause the grammar REJECTS and says nothing
/// about a pair whose clause the grammar ACCEPTS. A key or a value crafted to
/// close its own literal and continue with valid grammar parsed, so this
/// returned `None` for it and the model compiled — with a phantom value in a
/// closed facet's domain, or a declared value silently truncated. Do not
/// describe this function, or the diagnostic built from it, as a guard on what
/// a facet may contain.
///
/// What still reaches it is not a second class of value. These domains are
/// `ConditionModel::facet_domains`, which [`collect_facet_domains`] builds from
/// [`declared_facets_from_summary`] alone — the declared facets, plus the closed
/// facet each binding IS. No condition-inferred value lands here: the linker
/// never reads `imports.facets` into a domain (that map is header information,
/// not a link obligation), and the widening that does infer one —
/// `loader_api::shared_ops::register_facet_domains` — runs on the consumption
/// side over a `SelectionConstraintModel`, long after this.
///
/// So every pair here has passed the ingest rule, but by two different routes.
/// `compile --source` verifies the repository before the emit and then builds
/// its headers in memory from those same chunks, so the rule covers these
/// domains exactly. The link path verifies the chunk BODIES it loaded, and
/// these domains come from the on-disk object HEADERS merged before that — a
/// gap while the only header-versus-body check compared id SETS, and closed by
/// ADR-0063 Amendment 1 §2: `link_load::check_header_matches_bodies` rebuilds
/// each header from the bodies and refuses any difference, value domains and
/// entry rosters included. Headers equal bodies, and bodies are validated, so
/// the two routes now agree.
///
/// This stays anyway, as defence in depth. It is the last thing between a
/// domain and the emitted clause text, it costs one parse per declared pair,
/// and it is the only check in the chain that does not depend on the ingest
/// rule and the header rebuild both being correct. Nothing here should be read
/// as a claim that it would catch what they let through — it would not; it
/// catches only what the grammar rejects.
///
/// The introduction is a faithful proxy for the whole synthesized set: the
/// cardinality conjuncts are built from the same two predicates over the same
/// pair, so a pair whose introduction parses yields synthesized clauses that
/// parse too.
pub(crate) fn unrepresentable_facet_symbol(
    domains: &crate::conditions::FacetDomains,
) -> Option<(&str, &str)> {
    domains.iter().find_map(|(name, values)| {
        values
            .iter()
            .find(|value| {
                parse_condition_expr(&synthesize_symbol_introduction(name, value)).is_err()
            })
            .map(|value| (name.as_str(), value.as_str()))
    })
}

/// A synthesized equality predicate `f == '<value>'`. Facet values are
/// effectively identifiers; the condition grammar's quoted literal has no
/// escape syntax, so a value containing the chosen quote is unrepresentable.
/// Prefer single quotes (the authored convention) and fall back to double
/// quotes when the value itself contains a single quote.
///
/// The fallback was never a defence, and since ADR-0063 it is unreachable for
/// a DECLARED value: `link_verify::is_symbol_token` refuses a facet value
/// holding a quote of either kind at ingest, and `is_snake_id` does the same
/// for the keys this interpolates BARE.
///
/// That is where the guard had to go, because it cannot live here. Nothing in
/// this function quotes or escapes either argument — `name` is interpolated
/// bare and the quote character is picked by a `contains` test over `value` —
/// and the grammar offers nothing to escape INTO. configflux-mrm6 measured
/// what that cost: a name or a value crafted to close its own literal
/// continued into the surrounding clause as valid grammar, parsed, and
/// compiled to a model carrying a phantom value or a truncated one. The
/// fallback stays for a value that reached here without passing that rule (see
/// [`unrepresentable_facet_symbol`]), and configflux-7xsy is why the refusal it
/// backstops is reported as a model fault rather than as a failed write.
pub(crate) fn facet_eq_predicate(name: &str, value: &str) -> String {
    if value.contains('\'') {
        format!("{name} == \"{value}\"")
    } else {
        format!("{name} == '{value}'")
    }
}

pub(crate) fn facet_ne_predicate(name: &str, value: &str) -> String {
    if value.contains('\'') {
        format!("{name} != \"{value}\"")
    } else {
        format!("{name} != '{value}'")
    }
}

pub fn verify_ir_dir(output_dir: impl AsRef<Path>) -> Result<()> {
    let output_dir = output_dir.as_ref();
    let index_path = output_dir.join("index.cfir.json");
    let index = ir::load_index(&index_path)?;
    ir::verify_index_integrity(&index, output_dir)?;

    let mut definitions: HashMap<String, Parameter> = HashMap::new();
    let mut components: HashMap<String, Component> = HashMap::new();
    let mut artifacts: HashMap<String, schema::Artifact> = HashMap::new();
    let mut facets: HashMap<String, schema::Facet> = HashMap::new();
    let mut constraints: HashMap<String, schema::Constraint> = HashMap::new();
    let mut catalogues: HashMap<String, schema::Catalogue> = HashMap::new();
    let mut bindings: HashMap<String, schema::Binding> = HashMap::new();
    // ADR-0057 §D9: the re-verification path runs the SAME whole-model checks
    // over the SAME types as the compile path, so a package that only
    // `verify_ir_dir` ever sees cannot slip past a rule the compiler enforces.
    //
    // That parity is maintained BY HAND. The list at the bottom of this function
    // is a second spelling of `verify_complete_model`'s, over maps read back
    // from chunk files rather than over an authored `Config`, so a rule added
    // there has to be added here too, in the same slot. configflux-2yiq is what
    // it costs when one is not: the catalogue half of the finiteness rule
    // arrived here for free inside `validate_catalogues` and the parameter half
    // did not, leaving this comment claiming a coverage the code lacked.
    let mut summaries: Vec<crate::interface_summary::InterfaceSummary> = Vec::new();

    for chunk_ref in &index.chunks {
        let chunk_path = output_dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        let chunk = ir::load_chunk(&chunk_path)?;
        summaries.push(crate::interface_summary::summarize_ir_chunk(&chunk));

        for (id, def) in chunk.definitions {
            if definitions.insert(id.clone(), def).is_some() {
                bail!("Definition '{}' appears in multiple IR chunks", id);
            }
        }
        for (id, comp) in chunk.components {
            if components.insert(id.clone(), comp).is_some() {
                bail!("Component '{}' appears in multiple IR chunks", id);
            }
        }
        for (id, artifact) in chunk.artifacts {
            if artifacts.insert(id.clone(), artifact).is_some() {
                bail!("Artifact '{}' appears in multiple IR chunks", id);
            }
        }
        for (id, facet) in chunk.facets {
            if facets.insert(id.clone(), facet).is_some() {
                bail!("Facet '{}' is declared in more than one chunk", id);
            }
        }
        for (id, constraint) in chunk.constraints {
            if constraints.insert(id.clone(), constraint).is_some() {
                bail!("Duplicate constraint '{}' appears across chunks", id);
            }
        }
        for (id, catalogue) in chunk.catalogues {
            if catalogues.insert(id.clone(), catalogue).is_some() {
                bail!("Catalogue '{}' is declared in more than one chunk", id);
            }
        }
        for (id, binding) in chunk.bindings {
            if bindings.insert(id.clone(), binding).is_some() {
                bail!("Binding '{}' is declared in more than one chunk", id);
            }
        }
    }

    validate_definition_inheritance(&definitions)?;
    validate_component_dependencies(&components)?;
    validate_catalogues(&catalogues)?;
    // configflux-2yiq: the parameter half of the finiteness rule, in the slot
    // `verify_complete_model` runs it, so this list now matches that one check
    // for check and order for order.
    //
    // Defense in depth rather than a live path: `ir::load_chunk` is
    // `serde_json::from_slice`, JSON has no non-finite literal, and an
    // overflowing one is refused as `number out of range` before a `Value` is
    // built — so nothing this function can be handed today reaches the rule.
    // That guard belongs to the decoder rather than to this contract, and this
    // function is what a caller applies to a package another producer wrote.
    validate_parameter_values(&definitions, &components)?;
    validate_link_summary(&summaries)?;
    // ADR-0057 §D3: a binding is one of the declared facets from here down,
    // exactly as in `Compiler::link_and_verify_with`.
    facets.extend(interface_summary::binding_facets(&catalogues, &bindings));
    validate_facets(&facets, &components, &definitions)?;
    // ADR-0063 D4's parity rule: the same slot in this list as in
    // `verify_complete_model`, so the linker cannot write a package `compile`
    // would have rejected (ADR-0064 D2).
    validate_facet_bindings_scoped(&components, &facets, Scope::Complete)?;
    validate_constraints(&constraints, &facets)?;

    Ok(())
}

#[cfg(test)]
#[path = "compiler_core_tests.rs"]
mod compiler_core_tests;
