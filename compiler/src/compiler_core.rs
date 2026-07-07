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
use crate::link_verify::{validate_component_dependencies, validate_definition_inheritance};
use crate::schema::{self, Component, Config, Parameter};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeSet, HashMap};
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
        Ok(())
    }

    pub fn get_repo(&self) -> &Config {
        &self.repository
    }

    pub fn link_and_verify(&self) -> Result<()> {
        validate_definition_inheritance(&self.repository.definitions)?;
        validate_component_dependencies(&self.repository.components)
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

        // Pass 2: keep first-seen, parseable, non-empty conditions only.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut clauses: Vec<String> = Vec::new();
        for condition in raw {
            let trimmed = condition.trim();
            if trimmed.is_empty() || parse_condition_expr(trimmed).is_err() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                clauses.push(trimmed.to_string());
            }
        }
        clauses
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
    }

    validate_definition_inheritance(&definitions)?;
    validate_component_dependencies(&components)?;

    Ok(())
}
