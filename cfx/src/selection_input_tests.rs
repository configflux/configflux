// SPDX-License-Identifier: BUSL-1.1
//
// The same-level double-binding rule (ADR-0057 §D8), driven through the REAL
// `cfx` argument surface over REAL models compiled in-process.
//
// What is under test is a command's exit code and the line it prints, so the
// tests drive `run_args` rather than the two predicates directly — a rule that
// holds in `selection_input.rs` and never reaches a verb would be no rule at
// all. They reuse `tests.rs`'s harness (`run_args`, `compile_fixture`,
// `write_fixture_file`) instead of carrying a second copy of it.
//
// A separate module from `selection_input.rs` because the implementation plus
// these cases would put that file over the 400-line default; same arrangement
// as `diff_tests.rs` beside `tests.rs`.

use compiler::product_api::PRODUCT_SCHEMA_VERSION;

use crate::pipeline::{parse_select_pair, SelectPair};
use crate::selection_input::{first_duplicate, reject_conflicting_selects};
use crate::tests::{
    compile_fixture, run_args, tree, write_fixture_file, ModelFixture, HERO_COMPONENTS, HERO_DEFS,
};
use crate::{EXIT_OK, EXIT_USAGE};

/// A selection document with a literal `choices`/`context_tags` body, so a
/// test can write a duplicate key that no `BTreeMap` could have produced.
fn selection_doc(scope: &str, context_tags: &str, choices: &str) -> String {
    format!(
        r#"{{"schema_version":{PRODUCT_SCHEMA_VERSION},"model_hash":"","scope":"{scope}",
"context_tags":{context_tags},"choices":{choices},"selection_state_hash":""}}"#
    )
}

/// The hero model plus a selection file written into its fixture dir.
fn hero_with_selection(label: &str, file: &str, body: &str) -> (ModelFixture, String) {
    let fixture = compile_fixture(label, HERO_DEFS, HERO_COMPONENTS);
    let path = write_fixture_file(&fixture, file, body);
    (fixture, path)
}

const HERO_SCOPE: &str = "component:webapp";

#[test]
fn select_twice_different_options_is_usage_error() {
    // T1 (ADR-0057 §D8): the same-level double binding the interpreter's
    // `select` verb already refuses. Before this rule the BTreeMap fold in
    // `cell_inputs` silently kept `staging` and exited 0, so `cfx` and the
    // interpreter disagreed about whether the command was even meaningful.
    let fixture = compile_fixture("dup-select", HERO_DEFS, HERO_COMPONENTS);
    let out = fixture.out("dup_select_out");
    let out_arg = out.to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "environment=dev",
        "--select",
        "environment=staging",
        "--out",
        &out_arg,
    ]);

    assert_eq!(code, EXIT_USAGE, "a double binding must exit 2; stderr: {err}");
    assert!(
        err.contains(
            "facet 'environment' selected twice with different options: 'dev' and 'staging'"
        ),
        "stderr must name the facet and BOTH options: {err}"
    );
    assert!(
        !out.exists(),
        "a rejected command must write nothing; --out held {:?}",
        tree(&out)
    );
}

#[test]
fn select_twice_same_option_is_idempotent() {
    // T2: repeating one pair says one thing twice. Asserting byte equality
    // with the single-flag run (not just exit 0) is what proves the second
    // flag changed nothing at all, including the snapshot's name.
    let fixture = compile_fixture("same-select", HERO_DEFS, HERO_COMPONENTS);
    let once_out = fixture.out("once_out").to_string_lossy().into_owned();
    let twice_out = fixture.out("twice_out").to_string_lossy().into_owned();

    let (once_code, once_stdout, once_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "environment=prod",
        "--out",
        &once_out,
    ]);
    let (twice_code, twice_stdout, twice_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--select",
        "environment=prod",
        "--select",
        "environment=prod",
        "--out",
        &twice_out,
    ]);

    assert_eq!(once_code, EXIT_OK, "control run failed: {once_err}");
    assert_eq!(
        twice_code, EXIT_OK,
        "a repeated identical pair must be accepted; stderr: {twice_err}"
    );
    assert_eq!(
        once_stdout, twice_stdout,
        "the repeated pair must resolve identically"
    );
}

#[test]
fn selection_file_duplicate_choice_key_is_usage_error() {
    // T3: serde_json keeps the LAST duplicate key, so before this check the
    // file below resolved as `staging` and exited 0. The message names the
    // key and the object it sits in, because a hand-written selection file
    // with two `environment` lines gives the reader nothing else to go on.
    let (_fixture, path) = hero_with_selection(
        "dup-choice-key",
        "dup.selection.json",
        &selection_doc(
            HERO_SCOPE,
            "{}",
            r#"{"environment":"dev","environment":"staging"}"#,
        ),
    );

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "options",
        "--model",
        &_fixture.manifest,
        "--selection-file",
        &path,
    ]);

    assert_eq!(code, EXIT_USAGE, "a duplicate key must exit 2; stderr: {err}");
    assert!(
        err.contains(&format!(
            "malformed --selection-file '{path}': duplicate key 'environment' in choices"
        )),
        "stderr must name the file, the key and the object: {err}"
    );
}

#[test]
fn selection_file_duplicate_context_tag_key_is_usage_error() {
    // T4: same rule, other object. `context_tags` is the immutable half of
    // a selection, so a silently-collapsed duplicate there is worse than in
    // `choices` — nothing downstream can override it back.
    let (_fixture, path) = hero_with_selection(
        "dup-tag-key",
        "dup_tags.selection.json",
        &selection_doc(HERO_SCOPE, r#"{"site":"eu","site":"us"}"#, "{}"),
    );

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "options",
        "--model",
        &_fixture.manifest,
        "--selection-file",
        &path,
    ]);

    assert_eq!(code, EXIT_USAGE, "a duplicate tag key must exit 2; stderr: {err}");
    assert!(
        err.contains(&format!(
            "malformed --selection-file '{path}': duplicate key 'site' in context_tags"
        )),
        "stderr must name context_tags: {err}"
    );
}

#[test]
fn select_flag_still_overrides_selection_file() {
    // T5 (ADR-0042 §2, pinned UNCHANGED by ADR-0057 §D8): a flag and the
    // file are different LEVELS, so layering them is an override, not a
    // double binding. Asserted as byte equality against the file that pins
    // `prod` directly — exit 0 alone would not show WHICH value won.
    let fixture = compile_fixture("flag-overrides-file", HERO_DEFS, HERO_COMPONENTS);
    let dev_file = write_fixture_file(
        &fixture,
        "dev.selection.json",
        &selection_doc(HERO_SCOPE, "{}", r#"{"environment":"dev"}"#),
    );
    let prod_file = write_fixture_file(
        &fixture,
        "prod.selection.json",
        &selection_doc(HERO_SCOPE, "{}", r#"{"environment":"prod"}"#),
    );
    let overridden_out = fixture.out("overridden_out").to_string_lossy().into_owned();
    let direct_out = fixture.out("direct_out").to_string_lossy().into_owned();

    let (overridden_code, overridden_stdout, overridden_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &dev_file,
        "--select",
        "environment=prod",
        "--out",
        &overridden_out,
    ]);
    let (direct_code, direct_stdout, direct_err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &prod_file,
        "--out",
        &direct_out,
    ]);

    assert_eq!(
        overridden_code, EXIT_OK,
        "a flag overriding the file must stay legal; stderr: {overridden_err}"
    );
    assert_eq!(direct_code, EXIT_OK, "control run failed: {direct_err}");
    assert_eq!(
        overridden_stdout, direct_stdout,
        "the flag must win over the file's choice for the same facet"
    );
}

#[test]
fn explain_rejects_a_duplicate_key_selection_file() {
    // T6: all three verbs read the file through ONE loader, so the rule
    // cannot hold for `resolve` and lapse for `explain`. Driving the verb
    // (rather than asserting the loader is shared) is what would catch a
    // future verb that opened the file itself.
    let (_fixture, path) = hero_with_selection(
        "dup-key-explain",
        "dup_explain.selection.json",
        &selection_doc(
            HERO_SCOPE,
            "{}",
            r#"{"log_level":"info","log_level":"debug"}"#,
        ),
    );

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "explain",
        "--model",
        &_fixture.manifest,
        "--selection-file",
        &path,
    ]);

    assert_eq!(code, EXIT_USAGE, "explain must exit 2 as well; stderr: {err}");
    assert!(
        err.contains("duplicate key 'log_level' in choices"),
        "explain must give the same message: {err}"
    );
}

#[test]
fn resolve_rejects_a_duplicate_key_selection_file() {
    // T6, third verb.
    let (fixture, path) = hero_with_selection(
        "dup-key-resolve",
        "dup_resolve.selection.json",
        &selection_doc(
            HERO_SCOPE,
            "{}",
            r#"{"log_level":"info","log_level":"debug"}"#,
        ),
    );
    let out = fixture.out("dup_resolve_out").to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &path,
        "--out",
        &out,
    ]);

    assert_eq!(code, EXIT_USAGE, "resolve must exit 2; stderr: {err}");
    assert!(
        err.contains("duplicate key 'log_level' in choices"),
        "resolve must give the same message: {err}"
    );
}

#[test]
fn a_well_formed_selection_file_is_unaffected() {
    // The check must be invisible to every file that binds each key once —
    // including one that binds the SAME facet as another object's key,
    // which is not a duplicate.
    let (fixture, path) = hero_with_selection(
        "clean-selection",
        "clean.selection.json",
        &selection_doc(
            HERO_SCOPE,
            "{}",
            r#"{"environment":"prod","log_level":"info"}"#,
        ),
    );
    let out = fixture.out("clean_out").to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &path,
        "--out",
        &out,
    ]);

    assert_eq!(code, EXIT_OK, "a clean file must still resolve; stderr: {err}");
}

#[test]
fn a_malformed_file_keeps_its_own_message() {
    // The duplicate scan must not swallow the parse error of a document it
    // cannot read: a truncated file is still "malformed ... (<serde err>)",
    // not a silent success and not a duplicate-key claim.
    let (fixture, path) = hero_with_selection("broken-json", "broken.selection.json", "{ nope");
    let out = fixture.out("broken_out").to_string_lossy().into_owned();

    let (code, _stdout, err) = run_args(&[
        "cfx",
        "resolve",
        "--model",
        &fixture.manifest,
        "--selection-file",
        &path,
        "--out",
        &out,
    ]);

    assert_eq!(code, EXIT_USAGE, "a broken file must exit 2; stderr: {err}");
    assert!(
        err.contains("malformed --selection-file") && !err.contains("duplicate key"),
        "a parse failure must keep its own message: {err}"
    );
}

#[test]
fn conflicting_selects_report_the_first_conflict_only() {
    // The pure rule, without model I/O: argument order decides, and a
    // repeated identical pair never masks a later real conflict.
    let pairs: Vec<SelectPair> = ["a=1", "b=1", "b=1", "a=2", "b=2"]
        .iter()
        .map(|raw| parse_select_pair(raw).expect("valid pair"))
        .collect();
    let err = reject_conflicting_selects(&pairs).expect_err("a=1 vs a=2 must be refused");
    assert_eq!(err.exit_code, EXIT_USAGE);
    assert_eq!(
        err.message,
        "facet 'a' selected twice with different options: '1' and '2'"
    );
}

#[test]
fn first_duplicate_finds_the_earliest_repeat() {
    let keys: Vec<String> = ["z", "a", "b", "a", "b"]
        .iter()
        .map(|k| (*k).to_string())
        .collect();
    assert_eq!(first_duplicate(&keys), Some("a"));
    assert_eq!(first_duplicate(&keys[..3]), None);
}
