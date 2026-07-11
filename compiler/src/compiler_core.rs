// SPDX-License-Identifier: BUSL-1.1

use crate::ccm_emitter::{
    count_model_variables, emit_ccm_dir, emit_ccm_dir_with_budget, emit_ccm_dir_with_progress,
    ConditionModel, EmitBudgetOutcome,
};
// configflux-9pjy.3 / ADR-0039 §7: the compile-time progress tracker the
// emit chain reports against. `None`/absent keeps the byte-identical
// default path (ADR-0005 Amendment 2).
use crate::progress::ProgressTracker;
use crate::resource_budget::{derive_knobs, ResourceBudget};
use crate::conditions::parse_condition_expr;
use crate::ingest_merge::build_ir_index;
use crate::ir;
use crate::link_verify::{
    validate_component_dependencies, validate_definition_inheritance, validate_facets,
};
use crate::schema::{self, Component, Config, Parameter};
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
/// Detection is by *content*, not by a filename extension, on purpose: the
/// chunk's `source_id` is folded into `model_hash` (see [`ir::IrChunkRef`] —
/// chunks are sorted by and hashed with their `source_id`), so a CUE-emitted
/// JSON chunk and its equivalent TOML chunk must be presented under an
/// *identical* `source_id` to produce a byte-identical CMP (ADR 0021, phase 6).
/// A format-carrying extension would perturb the `source_id` and rotate the
/// hash, defeating the differential equivalence gate.
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
        self.merge_partial(partial.clone())?;

        let chunk_hash = ir::chunk_hash_from_config(&partial)?;
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
                bail!("Facet '{}' is declared in more than one chunk", key);
            }
            self.repository.facets.insert(key, facet);
        }
        Ok(())
    }

    pub fn get_repo(&self) -> &Config {
        &self.repository
    }

    pub fn link_and_verify(&self) -> Result<()> {
        validate_definition_inheritance(&self.repository.definitions)?;
        validate_component_dependencies(&self.repository.components)?;
        validate_facets(
            &self.repository.facets,
            &self.repository.components,
            &self.repository.definitions,
        )
    }

    pub fn emit_ir(&self, output_dir: impl AsRef<Path>) -> Result<ir::IrIndex> {
        self.link_and_verify()?;

        let output_dir = output_dir.as_ref();
        std::fs::create_dir_all(output_dir).with_context(|| {
            format!("Failed to create IR output dir '{}'", output_dir.display())
        })?;

        for chunk in &self.chunks {
            let ir_chunk =
                ir::IrChunk::from_config(&chunk.source_id, &chunk.chunk_hash, &chunk.config);
            let filename = format!("chunk-{}.cfir", chunk.chunk_hash);
            let path = output_dir.join(filename);
            let file = std::fs::File::create(&path)
                .with_context(|| format!("Failed to create IR chunk '{}'", path.display()))?;
            let mut writer = std::io::BufWriter::new(file);
            serde_json::to_writer(&mut writer, &ir_chunk)
                .with_context(|| format!("Failed to write IR chunk '{}'", path.display()))?;
        }

        let index = build_ir_index(&self.chunks)?;
        let index_path = output_dir.join("index.cfir.json");
        let file = std::fs::File::create(&index_path)
            .with_context(|| format!("Failed to create IR index '{}'", index_path.display()))?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &index)
            .with_context(|| format!("Failed to write IR index '{}'", index_path.display()))?;

        ir::verify_index_integrity(&index, output_dir)?;
        let manifest = ir::CmpManifest::from_index(&index);
        let manifest_path = output_dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME);
        ir::write_cmp_manifest(&manifest_path, &manifest)?;

        Ok(index)
    }

    /// Emit the v2 multi-part `.ccm` artifact directory as a sibling of the
    /// CMP package, under `<cmp_output_dir>/ccm` (configflux-9hi2).
    ///
    /// This is purely additive: it does not read, rewrite, or perturb any CMP
    /// file, so the CMP package layout and its `model_hash` are byte-stable.
    /// The artifact is bound to `model_hash` via `ConditionModel::
    /// bound_model_hash`, which the solver later byte-compares against the CMP
    /// `model_hash` on load (ADR-0005 §9). The `model_hash` passed here MUST be
    /// the CMP package's `model_hash` (i.e. `IrIndex::config_hash`, which
    /// `open_model` asserts equals `CmpManifest::model_hash`).
    ///
    /// `cluster_size` threads ADR-0012 §2 scope partitioning. `None` collapses
    /// to a single partition (`usize::MAX`), preserving the FAMA/SPLOT-shape
    /// single-partition v2 layout. Clauses are the model's component/override
    /// `condition`s, harvested from the in-memory chunks (the same condition
    /// strings the legacy `register_condition` selection path consumes).
    ///
    /// `budget` (configflux-9pjy.2 / ADR-0039) is the soft resource budget.
    /// When `None`, this is byte-for-byte identical to the pre-budget path:
    /// no memo cap is applied and `cluster_size` flows through unchanged.
    /// When `Some`, `derive_knobs` maps it to an apply-memo cap (a
    /// byte-neutral cache lever) and, if the projected unique table would
    /// exceed the budget, a derived `cluster_size`. Per ADR-0012
    /// Amendment 1 the explicit `cluster_size` argument **always wins**:
    /// `effective = cluster_size.or(derived.cluster_size)`.
    ///
    /// `progress` (configflux-9pjy.3 / ADR-0039 §7) is an optional
    /// compile-time progress tracker. When `Some`, it reports the
    /// VarOrder → BddApplyLoop band to its sink as the .ccm is emitted;
    /// the caller emits the terminal Serialize completion via
    /// [`ProgressTracker::finish`]. Progress is a SEPARATE stream — it
    /// never enters the artifact bytes (ADR-0005 Amendment 2), so a
    /// `progress` of `None` (or a `NullSink`-backed tracker) is
    /// byte-for-byte identical to today.
    pub(crate) fn emit_ccm_sibling_with_progress(
        &self,
        cmp_output_dir: impl AsRef<Path>,
        model_hash: &str,
        cluster_size: Option<usize>,
        budget: Option<&ResourceBudget>,
        progress: Option<&mut ProgressTracker>,
    ) -> Result<(std::path::PathBuf, EmitBudgetOutcome)> {
        let ccm_dir = cmp_output_dir.as_ref().join("ccm");
        let model = ConditionModel {
            bound_model_hash: model_hash.to_string(),
            clauses: self.collect_ccm_clauses(),
        };

        // Derive internal knobs from the soft budget. With no budget,
        // `memo_cap` stays `None` (byte-identical default apply memos) and
        // no `cluster_size` is derived, so the explicit value flows
        // through unchanged.
        let (effective_cluster_size, memo_cap) = match budget {
            Some(budget) => {
                let hint = count_model_variables(&model);
                let derived = derive_knobs(budget, Some(hint));
                // ADR-0012 Amendment 1: explicit cluster_size always wins.
                let effective = cluster_size.or(derived.cluster_size);
                (effective, Some(derived.memo_cap))
            }
            None => (cluster_size, None),
        };

        // configflux-9pjy.4 / ADR-0039 §5: convert the soft RSS target from
        // MiB (the operator-facing unit on `ResourceBudget`) to KiB (the
        // unit `proc_rss` samples and the in-crate apply loop's live shrink
        // compares against). `None` ⇒ no RSS budget ⇒ no sampling, no
        // shrink, byte-identical emission.
        let rss_budget_kib = budget
            .and_then(|b| b.max_rss_mb)
            .map(|mb| mb.saturating_mul(1024));

        let mut outcome = match (progress, effective_cluster_size, memo_cap) {
            // A progress tracker is wired: route through the progress-aware
            // entry point. The knob values still flow exactly as below
            // (`usize::MAX` collapses to a single partition); only the
            // observational progress stream is added — output bytes are
            // unchanged (ADR-0005 Amendment 2).
            (Some(tracker), cluster, cap) => emit_ccm_dir_with_progress(
                &model,
                &ccm_dir,
                "facet-name-ascending",
                "in-crate",
                cluster.unwrap_or(usize::MAX),
                cap,
                rss_budget_kib,
                tracker,
            ),
            // No progress, no partitioning override, no budget memo cap, no
            // RSS budget: take the exact pre-budget single-partition path
            // (byte-identical). Its outcome is the zeroed default.
            (None, None, None) if rss_budget_kib.is_none() => {
                emit_ccm_dir(&model, &ccm_dir).map(|()| EmitBudgetOutcome::default())
            }
            // No progress, but an effective cluster size, a budget memo cap,
            // an RSS budget, or any combination: go through the budget-aware
            // entry point so the live shrink + advisory are exercised.
            (None, cluster, cap) => emit_ccm_dir_with_budget(
                &model,
                &ccm_dir,
                "facet-name-ascending",
                "in-crate",
                cluster.unwrap_or(usize::MAX),
                cap,
                rss_budget_kib,
            ),
        }
        .with_context(|| format!("Failed to emit .ccm artifact at '{}'", ccm_dir.display()))?;
        // Record the effective cluster_size (the explicit-vs-derived
        // precedence result) on the outcome so the advisory can name a
        // concrete value, but only when a budget was actually in play —
        // the unbudgeted path leaves the outcome zeroed.
        if budget.is_some() {
            outcome.effective_cluster_size = Some(effective_cluster_size.unwrap_or(usize::MAX));
        }
        Ok((ccm_dir, outcome))
    }

    /// Harvest the model's constraint `condition`s into the
    /// `ccm_emitter`-grammar clause list. Walks every chunk's components and
    /// definitions in a stable, content-derived order (component/definition id
    /// ascending, then override order) so the emitted `.ccm` is reproducible
    /// byte-for-byte regardless of the chunks' `HashMap` iteration order
    /// (ADR-0005 §10 G1).
    ///
    /// Only conditions that parse under the typed condition grammar are
    /// included — exactly mirroring the legacy `register_condition`
    /// fall-through, where a condition that does not parse widens no facet
    /// domain and forms no group. This keeps the product compile from failing
    /// on a pass-through condition string the BDD grammar cannot represent.
    /// De-duplication keeps an identical condition that appears on multiple
    /// entities from inflating the clause set (the BDD AND-fold is idempotent,
    /// so this is also a semantics-preserving normalization).
    fn collect_ccm_clauses(&self) -> Vec<String> {
        // Pass 1: gather every raw condition string in a stable,
        // content-derived order (chunk order, then id ascending, then
        // override order).
        let mut raw: Vec<String> = Vec::new();
        for chunk in &self.chunks {
            // Definitions and components are stored in `HashMap`s; sort by id
            // for a deterministic, seed-independent walk.
            let mut definition_ids: Vec<&String> = chunk.config.definitions.keys().collect();
            definition_ids.sort();
            for id in definition_ids {
                collect_parameter_conditions(&chunk.config.definitions[id], &mut raw);
            }

            let mut component_ids: Vec<&String> = chunk.config.components.keys().collect();
            component_ids.sort();
            for id in component_ids {
                let component = &chunk.config.components[id];
                if let Some(condition) = component.condition.as_deref() {
                    raw.push(condition.to_string());
                }
                let mut param_keys: Vec<&String> = component.params.keys().collect();
                param_keys.sort();
                for key in param_keys {
                    collect_parameter_conditions(&component.params[key], &mut raw);
                }
            }
        }

        // Pass 2: keep first-seen, parseable, non-empty conditions only. A
        // condition that does not parse widens no facet and forms no clause,
        // mirroring the legacy `register_condition` fall-through.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut clauses: Vec<String> = Vec::new();
        for condition in raw {
            let trimmed = condition.trim();
            if trimmed.is_empty() {
                continue;
            }
            if parse_condition_expr(trimmed).is_err() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                clauses.push(trimmed.to_string());
            }
        }

        // Pass 3 (ADR-0047 §4, Amendment 1): append one symbol-introducing
        // tautology per DECLARED facet value, AFTER all authored condition
        // clauses, in facet-name-ascending order. This lands every declared
        // value — including a default arm no condition names — into the symbol
        // universe WITHOUT asserting any intra-facet constraint on the
        // permissive BDD root. The ordering is the byte-stability commitment:
        // deterministic, non-perturbing of the authored clauses, so `ccm_hash`
        // is a pure function of (authored conditions, declared facets). A model
        // that declares no facet appends nothing and is byte-identical to the
        // pre-ADR path. Duplicate facet keys across chunks are rejected at
        // ingest (`E_INGEST_DUPLICATE_FACET`), so the `BTreeMap` collapse below
        // never drops a distinct declaration.
        let mut declared: BTreeMap<String, &schema::Facet> = BTreeMap::new();
        for chunk in &self.chunks {
            for (name, facet) in &chunk.config.facets {
                declared.insert(name.clone(), facet);
            }
        }
        clauses.extend(synthesize_facet_clauses(&declared));
        clauses
    }
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
/// **Closed and open facets are handled identically** — symbol-introduction
/// only, with no `exactly_one_of`, no pairwise at-most-one, and no
/// at-least-one. Amendment 1 (configflux-5zqr) records why: the compiled model
/// is a *permissive feasibility screen* whose guards are hard assertions
/// AND-folded into the root, so a synthesized cardinality clause (AMO or
/// exactly-one) collides with a forced guard and prunes the unforced default
/// arm — re-creating the F2 trap. Intra-facet exclusivity never lived in the
/// BDD; it lives at the selection/tag layer (a single-valued choices map),
/// membership at validation (`E_FACET_VALUE_UNDECLARED`), and defaulting at
/// resolve time (§5 auto-bind). The closed/open distinction therefore matters
/// at the validation and render layers, not here.
fn synthesize_facet_clauses(facets: &BTreeMap<String, &schema::Facet>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (name, facet) in facets {
        for value in &facet.values {
            out.push(synthesize_symbol_introduction(name, value));
        }
    }
    out
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

/// A synthesized equality predicate `f == '<value>'`. Facet values are
/// effectively identifiers; the condition grammar's quoted literal has no
/// escape syntax, so a value containing the chosen quote is unrepresentable.
/// Prefer single quotes (the authored convention) and fall back to double
/// quotes when the value itself contains a single quote — an authoring
/// pathology that the CUE/Rust ingest layer guards upstream.
fn facet_eq_predicate(name: &str, value: &str) -> String {
    if value.contains('\'') {
        format!("{name} == \"{value}\"")
    } else {
        format!("{name} == '{value}'")
    }
}

fn facet_ne_predicate(name: &str, value: &str) -> String {
    if value.contains('\'') {
        format!("{name} != \"{value}\"")
    } else {
        format!("{name} != '{value}'")
    }
}

/// Recursively collect the `condition` strings from a parameter's override
/// chain into `out`, in override order then nested-override order. Mirrors the
/// legacy `register_parameter_conditions` traversal so the product `.ccm`
/// reflects exactly the conditions the selection model already understood.
fn collect_parameter_conditions(parameter: &Parameter, out: &mut Vec<String>) {
    for override_block in &parameter.overrides {
        out.push(override_block.condition.clone());
        collect_parameter_conditions(override_block.payload.as_ref(), out);
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

    for chunk_ref in &index.chunks {
        let chunk_path = output_dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        let chunk = ir::load_chunk(&chunk_path)?;

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
    }

    validate_definition_inheritance(&definitions)?;
    validate_component_dependencies(&components)?;
    validate_facets(&facets, &components, &definitions)?;

    Ok(())
}

#[cfg(test)]
mod facet_synthesis_tests {
    use super::*;

    fn facet(values: &[&str], open: bool) -> schema::Facet {
        schema::Facet {
            values: values.iter().map(|v| v.to_string()).collect(),
            default: None,
            open,
            doc: None,
        }
    }

    fn declared<'a>(pairs: &'a [(&str, &'a schema::Facet)]) -> BTreeMap<String, &'a schema::Facet> {
        pairs.iter().map(|(n, f)| (n.to_string(), *f)).collect()
    }

    #[test]
    fn closed_facet_introduces_each_declared_value_in_declared_order() {
        // Amendment 1: symbol-introduction only. Declared order (eu, us, apac)
        // is preserved verbatim — NOT sorted — so the emitted symbols are a pure
        // function of the declaration. No `exactly_one_of`, no constraint.
        let region = facet(&["eu", "us", "apac"], false);
        let clauses = synthesize_facet_clauses(&declared(&[("region", &region)]));
        assert_eq!(
            clauses,
            vec![
                "region == 'eu' || region != 'eu'".to_string(),
                "region == 'us' || region != 'us'".to_string(),
                "region == 'apac' || region != 'apac'".to_string(),
            ]
        );
    }

    #[test]
    fn single_value_closed_facet_introduces_its_lone_value() {
        let mode = facet(&["only"], false);
        let clauses = synthesize_facet_clauses(&declared(&[("mode", &mode)]));
        assert_eq!(clauses, vec!["mode == 'only' || mode != 'only'".to_string()]);
    }

    #[test]
    fn open_facet_introduces_declared_values_without_at_most_one() {
        // Amendment 1: open facets are handled identically to closed — symbol
        // introduction only, no AMO. Condition-referenced-but-undeclared values
        // enter the symbol universe via the authored clauses, not here, so the
        // synthesized output depends only on the declared values.
        let env = facet(&["prod", "staging"], true);
        let clauses = synthesize_facet_clauses(&declared(&[("env", &env)]));
        assert_eq!(
            clauses,
            vec![
                "env == 'prod' || env != 'prod'".to_string(),
                "env == 'staging' || env != 'staging'".to_string(),
            ]
        );
    }

    #[test]
    fn single_value_open_facet_emits_symbol_introducing_tautology() {
        let flag = facet(&["on"], true);
        let clauses = synthesize_facet_clauses(&declared(&[("flag", &flag)]));
        assert_eq!(clauses, vec!["flag == 'on' || flag != 'on'".to_string()]);
    }

    #[test]
    fn multiple_facets_emit_in_facet_name_ascending_order() {
        let region = facet(&["eu", "us"], false);
        let tier = facet(&["free", "paid"], false);
        // Insertion order (tier, region) is irrelevant: the BTreeMap key order
        // (region < tier) is the emission order; values keep declared order.
        let clauses =
            synthesize_facet_clauses(&declared(&[("tier", &tier), ("region", &region)]));
        assert_eq!(
            clauses,
            vec![
                "region == 'eu' || region != 'eu'".to_string(),
                "region == 'us' || region != 'us'".to_string(),
                "tier == 'free' || tier != 'free'".to_string(),
                "tier == 'paid' || tier != 'paid'".to_string(),
            ]
        );
    }

    #[test]
    fn value_with_single_quote_falls_back_to_double_quotes() {
        let odd = facet(&["a'b", "c"], false);
        let clauses = synthesize_facet_clauses(&declared(&[("odd", &odd)]));
        assert_eq!(
            clauses,
            vec![
                "odd == \"a'b\" || odd != \"a'b\"".to_string(),
                "odd == 'c' || odd != 'c'".to_string(),
            ]
        );
    }

    #[test]
    fn no_declared_facets_synthesizes_nothing() {
        let clauses = synthesize_facet_clauses(&BTreeMap::new());
        assert!(clauses.is_empty());
    }

    #[test]
    fn synthesized_clauses_reparse_under_the_condition_grammar() {
        // The synthesized strings must round-trip through the same parser the
        // emitter uses (parse_clauses), or the compile would fail downstream.
        let region = facet(&["eu", "us", "apac"], false);
        let env = facet(&["prod"], true);
        let clauses = synthesize_facet_clauses(&declared(&[("region", &region), ("env", &env)]));
        for clause in &clauses {
            assert!(
                parse_condition_expr(clause).is_ok(),
                "synthesized clause must parse: {clause}"
            );
        }
    }
}
