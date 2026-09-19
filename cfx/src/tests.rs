// SPDX-License-Identifier: BUSL-1.1
//
// Unit tests for the `cfx` argument surface: parse/dispatch, the exit-code
// contract, and the `cfx explain` command `emit_error` suggests on a rejection.
// Split out of `main.rs` to keep that file within the repo line budget; the
// module is declared there as `#[cfg(test)] mod tests`.
//
// This module also OWNS the CLI test harness — `run_args`, the in-process
// `compile_fixture*` model builders, `tree`, `write_fixture_file` — which the
// sibling `diff_tests` module reuses (configflux-dkmm.5). One harness, two test
// modules: the `cfx diff` tests need the same "compile a real model in a temp
// dir, drive the real CLI, assert on exit codes and bytes" machinery, and a
// second copy of it would be a second place for fixture semantics to drift.

use super::*;

pub(crate) fn run_args(args: &[&str]) -> (u8, String, String) {
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
        got.contains("--selection-file '/m/sel.json'"),
        "hint must echo the selection file: {got}"
    );
    assert!(
        got.contains("--select 'region=eu'"),
        "hint must keep the flags: {got}"
    );
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
    // The all-flags path already produced a correct hint; what moved is that
    // the model path (configflux-zz2g) and each pair (configflux-egyj) are now
    // quoted.
    let got = hint("/m/cmp.json", None, &["a=b", "c=d"]);
    assert_eq!(
        got,
        "cfx explain --model '/m/cmp.json' --select 'a=b' --select 'c=d'"
    );
}

#[test]
fn explain_hint_quotes_every_term_unconditionally() {
    // configflux-zz2g, extended to the pairs by configflux-egyj. EVERY
    // interpolated term is wrapped, metacharacter or not. Quoting only the
    // terms that need it would make the printed form depend on the directory
    // the user happens to be standing in and on the ids their model happens to
    // declare, and a hint that is quoted some of the time is one the reader has
    // to inspect before trusting.
    let got = hint("/m/cmp.json", Some("/m/sel.json"), &["a=b"]);
    assert_eq!(
        got,
        "cfx explain --model '/m/cmp.json' --selection-file '/m/sel.json' --select 'a=b'"
    );
}

#[test]
fn explain_hint_survives_a_space_in_either_path() {
    // The defect: unquoted, a POSIX shell splits `/my models/cmp.json` into two
    // arguments, so the printed command is not the command that runs. The shell
    // round trip itself is asserted end to end in
    // cfx/tests/cfx_resolve_explain_consistency.py, which parses this very line
    // out of stderr and hands it to /bin/sh.
    let got = hint("/my models/cmp.json", Some("/my sels/s.json"), &["region=eu"]);
    let want = "cfx explain --model '/my models/cmp.json' \
         --selection-file '/my sels/s.json' --select 'region=eu'";
    assert_eq!(got, want);
}

#[test]
fn explain_hint_quotes_a_select_pair_a_shell_would_re_split() {
    // configflux-egyj. The pairs were the one term zz2g left raw, on the
    // reasoning that they are not paths -- but nothing makes an id safe. A
    // facet's values are plain strings in BOTH the CUE authoring schema and the
    // Rust ingest, so a model may legitimately declare `de bug`, and the
    // rejection that names it prints a line the shell re-splits into arguments
    // `cfx explain` never sees. The whole `facet=option` is one word because
    // `parse_select_pair` splits on the first `=`: the pair has to arrive as a
    // single argv entry or the flag is malformed.
    let got = hint("/m/cmp.json", None, &["log_level=de bug"]);
    assert_eq!(
        got,
        "cfx explain --model '/m/cmp.json' --select 'log_level=de bug'"
    );
}

#[test]
fn explain_hint_neutralizes_a_command_separator_in_a_pair() {
    // The sharp end of the same defect: a `;` does not merely re-split the
    // line, it ENDS the `cfx explain` command, so a pasted hint runs whatever
    // follows as a command of its own. Quoted, the separator is data.
    let got = hint("/m/cmp.json", None, &["log_level=debug; id"]);
    assert_eq!(
        got,
        "cfx explain --model '/m/cmp.json' --select 'log_level=debug; id'"
    );
}

#[test]
fn explain_hint_escapes_a_single_quote_inside_a_pair() {
    // The one encoding a POSIX shell forces: no escape is honoured inside
    // single quotes, so an embedded `'` closes, emits an escaped quote, and
    // reopens -- the same rule the paths above use, applied to a pair.
    let got = hint("/m/cmp.json", None, &["region=eu 'west'"]);
    assert_eq!(
        got,
        r"cfx explain --model '/m/cmp.json' --select 'region=eu '\''west'\'''"
    );
}

#[test]
fn explain_hint_escapes_an_embedded_single_quote() {
    // A single quote cannot be escaped INSIDE single quotes -- a POSIX shell
    // honours no escape there -- so the only encoding is close, emit an escaped
    // quote, reopen.
    let got = hint("/it's/cmp.json", None, &[]);
    assert_eq!(got, r"cfx explain --model '/it'\''s/cmp.json'");
}

#[test]
fn shell_quote_wraps_every_shape_of_term() {
    // The encoding rule on its own, over the shapes that decide it: no
    // metacharacter at all, a space, a glob, a command separator, and the quote
    // character itself.
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("a b"), "'a b'");
    assert_eq!(shell_quote("a*b"), "'a*b'");
    assert_eq!(shell_quote("a; id"), "'a; id'");
    assert_eq!(shell_quote("a'b"), r"'a'\''b'");
}

#[test]
fn shell_quote_path_applies_the_same_rule_to_a_path() {
    // One encoder for both kinds of term (configflux-egyj). The path adapter
    // adds the lossy `display()` rendering and NOTHING else -- a second quoting
    // rule would be a second place for the two to disagree about what a shell
    // does with a quote.
    assert_eq!(shell_quote_path(std::path::Path::new("/plain")), "'/plain'");
    assert_eq!(shell_quote_path(std::path::Path::new("/a b")), "'/a b'");
    assert_eq!(shell_quote_path(std::path::Path::new("/a*b")), "'/a*b'");
    assert_eq!(shell_quote_path(std::path::Path::new("/a'b")), r"'/a'\''b'");
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

// ---------------------------------------------------------------------------
// `cfx resolve --out` writes the resolved snapshot itself (configflux-dkmm.1,
// ADR-0042 amendment). These drive the WHOLE pipeline over a real compiled
// model — the fixtures are compiled in-process with `compiler::product_api`,
// the same way the runtime crate builds its scenario fixtures — because the
// contract under test is a file on disk, not a pure function.
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use compiler::product_api::{
    compile_model, CompileModelRequest, SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};

const S1_DEFS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/10_components.json");
pub(crate) const HERO_DEFS: &str =
    include_str!("../../examples/00-service-multi-env/00_definitions.json");
pub(crate) const HERO_COMPONENTS: &str =
    include_str!("../../examples/00-service-multi-env/10_components.json");

/// Per-process monotonic counter: `#[test]` threads run in parallel and must
/// never collide on a fixture directory name.
static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A compiled-model package (plus its sibling `ccm/`) in a unique temp dir,
/// removed when the test ends.
pub(crate) struct ModelFixture {
    pub(crate) dir: PathBuf,
    pub(crate) manifest: String,
}

impl Drop for ModelFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl ModelFixture {
    /// A fresh, non-existent subdirectory to hand to `--out`.
    pub(crate) fn out(&self, label: &str) -> PathBuf {
        self.dir.join(label)
    }
}

pub(crate) fn compile_fixture(label: &str, defs: &str, components: &str) -> ModelFixture {
    compile_fixture_chunks(
        label,
        &[
            ("00_definitions.json", defs),
            ("10_components.json", components),
        ],
    )
}

/// Compile ANY number of chunks into one model fixture.
///
/// `--source` is repeatable and the merged model is the unit of identity, so a
/// model composed from three repositories is not a special mode — it is the
/// same compile with more entries (`examples/06-catalogue-polyrepo`). The
/// two-chunk `compile_fixture` above is the common case spelled shorter.
pub(crate) fn compile_fixture_chunks(label: &str, chunks: &[(&str, &str)]) -> ModelFixture {
    let seq = FIXTURE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cfx-{label}-{}-{seq}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create fixture dir");

    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: chunks
            .iter()
            .map(|(source_id, contents)| SourceManifestEntry {
                source_id: (*source_id).to_string(),
                inline_content: (*contents).to_string(),
            })
            .collect(),
        output_dir: Some(dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "fixture compile failed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
    let manifest = result
        .compiled_model_package_ref
        .expect("compile emitted no compiled_model_package_ref");
    ModelFixture { dir, manifest }
}

/// Every relative path under `root`, sorted — used to pin the exact file set
/// `--out` produces.
pub(crate) fn tree(root: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, acc: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, acc);
            } else {
                acc.push(
                    path.strip_prefix(root)
                        .expect("path under root")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut acc = Vec::new();
    walk(root, root, &mut acc);
    acc.sort();
    acc
}

/// A `SelectionState` JSON pinning a scope with no choices. `cfx` re-derives
/// the hash, so the two hash fields are placeholders.
fn write_scope_selection(path: &std::path::Path, scope: &str) {
    std::fs::write(
        path,
        format!(
            r#"{{"schema_version":{PRODUCT_SCHEMA_VERSION},"model_hash":"","scope":"{scope}",
"context_tags":{{}},"choices":{{}},"selection_state_hash":""}}"#
        ),
    )
    .expect("write selection file");
}

/// ADR-0059 D3 (T5): the text lineage gains `resolved_output_hash` DIRECTLY
/// after `resolve_hash`, and it carries the same value the JSON form does.
///
/// Position is part of the contract, not cosmetics: the two hashes are a pair —
/// one rotates with the model, one only with the delivered bytes — and a reader
/// scanning the lineage has to see them together to tell them apart. Driving
/// the real pipeline (rather than the `render_text` fixture, which is asserted
/// separately in render.rs) is what makes the equality meaningful: both values
/// come from one resolve of one real model.
#[test]
fn resolve_text_lineage_carries_resolved_output_hash_after_resolve_hash() {
    let fixture = compile_fixture("s1-output-hash", S1_DEFS, S1_COMPONENTS);
    let text_out = fixture.out("text_out");
    let text_out_arg = text_out.to_string_lossy().into_owned();

    // Named `fn` rather than a closure: both borrows must share one lifetime so
    // the returned Vec can outlive the call, which a closure cannot express.
    fn resolve_args<'a>(manifest: &'a str, out: &'a str) -> Vec<&'a str> {
        vec![
            "cfx",
            "resolve",
            "--model",
            manifest,
            "--select",
            "cooling_brand=hydra",
            "--select",
            "cooling_model=x200",
            "--select",
            "pump_type=dual",
            "--select",
            "region=eu",
            "--out",
            out,
        ]
    }

    let (code, text_stdout, err) = run_args(&resolve_args(&fixture.manifest, &text_out_arg));
    assert_eq!(code, EXIT_OK, "resolve must succeed, stderr: {err}");

    let lines: Vec<&str> = text_stdout.lines().collect();
    let resolve_hash_at = lines
        .iter()
        .position(|line| line.starts_with("resolve_hash: "))
        .expect("text lineage must carry resolve_hash");
    let next = lines
        .get(resolve_hash_at + 1)
        .expect("resolve_hash must not be the last lineage line");
    let text_value = next
        .strip_prefix("resolved_output_hash: ")
        .unwrap_or_else(|| {
            panic!("expected resolved_output_hash directly after resolve_hash, got {next:?}")
        });
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("resolved_output_hash: "))
            .count(),
        1,
        "exactly one resolved_output_hash line"
    );
    assert_eq!(text_value.len(), 64, "sha256 hex: {text_value}");

    // ...and it is the same value `--format json` reports.
    let json_out = fixture.out("json_out");
    let json_out_arg = json_out.to_string_lossy().into_owned();
    let mut json_args = resolve_args(&fixture.manifest, &json_out_arg);
    json_args.extend(["--format", "json"]);
    let (json_code, json_stdout, json_err) = run_args(&json_args);
    assert_eq!(json_code, EXIT_OK, "json resolve must succeed: {json_err}");
    let value: serde_json::Value =
        serde_json::from_str(&json_stdout).expect("json stdout must parse");
    assert_eq!(
        value["resolved_output_hash"], text_value,
        "text and JSON must report one value"
    );
    // The payload identity is NOT the resolution identity.
    assert_ne!(
        value["resolved_output_hash"], value["resolve_hash"],
        "the two hashes cover different pre-images"
    );
}

#[test]
fn resolve_writes_snapshot_file_byte_identical_to_json_stdout() {
    let fixture = compile_fixture("s1-snapshot", S1_DEFS, S1_COMPONENTS);
    let text_out = fixture.out("text_out");
    let text_out_arg = text_out.to_string_lossy().into_owned();

    let (code, text_stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "cooling_brand=hydra",
        "--select",
        "cooling_model=x200",
        "--select",
        "pump_type=dual",
        "--select",
        "region=eu",
        "--out",
        &text_out_arg,
    ]);
    assert_eq!(code, EXIT_OK, "resolve must succeed, stderr: {err}");

    // Default scope is `all`; the four explicit choices, sorted by facet, join
    // to `hydra-x200-dual-eu`.
    const NAME: &str = "resolve_result.all.hydra-x200-dual-eu.json";
    let snapshot = text_out.join(NAME);
    assert!(
        snapshot.is_file(),
        "--out must hold {NAME}; it held {:?}",
        tree(&text_out)
    );

    // The snapshot sorts after the exported `generated/` paths, so it is the
    // LAST `wrote:` line (ADR-0042 §3 lineage order is preserved).
    let last_wrote = text_stdout
        .lines()
        .filter(|line| line.starts_with("wrote: "))
        .next_back()
        .expect("text output must carry wrote: lines");
    assert_eq!(last_wrote, format!("wrote: {NAME}"));

    // `--out` holds exactly the snapshot plus the export profile's three files.
    assert_eq!(
        tree(&text_out),
        vec![
            "generated/config.hpp".to_string(),
            "generated/config_artifact_manifest.json".to_string(),
            "generated/config_build_flags.cmake".to_string(),
            NAME.to_string(),
        ]
    );

    // The written bytes ARE the `--format json` stdout bytes.
    let json_out = fixture.out("json_out");
    let json_out_arg = json_out.to_string_lossy().into_owned();
    let (json_code, json_stdout, json_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "cooling_brand=hydra",
        "--select",
        "cooling_model=x200",
        "--select",
        "pump_type=dual",
        "--select",
        "region=eu",
        "--out",
        &json_out_arg,
        "--format",
        "json",
    ]);
    assert_eq!(json_code, EXIT_OK, "json resolve must succeed: {json_err}");
    let written = std::fs::read(&snapshot).expect("read snapshot");
    assert_eq!(
        written,
        json_stdout.as_bytes(),
        "snapshot file must be byte-identical to --format json stdout"
    );

    // T8 — determinism: a second run writes the identical bytes.
    let again_out = fixture.out("again_out");
    let again_out_arg = again_out.to_string_lossy().into_owned();
    let (again_code, _, again_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "cooling_brand=hydra",
        "--select",
        "cooling_model=x200",
        "--select",
        "pump_type=dual",
        "--select",
        "region=eu",
        "--out",
        &again_out_arg,
    ]);
    assert_eq!(again_code, EXIT_OK, "re-resolve must succeed: {again_err}");
    assert_eq!(
        written,
        std::fs::read(again_out.join(NAME)).expect("read second snapshot"),
        "two runs must write byte-identical snapshots"
    );
}

#[test]
fn snapshot_name_uses_component_root_and_default_label() {
    let fixture = compile_fixture("hero-snapshot", HERO_DEFS, HERO_COMPONENTS);
    let selection = fixture.dir.join("webapp.selection.json");
    write_scope_selection(&selection, "component:webapp");
    let selection_arg = selection.to_string_lossy().into_owned();

    // No explicit choices: every facet auto-binds its declared default
    // (ADR-0047), and the <selection> label falls back to `default`.
    let bare_out = fixture.out("bare_out");
    let bare_out_arg = bare_out.to_string_lossy().into_owned();
    let (code, _, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &selection_arg,
        "--out",
        &bare_out_arg,
    ]);
    assert_eq!(code, EXIT_OK, "resolve must succeed: {err}");
    assert!(
        bare_out.join("resolve_result.webapp.default.json").is_file(),
        "choice-free resolve must label the snapshot `default`; --out held {:?}",
        tree(&bare_out)
    );

    // One explicit choice: the label is that choice's value. Auto-bound
    // defaults never enter the name (they live in `defaulted_choices`).
    let prod_out = fixture.out("prod_out");
    let prod_out_arg = prod_out.to_string_lossy().into_owned();
    let (prod_code, _, prod_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &selection_arg,
        "--select",
        "environment=prod",
        "--out",
        &prod_out_arg,
    ]);
    assert_eq!(prod_code, EXIT_OK, "resolve must succeed: {prod_err}");
    assert!(
        prod_out.join("resolve_result.webapp.prod.json").is_file(),
        "one explicit choice must label the snapshot with its value; --out held {:?}",
        tree(&prod_out)
    );
}

// ---------------------------------------------------------------------------
// `cfx resolve --manifest` — the environment manifest as a product input
// (configflux-dkmm.4, ADR-0059 D1/D2/M5). These drive the WHOLE CLI over real
// compiled models, because the contract under test is a directory tree plus an
// exit code, not a pure function.
// ---------------------------------------------------------------------------

const FLEET_DEFS: &str = include_str!("../../examples/05-compose-fleet/00_definitions.json");
const FLEET_COMPONENTS: &str = include_str!("../../examples/05-compose-fleet/10_components.json");
const FLEET_MANIFEST: &str = include_str!("../../examples/05-compose-fleet/environments.json");
pub(crate) const HERO_MANIFEST: &str =
    include_str!("../../examples/00-service-multi-env/environments.json");

/// Write `contents` into the fixture directory under `name` and return the path
/// as a `String` argument.
pub(crate) fn write_fixture_file(fixture: &ModelFixture, name: &str, contents: &str) -> String {
    let path = fixture.dir.join(name);
    std::fs::write(&path, contents).expect("write fixture file");
    path.to_string_lossy().into_owned()
}

/// The `cell:` header lines of a matrix run's stdout, in emission order.
fn cell_lines(stdout: &str) -> Vec<&str> {
    stdout.lines().filter(|l| l.starts_with("cell: ")).collect()
}

#[test]
fn manifest_single_environment_equals_selection_file() {
    // T1 (ADR-0059 D2 + M4): `--manifest --environment prod` must be the SAME
    // resolution as the selection file carrying that environment's (scope,
    // context_tags, choices) — same files, same bytes, same stdout, same flat
    // `<out>` layout. `prod` pins choices, so the snapshot name is identical
    // too; the environment-name fallback applies only to a choice-free
    // environment.
    let fixture = compile_fixture("hero-manifest", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);

    let manifest_out = fixture.out("manifest_out");
    let manifest_out_arg = manifest_out.to_string_lossy().into_owned();
    let (code, manifest_stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--manifest",
        &manifest,
        "--environment",
        "prod",
        "--out",
        &manifest_out_arg,
    ]);
    assert_eq!(code, EXIT_OK, "manifest resolve must succeed, stderr: {err}");

    // The same target spelled as a selection file.
    let selection = write_fixture_file(
        &fixture,
        "prod.selection.json",
        r#"{"schema_version":5,"model_hash":"","scope":"component:webapp",
"context_tags":{},"choices":{"beta_dashboard":"off","environment":"prod",
"log_level":"info","replica_class":"scaled"},"selection_state_hash":""}"#,
    );
    let file_out = fixture.out("file_out");
    let file_out_arg = file_out.to_string_lossy().into_owned();
    let (file_code, file_stdout, file_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &selection,
        "--out",
        &file_out_arg,
    ]);
    assert_eq!(file_code, EXIT_OK, "selection-file resolve must succeed: {file_err}");

    assert_eq!(
        manifest_stdout, file_stdout,
        "the manifest path must print the SAME lineage as the selection-file path"
    );
    let manifest_tree = tree(&manifest_out);
    assert_eq!(
        manifest_tree,
        tree(&file_out),
        "both paths must write the same file set (flat <out> layout, M4)"
    );
    assert!(
        manifest_tree.contains(&"resolve_result.webapp.off-prod-info-scaled.json".to_string()),
        "flat layout must hold the snapshot at the root of --out; held {manifest_tree:?}"
    );
    for rel in &manifest_tree {
        assert_eq!(
            std::fs::read(manifest_out.join(rel)).expect("read manifest file"),
            std::fs::read(file_out.join(rel)).expect("read selection-file file"),
            "{rel} must be byte-identical across the two input spellings"
        );
    }
}

#[test]
fn manifest_choice_free_environment_labels_the_snapshot_with_its_name() {
    // ADR-0059 D2: the ONE behavioural difference from the selection-file path
    // — a choice-free environment labels its snapshot with the environment
    // name, matching examples/resolve_environment.sh, instead of `default`.
    let fixture = compile_fixture("hero-bare-env", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(
        &fixture,
        "bare.environments.json",
        r#"{"schema_version":1,"environments":{"baseline":{"scope":"component:webapp"}}}"#,
    );
    let out = fixture.out("bare_env_out");
    let out_arg = out.to_string_lossy().into_owned();
    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--manifest",
        &manifest,
        "--environment",
        "baseline",
        "--out",
        &out_arg,
    ]);
    assert_eq!(code, EXIT_OK, "resolve must succeed: {err}");
    assert!(
        out.join("resolve_result.webapp.baseline.json").is_file(),
        "a choice-free environment must label the snapshot with its name; --out held {:?}",
        tree(&out)
    );
}

/// The four cells example 05's manifest produces across both service scopes,
/// in the sorted `(environment, scope)` order ADR-0059 M1 fixes.
const FLEET_CELL_DIRS: [&str; 4] = [
    "local/telemetry_service",
    "local/vision_service",
    "robot-alpha/telemetry_service",
    "robot-alpha/vision_service",
];
const FLEET_CELL_SNAPSHOTS: [&str; 4] = [
    "local/telemetry_service/resolve_result.telemetry_service.local-verbose.json",
    "local/vision_service/resolve_result.vision_service.local-verbose.json",
    "robot-alpha/telemetry_service/resolve_result.telemetry_service.robot-quiet.json",
    "robot-alpha/vision_service/resolve_result.vision_service.robot-quiet.json",
];
const FLEET_SCOPES: &str = "component:vision_service,component:telemetry_service";

fn fleet_matrix_args<'a>(model: &'a str, manifest: &'a str, out: &'a str) -> Vec<&'a str> {
    vec![
        "cfx", "resolve", "--model", model, "--manifest", manifest, "--all", "--scopes",
        FLEET_SCOPES, "--out", out,
    ]
}

#[test]
fn manifest_matrix_layout_and_order() {
    // T2 + T8 (ADR-0059 D2/M1): every environment x every --scopes entry, laid
    // out under <out>/<environment>/<root>/ exactly as the reference script's
    // --matrix does, processed in FULLY sorted (environment, scope) order —
    // note `--scopes` is passed vision-first and must still come out sorted,
    // because cell order is product output and must be a function of manifest
    // content, not of argument order.
    let fixture = compile_fixture("fleet-matrix", FLEET_DEFS, FLEET_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", FLEET_MANIFEST);
    let out = fixture.out("matrix_out");
    let out_arg = out.to_string_lossy().into_owned();

    let (code, stdout, err) = run_args(&fleet_matrix_args(&fixture.manifest, &manifest, &out_arg));
    assert_eq!(code, EXIT_OK, "matrix resolve must succeed, stderr: {err}");

    assert_eq!(
        cell_lines(&stdout),
        vec![
            "cell: local component:telemetry_service",
            "cell: local component:vision_service",
            "cell: robot-alpha component:telemetry_service",
            "cell: robot-alpha component:vision_service",
        ],
        "cells must be announced in sorted (environment, scope) order; stdout:\n{stdout}"
    );

    let mut expected: Vec<String> = Vec::new();
    for dir in FLEET_CELL_DIRS {
        expected.push(format!("{dir}/generated/config.hpp"));
        expected.push(format!("{dir}/generated/config_artifact_manifest.json"));
        expected.push(format!("{dir}/generated/config_build_flags.cmake"));
    }
    for snapshot in FLEET_CELL_SNAPSHOTS {
        expected.push(snapshot.to_string());
    }
    expected.sort();
    assert_eq!(
        tree(&out),
        expected,
        "each cell must hold its snapshot plus the export profile's generated/ files"
    );

    // The `wrote:` lineage is relative to --out, so a reader can copy a path
    // straight out of stdout.
    assert!(
        stdout.contains("wrote: robot-alpha/vision_service/generated/config.hpp"),
        "wrote: paths must be relative to --out; stdout:\n{stdout}"
    );

    // T8 — determinism: a second run reproduces the tree and the stdout.
    let again = fixture.out("matrix_again");
    let again_arg = again.to_string_lossy().into_owned();
    let (again_code, again_stdout, again_err) =
        run_args(&fleet_matrix_args(&fixture.manifest, &manifest, &again_arg));
    assert_eq!(again_code, EXIT_OK, "second matrix resolve must succeed: {again_err}");
    assert_eq!(again_stdout, stdout, "two matrix runs must print identical stdout");
    for rel in tree(&out) {
        assert_eq!(
            std::fs::read(out.join(&rel)).expect("read first run"),
            std::fs::read(again.join(&rel)).expect("read second run"),
            "{rel} must be byte-identical across two matrix runs"
        );
    }
}

#[test]
fn manifest_matrix_json_lines() {
    // T3 (ADR-0059 D2): `--format json` is JSON Lines — one existing
    // ResolveResult per cell, in the same sorted cell order, and NOTHING else
    // on stdout. Each line is compared against that cell's snapshot file, which
    // pins both the order and the "no new JSON shape" rule at once.
    let fixture = compile_fixture("fleet-json", FLEET_DEFS, FLEET_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", FLEET_MANIFEST);
    let out = fixture.out("json_matrix_out");
    let out_arg = out.to_string_lossy().into_owned();

    let mut args = fleet_matrix_args(&fixture.manifest, &manifest, &out_arg);
    args.push("--format");
    args.push("json");
    let (code, stdout, err) = run_args(&args);
    assert_eq!(code, EXIT_OK, "matrix resolve must succeed, stderr: {err}");

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "one JSON line per cell; got:\n{stdout}");
    for (index, line) in lines.iter().enumerate() {
        let parsed: serde_json::Value =
            serde_json::from_str(line).expect("each line must parse as a ResolveResult");
        assert_eq!(parsed["status"], "ok", "cell {index} must report status ok");
        let snapshot = std::fs::read(out.join(FLEET_CELL_SNAPSHOTS[index]))
            .expect("cell snapshot must exist");
        assert_eq!(
            format!("{line}\n").into_bytes(),
            snapshot,
            "line {index} must be the bytes of {}",
            FLEET_CELL_SNAPSHOTS[index]
        );
    }
}

#[test]
fn manifest_unsat_cell_exits_3_but_other_cells_land() {
    // T4 (ADR-0059 D2 exit contract): every cell is attempted. A cell rejected
    // as unsatisfiable writes NOTHING, is named on stderr with its diagnostic,
    // and turns the command's exit into 3 — but the cells that resolved keep
    // their files. `broken` sorts FIRST, so this also proves the loop does not
    // stop at the first rejection.
    let fixture = compile_fixture("hero-unsat-cell", HERO_DEFS, HERO_COMPONENTS);
    let mut manifest_json: serde_json::Value =
        serde_json::from_str(HERO_MANIFEST).expect("hero manifest parses");
    manifest_json["environments"]["broken"] = serde_json::json!({
        "scope": "component:webapp",
        "context_tags": {},
        "choices": {"environment": "prod", "log_level": "debug"},
    });
    let manifest = write_fixture_file(
        &fixture,
        "broken.environments.json",
        &serde_json::to_string(&manifest_json).expect("serialize manifest"),
    );
    let out = fixture.out("unsat_matrix_out");
    let out_arg = out.to_string_lossy().into_owned();

    let (code, stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--manifest",
        &manifest,
        "--all",
        "--out",
        &out_arg,
    ]);
    assert_eq!(code, EXIT_UNSAT, "a rejected cell must exit 3; stderr: {err}");
    assert!(
        err.contains("unsatisfiable: broken component:webapp"),
        "stderr must name the failing cell: {err}"
    );
    assert!(
        err.contains("E_SELECTION_CONFLICT"),
        "stderr must carry the diagnostic code: {err}"
    );
    assert!(
        !out.join("broken").exists() || tree(&out.join("broken")).is_empty(),
        "the rejected cell must write nothing; --out held {:?}",
        tree(&out)
    );

    // The three good environments still landed, in sorted order after `broken`.
    assert_eq!(
        cell_lines(&stdout),
        vec![
            "cell: dev component:webapp",
            "cell: prod component:webapp",
            "cell: staging component:webapp",
        ],
        "every cell after the rejection must still be attempted; stdout:\n{stdout}"
    );
    for env in ["dev", "prod", "staging"] {
        assert!(
            !tree(&out.join(env).join("webapp")).is_empty(),
            "{env} must keep its files"
        );
    }
}

#[test]
fn manifest_rejects_unknown_key_and_bad_version() {
    // T5 (ADR-0059 D1): the manifest schema is FROZEN. An unknown key at any
    // level, a schema_version other than 1, and an environment name that cannot
    // be a path component are each usage errors naming the offender and the
    // file — the drift class ADR-0059 Context §3 documents inside a single
    // file today.
    let fixture = compile_fixture("hero-bad-manifest", HERO_DEFS, HERO_COMPONENTS);
    let out_arg = fixture.out("bad_out").to_string_lossy().into_owned();

    let cases: [(&str, &str, &str); 4] = [
        (
            "unknown_env_key.json",
            r#"{"schema_version":1,"environments":{"prod":{"scope":"all","selection":{}}}}"#,
            "selection",
        ),
        (
            "unknown_top_key.json",
            r#"{"schema_version":1,"environments":{},"defaults":{}}"#,
            "defaults",
        ),
        (
            "bad_version.json",
            r#"{"schema_version":3,"environments":{"prod":{"scope":"all"}}}"#,
            "schema_version",
        ),
        (
            "bad_name.json",
            r#"{"schema_version":1,"environments":{"a b":{"scope":"all"}}}"#,
            "a b",
        ),
    ];

    for (name, body, offender) in cases {
        let manifest = write_fixture_file(&fixture, name, body);
        let (code, _stdout, err) = run_args(&[
            "cfx",
            "resolve",
            "--model",
            &fixture.manifest,
            "--manifest",
            &manifest,
            "--all",
            "--out",
            &out_arg,
        ]);
        assert_eq!(code, EXIT_USAGE, "{name} must be a usage error; stderr: {err}");
        assert!(
            err.contains(offender),
            "{name} stderr must name the offender '{offender}': {err}"
        );
        assert!(
            err.contains(name),
            "{name} stderr must name the manifest file: {err}"
        );
    }
}

#[test]
fn manifest_and_selection_file_are_mutually_exclusive() {
    // T6 (ADR-0059 D2): two spellings of the same three inputs cannot both be
    // authoritative, so supplying both is a usage error naming BOTH flags —
    // silently preferring one would resolve a target the user did not ask for.
    let fixture = compile_fixture("hero-both-inputs", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);
    let selection = fixture.dir.join("webapp.selection.json");
    write_scope_selection(&selection, "component:webapp");
    let selection_arg = selection.to_string_lossy().into_owned();
    let out_arg = fixture.out("both_out").to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--manifest",
        &manifest,
        "--environment",
        "prod",
        "--selection-file",
        &selection_arg,
        "--out",
        &out_arg,
    ]);
    assert_eq!(code, EXIT_USAGE, "both inputs must be refused; stderr: {err}");
    assert!(err.contains("--manifest"), "stderr must name --manifest: {err}");
    assert!(
        err.contains("--selection-file"),
        "stderr must name --selection-file: {err}"
    );
}

#[test]
fn unknown_environment_names_available_ones() {
    // T6 (ADR-0059 M5): an unknown narrowing name is a usage error that names
    // the name AND the manifest's available names, sorted — the reader can fix
    // the typo without opening the file.
    let fixture = compile_fixture("hero-unknown-env", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);
    let out_arg = fixture.out("unknown_out").to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--manifest",
        &manifest,
        "--environment",
        "prd",
        "--out",
        &out_arg,
    ]);
    assert_eq!(code, EXIT_USAGE, "an unknown environment must be a usage error");
    assert!(err.contains("prd"), "stderr must name the unknown name: {err}");
    assert!(
        err.contains("dev, prod, staging"),
        "stderr must list the available names, sorted: {err}"
    );
}

#[test]
fn manifest_narrowing_flags_are_validated() {
    // ADR-0059 M5: `--all` is the REQUIRED explicit opt-in for a fleet-wide
    // fan-out because `resolve` writes files, so `--manifest` alone is refused;
    // and `--scopes` overrides each environment's own scope, which only has a
    // meaning in matrix mode.
    let fixture = compile_fixture("hero-flag-validation", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);
    let out_arg = fixture.out("flags_out").to_string_lossy().into_owned();
    let base = [
        "cfx",
        "resolve",
        "--model",
        fixture.manifest.as_str(),
        "--manifest",
        manifest.as_str(),
        "--out",
        out_arg.as_str(),
    ];

    let (bare_code, _out, bare_err) = run_args(&base);
    assert_eq!(bare_code, EXIT_USAGE, "--manifest alone must be refused");
    assert!(
        bare_err.contains("--environment") && bare_err.contains("--all"),
        "stderr must name both narrowing spellings: {bare_err}"
    );

    let mut both = base.to_vec();
    both.extend_from_slice(&["--environment", "prod", "--all"]);
    let (both_code, _o, both_err) = run_args(&both);
    assert_eq!(both_code, EXIT_USAGE, "--environment with --all must be refused");
    assert!(
        both_err.contains("--environment") && both_err.contains("--all"),
        "stderr must name both flags: {both_err}"
    );

    let mut scoped = base.to_vec();
    scoped.extend_from_slice(&["--environment", "prod", "--scopes", "all"]);
    let (scoped_code, _o2, scoped_err) = run_args(&scoped);
    assert_eq!(
        scoped_code, EXIT_USAGE,
        "--scopes without --all must be refused"
    );
    assert!(
        scoped_err.contains("--scopes") && scoped_err.contains("--all"),
        "stderr must name --scopes and --all: {scoped_err}"
    );

    let mut orphan = vec!["cfx", "resolve", "--model", fixture.manifest.as_str()];
    orphan.extend_from_slice(&["--all", "--out", out_arg.as_str()]);
    let (orphan_code, _o3, orphan_err) = run_args(&orphan);
    assert_eq!(
        orphan_code, EXIT_USAGE,
        "--all without --manifest must be refused"
    );
    assert!(
        orphan_err.contains("--manifest"),
        "stderr must name --manifest: {orphan_err}"
    );
}

// ---------------------------------------------------------------------------
// configflux-secb.3 (ADR-0057 §D6) — T6 / T9b / T10: `cfx resolve` surfaces the
// inference, over a model whose implied facet is a declared BINDING
// ---------------------------------------------------------------------------

/// Three declared closed things: a plain facet `site`, a `line_container`
/// BINDING over the `containers` catalogue, and a `tier` facet nothing
/// constrains. Choosing `site=factory_b` forces the binding to `c2`, while
/// `tier` is left for its default — so one resolve produces BOTH an `implied:`
/// line and a `defaulted:` line and their order can be asserted.
///
/// The implied thing is deliberately a binding rather than a facet: ADR-0057 §D3
/// makes a binding one more declared closed facet, and this is what proves the
/// inference path treats it as one with no special case (configflux-secb.4).
const CFX_INFERENCE_DEFS: &str = r#"{
    "package": "secb3_cfx_inference",
    "version": "1.0.0",
    "definitions": {
        "frame_width": {
            "type": "integer",
            "doc": "Width the service crops to",
            "lifecycle": "runtime",
            "safety": "q_m",
            "access": "technician"
        }
    },
    "catalogues": {
        "containers": {
            "doc": "Containers this plant runs on the line.",
            "fields": {
                "width_mm": { "type": "integer", "unit": "mm", "doc": "Internal width" }
            },
            "entries": {
                "c1": { "width_mm": 800 },
                "c2": { "width_mm": 600 }
            }
        }
    },
    "bindings": {
        "line_container": {
            "catalogue": "containers",
            "default": "c1",
            "doc": "The container every service on this line draws from."
        }
    },
    "facets": {
        "site": { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "tier": { "values": ["t1", "t2"], "default": "t1" }
    },
    "constraints": {
        "factory_b_uses_c2": {
            "condition": "site != 'factory_b' || line_container == 'c2'",
            "doc": "Factory B only stocks c2 containers."
        }
    }
}"#;

const CFX_INFERENCE_COMPONENTS: &str = r#"{
    "package": "secb3_cfx_inference",
    "version": "1.0.0",
    "components": {
        "vision_service": {
            "type": "service",
            "params": {
                "frame_width": {
                    "inherits": "frame_width",
                    "type": "integer",
                    "unit": "mm",
                    "doc": "Width the service crops to",
                    "lifecycle": "runtime",
                    "safety": "q_m",
                    "access": "technician",
                    "value": 600,
                    "overrides": [
                        {"condition": "line_container == 'c1'", "value": 800}
                    ]
                }
            }
        }
    }
}"#;

/// T9b + T6 (text) + T10 (determinism). `--select site=factory_b` ALONE is the
/// invocation that used to exit 3: the binding defaulted to `c1`, contradicted
/// `factory_b_uses_c2`, and the resolve failed closed while `cfx options`
/// already listed `c2` as the only valid entry. It now resolves, and says so.
#[test]
fn cfx_resolve_infers_a_binding_and_prints_implied_before_defaulted() {
    let fixture = compile_fixture(
        "secb3-inference",
        CFX_INFERENCE_DEFS,
        CFX_INFERENCE_COMPONENTS,
    );
    let out_dir = fixture.dir.join("out");

    let (code, stdout, stderr) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "site=factory_b",
        "--out",
        &out_dir.to_string_lossy(),
    ]);
    assert_eq!(code, 0, "site=factory_b alone must now resolve: {stderr}");

    let implied_at = stdout
        .find("implied: line_container=c2")
        .unwrap_or_else(|| panic!("stdout must report the inferred binding:\n{stdout}"));
    let defaulted_at = stdout
        .find("defaulted: tier=t1")
        .unwrap_or_else(|| panic!("stdout must report the untouched default:\n{stdout}"));
    assert!(
        implied_at < defaulted_at,
        "`implied:` must precede `defaulted:` — precedence order, so the stronger \
         provenance is read first:\n{stdout}"
    );
    assert!(
        !stdout.contains("defaulted: line_container"),
        "an implied binding must NOT also be reported as defaulted:\n{stdout}"
    );

    // T10: byte-identical on a second run.
    let out_dir2 = fixture.dir.join("out2");
    let (code2, stdout2, _) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "site=factory_b",
        "--out",
        &out_dir2.to_string_lossy(),
    ]);
    assert_eq!(code2, 0);
    assert_eq!(
        stdout.replace("out2", "out"),
        stdout2.replace("out2", "out"),
        "inference must be deterministic"
    );
}

/// T6 (JSON): `--format json` carries `implied_choices`, and an implied facet
/// never appears in `defaulted_choices`.
#[test]
fn cfx_resolve_json_carries_implied_choices() {
    let fixture = compile_fixture(
        "secb3-inference-json",
        CFX_INFERENCE_DEFS,
        CFX_INFERENCE_COMPONENTS,
    );
    let out_dir = fixture.dir.join("out");

    let (code, stdout, stderr) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "site=factory_b",
        "--format",
        "json",
        "--out",
        &out_dir.to_string_lossy(),
    ]);
    assert_eq!(code, 0, "resolve must succeed: {stderr}");

    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("--format json emits one JSON document");
    assert_eq!(
        value["implied_choices"]["line_container"],
        serde_json::Value::from("c2"),
        "the JSON envelope must carry the inference: {stdout}"
    );
    assert!(
        value["defaulted_choices"].get("line_container").is_none(),
        "an implied binding must not also be reported as defaulted: {stdout}"
    );
    assert_eq!(
        value["defaulted_choices"]["tier"],
        serde_json::Value::from("t1"),
        "a facet no rule touches still takes its declared default: {stdout}"
    );
}

/// configflux-egyj, as ADR-0063 leaves it. A model whose `log_level` domain a
/// user can be refused against, selected with a pair carrying a space.
///
/// The space used to come from the MODEL: `values` was a plain list of strings
/// on the Rust ingest path, nothing screened the characters, and this fixture
/// declared `de bug` as a legal option id. ADR-0063 D2 ends that — a declared
/// facet value is now a file-safe token, refused at ingest — so the fixture
/// declares `debug` and the metacharacter arrives where it still can: from the
/// USER, on the command line. That is deliberate rather than a leftover. A
/// `--select` pair is a RUNTIME input, and ADR-0063's non-goals keep runtime
/// inputs (scope selectors, context tags, and these pairs) unconstrained —
/// so `log_level=de bug` is still a line a real user can type and still be
/// refused for, which is exactly the line this test says must be pasteable.
const CFX_METACHAR_DEFS: &str = r#"{
    "package": "egyj_metachar_option",
    "version": "1.0.0",
    "definitions": {
        "log_target": {
            "type": "string",
            "lifecycle": "startup",
            "access": "integrator",
            "doc": "Where the service writes its log"
        }
    },
    "facets": {
        "environment": { "values": ["dev", "prod"], "default": "dev" },
        "log_level": { "values": ["info", "debug"], "default": "info" }
    },
    "constraints": {
        "prod_forbids_debug": {
            "condition": "environment != 'prod' || log_level != 'debug'",
            "doc": "Debug logging is not permitted in production."
        }
    }
}"#;

const CFX_METACHAR_COMPONENTS: &str = r#"{
    "package": "egyj_metachar_option",
    "version": "1.0.0",
    "components": {
        "webapp": {
            "type": "service",
            "params": {
                "log_target": {
                    "inherits": "log_target",
                    "type": "string",
                    "lifecycle": "startup",
                    "access": "integrator",
                    "doc": "Where the service writes its log",
                    "value": "stdout",
                    "overrides": [
                        {"condition": "environment == 'prod'", "value": "journal"}
                    ]
                }
            }
        }
    }
}"#;

/// configflux-egyj, end to end through the real pipeline rather than through
/// `explain_hint` alone: a rejection a user earns, and the guidance line that
/// rejection prints.
///
/// Unquoted, that line ends `--select log_level=de bug`, which a shell hands
/// `cfx explain` as two arguments — a malformed pair and a positional the verb
/// does not take. The unit cases above pin the encoding; this one pins that the
/// encoding is reached from the surface a user actually stands on, which is the
/// gap that let the pairs stay raw while the paths were fixed.
///
/// The exit code is asserted, not the reason text: what the pair is refused FOR
/// is not this test's claim, and ADR-0063 moved it — the value is no longer in
/// `log_level`'s declared domain, where it used to be a declared option the
/// `prod_forbids_debug` constraint forbade. Either way the user is refused and
/// handed a command to run, and the command has to survive a shell.
#[test]
fn cfx_resolve_quotes_a_pair_the_user_made_unpasteable() {
    let fixture = compile_fixture(
        "egyj-metachar",
        CFX_METACHAR_DEFS,
        CFX_METACHAR_COMPONENTS,
    );
    // The immutable half of the selection. A context tag is a RUNTIME input —
    // whatever the deployment's CI puts there — so its id and value are plain
    // strings, and a `--select` that contradicts one is refused as a conflict
    // rather than as a usage mistake. That is the shape that still reaches the
    // guidance line with a metacharacter in the pair now that a MODEL cannot
    // declare one (ADR-0063 D2).
    let selection = write_fixture_file(
        &fixture,
        "pinned.selection.json",
        r#"{"schema_version":5,"model_hash":"","scope":"all",
"context_tags":{"log level":"qu iet"},"choices":{},"selection_state_hash":""}"#,
    );
    let out_dir = fixture.out("out");

    let (code, _stdout, stderr) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &selection,
        "--select",
        "environment=prod",
        "--select",
        "log level=de bug",
        "--out",
        &out_dir.to_string_lossy(),
    ]);

    assert_eq!(
        code, EXIT_UNSAT,
        "the pinned tag must refuse this selection: {stderr}"
    );
    assert!(
        stderr.contains("--select 'log level=de bug'"),
        "the guidance line must hand the pair over as ONE shell word: {stderr}"
    );
}

/// configflux-ineg. The most common selection typo — a value that is not in
/// the facet's declared domain — was the one refusal that handed the user
/// nowhere to go next: it exits 2 with the reason line alone, while the RARER
/// unsatisfiable case exits 3 and prints a pasteable command. This pins the
/// pointer that closes that gap, and pins that closing it did not move the case
/// into the unsat class: a value that does not exist is a usage mistake, not a
/// model that cannot be satisfied, so the code stays 2.
///
/// The command is `cfx options --model <m>`, NOT `... --facet <f>`: `OptionsArgs`
/// declares no `--facet`, so the flag form would print a line clap answers with
/// `unexpected argument '--facet' found` — the one thing a guidance line must
/// never do, since its whole purpose is that the printed command is the command
/// that runs. The listing covers every facet, the named one included, so the
/// facet is carried in the verdict half instead; the command stays LAST so
/// everything after `run: ` can be pasted whole.
///
/// The fixture directory holds a space on purpose. The model path is the only
/// term of this line a user can still make unpasteable — ADR-0063 D2 keeps a
/// metacharacter out of a DECLARED facet id, so the facet half cannot supply
/// the case — and unquoted, a path with a space reaches `cfx options` as two
/// arguments and a `--model` that names a file which does not exist.
#[test]
fn cfx_resolve_points_an_invalid_option_at_the_options_listing() {
    let fixture = compile_fixture("ineg pointer", HERO_DEFS, HERO_COMPONENTS);
    assert!(
        fixture.manifest.contains(' '),
        "the fixture must carry the space this case exists to quote: {}",
        fixture.manifest
    );
    let out_dir = fixture.out("out");

    // `de bug` is not in `log_level`'s declared domain (`["info", "debug"]`),
    // so the facet IS declared and the value is not — the invalid-option path,
    // not the unknown-facet one.
    let (code, _stdout, stderr) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "log_level=de bug",
        "--out",
        &out_dir.to_string_lossy(),
    ]);

    assert_eq!(
        code, EXIT_USAGE,
        "an option that does not exist stays a usage error: {stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "facet 'log_level' has no such option; run: cfx options --model '{}'",
            fixture.manifest
        )),
        "the refusal must hand the user the listing command, quoted: {stderr}"
    );
}
