// SPDX-License-Identifier: BUSL-1.1
//
// `cfx diff` — the per-deployment change report (ADR-0059 D4/M5/M6).
//
// The question this verb answers is the one `resolve_hash` cannot: "which
// deployments did my change touch, and how". `resolve_hash` carries
// `model_hash` in its pre-image (twice), so ANY edit anywhere rotates it for
// EVERY target, including targets whose delivered bytes are byte-identical
// before and after. `resolved_output_hash` (ADR-0059 D3) identifies the
// delivered payload alone, so comparing it per cell is a truthful
// "did this deployment change" test.
//
// Both sides of a cell get the SAME question (ADR-0059 M5): the shared
// `manifest::resolve_cells()` enumerator computes each cell's
// `(scope, context_tags, choices)` once and both resolves are handed that one
// tuple. A `changed` status can then only mean the model changed, never that
// the two sides were asked different things.
//
// Nothing is written. `pipeline::resolve_only` stops after `resolve` — no
// export, no files — so `cfx diff` in an empty directory leaves it empty.
//
// The envelope types live HERE and not in the `compiler` crate (ADR-0059 M6):
// a manifest is a user-side deployment container and a diff is a comparison of
// two runs, neither of which is a compiler concept, and an envelope owned by
// the tool that emits it can be revised without touching the resolution
// contract every SDK and the C ABI depend on. `schema_version` is nonetheless
// `PRODUCT_SCHEMA_VERSION`: every payload-bearing value in a report — the
// `before`/`after` fragments of `resolved_output`, and the hashes whose
// pre-images carry that constant — is product-schema-shaped, so a consumer
// reading a fragment needs the product's number rather than one it must map.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use compiler::loader_api::{ModelHandle, ResolveResult};
use compiler::product_api::PRODUCT_SCHEMA_VERSION;
use serde::Serialize;
use serde_json::Value;

use crate::manifest::Cell;
use crate::pipeline::{CellSource, PipelineError};

/// The key under a root object that holds the component map; every OTHER root
/// key (`package`, `version`) is compared as a scalar field of the payload.
const COMPONENTS_KEY: &str = "components";
/// The key under a component that holds its parameter map; every OTHER
/// component key (`type`) is compared as a scalar field of that component.
const PARAMS_KEY: &str = "params";
/// The key under a component that holds its delivered catalogue entries
/// (ADR-0057 §D7), slot -> `{binding, entry, fields}`. Walked per slot and per
/// field rather than compared whole: a shared-catalogue edit is exactly the
/// change a reviewer needs to see NAMED, and printing the entire block would
/// bury one changed millimetre in a wall of unchanged ones.
const REQUIRES_KEY: &str = "requires";
/// The key under one delivered requirement that holds the entry's values;
/// `binding` and `entry` beside it are compared as scalar fields of the slot.
const FIELDS_KEY: &str = "fields";
/// The parameter field that carries the delivered value itself. It is reported
/// SEPARATELY from — and before — the metadata fields beside it, because a
/// value change is what a reviewer is looking for and a metadata-only change
/// is a different claim (ADR-0059 D4).
const VALUE_FIELD: &str = "value";

/// One cell's verdict (ADR-0059 D4). Serialized snake_case, exactly as D4
/// spells the five statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Unchanged,
    Changed,
    NowUnsatisfiable,
    NowSatisfiable,
    UnsatisfiableBoth,
}

impl CellStatus {
    /// The token the text report prints in column one.
    fn token(self) -> &'static str {
        match self {
            CellStatus::Unchanged => "unchanged",
            CellStatus::Changed => "changed",
            CellStatus::NowUnsatisfiable => "now_unsatisfiable",
            CellStatus::NowSatisfiable => "now_satisfiable",
            CellStatus::UnsatisfiableBoth => "unsatisfiable_both",
        }
    }
}

/// What KIND of difference a `Change` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Changed,
    Added,
    Removed,
}

/// One difference between the two delivered payloads.
///
/// `path` addresses the entity (`component.<id>`, `component.<id>.param.<key>`,
/// `component.<id>.requires.<slot>`, `component.<id>.requires.<slot>.<field>`,
/// `package`, `version`, `selection.defaulted.<facet>`,
/// `selection.implied.<facet>`) and `field` names the
/// attribute of it that differs, or is absent when the whole entity was added,
/// removed, or is itself the scalar being compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

impl Change {
    /// The path as the TEXT report spells it — the sort key for the whole
    /// change list.
    ///
    /// Sorting on this ONE string gives both rules D4 states at once. Paths
    /// come out lexicographically sorted, and a parameter's `value` line comes
    /// out before that parameter's metadata lines, because the value line's
    /// text path is the bare `component.x.param.y` while a metadata line
    /// appends `.<field>` to it — and `'.'` sorts below every character a
    /// field name or a sibling key can start with. Two rules, one comparison,
    /// no second ordering table to drift.
    fn text_path(&self) -> String {
        match &self.field {
            Some(field) if field != VALUE_FIELD => format!("{}.{}", self.path, field),
            _ => self.path.clone(),
        }
    }

    fn changed(path: String, field: Option<&str>, before: &Value, after: &Value) -> Self {
        Change {
            path,
            field: field.map(str::to_string),
            kind: ChangeKind::Changed,
            before: Some(before.clone()),
            after: Some(after.clone()),
        }
    }

    fn added(path: String, after: &Value) -> Self {
        Change {
            path,
            field: None,
            kind: ChangeKind::Added,
            before: None,
            after: Some(after.clone()),
        }
    }

    fn removed(path: String, before: &Value) -> Self {
        Change {
            path,
            field: None,
            kind: ChangeKind::Removed,
            before: Some(before.clone()),
            after: None,
        }
    }
}

/// Why one side of a cell was rejected.
///
/// D4 defines four of its five statuses in terms of a rejected side but gives
/// the cell no place to say WHY; a `now_unsatisfiable` cell that cannot name
/// its diagnostic sends the reviewer to a second command, which defeats the
/// verb (ADR-0059 M6).
#[derive(Debug, Clone, Serialize)]
pub struct Rejection {
    pub side: String,
    pub code: String,
    pub message: String,
}

/// One `(environment, scope)` cell's report. Field order IS the JSON key order
/// ADR-0059 D4 pins; `serde` emits struct fields in declaration order.
#[derive(Debug, Clone, Serialize)]
pub struct CellDiff {
    pub environment: String,
    pub scope: String,
    pub status: CellStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_resolved_output_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_resolved_output_hash: Option<String>,
    pub changes: Vec<Change>,
    pub rejections: Vec<Rejection>,
}

/// How many cells landed in each status.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub unchanged: usize,
    pub changed: usize,
    pub now_unsatisfiable: usize,
    pub now_satisfiable: usize,
    pub unsatisfiable_both: usize,
}

impl DiffSummary {
    fn count(&mut self, status: CellStatus) {
        let slot = match status {
            CellStatus::Unchanged => &mut self.unchanged,
            CellStatus::Changed => &mut self.changed,
            CellStatus::NowUnsatisfiable => &mut self.now_unsatisfiable,
            CellStatus::NowSatisfiable => &mut self.now_satisfiable,
            CellStatus::UnsatisfiableBoth => &mut self.unsatisfiable_both,
        };
        *slot += 1;
    }

    /// Whether every cell was `unchanged` — the exit-`0` condition.
    fn all_unchanged(&self) -> bool {
        self.changed == 0
            && self.now_unsatisfiable == 0
            && self.now_satisfiable == 0
            && self.unsatisfiable_both == 0
    }
}

/// The whole report. The two `--base`/`--head` PATHS are deliberately absent:
/// ADR-0042 §3 forbids absolute paths in output, and a model's identity is its
/// `model_hash`, not where its manifest happened to sit.
#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub schema_version: u32,
    pub base_model_hash: String,
    pub head_model_hash: String,
    pub cells: Vec<CellDiff>,
    pub summary: DiffSummary,
}

impl DiffReport {
    /// `0` when every cell is unchanged, `1` when at least one is not
    /// (mirroring `diff(1)` and `git diff --exit-code`). NEVER `3`:
    /// unsatisfiability is a cell status here, not a command failure.
    pub fn exit_code(&self) -> u8 {
        if self.summary.all_unchanged() {
            crate::EXIT_OK
        } else {
            crate::EXIT_DIFFERENCES
        }
    }
}

// ---------------------------------------------------------------------------
// The pure comparison
// ---------------------------------------------------------------------------

/// Compare two `resolved_output` values and return every difference, sorted.
///
/// PURE: two `serde_json::Value`s in, a change list out — no model, no disk, no
/// resolve. That is what makes D4's change-list rules cheap to pin in tests
/// (ADR-0059 M6) and what keeps the rules themselves in one readable place.
///
/// The walk is ONE rule applied at three depths — "compare the sub-map named by
/// this level's container key, and treat every other key as a scalar field of
/// this level" — rather than a hardcoded list of parameter fields. A hardcoded
/// list would be a second source of truth for the payload shape: a field added
/// to `ResolvedParameter` would then be silently invisible here, and an
/// invisible difference is a `changed` cell with an empty change list — a wrong
/// answer with a clean exit code.
pub fn diff_resolved_output(base: &Value, head: &Value) -> Vec<Change> {
    let mut changes = Vec::new();

    // Root level. `resolved_output` is `map<scope_root, resolved_config>`, and
    // a multi-selector scope produces several roots (`resolver::resolve_scoped`),
    // so both halves are per-root — but the PATHS carry no root, because
    // ADR-0059 D1 freezes `scope` to `"component:<id>" | "all"` and M2 leaves
    // the list-valued `<root>` rule undefined until a consumer needs one.
    for root in union_keys(base, head) {
        let base_root = child(base, &root);
        let head_root = child(head, &root);
        for key in union_keys(base_root, head_root) {
            if key == COMPONENTS_KEY {
                continue;
            }
            push_scalar(&mut changes, key.clone(), None, child(base_root, &key), child(head_root, &key));
        }
    }

    // Components, MERGED across roots. Two roots of one scope can share a
    // dependency component (both services `depends_on` the same catalogue
    // fixture), and merging is what stops one difference being reported twice.
    // The two roots resolve in the same context, so their copies agree.
    let base_components = merge_components(base);
    let head_components = merge_components(head);
    for id in union_map_keys(&base_components, &head_components) {
        let path = format!("component.{id}");
        match (base_components.get(&id), head_components.get(&id)) {
            (None, Some(after)) => changes.push(Change::added(path, after)),
            (Some(before), None) => changes.push(Change::removed(path, before)),
            (Some(before), Some(after)) => diff_component(&mut changes, &path, before, after),
            (None, None) => unreachable!("the id came from one of the two maps"),
        }
    }

    changes.sort_by(|a, b| a.text_path().cmp(&b.text_path()));
    changes.dedup();
    changes
}

/// One component: its `params` map, its `requires` map, plus every other key
/// (`type`) as a field.
fn diff_component(changes: &mut Vec<Change>, path: &str, base: &Value, head: &Value) {
    for key in union_keys(base, head) {
        if key == PARAMS_KEY || key == REQUIRES_KEY {
            continue;
        }
        push_scalar(changes, path.to_string(), Some(&key), child(base, &key), child(head, &key));
    }

    let base_params = child(base, PARAMS_KEY);
    let head_params = child(head, PARAMS_KEY);
    for key in union_keys(base_params, head_params) {
        let param_path = format!("{path}.param.{key}");
        match (object_entry(base_params, &key), object_entry(head_params, &key)) {
            (None, Some(after)) => changes.push(Change::added(param_path, after)),
            (Some(before), None) => changes.push(Change::removed(param_path, before)),
            (Some(before), Some(after)) => diff_parameter(changes, &param_path, before, after),
            (None, None) => unreachable!("the key came from one of the two objects"),
        }
    }

    // `requires` is skip-if-empty in the payload, so a component that declares
    // no requirement contributes nothing here and the walk costs a model that
    // does not use the feature exactly one absent-key lookup.
    let base_requires = child(base, REQUIRES_KEY);
    let head_requires = child(head, REQUIRES_KEY);
    for slot in union_keys(base_requires, head_requires) {
        let slot_path = format!("{path}.requires.{slot}");
        match (
            object_entry(base_requires, &slot),
            object_entry(head_requires, &slot),
        ) {
            (None, Some(after)) => changes.push(Change::added(slot_path, after)),
            (Some(before), None) => changes.push(Change::removed(slot_path, before)),
            (Some(before), Some(after)) => diff_requirement(changes, &slot_path, before, after),
            (None, None) => unreachable!("the slot came from one of the two objects"),
        }
    }
}

/// One delivered requirement: `binding` and `entry` as fields of the slot, and
/// each of the entry's values at `<slot>.<field>` — the same path the runtime
/// read API uses, so a reviewer can take a line out of the report and read it
/// back off a device.
fn diff_requirement(changes: &mut Vec<Change>, path: &str, base: &Value, head: &Value) {
    for key in union_keys(base, head) {
        if key == FIELDS_KEY {
            continue;
        }
        push_scalar(changes, path.to_string(), Some(&key), child(base, &key), child(head, &key));
    }

    let base_fields = child(base, FIELDS_KEY);
    let head_fields = child(head, FIELDS_KEY);
    for field in union_keys(base_fields, head_fields) {
        push_scalar(
            changes,
            format!("{path}.{field}"),
            None,
            child(base_fields, &field),
            child(head_fields, &field),
        );
    }
}

/// One parameter: every key compared as a field. `value` is a field like any
/// other here; its precedence over the metadata fields is carried entirely by
/// `Change::text_path`'s sort key, so there is no second ordering rule.
fn diff_parameter(changes: &mut Vec<Change>, path: &str, base: &Value, head: &Value) {
    for key in union_keys(base, head) {
        push_scalar(changes, path.to_string(), Some(&key), child(base, &key), child(head, &key));
    }
}

/// Record `path`/`field` as changed when the two values differ.
///
/// An absent key and an explicit `null` are the SAME observation here — a
/// consumer reading the payload sees no value either way — so a key that is
/// omitted on one side and `null` on the other is not a difference.
fn push_scalar(
    changes: &mut Vec<Change>,
    path: String,
    field: Option<&str>,
    before: &Value,
    after: &Value,
) {
    if before != after {
        changes.push(Change::changed(path, field, before, after));
    }
}

/// Every component of every root of one side, keyed by component id.
fn merge_components(output: &Value) -> BTreeMap<String, Value> {
    let mut merged = BTreeMap::new();
    for root in union_keys(output, &Value::Null) {
        let components = child(child(output, &root), COMPONENTS_KEY);
        if let Some(map) = components.as_object() {
            for (id, component) in map {
                merged.insert(id.clone(), component.clone());
            }
        }
    }
    merged
}

/// The sorted union of two values' object keys; a non-object contributes none.
fn union_keys(left: &Value, right: &Value) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for value in [left, right] {
        if let Some(map) = value.as_object() {
            keys.extend(map.keys().cloned());
        }
    }
    keys.into_iter().collect()
}

/// The sorted union of two maps' keys.
fn union_map_keys(left: &BTreeMap<String, Value>, right: &BTreeMap<String, Value>) -> Vec<String> {
    let mut keys: BTreeSet<String> = left.keys().cloned().collect();
    keys.extend(right.keys().cloned());
    keys.into_iter().collect()
}

/// A value's named child, or `Value::Null` when absent — so a missing branch
/// compares as "no value" instead of needing an `Option` at every call site.
fn child<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// A value's named child ONLY when the key is genuinely present, so an
/// added/removed entry can be told from one explicitly set to `null`.
fn object_entry<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|map| map.get(key))
}

/// The `selection.<kind>.<facet>` differences between two resolves, for one
/// class of selection provenance.
///
/// `defaulted_choices` (auto-bound defaults, ADR-0047 §5) and `implied_choices`
/// (solver-inferred bindings, ADR-0057 §D6) are PROVENANCE, not part of
/// `resolved_output`, so they are compared here rather than inside the pure
/// payload walk. Both classes go through this one function: an implied binding
/// that moves is the same class of news as a default that moves, and reporting
/// only one of them would let `cfx diff` call a target unchanged when the
/// constraints started deciding a facet for it. An absent facet renders as JSON
/// `null` on its side: D4 spells these lines with `~` in every case, because
/// "this target stopped relying on a default" and "this target's default moved"
/// are, again, the same class of news.
fn diff_selection_provenance(
    kind: &str,
    base: &BTreeMap<String, String>,
    head: &BTreeMap<String, String>,
) -> Vec<Change> {
    let mut facets: BTreeSet<&String> = base.keys().collect();
    facets.extend(head.keys());
    let as_value = |choice: Option<&String>| match choice {
        Some(option) => Value::String(option.clone()),
        None => Value::Null,
    };

    let mut changes = Vec::new();
    for facet in facets {
        let (before, after) = (as_value(base.get(facet)), as_value(head.get(facet)));
        if before != after {
            changes.push(Change::changed(
                format!("selection.{kind}.{facet}"),
                None,
                &before,
                &after,
            ));
        }
    }
    changes
}

// ---------------------------------------------------------------------------
// The verb
// ---------------------------------------------------------------------------

/// One side of the comparison: the model opened ONCE, and the label its
/// failures are reported under.
struct Side {
    label: &'static str,
    handle: ModelHandle,
}

/// Open both models and compare every cell (ADR-0059 D4).
///
/// Each side is opened ONCE, not once per cell: the report's
/// `base_model_hash`/`head_model_hash` must exist even when every cell is
/// rejected, and an unopenable model must fail as a usage error before any cell
/// is attempted rather than as a side effect of the first one.
pub fn run(
    base_model: &Path,
    head_model: &Path,
    cells: &[Cell],
) -> Result<DiffReport, PipelineError> {
    let base = open_side("base", base_model)?;
    let head = open_side("head", head_model)?;

    let mut report = DiffReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        base_model_hash: base.handle.model_hash.clone(),
        head_model_hash: head.handle.model_hash.clone(),
        cells: Vec::with_capacity(cells.len()),
        summary: DiffSummary::default(),
    };

    for cell in cells {
        let diff = compare_cell(&base, &head, cell)?;
        report.summary.count(diff.status);
        report.cells.push(diff);
    }
    Ok(report)
}

/// Open one side, naming it in any failure so a reader knows WHICH model could
/// not be read.
fn open_side(label: &'static str, model: &Path) -> Result<Side, PipelineError> {
    match crate::pipeline::open(model) {
        Ok(handle) => Ok(Side { label, handle }),
        Err(err) => Err(PipelineError::usage(format!("{label}: {}", err.message))),
    }
}

/// Resolve one cell on both sides and classify the pair.
///
/// The SAME `CellSource::Manifest(cell)` goes to both resolves, so the two
/// sides are asked the identical question (ADR-0059 M5).
///
/// Which failures are a cell STATUS and which abort the command is
/// `classify()`'s decision and is not restated here (ADR-0059 M6): a rejection
/// it puts in the exit-3 class is a status, and anything else — an unreadable
/// package, an unknown facet, a scope naming no component — means the question
/// itself could not be asked, so no cell verdict would be truthful.
fn compare_cell(base: &Side, head: &Side, cell: &Cell) -> Result<CellDiff, PipelineError> {
    let source = CellSource::Manifest(cell);
    let base_result = resolve_side(base, &source, cell)?;
    let head_result = resolve_side(head, &source, cell)?;

    let mut diff = CellDiff {
        environment: cell.environment.clone(),
        scope: cell.scope.clone(),
        status: CellStatus::UnsatisfiableBoth,
        base_resolve_hash: None,
        head_resolve_hash: None,
        base_resolved_output_hash: None,
        head_resolved_output_hash: None,
        changes: Vec::new(),
        rejections: Vec::new(),
    };

    match (base_result, head_result) {
        (Ok(base_resolve), Ok(head_resolve)) => {
            diff.base_resolve_hash = base_resolve.resolve_hash.clone();
            diff.head_resolve_hash = head_resolve.resolve_hash.clone();
            diff.base_resolved_output_hash = base_resolve.resolved_output_hash.clone();
            diff.head_resolved_output_hash = head_resolve.resolved_output_hash.clone();
            // The payload identity, not the resolution identity: an unrelated
            // model edit rotates `resolve_hash` for a target whose delivered
            // bytes never moved, and calling that `changed` is the false
            // positive this whole verb exists to remove (ADR-0059 D3).
            if diff.base_resolved_output_hash == diff.head_resolved_output_hash {
                diff.status = CellStatus::Unchanged;
            } else {
                diff.status = CellStatus::Changed;
                diff.changes = diff_resolved_output(
                    base_resolve.resolved_output.as_ref().unwrap_or(&Value::Null),
                    head_resolve.resolved_output.as_ref().unwrap_or(&Value::Null),
                );
                diff.changes.extend(diff_selection_provenance(
                    "defaulted",
                    &base_resolve.defaulted_choices,
                    &head_resolve.defaulted_choices,
                ));
                diff.changes.extend(diff_selection_provenance(
                    "implied",
                    &base_resolve.implied_choices,
                    &head_resolve.implied_choices,
                ));
                diff.changes.sort_by(|a, b| a.text_path().cmp(&b.text_path()));
            }
        }
        (Ok(_), Err(rejection)) => {
            diff.status = CellStatus::NowUnsatisfiable;
            diff.rejections.push(rejection);
        }
        (Err(rejection), Ok(_)) => {
            diff.status = CellStatus::NowSatisfiable;
            diff.rejections.push(rejection);
        }
        (Err(base_rejection), Err(head_rejection)) => {
            diff.status = CellStatus::UnsatisfiableBoth;
            diff.rejections.push(base_rejection);
            diff.rejections.push(head_rejection);
        }
    }
    Ok(diff)
}

/// Resolve one cell on one side, splitting the two failure classes:
/// `Ok(Err(rejection))` is a cell status, `Err(_)` aborts the command.
#[allow(clippy::type_complexity)]
fn resolve_side(
    side: &Side,
    source: &CellSource,
    cell: &Cell,
) -> Result<Result<ResolveResult, Rejection>, PipelineError> {
    match crate::pipeline::resolve_only(&side.handle, source, &[]) {
        Ok(result) => Ok(Ok(result)),
        Err(err) if err.unsatisfiable => {
            let (code, message) = err
                .diagnostic
                .clone()
                .unwrap_or_else(|| ("E_UNKNOWN".to_string(), err.message.clone()));
            Ok(Err(Rejection {
                side: side.label.to_string(),
                code,
                message,
            }))
        }
        Err(err) => Err(PipelineError::usage(format!(
            "{}: {} {}: {}",
            side.label, cell.environment, cell.scope, err.message
        ))),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the human report (ADR-0059 D4): one `<status>  <environment>
/// <scope>` line per cell, its change or rejection lines indented two spaces
/// below it, then the summary counts. Determinism-safe: no hashes on the text
/// path, no timestamps, no absolute paths.
pub fn render_text<W: Write>(report: &DiffReport, out: &mut W) -> std::io::Result<()> {
    for cell in &report.cells {
        writeln!(
            out,
            "{}  {}  {}",
            cell.status.token(),
            cell.environment,
            cell.scope
        )?;
        for change in &cell.changes {
            writeln!(out, "  {}", render_change(change))?;
        }
        for rejection in &cell.rejections {
            writeln!(
                out,
                "  {}: {}: {}",
                rejection.side, rejection.code, rejection.message
            )?;
        }
    }
    let s = &report.summary;
    writeln!(
        out,
        "summary: unchanged={} changed={} now_unsatisfiable={} now_satisfiable={} \
         unsatisfiable_both={}",
        s.unchanged, s.changed, s.now_unsatisfiable, s.now_satisfiable, s.unsatisfiable_both
    )
}

/// One change line. Values are compact JSON, so a string keeps its quotes and
/// is never confused with the bare token of an enum-like value.
fn render_change(change: &Change) -> String {
    let path = change.text_path();
    match change.kind {
        ChangeKind::Added => format!("+ {path}"),
        ChangeKind::Removed => format!("- {path}"),
        ChangeKind::Changed => format!(
            "~ {path}: {} -> {}",
            compact(change.before.as_ref()),
            compact(change.after.as_ref())
        ),
    }
}

/// A value as compact JSON; an absent side renders `null`.
fn compact(value: Option<&Value>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    }
}

/// Emit the `DiffReport` envelope plus one trailing newline, serialized
/// DIRECTLY from the struct — never via `serde_json::Value`, whose map would
/// re-sort the keys out of the order ADR-0059 D4 pins.
pub fn render_json<W: Write>(report: &DiffReport, out: &mut W) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(report)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    bytes.push(b'\n');
    out.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One resolved payload with a single root, one component and one
    /// parameter — the shape `docs/service-integration-guide.md` documents.
    fn payload(value: serde_json::Value, doc: serde_json::Value) -> Value {
        json!({
            "webapp": {
                "package": "service_multi_env",
                "version": "1.0.0",
                "components": {
                    "webapp": {
                        "type": "service",
                        "params": {
                            "request_timeout_ms": {
                                "value": value,
                                "type": "integer",
                                "unit": "ms",
                                "safety": "q_m",
                                "lifecycle": "startup",
                                "access": "integrator",
                                "req_id": null,
                                "doc": doc,
                                "limits": null
                            }
                        }
                    }
                }
            }
        })
    }

    fn lines(changes: &[Change]) -> Vec<String> {
        changes.iter().map(render_change).collect()
    }

    #[test]
    fn diff_is_empty_for_equal_payloads() {
        let same = payload(json!(5000), json!("Inbound HTTP request timeout"));
        assert!(diff_resolved_output(&same, &same).is_empty());
    }

    #[test]
    fn diff_reports_value_before_metadata() {
        // D4: the `value` difference is listed FIRST and SEPARATELY from the
        // metadata differences of the same parameter — a reviewer reads "what
        // does the service receive now" before "what did we say about it".
        let base = payload(json!(5000), json!("old doc"));
        let head = payload(json!(4000), json!("new doc"));
        assert_eq!(
            lines(&diff_resolved_output(&base, &head)),
            vec![
                "~ component.webapp.param.request_timeout_ms: 5000 -> 4000",
                "~ component.webapp.param.request_timeout_ms.doc: \"old doc\" -> \"new doc\"",
            ]
        );
    }

    #[test]
    fn diff_reports_added_and_removed_components_and_params() {
        let mut base = payload(json!(5000), json!("doc"));
        let mut head = base.clone();
        // A component only the head has, and one only the base has.
        head["webapp"]["components"]["sidecar"] = json!({"type": "service", "params": {}});
        base["webapp"]["components"]["legacy"] = json!({"type": "service", "params": {}});
        // A parameter only the head has.
        head["webapp"]["components"]["webapp"]["params"]["health_check_path"] =
            json!({"value": "/healthz", "type": "string"});
        // ...and the root's own scalars move too.
        head["webapp"]["version"] = json!("1.1.0");

        // Sorted on the text path throughout, so the added parameter sorts
        // under ITS component (`component.webapp....`) rather than beside the
        // component-level lines, and the root's `version` line comes last.
        assert_eq!(
            lines(&diff_resolved_output(&base, &head)),
            vec![
                "- component.legacy",
                "+ component.sidecar",
                "+ component.webapp.param.health_check_path",
                "~ version: \"1.0.0\" -> \"1.1.0\"",
            ]
        );
    }

    #[test]
    fn diff_paths_are_sorted() {
        // Sorted on the TEXT path, so a parameter's own line precedes its
        // metadata lines AND precedes a sibling parameter whose name extends
        // it — `component.x.param.y` < `component.x.param.y.doc` <
        // `component.x.param.yy`, because '.' sorts below every name character.
        let base = json!({"r": {"components": {"c": {"params": {
            "y": {"value": 1, "doc": "a"},
            "yy": {"value": 1},
            "a": {"value": 1}
        }}}}});
        let head = json!({"r": {"components": {"c": {"params": {
            "y": {"value": 2, "doc": "b"},
            "yy": {"value": 2},
            "a": {"value": 2}
        }}}}});
        assert_eq!(
            lines(&diff_resolved_output(&base, &head)),
            vec![
                "~ component.c.param.a: 1 -> 2",
                "~ component.c.param.y: 1 -> 2",
                "~ component.c.param.y.doc: \"a\" -> \"b\"",
                "~ component.c.param.yy: 1 -> 2",
            ]
        );
    }

    #[test]
    fn a_component_shared_by_two_roots_is_reported_once() {
        // A multi-selector scope resolves several roots, and two services can
        // share a catalogue fixture through `depends_on`. Both roots carry the
        // same resolved copy, so the difference must be reported ONCE.
        let base = json!({
            "vision": {"components": {"container": {"params": {"width_mm": {"value": 600}}}}},
            "compute": {"components": {"container": {"params": {"width_mm": {"value": 600}}}}}
        });
        let head = json!({
            "vision": {"components": {"container": {"params": {"width_mm": {"value": 650}}}}},
            "compute": {"components": {"container": {"params": {"width_mm": {"value": 650}}}}}
        });
        assert_eq!(
            lines(&diff_resolved_output(&base, &head)),
            vec!["~ component.container.param.width_mm: 600 -> 650"]
        );
    }

    #[test]
    fn defaulted_choices_render_null_for_an_absent_side() {
        let mut base = BTreeMap::new();
        base.insert("log_level".to_string(), "info".to_string());
        base.insert("dropped".to_string(), "x".to_string());
        let mut head = BTreeMap::new();
        head.insert("log_level".to_string(), "debug".to_string());
        head.insert("added".to_string(), "y".to_string());

        assert_eq!(
            lines(&diff_selection_provenance("defaulted", &base, &head)),
            vec![
                "~ selection.defaulted.added: null -> \"y\"",
                "~ selection.defaulted.dropped: \"x\" -> null",
                "~ selection.defaulted.log_level: \"info\" -> \"debug\"",
            ]
        );
    }

    /// The solver-inferred class reports under its own prefix (ADR-0057 §D6):
    /// a facet the constraints started deciding, one they stopped deciding, and
    /// one they now decide differently are all `cfx diff` news.
    #[test]
    fn implied_choices_report_under_their_own_prefix() {
        let mut base = BTreeMap::new();
        base.insert("sorter_container".to_string(), "c1".to_string());
        base.insert("retired".to_string(), "x".to_string());
        let mut head = BTreeMap::new();
        head.insert("sorter_container".to_string(), "c2".to_string());
        head.insert("newly_forced".to_string(), "y".to_string());

        assert_eq!(
            lines(&diff_selection_provenance("implied", &base, &head)),
            vec![
                "~ selection.implied.newly_forced: null -> \"y\"",
                "~ selection.implied.retired: \"x\" -> null",
                "~ selection.implied.sorter_container: \"c1\" -> \"c2\"",
            ]
        );
    }
}
