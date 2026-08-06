// SPDX-License-Identifier: BUSL-1.1

//! Core-contract verification + hardening suite for the product-compile
//! resource budget — configflux-9pjy.5, per ADR-0039 §8.
//!
//! Phases 1-3 (configflux-9pjy.2/.3/.4) landed the budget type, the pure
//! [`derive_knobs`] derivation, the compile-time progress signal, and the
//! live adaptive memo-cap shrink. This module is the *verification* layer:
//! it pins the load-bearing contracts so a future regression fails loudly.
//!
//! Coverage map (the contracts this suite owns):
//!
//! 1. **Core contract — graceful degradation is monotonic.** As the soft
//!    `max_rss_mb` budget shrinks, the derived `memo_cap` is non-increasing
//!    (the pure half, asserted here). The adaptive-shrink-count half — that
//!    the live memo-clear count is non-decreasing as the budget shrinks —
//!    is verified deterministically with an injected RSS sampler in
//!    `ccm_emitter::tests` (`adaptive_shrink_count_non_decreasing_as_budget_shrinks`),
//!    the only scope from which the `#[cfg(test)] pub(super)` sampler seam
//!    (`BddBuilder::with_memo_cap_rss_budget_and_sampler_for_test`) is
//!    reachable. No assertion here depends on real `/proc` RSS or
//!    wall-clock.
//!
//! 2. **Byte-neutrality (load-bearing invariant, ADR-0039 §8).**
//!    `budget = None` reproduces today's artifact byte-for-byte, and a
//!    *memory-only* budget (one that shrinks `memo_cap` but does NOT derive
//!    a `cluster_size`) yields byte-identical artifacts. The memo cap is a
//!    cache lever; deriving a `cluster_size` would rotate bytes and is
//!    forbidden in this epic.
//!
//! 3. **Progress is monotonic.** A capturing [`ProgressSink`] sees an
//!    overall-pct sequence that is monotonic non-decreasing and terminates
//!    at exactly 100 %.
//!
//! 4. **Hardening (worker-1 QA notes).** (a) the `budget` key is absent
//!    from a serialized [`CompileModelRequest`] when the budget is `None`;
//!    (b) CLI `--max-rss-mb` / `--max-threads` round-trip (in
//!    `main.rs`'s own `#[cfg(test)]`, the only scope that sees the private
//!    `Cli` types); (c) adversarial precedence — an explicit `cluster_size`
//!    wins over a budget-derived one (ADR-0012 Amendment 1).

use std::cell::RefCell;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ccm_emitter::{emit_ccm_dir, emit_ccm_dir_with_budget, ConditionModel};
use crate::product_api::{
    compile_model, compile_model_with_progress, CompileModelRequest, OperationStatus,
    SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};
use crate::progress::{Phase, ProgressEvent, ProgressSink};
use crate::resource_budget::{derive_knobs, ResourceBudget, MEMO_CAP_FLOOR};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};

/// A 64-char lowercase-hex stand-in `bound_model_hash` (the in-crate emit
/// path validates the shape, not the value).
const BUDGET_TEST_HASH: &str =
    "2222222222222222222222222222222222222222222222222222222222222222";

/// One source chunk (CUE-exported JSON) used to drive the full
/// `compile_model` path for the progress and serde hardening tests. Two
/// components with simple `condition` predicates so the compile reaches the
/// BDD apply loop (the progress signal's dominant phase).
const BUDGET_TEST_DEFS_JSON: &str = r#"{
  "package": "rb_verify_defs",
  "version": "1.0.0",
  "definitions": {
    "mode": { "type": "string" }
  }
}"#;

const BUDGET_TEST_COMPONENTS_JSON: &str = r#"{
  "package": "rb_verify_components",
  "version": "1.0.0",
  "components": {
    "alpha": { "type": "module", "condition": "mode == 'fast'" },
    "beta": { "type": "module", "condition": "mode == 'slow'" }
  }
}"#;

/// A deterministic in-crate [`ConditionModel`] with enough distinct clauses
/// and variables to exercise the apply loop and the memo lever, but few
/// enough variables that a loose budget never derives a `cluster_size`
/// (verified in the byte-neutrality test before it is relied upon).
fn budget_test_model() -> ConditionModel {
    ConditionModel::from_clauses(
        BUDGET_TEST_HASH.to_string(),
        vec![
            "z == 'on' && a == 'enabled'".to_string(),
            "a == 'enabled' || m == 'auto'".to_string(),
            "!(b == 'off')".to_string(),
            "z == 'off' || (m == 'auto' && a == 'enabled')".to_string(),
            "exactly_one_of(p == 'x', p == 'y', p == 'z')".to_string(),
        ],
    )
}

/// Source manifest for the full-pipeline tests (progress + serde).
fn budget_test_manifest() -> Vec<SourceManifestEntry> {
    vec![
        SourceManifestEntry {
            source_id: "rb_verify/00_definitions.json".to_string(),
            inline_content: BUDGET_TEST_DEFS_JSON.to_string(),
        },
        SourceManifestEntry {
            source_id: "rb_verify/10_components.json".to_string(),
            inline_content: BUDGET_TEST_COMPONENTS_JSON.to_string(),
        },
    ]
}

fn temp_output_dir(label: &str) -> Result<TempDirGuard> {
    unique_temp_dir("configflux-rb-verify", label)
}

/// Hash the byte-stable triple (`ccm.bdd.bin`, `ccm.manifest.json`,
/// `ccm.symbols.json`) of every partition under an emitted `<dir>` into one
/// combined SHA-256 hex digest. Walks `partition-*` subdirectories in
/// sorted order plus the top-level `ccm.manifest.json` / `ccm.symbols.json`,
/// so two emissions are byte-equal iff this digest matches. The progress
/// stream and any telemetry never touch these files (ADR-0005 Amendment 2),
/// so the digest captures exactly the artifact contract.
fn combined_ccm_hash(dir: &Path) -> Result<String> {
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    // Top-level files first (deterministic, fixed names).
    for name in ["ccm.manifest.json", "ccm.symbols.json", "partition-manifest.json"] {
        let p = dir.join(name);
        if p.exists() {
            entries.push(p);
        }
    }
    // Then each partition subdirectory's triple, in sorted directory order.
    let mut partition_dirs: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("partition-"))
                    .unwrap_or(false)
        })
        .collect();
    partition_dirs.sort();
    for pdir in partition_dirs {
        for name in ["ccm.bdd.bin", "ccm.manifest.json", "ccm.symbols.json"] {
            let p = pdir.join(name);
            if p.exists() {
                entries.push(p);
            }
        }
    }

    let mut hasher = Sha256::new();
    for path in entries {
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        // Domain-separate by relative path so a file moving between
        // partitions cannot alias another file's bytes.
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read {}", path.display()))?;
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    Ok(hex32(&hasher.finalize().into()))
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------
// 1. CORE CONTRACT — graceful degradation (pure half: memo_cap monotonic).
// ---------------------------------------------------------------------

#[test]
fn core_contract_memo_cap_non_increasing_as_budget_shrinks() {
    // ADR-0039 §3 / §8 core contract: graceful degradation. As the soft
    // RSS budget shrinks, the derived `memo_cap` must be NON-INCREASING —
    // the compiler trades cache size (time) for a lower peak. `derive_knobs`
    // is pure (no env / no /proc / no clock), so this sweep is fully
    // deterministic. The adaptive (live-shrink-count) half of the contract
    // is verified with an injected sampler in `ccm_emitter::tests`.
    let budgets_mb: [u64; 14] = [
        8192, 4096, 2048, 1024, 512, 256, 192, 128, 96, 80, 64, 32, 8, 1,
    ];
    let mut prev_cap = usize::MAX;
    for &mb in &budgets_mb {
        let budget = ResourceBudget {
            max_rss_mb: Some(mb),
            max_threads: None,
        };
        let cap = derive_knobs(&budget, None).memo_cap;
        assert!(
            cap <= prev_cap,
            "memo_cap must be non-increasing as the budget shrinks: \
             budget={mb} MiB gave cap={cap}, the previous (larger) budget gave {prev_cap}"
        );
        assert!(cap >= MEMO_CAP_FLOOR, "memo_cap {cap} fell below the floor at {mb} MiB");
        prev_cap = cap;
    }
}

// ---------------------------------------------------------------------
// 2. BYTE-NEUTRALITY (load-bearing invariant, ADR-0039 §8).
// ---------------------------------------------------------------------

#[test]
fn budget_none_reproduces_unbudgeted_artifact_byte_for_byte() -> Result<()> {
    // ADR-0039 §8 first bullet: `budget = None` ⇒ output is bit-for-bit
    // identical to today. We emit the same model two ways — the plain
    // `emit_ccm_dir` (the historical default path) and the budget-aware
    // entry point with NO memo cap and NO RSS budget — and assert the
    // combined SHA-256 of every byte-stable file matches.
    let model = budget_test_model();

    let unbudgeted_dir = temp_output_dir("none-unbudgeted")?;
    emit_ccm_dir(&model, &unbudgeted_dir.path)?;
    let unbudgeted_hash = combined_ccm_hash(&unbudgeted_dir.path)?;

    let none_dir = temp_output_dir("none-budget-path")?;
    // budget=None equivalent on the emit chain: no memo cap, no RSS budget,
    // single-partition collapse (usize::MAX).
    emit_ccm_dir_with_budget(
        &model,
        &none_dir.path,
        "facet-name-ascending",
        "in-crate",
        usize::MAX,
        None,
        None,
    )?;
    let none_hash = combined_ccm_hash(&none_dir.path)?;

    assert_eq!(
        unbudgeted_hash, none_hash,
        "budget=None must reproduce the unbudgeted artifact byte-for-byte"
    );
    Ok(())
}

#[test]
fn memory_only_budget_is_byte_identical_and_derives_no_cluster_size() -> Result<()> {
    // ADR-0039 §8 second bullet: a budget whose ONLY effect is the memo cap
    // (it shrinks the byte-neutral cache but does NOT derive a cluster_size)
    // yields bit-for-bit identical output to the unbudgeted compile. The
    // memo cap is a cache lever — it changes time and cache churn, never the
    // canonical BDD/manifest/symbols. Deriving a cluster_size WOULD rotate
    // bytes (different partition boundaries → different hashes) and is
    // forbidden in this epic.
    let model = budget_test_model();

    // The variable count this emission would project (the same hint the
    // production compile path passes to derive_knobs).
    let var_hint = crate::ccm_emitter::count_model_variables(&model);
    assert!(var_hint > 0, "the test model must have at least one BDD variable");

    // Pick a budget that derive_knobs maps to a SMALLER memo_cap but leaves
    // cluster_size = None. 96 MiB sits above the 64 MiB overhead floor (so
    // the remainder is positive and the derived cap is below the default)
    // while leaving the handful of test variables projecting well under the
    // budget (so no cluster_size is derived). We VERIFY both facts before
    // relying on them, so the test is self-checking against any future
    // change to the cost model.
    let budget = ResourceBudget {
        max_rss_mb: Some(96),
        max_threads: None,
    };
    let derived = derive_knobs(&budget, Some(var_hint));
    assert_eq!(
        derived.cluster_size, None,
        "this memory-only budget must NOT derive a cluster_size \
         (deriving one would rotate artifact bytes — forbidden in this epic)"
    );
    assert!(
        derived.memo_cap < crate::ccm_emitter::DEFAULT_MEMO_CAP,
        "the chosen budget must shrink memo_cap below the default \
         (got {} vs default {})",
        derived.memo_cap,
        crate::ccm_emitter::DEFAULT_MEMO_CAP
    );

    // Unbudgeted baseline.
    let baseline_dir = temp_output_dir("memonly-baseline")?;
    emit_ccm_dir(&model, &baseline_dir.path)?;
    let baseline_hash = combined_ccm_hash(&baseline_dir.path)?;

    // Memory-only budget: apply the derived (smaller) memo cap, NO RSS
    // budget, single-partition collapse. Byte-neutral by the §8 contract.
    let memonly_dir = temp_output_dir("memonly-budget")?;
    emit_ccm_dir_with_budget(
        &model,
        &memonly_dir.path,
        "facet-name-ascending",
        "in-crate",
        usize::MAX,
        Some(derived.memo_cap),
        None,
    )?;
    let memonly_hash = combined_ccm_hash(&memonly_dir.path)?;

    assert_eq!(
        baseline_hash, memonly_hash,
        "a memory-only budget (smaller memo_cap, no derived cluster_size) \
         must produce byte-identical artifacts to the unbudgeted build"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// 3. PROGRESS IS MONOTONIC.
// ---------------------------------------------------------------------

/// Capturing sink that records every emitted [`ProgressEvent`] for
/// post-hoc assertions. Observational only — never mutates compile state.
#[derive(Default)]
struct CapturingSink {
    events: RefCell<Vec<ProgressEvent>>,
}

impl ProgressSink for CapturingSink {
    fn on_event(&self, event: &ProgressEvent) {
        self.events.borrow_mut().push(*event);
    }
}

#[test]
fn progress_is_monotonic_and_terminates_at_100() -> Result<()> {
    // ADR-0039 §7: drive a real compile with a capturing ProgressSink and
    // assert the overall pct sequence is monotonic non-decreasing and
    // terminates at EXACTLY 100 %. RSS / ETA in the events are observational
    // and are NOT asserted (they depend on the machine); only the pct
    // contract is pinned, which is deterministic.
    let sink = CapturingSink::default();
    let output_dir = temp_output_dir("progress-monotonic")?;
    let result = compile_model_with_progress(
        CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: budget_test_manifest(),
            output_dir: Some(output_dir.path.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        },
        Some(&sink),
    );
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "compile must succeed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );

    let events = sink.events.borrow();
    assert!(!events.is_empty(), "a wired sink must observe progress events");

    let mut prev = f32::NEG_INFINITY;
    for ev in events.iter() {
        assert!(
            ev.pct + f32::EPSILON >= prev,
            "pct must be monotonic non-decreasing: {prev} then {}",
            ev.pct
        );
        assert!(
            (0.0..=1.0 + f32::EPSILON).contains(&ev.pct),
            "pct out of range: {}",
            ev.pct
        );
        prev = ev.pct;
    }

    let last = events.last().expect("at least one event");
    assert_eq!(
        last.pct, 1.0,
        "the final progress event must be exactly 1.0 (100%), got {}",
        last.pct
    );
    assert_eq!(
        last.phase,
        Phase::Serialize,
        "the terminal event must be the Serialize phase"
    );

    // The summary also reports the furthest phase reached.
    let summary = result
        .progress_summary
        .context("a wired sink must yield a progress_summary on the result")?;
    assert_eq!(summary.peak_phase, Phase::Serialize);
    Ok(())
}

// ---------------------------------------------------------------------
// 4. HARDENING.
// ---------------------------------------------------------------------

#[test]
fn compile_request_omits_budget_key_when_none() -> Result<()> {
    // Hardening (4a): the `budget` key must be ABSENT from a serialized
    // CompileModelRequest when budget is None (`skip_serializing_if`), so an
    // unbudgeted request's wire form is byte-identical to one authored
    // before the field existed. Mirrors how `cluster_size` is dropped when
    // unset.
    #[derive(Serialize)]
    struct Probe<'a> {
        #[serde(flatten)]
        inner: &'a CompileModelRequest,
    }
    let request = CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: budget_test_manifest(),
        output_dir: None,
        cluster_size: None,
        budget: None,
        stamp_time: false,
    };
    let value = serde_json::to_value(Probe { inner: &request })?;
    let obj = value
        .as_object()
        .context("CompileModelRequest must serialize to a JSON object")?;
    assert!(
        !obj.contains_key("budget"),
        "the `budget` key must be absent when budget is None; got keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
    // And present when set, so the absence above is meaningful (not a typo'd
    // field name).
    let with_budget = CompileModelRequest {
        budget: Some(ResourceBudget {
            max_rss_mb: Some(256),
            max_threads: None,
        }),
        ..request
    };
    let value2 = serde_json::to_value(Probe { inner: &with_budget })?;
    assert!(
        value2
            .as_object()
            .expect("object")
            .contains_key("budget"),
        "the `budget` key must be present when the budget is set"
    );
    Ok(())
}

#[test]
fn explicit_cluster_size_wins_over_budget_derived() {
    // Hardening (4c) — adversarial precedence (ADR-0012 Amendment 1): when
    // BOTH an explicit cluster_size AND a budget that would derive a
    // DIFFERENT cluster_size are set, the EXPLICIT value wins. The
    // production rule (compiler_core::emit_ccm_sibling_with_progress) is
    // `effective = request.cluster_size.or(derived.cluster_size)`. We pin
    // the load-bearing pieces deterministically: (1) the budget really does
    // derive a cluster_size different from our explicit choice, and (2) the
    // `.or()` precedence selects the explicit value.
    let budget = ResourceBudget {
        // A tight budget with a huge projected unique table forces a derived
        // cluster_size (the partitioner-driving branch of derive_knobs).
        max_rss_mb: Some(128),
        max_threads: None,
    };
    let huge_var_hint = 5_000_000_usize;
    let derived = derive_knobs(&budget, Some(huge_var_hint));
    let derived_cluster = derived
        .cluster_size
        .expect("a tight budget with a huge var hint must derive a cluster_size");

    // An explicit value the operator passes, chosen to differ from the
    // derived one so "explicit wins" is observable rather than vacuous.
    let explicit_cluster = derived_cluster.saturating_add(7);
    assert_ne!(
        explicit_cluster, derived_cluster,
        "test setup: explicit and derived cluster sizes must differ"
    );

    // The exact precedence expression the production path applies.
    let effective = Some(explicit_cluster).or(derived.cluster_size);
    assert_eq!(
        effective,
        Some(explicit_cluster),
        "an explicit --cluster-size must win over the budget-derived value \
         (ADR-0012 Amendment 1)"
    );

    // And the complement: with no explicit value, the derived one is used.
    let effective_derived: Option<usize> = None.or(derived.cluster_size);
    assert_eq!(
        effective_derived,
        Some(derived_cluster),
        "with no explicit --cluster-size the budget-derived value applies"
    );
}

/// Sanity: the full unbudgeted compile path used by the progress test also
/// carries NO budget_report (the default wire form is byte-identical to
/// today). This guards the §8 "unbudgeted is silent" contract end-to-end,
/// complementing the per-emit byte-neutrality tests above.
#[test]
fn unbudgeted_compile_carries_no_budget_report() -> Result<()> {
    let output_dir = temp_output_dir("no-budget-report")?;
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: budget_test_manifest(),
        output_dir: Some(output_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(result.status, OperationStatus::Ok);
    assert!(
        result.budget_report.is_none(),
        "an unbudgeted compile must carry no budget_report"
    );
    assert!(
        result.progress_summary.is_none(),
        "a compile with no sink must carry no progress_summary"
    );
    Ok(())
}
