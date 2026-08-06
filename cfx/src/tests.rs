// SPDX-License-Identifier: BUSL-1.1
//
// Unit tests for the `cfx` argument surface: parse/dispatch, the exit-code
// contract, and the `cfx explain` command `emit_error` suggests on a rejection.
// Split out of `main.rs` to keep that file within the repo line budget; the
// module is declared there as `#[cfg(test)] mod tests`.

use super::*;

fn run_args(args: &[&str]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(args.iter().map(|s| s.to_string()), &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn hint(model: &str, selection_file: Option<&str>, selects: &[&str]) -> String {
    let owned: Vec<String> = selects.iter().map(|s| s.to_string()).collect();
    explain_hint(
        std::path::Path::new(model),
        selection_file.map(std::path::Path::new),
        &owned,
    )
}

#[test]
fn explain_hint_echoes_the_selection_file() {
    // configflux-0qk2: the file carries the scope and the immutable context
    // tags, so a hint that drops it names a DIFFERENT selection.
    let got = hint("/m/cmp.json", Some("/m/sel.json"), &["region=eu"]);
    assert!(
        got.contains("--selection-file /m/sel.json"),
        "hint must echo the selection file: {got}"
    );
    assert!(got.contains("--select region=eu"), "hint must keep the flags: {got}");
}

#[test]
fn explain_hint_puts_the_file_before_the_flags() {
    // Same precedence `cfx resolve` applies: file first, flags override.
    let got = hint("/m/cmp.json", Some("/m/sel.json"), &["a=b"]);
    let file_at = got.find("--selection-file").expect("no --selection-file");
    let select_at = got.find("--select ").expect("no --select");
    assert!(file_at < select_at, "file must precede the flags: {got}");
}

#[test]
fn explain_hint_without_a_selection_file_is_flags_only() {
    // The all-flags path already produced a correct hint; keep it byte-equal.
    let got = hint("/m/cmp.json", None, &["a=b", "c=d"]);
    assert_eq!(got, "cfx explain --model /m/cmp.json --select a=b --select c=d");
}

#[test]
fn no_subcommand_is_usage_error() {
    let (code, _out, _err) = run_args(&["cfx"]);
    assert_eq!(code, EXIT_USAGE);
}

#[test]
fn help_is_success() {
    let (code, out, _err) = run_args(&["cfx", "--help"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("resolve"));
    assert!(out.contains("options"), "help must list the options verb: {out}");
    assert!(out.contains("explain"), "help must list the explain verb: {out}");
}

#[test]
fn explain_malformed_select_is_usage_error_naming_arg() {
    let (code, _out, err) = run_args(&[
        "cfx", "explain", "--model", "/nonexistent/cmp.json", "--select", "bogus",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
}

#[test]
fn explain_missing_model_file_is_usage_error() {
    let (code, _out, err) = run_args(&[
        "cfx",
        "explain",
        "--model",
        "/nonexistent/cmp.manifest.json",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.starts_with("cfx:"), "stderr: {err}");
}

#[test]
fn options_malformed_select_is_usage_error_naming_arg() {
    let (code, _out, err) = run_args(&[
        "cfx", "options", "--model", "/nonexistent/cmp.json", "--select", "bogus",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
}

#[test]
fn options_missing_model_file_is_usage_error() {
    let (code, _out, err) = run_args(&[
        "cfx",
        "options",
        "--model",
        "/nonexistent/cmp.manifest.json",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.starts_with("cfx:"), "stderr: {err}");
}

#[test]
fn malformed_select_is_usage_error_naming_arg() {
    let (code, _out, err) = run_args(&[
        "cfx", "resolve", "--model", "/nonexistent/cmp.json", "--out", "/tmp/x", "--select",
        "bogus",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
}

#[test]
fn missing_model_file_is_usage_error() {
    // `--out` is inert: resolve fails at model open, before any write (configflux-rvpb).
    let (code, _out, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        "/nonexistent/cmp.manifest.json",
        "--out",
        "/tmp/x",
    ]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.starts_with("cfx:"), "stderr: {err}");
}
