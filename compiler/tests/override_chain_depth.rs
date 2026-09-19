// SPDX-License-Identifier: BUSL-1.1

//! configflux-dw9i: a deeply nested authored `overrides` chain must fail closed
//! with a diagnostic, and must never take the process down.
//!
//! MEASURED, not reasoned. `configflux-compiler verify --source` was driven
//! against generated chunks whose only content is one definition with N nested
//! override blocks. Every form refuses far below any depth that could exhaust a
//! stack, and no input of any depth or size produced a signal:
//!
//! | authored form            | deepest accepted | probed as far as    |
//! |--------------------------|------------------|---------------------|
//! | JSON                     | 62               | 500,000 lvl / 30 MB |
//! | TOML inline table        | 39               | 100,000 lvl / 6 MB  |
//! | TOML `[[a.b.overrides]]` | 78               | 10,000 lvl / 500 MB |
//!
//! One level past "deepest accepted", and at every probed depth above it, the
//! answer is the same: exit 2, one `E_COMPILE_INPUT_INVALID` diagnostic, and no
//! signal. The premise the issue was filed on — that such a chunk overflows the
//! stack and ABORTS the process — does not reproduce.
//!
//! It does not reproduce because both parsers carry their own nesting ceiling
//! (`serde_json`'s recursion limit and `toml`'s), and neither is disabled here.
//! That is a property of two pinned dependencies, not of this repository: a
//! version bump, or any crate in the graph enabling the cargo feature that
//! compiles the check out, removes it with no local signal. This test is what
//! makes the property ours. It asserts the OUTCOME an operator sees — a
//! diagnostic, from a process that exited by code rather than by signal — so it
//! keeps holding whichever layer ends up enforcing it: the parser today, the
//! `MAX_CHAIN_DEPTH` ceiling on the override walks if the parse limit ever
//! rises.
//!
//! A subprocess, deliberately. A stack overflow aborts the whole process, so a
//! regression asserted in-process would take the test runner down with it and
//! report as infrastructure noise rather than as this failure.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "temp_dirs.rs"]
mod temp_dirs;

/// The ceiling `link_verify` puts on the override walks. Not imported from the
/// crate: this test speaks only through the binary, and the number is part of
/// what it is checking rather than a value it should inherit.
const MAX_CHAIN_DEPTH: usize = 1_000;

/// Deep enough that no plausible parser limit sits above it, shallow enough
/// that the JSON and inline-TOML chunks stay near a megabyte.
const PATHOLOGICAL_DEPTH: usize = 20_000;

/// What a refused chunk must report. Every depth refusal in this compiler lands
/// on the same code as the two sibling `MAX_CHAIN_DEPTH` ceilings.
const EXPECTED_CODE: &str = "E_COMPILE_INPUT_INVALID";

/// `configflux-compiler`'s exit code for a model it will not accept
/// (`0`=success, `1`=user/input error, `2`=compilation error).
const EXIT_COMPILATION_ERROR: i32 = 2;

/// The product binary, out of this test's runfiles tree.
fn compiler_binary() -> PathBuf {
    let rlocation = std::env::var("COMPILER_RLOCATION").expect(
        "COMPILER_RLOCATION is not set; it reaches this test through the `env` \
         attribute on //compiler:override_chain_depth_test",
    );
    let srcdir = std::env::var("TEST_SRCDIR").expect("TEST_SRCDIR is not set; run under bazel test");
    let workspace = std::env::var("TEST_WORKSPACE").unwrap_or_else(|_| "_main".to_string());
    let path = PathBuf::from(&srcdir).join(&workspace).join(&rlocation);
    assert!(
        path.is_file(),
        "compiler binary not in the runfiles tree at {path:?}"
    );
    path
}

/// One definition, `depth` override blocks deep, as JSON.
///
/// Built by concatenation rather than by a recursive serializer, for the reason
/// the chunk exists at all: a recursive builder would overflow before the
/// compiler ever saw the input.
fn json_chunk(depth: usize) -> String {
    // `ConditionalBlock::payload` is `#[serde(flatten)]`, so a nested
    // parameter's fields sit INLINE beside `condition` rather than inside a
    // object of their own. The recursion is therefore over field lists, and one
    // level costs exactly one `[` and one `{`.
    let open = r#""type":"float","overrides":[{"condition":"region == 'eu'","#;
    let mut out = String::from(r#"{"package":"p1","version":"1.0","definitions":{"d0":{"#);
    out.push_str(&open.repeat(depth));
    out.push_str(r#""type":"float","value":1.0"#);
    out.push_str(&"}]".repeat(depth));
    out.push_str("}}}");
    out
}

/// The same chain as one TOML inline table.
fn toml_inline_chunk(depth: usize) -> String {
    let open = r#"type = "float", overrides = [{ condition = "region == 'eu'", "#;
    let mut out = String::from("package = \"p1\"\nversion = \"1.0\"\n\n[definitions]\nd0 = { ");
    out.push_str(&open.repeat(depth));
    out.push_str(r#"type = "float", value = 1.0"#);
    out.push_str(&" }]".repeat(depth));
    out.push_str(" }\n");
    out
}

/// The same chain as TOML array-of-table headers.
///
/// The header path grows one segment per level, so this form costs O(depth²)
/// bytes. Callers keep its depth near the ceiling rather than at
/// [`PATHOLOGICAL_DEPTH`], where it would be gigabytes.
fn toml_header_chunk(depth: usize) -> String {
    let mut out =
        String::from("package = \"p1\"\nversion = \"1.0\"\n\n[definitions.d0]\ntype = \"float\"\nvalue = 0.0\n\n");
    let mut path = String::from("definitions.d0.overrides");
    for _ in 0..depth {
        out.push_str(&format!("[[{path}]]\ncondition = \"region == 'eu'\"\nvalue = 1.0\n\n"));
        path.push_str(".overrides");
    }
    out
}

/// Every authored form the compiler sniffs, as (label, extension, source).
fn all_forms(depth: usize) -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("json", "json", json_chunk(depth)),
        ("toml_inline", "toml", toml_inline_chunk(depth)),
        ("toml_header", "toml", toml_header_chunk(depth)),
    ]
}

/// What `verify` did with one chunk.
struct VerifyOutcome {
    exit_code: Option<i32>,
    stdout: String,
}

fn verify(label: &str, extension: &str, source: &str) -> VerifyOutcome {
    let dir = temp_dirs::unique_temp_dir("dw9i-override-depth", label);
    let chunk = dir.join(format!("chunk.{extension}"));
    fs::write(&chunk, source).expect("write the generated chunk");

    let output = Command::new(compiler_binary())
        .arg("verify")
        .arg("--source")
        .arg(&chunk)
        .output()
        .expect("run the compiler binary");

    fs::remove_dir_all(&dir).ok();
    VerifyOutcome {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

/// The assertion this whole file exists for: the tool answered, rather than
/// died. `ExitStatus::code()` is `None` exactly when a signal ended the
/// process, which is how a stack overflow ends one.
fn assert_refused_by_diagnostic(label: &str, outcome: &VerifyOutcome) {
    let Some(code) = outcome.exit_code else {
        panic!("{label}: the compiler was killed by a signal instead of reporting a diagnostic");
    };
    assert_eq!(
        code, EXIT_COMPILATION_ERROR,
        "{label}: expected the compilation-error exit code, got {code}"
    );
    assert!(
        outcome.stdout.contains(EXPECTED_CODE),
        "{label}: no {EXPECTED_CODE} in the report: {}",
        &outcome.stdout[..outcome.stdout.len().min(2_000)]
    );
}

#[test]
fn a_chain_past_the_ceiling_is_refused_by_diagnostic_in_every_authored_form() {
    for (label, extension, source) in all_forms(MAX_CHAIN_DEPTH + 1) {
        let outcome = verify(label, extension, &source);
        assert_refused_by_diagnostic(label, &outcome);
    }
}

#[test]
fn a_pathologically_deep_chain_never_aborts_the_process() {
    // The header form is excluded by size, not by kind: its O(depth²) growth
    // puts it in the gigabytes here. The case above already drives it one level
    // past the ceiling, at roughly 5 MB.
    for (label, extension, source) in all_forms(PATHOLOGICAL_DEPTH)
        .into_iter()
        .filter(|(label, _, _)| *label != "toml_header")
    {
        let outcome = verify(label, extension, &source);
        assert_refused_by_diagnostic(label, &outcome);
    }
}

#[test]
fn the_nesting_depth_the_docs_teach_still_verifies() {
    // Without this the file could pass by refusing everything. Two levels is
    // the depth `test_nested_override_applies_depth_first` uses and the depth
    // the authoring guide shows.
    for (label, extension, source) in all_forms(2) {
        let outcome = verify(label, extension, &source);
        assert_eq!(
            outcome.exit_code,
            Some(0),
            "{label}: a two-level override chain must still verify: {}",
            &outcome.stdout[..outcome.stdout.len().min(2_000)]
        );
    }
}
