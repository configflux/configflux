// SPDX-License-Identifier: BUSL-1.1

//! configflux-l4e7: a deep definition `inherits` chain must fail closed on the
//! INSPECT path, not take the process down.
//!
//! The `overrides` chain that configflux-dw9i bounded is nested authored
//! structure, so both parsers refuse it far below any dangerous depth (62 levels
//! via JSON, 39 inline / 78 dotted-header via TOML — see
//! `tests/override_chain_depth.rs`). The `inherits` chain is not that shape. It
//! is a FLAT map of string pointers:
//!
//! ```text
//! "definitions": {"d000000": {...}, "d000001": {"inherits": "d000000"}, ...}
//! ```
//!
//! Nesting depth 3, whatever the chain's length. No parser limit applies, so the
//! pinned limits configflux-jz33 accepts as the bound on nesting do not cover
//! this at all.
//!
//! MEASURED, not reasoned, against the real `configflux-compiler` binary before
//! the ceiling was added:
//!
//! | chain length | `verify` | `inspect parameter`            |
//! |--------------|----------|--------------------------------|
//! | 1_000        | exit 0   | exit 0                         |
//! | 1_001        | exit 0   | exit 0  (ran 1_001 frames deep)|
//! | 25_000       | exit 0   | exit 0                         |
//! | 30_000       | exit 0   | SIGABRT, "stack overflow"      |
//! | 40_000       | exit 0   | SIGABRT, "stack overflow"      |
//!
//! So the abort premise dw9i was filed on, which did NOT reproduce for the
//! `overrides` chain, DOES reproduce here — through a shipped verb, on authored
//! input, with no signal other than the process dying.
//!
//! WHY THE LINK STAGE DOES NOT CATCH IT. `link_verify::detect_definition_cycle`
//! has carried the `MAX_CHAIN_DEPTH` ceiling since configflux-xowl.5, but it
//! measures `stack.len()` inside a DFS that shares ONE `states` map across every
//! root and returns early on `VisitState::Done`. The roots are visited in sorted
//! id order, so when the ids sort such that the chain's TAIL is reached first,
//! every node is already `Done` before the node below it becomes a root, the
//! stack never exceeds depth 1, and the ceiling never fires. That is why the
//! chains below are built with ascending zero-padded ids and `d_k` inheriting
//! `d_{k-1}`: the ORDER is load-bearing, not incidental.
//!
//! A subprocess, deliberately, for the same reason `tests/override_chain_depth.rs`
//! uses one: a stack overflow aborts the whole process, so a regression asserted
//! in-process would take the test runner down with it and report as
//! infrastructure noise rather than as this failure.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "temp_dirs.rs"]
mod temp_dirs;

/// The ceiling `link_verify` puts on every chain walk. Not imported from the
/// crate: this test speaks only through the binary, and the number is part of
/// what it is checking rather than a value it should inherit.
const MAX_CHAIN_DEPTH: usize = 1_000;

/// Past the measured abort threshold (between 25_000 and 30_000 on a release
/// build), so the no-abort case is a real test of the ceiling rather than of a
/// depth the stack happened to survive. ~2 MB of flat JSON.
const PATHOLOGICAL_DEPTH: usize = 40_000;

/// What an inspect-path refusal reports. The ceiling itself raises a CODELESS
/// `bail!` — the same shape as its link-side twin, which is how a depth refusal
/// lands on the product mappers' generic bucket (configflux-py7w). On this path
/// the inspect item builder wraps any such error in its own query diagnostic, so
/// the code an operator sees is this one rather than `E_COMPILE_INPUT_INVALID`.
const EXPECTED_CODE: &str = "E_INSPECT_QUERY_INVALID";

/// The wording every `MAX_CHAIN_DEPTH` refusal in this compiler shares. Asserted
/// as a substring so this test is not coupled to the inspect wrapper's prose.
const CEILING_MESSAGE: &str = "exceeds the maximum supported depth";

/// `configflux-compiler`'s exit code for a model it will not accept
/// (`0`=success, `1`=user/input error, `2`=compilation error).
const EXIT_COMPILATION_ERROR: i32 = 2;

const COMPONENT: &str = "motor";
const PARAM: &str = "speed";

/// The product binary, out of this test's runfiles tree.
fn compiler_binary() -> PathBuf {
    let rlocation = std::env::var("COMPILER_RLOCATION").expect(
        "COMPILER_RLOCATION is not set; it reaches this test through the `env` \
         attribute on //compiler:inherits_chain_depth_test",
    );
    let srcdir =
        std::env::var("TEST_SRCDIR").expect("TEST_SRCDIR is not set; run under bazel test");
    let workspace = std::env::var("TEST_WORKSPACE").unwrap_or_else(|_| "_main".to_string());
    let path = PathBuf::from(&srcdir).join(&workspace).join(&rlocation);
    assert!(
        path.is_file(),
        "compiler binary not in the runfiles tree at {path:?}"
    );
    path
}

/// `length` definitions in one chain, plus a component parameter inheriting its
/// head.
///
/// `d000000` is the chain TAIL and sorts FIRST, so the link-side DFS marks each
/// node `Done` before the node below it becomes a root (see the module header).
/// Built by concatenation: the shape is flat, so there is nothing to recurse
/// over, and a serializer would only cost memory.
fn chain_chunk(length: usize) -> String {
    let mut out = String::from(r#"{"package":"p1","version":"1.0","definitions":{"#);
    for index in 0..length {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(r#""d{index:06}":{{"type":"float""#));
        if index > 0 {
            out.push_str(&format!(r#","inherits":"d{:06}""#, index - 1));
        }
        out.push('}');
    }
    out.push_str(&format!(
        r#"}},"components":{{"{COMPONENT}":{{"type":"actuator","params":{{"#
    ));
    out.push_str(&format!(
        r#""{PARAM}":{{"type":"float","value":1.0,"inherits":"d{:06}"}}"#,
        length - 1
    ));
    out.push_str("}}}}");
    out
}

/// What the binary did with one chunk.
struct Outcome {
    exit_code: Option<i32>,
    stdout: String,
}

/// Run one verb against a generated chunk. `args` is everything after `--source`.
fn run(label: &str, verb: &str, extra: &[&str], source: &str) -> Outcome {
    let dir = temp_dirs::unique_temp_dir("l4e7-inherits-depth", label);
    let chunk = dir.join("chunk.json");
    fs::write(&chunk, source).expect("write the generated chunk");

    let output = Command::new(compiler_binary())
        .arg(verb)
        .arg("--source")
        .arg(&chunk)
        .args(extra)
        .output()
        .expect("run the compiler binary");

    fs::remove_dir_all(&dir).ok();
    Outcome {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

fn inspect_parameter(label: &str, length: usize) -> Outcome {
    run(
        label,
        "inspect",
        &["parameter", COMPONENT, PARAM],
        &chain_chunk(length),
    )
}

fn head(text: &str) -> &str {
    &text[..text.len().min(2_000)]
}

/// The assertion this whole file exists for: the tool answered, rather than
/// died. `ExitStatus::code()` is `None` exactly when a signal ended the process,
/// which is how a stack overflow ends one.
fn assert_answered_rather_than_died(label: &str, outcome: &Outcome) {
    assert!(
        outcome.exit_code.is_some(),
        "{label}: the compiler was killed by a signal instead of reporting a diagnostic"
    );
}

#[test]
fn an_inherits_chain_past_the_ceiling_is_refused_by_diagnostic_on_the_inspect_path() {
    let outcome = inspect_parameter("past-ceiling", MAX_CHAIN_DEPTH + 1);
    assert_answered_rather_than_died("past-ceiling", &outcome);
    assert_eq!(
        outcome.exit_code,
        Some(EXIT_COMPILATION_ERROR),
        "past-ceiling: expected the compilation-error exit code: {}",
        head(&outcome.stdout)
    );
    assert!(
        outcome.stdout.contains(EXPECTED_CODE),
        "past-ceiling: no {EXPECTED_CODE} in the report: {}",
        head(&outcome.stdout)
    );
    assert!(
        outcome.stdout.contains(CEILING_MESSAGE),
        "past-ceiling: the refusal is not the depth ceiling's: {}",
        head(&outcome.stdout)
    );
}

#[test]
fn an_inherits_chain_at_the_ceiling_still_inspects() {
    // Without this the file could pass by refusing everything. A chain exactly
    // at the ceiling is legitimate and must still produce an inspection item.
    let outcome = inspect_parameter("at-ceiling", MAX_CHAIN_DEPTH);
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "at-ceiling: a chain at the ceiling must still inspect: {}",
        head(&outcome.stdout)
    );
}

#[test]
fn a_pathologically_deep_inherits_chain_never_aborts_the_inspect_path() {
    // The case the ceiling is actually for. At this length the unguarded
    // recursion overflowed the stack and aborted; the only acceptable answer is
    // a diagnostic.
    let outcome = inspect_parameter("pathological", PATHOLOGICAL_DEPTH);
    assert_answered_rather_than_died("pathological", &outcome);
    assert_eq!(
        outcome.exit_code,
        Some(EXIT_COMPILATION_ERROR),
        "pathological: expected the compilation-error exit code: {}",
        head(&outcome.stdout)
    );
    assert!(
        outcome.stdout.contains(CEILING_MESSAGE),
        "pathological: the refusal is not the depth ceiling's: {}",
        head(&outcome.stdout)
    );
}

#[test]
fn the_link_stage_accepts_the_chain_the_inspect_ceiling_has_to_refuse() {
    // This is WHY the inspect-path ceiling is not dead code. `verify` runs the
    // whole link stage, including `detect_definition_cycle`'s own
    // `MAX_CHAIN_DEPTH` guard, and accepts a chain far past that number because
    // the guard's `Done` short-circuit is defeated by root order (see the module
    // header). Nothing upstream of the inspect walk refuses this input, so the
    // inspect walk has to refuse it itself.
    //
    // If the link-side guard is ever made order-independent, this assertion is
    // the one that should fail — the inspect ceiling stays either way (one
    // ceiling per shape), but its justification will have changed.
    let outcome = run(
        "link-accepts",
        "verify",
        &[],
        &chain_chunk(MAX_CHAIN_DEPTH + 1),
    );
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "link-accepts: the link stage no longer accepts this chain: {}",
        head(&outcome.stdout)
    );
}
