// SPDX-License-Identifier: BUSL-1.1

//! `link --progress` — the compile-time progress signal on the link verb
//! (ADR-0039 §7, ADR-0005 Amendment 2; ADR-0058 §D4 item 1).
//!
//! §D4 gives `link` "the CCM and resource flags as `compile`", and `--progress`
//! is one of them: the constraint model is a link product, so the step that
//! builds it is the step whose progress an operator wants to watch. A link over
//! a large model runs the same var-order -> apply -> serialize band a compile
//! does, and it was the only one of the five CCM/resource flags the verb did not
//! accept.
//!
//! Two claims, and the second is the load-bearing one:
//!
//! 1. A wired sink receives events and the returned `progress_summary` is
//!    populated, so the signal is real rather than a flag that parses.
//! 2. The package a link with a sink writes is BYTE-IDENTICAL to the one the
//!    same link writes with no sink. Progress is a separate stream (ADR-0005
//!    Amendment 2): it never enters `ccm.manifest.json`, `ccm.symbols.json`,
//!    `ccm.bdd.bin`, `partition-manifest.json`, or any package file. Without
//!    this assertion, wiring a sink into the link could silently break the §D8
//!    byte-identity oracle.
//!
//! Black box through the public product API (`compile_object`,
//! `link_model`, `link_model_with_progress`), over the shipped four-unit
//! example, so the model has real facets and clauses for the emitter to report
//! phases over.

use compiler::product_api::{
    link_model, link_model_with_progress, LinkModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use compiler::progress::{Phase, ProgressEvent, ProgressSink};
use compiler::object_compile::{compile_object, CompileObjectRequest};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "temp_dirs.rs"]
mod temp_dirs;

const CATALOGUE: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json");
const VISION: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/vision/10_vision.json");

/// Records every event the link emits, so the test can assert on the stream
/// rather than on a side effect of printing it.
#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<ProgressEvent>>,
}

impl ProgressSink for RecordingSink {
    fn on_event(&self, event: &ProgressEvent) {
        self.events.lock().expect("sink lock").push(event.clone());
    }
}

#[test]
fn a_wired_sink_receives_the_links_progress_and_a_summary_comes_back() {
    let dir = temp_dirs::unique_temp_dir("link-progress", "sink");
    let objects = build_objects(&dir);

    let sink = RecordingSink::default();
    let result = run_link(&objects, &dir.join("out"), Some(&sink));

    let events = sink.events.lock().expect("sink lock").clone();
    assert!(
        !events.is_empty(),
        "a wired sink must receive the link's progress events"
    );
    assert!(
        events.iter().any(|event| event.phase == Phase::Link),
        "the link band must be reported: {:?}",
        events.iter().map(|e| e.phase).collect::<Vec<_>>()
    );
    let summary = result
        .progress_summary
        .expect("a link with a sink must return a progress summary");
    assert_eq!(
        summary.peak_phase,
        Phase::Serialize,
        "a successful link reaches the terminal phase"
    );
}

#[test]
fn progress_never_changes_a_byte_of_the_linked_package() {
    // ADR-0005 Amendment 2: progress is observational. If wiring the sink moved
    // one byte, the §D8 oracle (`compile` == `compile-object` + `link`) would
    // hold only for links that were not being watched.
    let dir = temp_dirs::unique_temp_dir("link-progress", "byte-identity");
    let objects = build_objects(&dir);

    let quiet = dir.join("quiet");
    let watched = dir.join("watched");
    run_link(&objects, &quiet, None);
    let sink = RecordingSink::default();
    run_link(&objects, &watched, Some(&sink));

    assert!(
        !sink.events.lock().expect("sink lock").is_empty(),
        "the watched link must actually have been watched"
    );
    assert_trees_identical(&quiet, &watched);
}

#[test]
fn a_link_with_no_sink_reports_no_progress_summary() {
    // The default path stays absent from the wire form, exactly as `compile`'s
    // does — `progress_summary` is `skip_serializing_if = "Option::is_none"`.
    let dir = temp_dirs::unique_temp_dir("link-progress", "no-sink");
    let objects = build_objects(&dir);

    let result = run_link(&objects, &dir.join("out"), None);
    assert!(
        result.progress_summary.is_none(),
        "an unwatched link must not report a progress summary"
    );
}

// ----------------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------------

/// The catalogue and the service that imports it, one object each.
fn build_objects(dir: &Path) -> Vec<String> {
    [("site_catalogue", CATALOGUE), ("vision_service", VISION)]
        .into_iter()
        .map(|(unit, content)| {
            let out = dir.join(format!("{unit}.cfo")).to_string_lossy().into_owned();
            compile_object(CompileObjectRequest {
                sources: vec![SourceManifestEntry {
                    source_id: format!("{unit}.json"),
                    inline_content: content.to_string(),
                }],
                interfaces: Vec::new(),
                output_dir: out.clone(),
                stamp_time: false,
            })
            .unwrap_or_else(|diagnostic| panic!("compile-object failed: {diagnostic:?}"));
            out
        })
        .collect()
}

fn run_link(
    object_dirs: &[String],
    out: &Path,
    sink: Option<&dyn ProgressSink>,
) -> compiler::product_api::CompileResult {
    let result = link_model_or_plain(object_dirs, out, sink);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "link failed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
    result
}

/// Route through the plain entry point when no sink is wired, so the test
/// exercises the SAME two functions `main.rs` picks between — a regression in
/// either one is visible here.
fn link_model_or_plain(
    object_dirs: &[String],
    out: &Path,
    sink: Option<&dyn ProgressSink>,
) -> compiler::product_api::CompileResult {
    let request = LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: object_dirs.to_vec(),
        output_dir: out.to_string_lossy().into_owned(),
        cluster_size: None,
        budget: None,
        stamp_time: false,
        lock_path: None,
        lock_allow_extra: false,
        write_lock_path: None,
        lock_sources: Default::default(),
        force_lock: false,
    };
    match sink {
        None => link_model(request),
        Some(sink) => link_model_with_progress(request, Some(sink)),
    }
}

fn assert_trees_identical(left: &Path, right: &Path) {
    let a = tree(left);
    let b = tree(right);
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "the two packages hold different files"
    );
    for (name, left_bytes) in &a {
        let right_bytes = &b[name];
        assert_eq!(
            left_bytes,
            right_bytes,
            "'{name}' differs ({} vs {} bytes)",
            left_bytes.len(),
            right_bytes.len()
        );
    }
}

/// Every file under `root`, keyed by its path relative to `root`.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|err| panic!("read '{}': {err}", dir.display()));
        for entry in entries {
            let path: PathBuf = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .into_owned();
            out.insert(relative, std::fs::read(&path).expect("read file"));
        }
    }
    out
}
